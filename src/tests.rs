use crate::{
    discovery,
    model::{Catalog, Entry, Script},
    runtime::{Runtime, State},
    service::{Filter, Service},
    store::Store,
};
use std::{
    fs,
    path::PathBuf,
    time::{Duration, Instant},
};
// Limit concurrent integration cases, not concurrent processes within each case.
// Login-shell and process-inspection tests otherwise compete with dozens of sibling fixtures.
type FixtureCases = std::collections::HashMap<std::thread::ThreadId, usize>;
type FixtureGate = (std::sync::Mutex<FixtureCases>, std::sync::Condvar);
fn fixture_gate() -> &'static FixtureGate {
    static GATE: std::sync::OnceLock<FixtureGate> = std::sync::OnceLock::new();
    GATE.get_or_init(|| (Default::default(), Default::default()))
}
fn enter_fixture() -> std::thread::ThreadId {
    let id = std::thread::current().id();
    let (lock, wake) = fixture_gate();
    let mut cases = lock.lock().expect("fixture gate");
    while !cases.contains_key(&id) && cases.len() >= 4 {
        cases = wake.wait(cases).expect("fixture gate");
    }
    *cases.entry(id).or_default() += 1;
    id
}
struct Fixture(PathBuf, std::thread::ThreadId);
impl Fixture {
    fn new() -> Self {
        let p = std::env::temp_dir().join(format!(
            "dev-start-test-{}-{}",
            std::process::id(),
            crate::model::unique_id()
        ));
        fs::create_dir_all(p.join("logs")).unwrap();
        Self(p, enter_fixture())
    }
    fn script(&self, text: &str) -> Script {
        let file = self.0.join("dev.sh");
        fs::write(&file, text).unwrap();
        Script {
            display_name: None,
            enabled: true,
            display_command: None,
            id: 10,
            name: "dev.sh".into(),
            file,
            cwd: self.0.clone(),
            entry: Entry::File {
                interpreter: "/bin/bash".into(),
            },
            description: String::new(),
            favorite: false,
            environment: Default::default(),
            expected_port: None,
            args: vec![],
            custom: false,
            resource_key: None,
        }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
        let (lock, wake) = fixture_gate();
        let mut cases = lock.lock().expect("fixture gate");
        let depth = cases.get_mut(&self.1).expect("fixture registered");
        *depth -= 1;
        if *depth == 0 {
            cases.remove(&self.1);
        }
        wake.notify_all();
    }
}
fn wait(runtime: &mut Runtime, predicate: impl Fn(&Runtime) -> bool) {
    let start = Instant::now();
    loop {
        runtime.poll();
        if predicate(runtime) {
            return;
        }
        assert!(
            start.elapsed() < Duration::from_secs(12),
            "runtime timed out: {:?}",
            runtime.runs.first().map(|r| r.state)
        );
        std::thread::sleep(Duration::from_millis(30));
    }
}
#[test]
fn discovery_reads_without_execution_and_skips_dependencies() {
    let f = Fixture::new();
    f.script("#!/bin/bash\n# Start local API\ntouch should-not-exist\n");
    fs::write(
        f.0.join("package.json"),
        r#"{"scripts":{"dev":"vite","test":"vitest"}}"#,
    )
    .unwrap();
    fs::create_dir(f.0.join("node_modules")).unwrap();
    fs::write(f.0.join("node_modules/bad.sh"), "exit 1").unwrap();
    let scripts = discovery::discover(&f.0).unwrap();
    assert_eq!(scripts.len(), 2);
    assert!(!f.0.join("should-not-exist").exists());
    assert!(scripts.iter().any(|s| s.description == "Start local API"));
}
#[test]
fn catalog_rescan_preserves_identity_favorites_and_description() {
    let f = Fixture::new();
    f.script("# server\necho ready");
    let store = Store::at(f.0.clone()).unwrap();
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store,
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    let pid = service.import(&f.0, 1).unwrap();
    let sid = service.catalog.projects[0].scripts[0].id;
    service.script_favorite(sid).unwrap();
    service.describe(sid, "custom description".into()).unwrap();
    service.rescan(pid).unwrap();
    assert_eq!(
        service.script(sid).unwrap().description,
        "custom description"
    );
    assert!(service.script(sid).unwrap().favorite);
    assert_eq!(service.visible_projects(Filter::Favorites, "").len(), 1);
    let reloaded = service.store.load().unwrap();
    assert!(reloaded.projects[0].scripts[0].favorite);
}
#[test]
fn runtime_captures_stdout_stderr_and_nonzero_exit() {
    let f = Fixture::new();
    let script = f.script("printf 'hello stdout\\n'; printf 'real error\\n' >&2; exit 7");
    let mut rt = Runtime::default();
    rt.start("Test", &script, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0].state == State::Failed
            && r.runs[0]
                .lines
                .iter()
                .any(|(_, s)| s.contains("real error"))
    });
    assert_eq!(rt.runs[0].exit, Some(7));
    assert!(
        rt.runs[0]
            .lines
            .iter()
            .any(|(e, s)| !e && s.contains("hello stdout"))
    );
    assert!(
        fs::read_to_string(&rt.runs[0].log_path)
            .unwrap()
            .contains("real error")
    );
}
#[test]
fn stop_terminates_descendants_and_rejects_duplicate_start() {
    let f = Fixture::new();
    let script = f.script("sleep 60 &\necho child=$!\nwait\n");
    let mut rt = Runtime::default();
    let id = rt.start("Test", &script, f.0.join("logs")).unwrap();
    assert!(rt.start("Test", &script, f.0.join("logs")).is_err());
    wait(&mut rt, |r| {
        r.runs[0].lines.iter().any(|(_, s)| s.contains("child="))
    });
    let child = rt.runs[0]
        .lines
        .iter()
        .find_map(|(_, s)| {
            s.trim()
                .strip_prefix("child=")
                .and_then(|p| p.parse::<i32>().ok())
        })
        .unwrap();
    rt.stop(id);
    wait(&mut rt, |r| r.runs[0].state == State::Stopped);
    assert_ne!(unsafe { libc::kill(child, 0) }, 0);
}
#[test]
fn parent_exit_does_not_hide_background_services() {
    let f = Fixture::new();
    let script = f.script("sleep 60 &\necho background=$!\nexit 0\n");
    let mut rt = Runtime::default();
    let id = rt.start("Test", &script, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0]
            .lines
            .iter()
            .any(|(_, s)| s.contains("background="))
    });
    std::thread::sleep(Duration::from_millis(200));
    rt.poll();
    assert_eq!(rt.runs[0].state, State::Running);
    rt.stop(id);
    wait(&mut rt, |r| r.runs[0].state == State::Stopped);
}
#[test]
fn malformed_catalog_is_not_overwritten() {
    let f = Fixture::new();
    fs::write(f.0.join("catalog.json"), "broken json").unwrap();
    let store = Store::at(f.0.clone()).unwrap();
    assert!(store.load().is_err());
    assert_eq!(
        fs::read_to_string(f.0.join("catalog.json")).unwrap(),
        "broken json"
    );
}
#[test]
fn streaming_console_handles_utf8_and_ansi_across_chunks() {
    let mut d = crate::output::Decoder::default();
    assert_eq!(d.push(&[0xe5]), "");
    assert_eq!(d.push(&[0x90, 0xaf]), "启");
    assert_eq!(d.push(b"\x1b[3"), "");
    assert_eq!(d.push(b"1merror\x1b[0m\r\n"), "error\n");
    assert_eq!(d.finish(), "");
}
#[test]
fn store_blocks_competing_writers() {
    let f = Fixture::new();
    let _store = Store::at(f.0.clone()).unwrap();
    assert!(Store::at(f.0.clone()).is_err());
}
#[test]
fn closing_runtime_cleans_its_active_process_group() {
    let f = Fixture::new();
    let script = f.script("sleep 60 &\necho child=$!\nwait\n");
    let mut rt = Runtime::default();
    rt.start("Test", &script, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0].lines.iter().any(|(_, s)| s.contains("child="))
    });
    let child = rt.runs[0]
        .lines
        .iter()
        .find_map(|(_, s)| {
            s.trim()
                .strip_prefix("child=")
                .and_then(|p| p.parse::<i32>().ok())
        })
        .unwrap();
    drop(rt);
    assert_ne!(unsafe { libc::kill(child, 0) }, 0);
}

#[test]
fn different_projects_run_concurrently_with_isolated_output_and_stop() {
    let first = Fixture::new();
    let second = Fixture::new();
    let a = first.script("echo project-alpha\nsleep 60 &\nwait\n");
    let mut b = second.script("echo project-beta\nsleep 60 &\nwait\n");
    b.id = 11;
    let mut rt = Runtime::default();
    let id_a = rt.start("Alpha", &a, first.0.join("logs")).unwrap();
    let id_b = rt.start("Beta", &b, second.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs.len() == 2
            && r.runs[0]
                .lines
                .iter()
                .any(|(_, s)| s.contains("project-alpha"))
            && r.runs[1]
                .lines
                .iter()
                .any(|(_, s)| s.contains("project-beta"))
    });
    assert_eq!(rt.runs[0].state, State::Running);
    assert_eq!(rt.runs[1].state, State::Running);
    assert!(
        !rt.runs[0]
            .lines
            .iter()
            .any(|(_, s)| s.contains("project-beta"))
    );
    rt.stop(id_a);
    wait(&mut rt, |r| r.runs[0].state == State::Stopped);
    assert_eq!(rt.runs[1].state, State::Running);
    rt.stop(id_b);
    wait(&mut rt, |r| r.runs[1].state == State::Stopped);
}
#[test]
fn console_tiles_stay_in_bounds_without_overlap() {
    use eframe::egui::{Rect, pos2, vec2};
    let area = Rect::from_min_size(pos2(-1920.0, 28.0), vec2(1920.0, 1000.0));
    assert!(crate::windows::tiles(area, 0).is_empty());
    for count in 1..=64 {
        let tiles = crate::windows::tiles(area, count);
        assert_eq!(tiles.len(), count);
        for (i, rect) in tiles.iter().enumerate() {
            assert!(area.contains_rect(*rect));
            assert!(rect.width() > 0.0 && rect.height() > 0.0);
            for other in tiles.iter().skip(i + 1) {
                assert!(!rect.intersects(*other));
            }
        }
    }
}
#[test]
fn console_viewports_have_stable_distinct_identity() {
    assert_eq!(
        crate::windows::viewport_id(7),
        crate::windows::viewport_id(7)
    );
    assert_ne!(
        crate::windows::viewport_id(7),
        crate::windows::viewport_id(8)
    );
    assert_ne!(
        crate::windows::viewport_id(7),
        eframe::egui::ViewportId::ROOT
    );
}

#[test]
fn explicit_profile_import_is_idempotent_and_rescan_never_adds_release_commands() {
    let f = Fixture::new();
    let package = f.0.join("package.json");
    fs::write(
        &package,
        r#"{"scripts":{"dev":"echo development","build:upload":"touch published"}}"#,
    )
    .unwrap();
    let profile = f.0.join("profile.json");
    fs::write(&profile,serde_json::json!({"group":"Example","projects":[{"name":"Frontend","path":f.0,"scripts":[{"name":"Development","file":"package.json","entry":{"Package":{"manager":"npm","name":"dev"}},"description":"development only"}]}]}).to_string()).unwrap();
    let mut s = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
        store: Store::at(f.0.clone()).unwrap(),
    };
    s.import_profile(&profile).unwrap();
    let id = s.catalog.projects[0].id;
    let script = s.catalog.projects[0].scripts[0].id;
    s.import_profile(&profile).unwrap();
    s.rescan(id).unwrap();
    assert_eq!(s.catalog.projects.len(), 1);
    assert_eq!(s.catalog.projects[0].scripts.len(), 1);
    assert_eq!(s.catalog.projects[0].scripts[0].id, script);
    assert!(!f.0.join("published").exists());
}
#[test]
fn occupied_endpoint_refuses_launch_without_side_effects() {
    let f = Fixture::new();
    let mut s = f.script("touch should-not-run");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    s.expected_port = Some(listener.local_addr().unwrap().port());
    let mut rt = Runtime::default();
    let error = rt.start("Test", &s, f.0.join("logs")).unwrap_err();
    assert!(error.contains("已有服务"));
    assert!(rt.runs.is_empty());
    assert!(!f.0.join("should-not-run").exists());
}
#[test]
fn environment_values_are_forwarded_literally_after_login_shell() {
    let f = Fixture::new();
    let mut script = f.script("printf '%s\\n' \"$DEV_START_EXAMPLE\"");
    script.environment.insert(
        "DEV_START_EXAMPLE".into(),
        "$(touch malicious-marker) literal value".into(),
    );
    let mut rt = Runtime::default();
    rt.start("Test", &script, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0].state == State::Succeeded
            && r.runs[0]
                .lines
                .iter()
                .any(|(_, s)| s.contains("literal value"))
    });
    assert!(!f.0.join("malicious-marker").exists());
}

#[test]
fn argument_presets_remain_distinct_and_survive_rescan() {
    let f = Fixture::new();
    f.script("printf '%s\\n' \"$1\"; sleep 60 & wait");
    let mut s = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
        store: Store::at(f.0.clone()).unwrap(),
    };
    let project = s.import(&f.0, 1).unwrap();
    let original = s.catalog.projects[0].scripts[0].id;
    let a = s
        .add_preset(original, "Local", vec!["local mode".into()], None)
        .unwrap();
    let b = s
        .add_preset(original, "Test", vec!["test mode".into()], None)
        .unwrap();
    assert_ne!(a, b);
    assert!(
        s.add_preset(original, "Duplicate", vec!["local mode".into()], None)
            .is_err()
    );
    s.rescan(project).unwrap();
    assert_eq!(s.script(a).unwrap().args, vec!["local mode"]);
    let ra = s.launch(a).unwrap();
    let rb = s.launch(b).unwrap();
    wait(&mut s.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == ra && r.lines.iter().any(|(_, s)| s.contains("local mode")))
            && r.runs
                .iter()
                .any(|r| r.id == rb && r.lines.iter().any(|(_, s)| s.contains("test mode")))
    });
    assert_eq!(s.run_tabs().len(), 2);
    s.runtime.stop_all();
}
#[test]
fn tab_labels_have_twelve_visible_characters_and_full_tooltips() {
    use unicode_segmentation::UnicodeSegmentation;
    for input in [
        "short",
        "123456789012",
        "1234567890123",
        "启动脚本带不同参数本地调试环境",
        "e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}e\u{301}",
    ] {
        let label = crate::presentation::tab_label(input);
        assert!(label.graphemes(true).count() <= 12);
        if input.graphemes(true).count() > 12 {
            assert!(label.ends_with("..."));
        } else {
            assert_eq!(label, input);
        }
    }
}
#[test]
fn command_arguments_are_literal_and_run_snapshot_retains_them() {
    let f = Fixture::new();
    let mut s = f.script("# source reference only\n");
    s.entry = Entry::Command {
        program: "/usr/bin/printf".into(),
    };
    s.args = vec![
        "%s\\n".into(),
        "literal spaces; $(touch should-not-exist)".into(),
    ];
    let mut rt = Runtime::default();
    rt.start("Command", &s, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0].state == State::Succeeded
            && r.runs[0]
                .lines
                .iter()
                .any(|(_, s)| s.contains("literal spaces; $(touch should-not-exist)"))
    });
    assert!(!f.0.join("should-not-exist").exists());
    assert!(rt.runs[0].command.contains("literal spaces"));
}
#[test]
fn package_arguments_use_the_package_manager_separator() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let manager = f.0.join("manager.sh");
    fs::write(&manager, "#!/bin/sh\nprintf '<%s>\\n' \"$@\"\n").unwrap();
    fs::set_permissions(&manager, fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = f.script("# reference");
    s.file = f.0.join("package.json");
    fs::write(&s.file, r#"{"scripts":{"dev":"unused"}}"#).unwrap();
    s.entry = Entry::Package {
        manager: manager.display().to_string(),
        name: "dev".into(),
    };
    s.args = vec!["--port".into(), "8123".into()];
    let mut rt = Runtime::default();
    rt.start("Package", &s, f.0.join("logs")).unwrap();
    wait(&mut rt, |r| {
        r.runs[0].state == State::Succeeded
            && r.runs[0].lines.iter().any(|(_, s)| s.contains("<8123>"))
    });
    let output = fs::read_to_string(&rt.runs[0].log_path).unwrap();
    assert!(output.contains("<run>\n<dev>\n<-->\n<--port>\n<8123>"));
}
#[test]
fn preset_form_parsing_is_atomic_and_port_constraint_is_configurable() {
    let f = Fixture::new();
    f.script("echo development");
    let mut s = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
        store: Store::at(f.0.clone()).unwrap(),
    };
    s.import(&f.0, 1).unwrap();
    let id = s.catalog.projects[0].scripts[0].id;
    assert!(
        s.create_preset(id, "Invalid", "--port\n9001", "bad port")
            .is_err()
    );
    assert_eq!(s.catalog.projects[0].scripts.len(), 1);
    let configured = s
        .create_preset(id, "Other port", "--port\n9001", "9001")
        .unwrap();
    assert_eq!(s.script(configured).unwrap().expected_port, Some(9001));
    assert_eq!(s.script(configured).unwrap().args, vec!["--port", "9001"]);
}
#[test]
fn default_console_is_the_last_started_entry_even_after_it_finishes() {
    let f = Fixture::new();
    let mut first = f.script("sleep 60 & wait");
    first.id = 30;
    let mut second = first.clone();
    second.id = 31;
    second.entry = Entry::Command {
        program: "/usr/bin/printf".into(),
    };
    second.args = vec!["finished".into()];
    let mut s = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
        store: Store::at(f.0.clone()).unwrap(),
    };
    let a = s.runtime.start("One", &first, f.0.join("logs")).unwrap();
    let b = s.runtime.start("Two", &second, f.0.join("logs")).unwrap();
    wait(&mut s.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == b && r.state == State::Succeeded)
    });
    assert_eq!(s.selected_run(None), Some(b));
    assert_eq!(s.selected_run(Some(a)), Some(a));
    s.runtime.stop_all();
}
#[test]
fn grouped_project_projection_keeps_launch_choices_under_their_group() {
    let f = Fixture::new();
    f.script("echo development");
    let mut s = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
        store: Store::at(f.0.clone()).unwrap(),
    };
    let project = s.import(&f.0, 1).unwrap();
    s.add_group("Another group").unwrap();
    let group = s.catalog.groups.last().unwrap().id;
    s.move_project(project, group).unwrap();
    let source = s.catalog.projects[0].scripts[0].id;
    s.create_preset(source, "Other port", "--port\n9001", "9001")
        .unwrap();
    let groups = s.grouped_projects(Filter::All, "");
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].1[0].scripts.len(), 2);
    let matches = s.grouped_projects(Filter::All, "--port 9001");
    assert_eq!(matches.len(), 1);
    assert_eq!(matches[0].0.id, group);
    assert_eq!(matches[0].1[0].scripts.len(), 1);
}

#[test]
fn shared_output_resource_rejects_competing_modes_without_starting_them() {
    let f = Fixture::new();
    let mut a = f.script("sleep 60 & wait");
    a.resource_key = Some("shared-compiler-output".into());
    let mut b = a.clone();
    b.id = 99;
    b.args = vec!["other mode".into()];
    let mut rt = Runtime::default();
    let id = rt.start("One", &a, f.0.join("logs")).unwrap();
    assert!(
        rt.start("Two", &b, f.0.join("logs"))
            .unwrap_err()
            .contains("共享调试资源")
    );
    assert_eq!(rt.runs.len(), 1);
    rt.stop(id);
    wait(&mut rt, |r| r.runs[0].state == State::Stopped);
    assert!(rt.start("Two", &b, f.0.join("logs")).is_ok());
}

struct ExternalServer {
    child: std::process::Child,
    script: Script,
    project: crate::model::Project,
}
impl ExternalServer {
    fn new(f: &Fixture) -> Self {
        let file = f.0.join("server.py");
        fs::write(&file, "import socket,sys,time\ns=socket.socket();s.setsockopt(socket.SOL_SOCKET,socket.SO_REUSEADDR,1);s.bind(('127.0.0.1',int(sys.argv[1])));s.listen()\nwith open('bound-port','w') as ready: ready.write(str(s.getsockname()[1]))\nprint('SERVER_READY',flush=True)\nwhile True: print('TICK',flush=True);time.sleep(0.1)\n").unwrap();
        let mut script = f.script("# reference");
        script.file = file.clone();
        script.entry = Entry::File {
            interpreter: "python3".into(),
        };
        let mut child = std::process::Command::new("python3")
            .arg(file)
            .arg("0")
            .current_dir(&f.0)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let started = Instant::now();
        let port = loop {
            if let Ok(text) = fs::read_to_string(f.0.join("bound-port"))
                && let Ok(port) = text.parse::<u16>()
            {
                break port;
            }
            assert!(started.elapsed() < Duration::from_secs(5));
            assert!(
                child.try_wait().unwrap().is_none(),
                "fixture server exited before binding"
            );
            std::thread::sleep(Duration::from_millis(20));
        };
        script.expected_port = Some(port);
        script.args = vec![port.to_string()];
        let project = crate::model::Project {
            description: String::new(),
            id: 1,
            name: "External fixture".into(),
            path: f.0.canonicalize().unwrap(),
            group: 1,
            favorite: false,
            scripts: vec![script.clone()],
            explicit_scripts: true,
        };
        Self {
            child,
            script,
            project,
        }
    }
    fn candidate(&self) -> crate::external::Candidate {
        let found =
            crate::external::discover(std::slice::from_ref(&self.project), &Default::default())
                .unwrap();
        assert_eq!(found.len(), 1);
        let candidate = found.into_iter().next().unwrap();
        assert_eq!(candidate.process.pid, self.child.id());
        candidate
    }
}
impl Drop for ExternalServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[test]
fn external_discovery_requires_project_and_preserves_parameter_ambiguity() {
    let f = Fixture::new();
    let mut server = ExternalServer::new(&f);
    let mut alternative = server.script.clone();
    alternative.id += 1;
    alternative.args.push("other-mode".into());
    server.project.scripts.push(alternative);
    let candidate = server.candidate();
    assert_eq!(candidate.scripts, vec![10, 11]);
    let excluded = std::collections::HashSet::from([server.child.id()]);
    assert!(
        crate::external::discover(std::slice::from_ref(&server.project), &excluded)
            .unwrap()
            .is_empty()
    );
    let unrelated = Fixture::new();
    server.project.path = unrelated.0.clone();
    assert!(
        crate::external::discover(&[server.project.clone()], &Default::default())
            .unwrap()
            .is_empty()
    );
}
#[test]
fn external_identity_guard_and_drop_preserve_original_service() {
    let f = Fixture::new();
    let server = ExternalServer::new(&f);
    let candidate = server.candidate();
    let mut wrong = candidate.process.clone();
    wrong.started.push_str("stale");
    crate::external::signal_checked(&wrong, libc::SIGTERM).unwrap();
    assert!(crate::external::same_process(&candidate.process).unwrap());
    {
        let mut runtime = Runtime::default();
        runtime.adopt(candidate.clone(), &f.0.join("logs")).unwrap();
    }
    assert!(crate::external::same_process(&candidate.process).unwrap());
}
#[test]
fn external_restart_waits_for_exit_and_captures_new_stdout() {
    let f = Fixture::new();
    let server = ExternalServer::new(&f);
    let other_fixture = Fixture::new();
    let other = ExternalServer::new(&other_fixture);
    let other_identity = other.candidate().process;
    let mut runtime = Runtime::default();
    runtime
        .adopt(server.candidate(), &f.0.join("logs"))
        .unwrap();
    runtime
        .launch_checked(&server.project, &server.script, f.0.join("logs"))
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs.len() == 2
            && r.runs[1]
                .lines
                .iter()
                .any(|(_, t)| t.contains("SERVER_READY"))
    });
    assert_eq!(runtime.runs[0].state, State::Stopped);
    assert!(runtime.runs[1].external.is_none());
    assert_ne!(runtime.runs[1].pid, server.child.id());
    assert!(crate::external::same_process(&other_identity).unwrap());
    runtime.stop_all();
    wait(&mut runtime, |r| r.runs.iter().all(|r| !r.state.active()));
    assert!(crate::external::same_process(&other_identity).unwrap());
}
#[test]
fn owned_restart_is_serial_and_replaces_the_run() {
    let f = Fixture::new();
    let script = f.script("echo READY; while true; do sleep 1; done");
    let mut r = Runtime::default();
    r.start("Fixture", &script, f.0.join("logs")).unwrap();
    let project = crate::model::Project {
        description: String::new(),
        id: 1,
        name: "Fixture".into(),
        path: f.0.clone(),
        group: 1,
        favorite: false,
        scripts: vec![script.clone()],
        explicit_scripts: true,
    };
    r.launch_checked(&project, &script, f.0.join("logs"))
        .unwrap();
    wait(&mut r, |r| {
        r.runs.len() == 2 && r.runs[1].state == State::Running && r.runs[0].state == State::Stopped
    });
    assert_eq!(r.runs[0].state, State::Stopped);
    assert_eq!(r.runs[1].state, State::Running);
    r.stop_all();
    wait(&mut r, |r| r.runs.iter().all(|r| !r.state.active()));
}

#[test]
fn external_discovery_resolves_symlinked_command_paths() {
    use std::os::unix::fs::symlink;
    let f = Fixture::new();
    let alias = Fixture::new();
    let mut server = ExternalServer::new(&f);
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    symlink(&f.0, alias.0.join("workspace")).unwrap();
    fs::remove_file(f.0.join("bound-port")).unwrap();
    server.child = std::process::Command::new("python3")
        .arg(alias.0.join("workspace/server.py"))
        .arg("0")
        .current_dir(&f.0)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let start = Instant::now();
    let port = loop {
        if let Ok(text) = fs::read_to_string(f.0.join("bound-port"))
            && let Ok(port) = text.parse::<u16>()
        {
            break port;
        }
        assert!(start.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(20));
    };
    server.script.expected_port = Some(port);
    server.script.args = vec![port.to_string()];
    server.project.scripts[0] = server.script.clone();
    assert_eq!(server.candidate().process.pid, server.child.id());
}

#[test]
fn console_cache_keeps_latest_lines_and_joins_stream_chunks() {
    use crate::console_buffer::{ConsoleBuffer, MAX_BYTES, MAX_LINE_BYTES, MAX_LINES};
    let mut buffer = ConsoleBuffer::default();
    buffer.append(false, "half");
    buffer.append(true, " line\nnext\n");
    assert_eq!(buffer.len(), 2);
    assert_eq!(buffer.get(0), Some(&(true, "half line\n".into())));
    buffer.clear();
    for i in 0..MAX_LINES + 500 {
        buffer.append(i % 2 == 0, &format!("line-{i}\n"));
    }
    assert_eq!(buffer.len(), MAX_LINES);
    assert_eq!(buffer.dropped, 500);
    assert_eq!(buffer.get(0).unwrap().1, "line-500\n");
    buffer.append(false, &"中".repeat(MAX_LINE_BYTES));
    assert!(buffer.shortened);
    assert!(buffer.get(buffer.len() - 1).unwrap().1.len() <= MAX_LINE_BYTES);
    for _ in 0..500 {
        buffer.append(true, &format!("{}\n", "x".repeat(MAX_LINE_BYTES)));
    }
    assert!(buffer.bytes() <= MAX_BYTES);
    assert!(buffer.len() <= MAX_LINES);
}
#[test]
fn console_cache_bounds_final_messages_and_preserves_complete_raw_output() {
    let f = Fixture::new();
    let script=f.script(r#"python3 -c 'import sys;sys.stdout.write("".join("line-%d\n"%i for i in range(4500)));sys.stdout.flush()'; echo last-error >&2; exit 3"#);
    let mut runtime = Runtime::default();
    runtime.start("Fixture", &script, f.0.join("logs")).unwrap();
    wait(&mut runtime, |r| !r.runs[0].state.active());
    let run = &runtime.runs[0];
    assert!(run.lines.len() <= crate::console_buffer::MAX_LINES);
    let displayed = run
        .lines
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<String>();
    assert!(displayed.contains("line-4499"));
    assert!(!displayed.contains("line-0\n"));
    assert!(displayed.contains("退出码 3"));
    let raw = fs::read_to_string(&run.log_path).unwrap();
    assert!(raw.contains("line-0\n"));
    assert!(raw.contains("line-4499"));
}
#[test]
fn stopped_console_can_restart_but_old_console_cannot_stop_a_new_instance() {
    let f = Fixture::new();
    let store = Store::at(f.0.clone()).unwrap();
    f.script("echo READY; while true; do sleep 1; done");
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store,
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    service.import(&f.0, 1).unwrap();
    let sid = service.catalog.projects[0].scripts[0].id;
    let old = service.launch(sid).unwrap();
    service.runtime.stop(old);
    assert!(!service.can_restart(old));
    wait(&mut service.runtime, |r| !r.runs[0].state.active());
    assert!(service.can_restart(old));
    service.restart_run(old, sid).unwrap();
    assert_eq!(service.active_count(), 1);
    assert!(!service.can_restart(old));
    let new = service.active_run(sid).unwrap();
    assert_ne!(new, old);
    assert!(service.restart_run(old, sid).is_err());
    assert_eq!(service.active_run(sid), Some(new));
    service.runtime.stop_all();
    wait(&mut service.runtime, |r| {
        r.runs.iter().all(|r| !r.state.active())
    });
}
#[test]
fn run_tabs_use_launch_names_and_short_command_tooltips() {
    let f = Fixture::new();
    f.script("echo done");
    let store = Store::at(f.0.clone()).unwrap();
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store,
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    service.import(&f.0, 1).unwrap();
    service.catalog.projects[0].scripts[0].name = "H5 本地调试".into();
    fs::create_dir(f.0.join("work")).unwrap();
    service.catalog.projects[0].scripts[0].cwd = f.0.join("work");
    let sid = service.catalog.projects[0].scripts[0].id;
    service.launch(sid).unwrap();
    let tabs = service.run_tabs();
    assert_eq!(tabs[0].label, "H5 本地调试");
    assert_eq!(
        tabs[0].tooltip,
        format!(
            "H5 本地调试\n{}\n{}",
            crate::presentation::command_preview(&service.script(sid).unwrap().tab_command()),
            crate::service::path_label(&service.catalog.projects[0].path)
        )
    );
    assert!(service.runtime.runs[0].command.contains("/bin/bash"));
    let full = "Java 后端 · 本地开发服务启动项";
    service.set_display_name(sid, Some(full.into())).unwrap();
    let expanded = service.run_tabs();
    assert_eq!(expanded[0].full_name, full);
    assert_eq!(expanded[0].label, crate::presentation::tab_label(full));
    assert_ne!(expanded[0].full_name, expanded[0].label);
    assert!(expanded[0].tooltip.starts_with(full));
}
#[test]
fn console_global_budget_limits_bytes_and_rows_across_history() {
    use crate::console_buffer::{MAX_TOTAL_BYTES, MAX_TOTAL_LINES};
    let f = Fixture::new();
    let mut r = Runtime::default();
    for id in 1..=20 {
        let mut script = f.script("true");
        script.id = id;
        r.start("Fixture", &script, f.0.join("logs")).unwrap();
    }
    wait(&mut r, |r| r.runs.iter().all(|r| !r.state.active()));
    let large_line = "a".repeat(1500) + "\n";
    for run in &mut r.runs {
        for _ in 0..1500 {
            run.lines.append(false, &large_line);
        }
    }
    r.trim_console_budget();
    assert!(r.runs.iter().map(|r| r.lines.bytes()).sum::<usize>() <= MAX_TOTAL_BYTES);
    for run in &mut r.runs {
        run.lines.clear();
        run.lines.append(true, &"x\n".repeat(3000));
    }
    r.trim_console_budget();
    assert_eq!(
        r.runs.iter().map(|r| r.lines.len()).sum::<usize>(),
        MAX_TOTAL_LINES
    );
}

#[test]
fn java_tab_tooltips_use_main_class_and_app_arguments_only() {
    let f = Fixture::new();
    let mut script = f.script("# reference");
    script.name = "Starter".into();
    script.entry = Entry::Java {
        configuration: "Starter".into(),
        program: "/opt/jdk/bin/java".into(),
        main_class: "com.example.Main".into(),
        vm_args: vec!["-Dfile.encoding=UTF-8".into()],
        class_paths: vec![f.0.join("classes"), f.0.join("dependency.jar")],
        enabled: true,
        snapshot: "{}".into(),
        manifest: Default::default(),
        classpath_file: None,
    };
    assert_eq!(
        format!("{}\n{}", script.name, script.tab_command()),
        "Starter\njava com.example.Main"
    );
    script.name = "Starter (测试环境)".into();
    script.args = vec!["test".into()];
    assert_eq!(
        format!("{}\n{}", script.name, script.tab_command()),
        "Starter (测试环境)\njava com.example.Main test"
    );
    assert!(script.invocation().contains("-cp"));
    assert!(script.invocation().contains("-Dfile.encoding"));
}

fn native_project_fixture(f: &Fixture) -> PathBuf {
    fs::create_dir_all(f.0.join("src/example")).unwrap();
    fs::create_dir_all(f.0.join("classes/example")).unwrap();
    fs::write(
        f.0.join("src/example/Server.java"),
        "package example; public class Server { public static void main(String[] args) {} }",
    )
    .unwrap();
    fs::write(f.0.join("classes/example/Server.class"), b"fixture").unwrap();
    fs::write(f.0.join("lib.jar"), b"fixture").unwrap();
    fs::write(
        f.0.join("runtime-classpath.txt"),
        f.0.join("lib.jar").display().to_string(),
    )
    .unwrap();
    let java = f.0.join("java");
    fs::write(&java, "#!/bin/bash\nprintf '<%s>\\n' \"$@\"\n").unwrap();
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(&java, fs::Permissions::from_mode(0o755)).unwrap();
    let manifest = f.0.join(crate::project_manifest::FILE_NAME);
    let entry = serde_json::json!({"Java":{"program":java,"main_class":"example.Server","vm_args":["-Dlabel=two words"],"class_paths":["classes"],"classpath_file":"runtime-classpath.txt","enabled":true}});
    fs::write(&manifest,serde_json::json!({"version":1,"launches":[
        {"name":"Service","file":"src/example/Server.java","entry":entry,"description":"native source","args":["test","$(touch marker)"]},
        {"name":"Deploy","file":"src/example/Server.java","entry":{"Java":{"program":java,"main_class":"example.Server","vm_args":[],"class_paths":["classes"],"enabled":false}},"description":"not enabled"}
    ]}).to_string()).unwrap();
    manifest
}
#[test]
fn project_native_launches_ignore_ide_configuration_entirely() {
    let f = Fixture::new();
    native_project_fixture(&f);
    fs::create_dir_all(f.0.join(".vscode")).unwrap();
    fs::write(
        f.0.join(".vscode/launch.json"),
        "not valid JSON; should never be read",
    )
    .unwrap();
    let entries = discovery::discover(&f.0).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].name, "Service");
    assert!(entries[0].file.ends_with("src/example/Server.java"));
    assert!(entries[1].launch_block_reason().is_some());
    fs::remove_dir_all(f.0.join(".vscode")).unwrap();
    assert_eq!(discovery::discover(&f.0).unwrap().len(), 2);
}
#[test]
fn project_native_java_uses_direct_class_and_literal_args_without_maven() {
    let f = Fixture::new();
    native_project_fixture(&f);
    let mut entries = discovery::discover(&f.0).unwrap();
    entries[0].id = 1;
    let mut runtime = Runtime::default();
    runtime
        .start("Fixture", &entries[0], f.0.join("logs"))
        .unwrap();
    wait(&mut runtime, |r| !r.runs[0].state.active());
    let output = runtime.runs[0]
        .lines
        .iter()
        .map(|(_, s)| s.as_str())
        .collect::<String>();
    assert!(output.contains("<-Dlabel=two words>"));
    assert!(output.contains("<example.Server>\n<test>\n<$(touch marker)>"));
    assert!(!f.0.join("marker").exists());
    assert!(crate::project_manifest::validate_launch(&entries[1]).is_err());
}
#[test]
fn project_native_manifest_changes_are_checked_and_source_remains_authoritative() {
    let f = Fixture::new();
    let manifest = native_project_fixture(&f);
    let old = discovery::discover(&f.0).unwrap();
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    json["launches"][0]["args"] = serde_json::json!(["changed"]);
    fs::write(&manifest, json.to_string()).unwrap();
    assert!(crate::project_manifest::validate_launch(&old[0]).is_err());
    let new = discovery::discover(&f.0).unwrap();
    assert_eq!(new[0].args, vec!["changed"]);
    assert!(crate::project_manifest::validate_launch(&new[0]).is_ok());
}
#[test]
fn project_native_rescan_keeps_ids_and_reads_current_project_parameters() {
    let f = Fixture::new();
    let manifest = native_project_fixture(&f);
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store: Store::at(f.0.clone()).unwrap(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    let project = service.import(&f.0, 1).unwrap();
    let id = service.catalog.projects[0].scripts[0].id;
    let mut json: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    json["launches"][0]["args"] = serde_json::json!(["fresh"]);
    fs::write(&manifest, json.to_string()).unwrap();
    service.rescan(project).unwrap();
    assert_eq!(service.catalog.projects[0].scripts[0].id, id);
    assert_eq!(service.catalog.projects[0].scripts[0].args, vec!["fresh"]);
    assert!(service.runtime.runs.is_empty());
}
#[test]
fn project_native_missing_classpath_never_triggers_a_build() {
    let f = Fixture::new();
    native_project_fixture(&f);
    fs::remove_file(f.0.join("runtime-classpath.txt")).unwrap();
    assert!(discovery::discover(&f.0).is_err());
    assert!(!f.0.join("marker").exists());
}

#[test]
fn package_and_shell_discovery_does_not_read_ide_files() {
    let f = Fixture::new();
    f.script("echo ready");
    fs::create_dir_all(f.0.join(".vscode")).unwrap();
    fs::write(f.0.join(".vscode/launch.json"), "malformed IDE content").unwrap();
    fs::write(
        f.0.join("package.json"),
        r#"{"scripts":{"dev":"echo ready","release":"touch forbidden"}}"#,
    )
    .unwrap();
    let scripts = discovery::discover(&f.0).unwrap();
    assert_eq!(scripts.len(), 2);
    assert!(
        scripts
            .iter()
            .any(|s| matches!(&s.entry,Entry::Package{name,..} if name=="dev"))
    );
    assert!(!f.0.join("forbidden").exists());
}
#[test]
fn custom_parameters_survive_project_native_refresh_without_an_ide() {
    let f = Fixture::new();
    native_project_fixture(&f);
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store: Store::at(f.0.clone()).unwrap(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    let project = service.import(&f.0, 1).unwrap();
    let source = service.catalog.projects[0].scripts[0].id;
    let preset = service
        .add_preset(source, "Service custom", vec!["custom-args".into()], None)
        .unwrap();
    service.rescan(project).unwrap();
    assert_eq!(service.script(preset).unwrap().args, vec!["custom-args"]);
    assert!(service.runtime.runs.is_empty());
}

fn fixture_project(f: &Fixture, script: Script) -> crate::model::Project {
    crate::model::Project {
        description: String::new(),
        id: 1,
        name: "Lifecycle fixture".into(),
        path: f.0.canonicalize().unwrap(),
        group: 1,
        favorite: false,
        scripts: vec![script],
        explicit_scripts: true,
    }
}
fn run_text(runtime: &Runtime, id: u64) -> String {
    runtime
        .runs
        .iter()
        .find(|r| r.id == id)
        .unwrap()
        .lines
        .iter()
        .map(|(_, s)| s.as_str())
        .collect()
}
#[test]
fn checked_start_replaces_unregistered_port_owner_and_reports_every_stage() {
    let f = Fixture::new();
    let server = ExternalServer::new(&f);
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(&server.project, &server.script, f.0.join("logs"))
        .unwrap();
    assert_eq!(runtime.runs[0].state, State::Starting);
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("SERVER_READY")
    });
    assert_ne!(runtime.runs[0].pid, server.child.id());
    let text = run_text(&runtime, id);
    for stage in [
        "检查项目原实例",
        "已核实外部实例 1 个",
        "SIGTERM",
        "已优雅退出",
        "再次校验",
        "新实例已启动",
    ] {
        assert!(text.contains(stage), "missing {stage}: {text}");
    }
    let raw = fs::read_to_string(&runtime.runs[0].log_path).unwrap();
    assert!(raw.contains("SIGTERM") && raw.contains("SERVER_READY"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}
#[test]
fn checked_start_forces_a_term_ignoring_owned_group_before_relaunch() {
    let f = Fixture::new();
    let script = f.script("trap '' TERM; echo OLD_READY; while true; do sleep 1; done");
    let mut runtime = Runtime::default();
    runtime.start("Fixture", &script, f.0.join("logs")).unwrap();
    wait(&mut runtime, |r| {
        run_text(r, r.runs[0].id).contains("OLD_READY")
    });
    fs::write(
        &script.file,
        "trap 'exit 0' TERM; echo NEW_READY; while true; do sleep 1; done",
    )
    .unwrap();
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == id && r.state == State::Running)
            && run_text(r, id).contains("NEW_READY")
    });
    assert_eq!(runtime.runs[0].state, State::Stopped);
    let text = run_text(&runtime, id);
    assert!(
        text.contains("优雅关闭超时") && text.contains("SIGKILL") && text.contains("确认原实例"),
        "{text}"
    );
    runtime.stop_all();
    wait(&mut runtime, |r| r.runs.iter().all(|r| !r.state.active()));
}
#[test]
fn checked_start_refuses_an_unrelated_listener_and_keeps_it_alive() {
    let old_fixture = Fixture::new();
    let server = ExternalServer::new(&old_fixture);
    let identity = server.candidate().process;
    let f = Fixture::new();
    let mut script = f.script("trap 'exit 0' TERM; echo OLD_READY; while true; do sleep 1; done");
    let mut runtime = Runtime::default();
    let old = runtime.start("Old", &script, f.0.join("logs")).unwrap();
    wait(&mut runtime, |r| run_text(r, old).contains("OLD_READY"));
    fs::write(&script.file, "touch forbidden-new-instance").unwrap();
    script.expected_port = server.script.expected_port;
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == id && r.state == State::Failed)
    });
    assert!(run_text(&runtime, id).contains("无法确认归属"));
    assert!(!run_text(&runtime, id).contains("SIGTERM"));
    assert_eq!(runtime.runs[0].state, State::Running);
    assert!(crate::external::same_process(&identity).unwrap());
    assert!(!f.0.join("forbidden-new-instance").exists());
    runtime.stop_all();
    wait(&mut runtime, |r| r.runs.iter().all(|r| !r.state.active()));
}
struct ShellFixture(std::process::Child);
impl Drop for ShellFixture {
    fn drop(&mut self) {
        unsafe {
            libc::kill(-(self.0.id() as i32), libc::SIGKILL);
        }
        let _ = self.0.wait();
    }
}
fn standalone_shell(f: &Fixture, arg: &str) -> ShellFixture {
    use std::os::unix::process::CommandExt;
    let child = std::process::Command::new("/bin/bash")
        .arg("dev.sh")
        .arg(arg)
        .current_dir(&f.0)
        .process_group(0)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let result = ShellFixture(child);
    let start = Instant::now();
    while !f.0.join(format!("{arg}.ready")).exists() {
        assert!(start.elapsed() < Duration::from_secs(3));
        std::thread::sleep(Duration::from_millis(10));
    }
    result
}
#[test]
fn checked_start_without_port_matches_relative_script_and_space_args_but_keeps_other_mode() {
    let f = Fixture::new();
    let mut script = f.script(
        "trap 'exit 0' TERM; touch \"$1.ready\"; echo SHELL_READY; while true; do sleep 1; done",
    );
    script.args = vec!["target mode".into()];
    let target = standalone_shell(&f, "target mode");
    let other = standalone_shell(&f, "other mode");
    let other_identity = crate::external::process(other.0.id()).unwrap().unwrap();
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("SHELL_READY")
    });
    assert!(crate::external::process(target.0.id()).unwrap().is_none());
    assert!(crate::external::same_process(&other_identity).unwrap());
    assert!(run_text(&runtime, id).contains("已核实外部实例 1 个"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}
#[test]
fn checked_start_merges_repeated_clicks_and_cancellation_prevents_spawn() {
    let f = Fixture::new();
    let script = f.script("touch should-not-start; while true; do sleep 1; done");
    let project = fixture_project(&f, script.clone());
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(&project, &script, f.0.join("logs"))
        .unwrap();
    assert_eq!(
        id,
        runtime
            .launch_checked(&project, &script, f.0.join("logs"))
            .unwrap()
    );
    assert_eq!(runtime.runs.len(), 1);
    runtime.stop(id);
    wait(&mut runtime, |r| r.runs[0].state == State::Stopped);
    assert_eq!(runtime.runs[0].pid, 0);
    assert!(!f.0.join("should-not-start").exists());
    assert!(run_text(&runtime, id).contains("取消启动"));
}

#[test]
fn checked_start_forces_unregistered_term_ignoring_script_and_logs_it() {
    let f = Fixture::new();
    let mut script = f.script("trap '' TERM; touch \"$1.ready\"; while true; do sleep 1; done");
    script.args = vec!["forced".into()];
    let old = standalone_shell(&f, "forced");
    fs::write(
        &script.file,
        "trap 'exit 0' TERM; echo NEW_READY; while true; do sleep 1; done",
    )
    .unwrap();
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("NEW_READY")
    });
    let text = run_text(&runtime, id);
    assert!(
        text.contains("SIGTERM")
            && text.contains("SIGKILL")
            && text.contains("已强制结束并确认退出"),
        "{text}"
    );
    assert!(crate::external::process(old.0.id()).unwrap().is_none());
    let raw = fs::read_to_string(&runtime.runs[0].log_path).unwrap();
    assert!(raw.contains("SIGKILL") && raw.contains("NEW_READY"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}

#[test]
fn checked_start_replaces_a_shared_resource_mode_launched_by_npm() {
    use std::os::unix::process::CommandExt;
    let f = Fixture::new();
    fs::write(
        f.0.join("package.json"),
        r#"{"scripts":{"watch:a":"node watch.js a","watch:b":"node watch.js b"}}"#,
    )
    .unwrap();
    fs::write(f.0.join("watch.js"),"const fs=require('fs');fs.writeFileSync(process.argv[2]+'.ready','ready');console.log('WATCH_READY',process.argv[2]);process.on('SIGTERM',()=>process.exit(0));setInterval(()=>{},1000);").unwrap();
    let mut a = f.script("# reference");
    a.file = f.0.join("package.json");
    a.id = 10;
    a.entry = Entry::Package {
        manager: "npm".into(),
        name: "watch:a".into(),
    };
    a.environment
        .insert("NPM_CONFIG_UPDATE_NOTIFIER".into(), "false".into());
    a.resource_key = Some(f.0.join("shared-output").display().to_string());
    let mut b = a.clone();
    b.id = 11;
    b.entry = Entry::Package {
        manager: "npm".into(),
        name: "watch:b".into(),
    };
    let mut project = fixture_project(&f, a.clone());
    project.scripts.push(b);
    let old = ShellFixture(
        std::process::Command::new("npm")
            .args(["run", "watch:b"])
            .env("NPM_CONFIG_UPDATE_NOTIFIER", "false")
            .current_dir(&f.0)
            .process_group(0)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(5);
    while !f.0.join("b.ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(30));
    }
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(&project, &a, f.0.join("logs"))
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("WATCH_READY a")
    });
    assert!(crate::external::process(old.0.id()).unwrap().is_none());
    assert!(run_text(&runtime, id).contains("已核实外部实例 1 个"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}

#[test]
fn checked_start_detects_python_script_without_port_configuration() {
    let f = Fixture::new();
    let mut old = ExternalServer::new(&f);
    old.script.expected_port = None;
    old.script.args = vec!["0".into()];
    old.project.scripts[0] = old.script.clone();
    let observed = crate::external::inspect_arguments(old.child.id()).unwrap();
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(&old.project, &old.script, f.0.join("logs"))
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("SERVER_READY")
    });
    assert!(
        crate::external::process(old.child.id()).unwrap().is_none(),
        "argv={observed:?}; script={:?}; logs={}",
        old.script.args,
        run_text(&runtime, id)
    );
    assert!(run_text(&runtime, id).contains("已核实外部实例 1 个"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}
#[test]
fn checked_start_detects_direct_java_main_without_an_endpoint() {
    use std::os::unix::process::CommandExt;
    let f = Fixture::new();
    native_project_fixture(&f);
    let mut script = discovery::discover(&f.0).unwrap().remove(0);
    script.id = 1;
    fs::write(f.0.join("java"),"#!/bin/bash\ntrap 'exit 0' TERM; touch java.ready; echo JAVA_READY; while true; do sleep 1; done").unwrap();
    let (program, main) = match &script.entry {
        Entry::Java {
            program,
            main_class,
            ..
        } => (program.clone(), main_class.clone()),
        _ => unreachable!(),
    };
    let old = ShellFixture(
        std::process::Command::new(program)
            .arg(main)
            .args(&script.args)
            .current_dir(&f.0)
            .process_group(0)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap(),
    );
    let deadline = Instant::now() + Duration::from_secs(3);
    while !f.0.join("java.ready").exists() {
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(10));
    }
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("JAVA_READY")
    });
    assert!(crate::external::process(old.0.id()).unwrap().is_none());
    assert!(run_text(&runtime, id).contains("已核实外部实例 1 个"));
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}

#[test]
fn forced_external_shutdown_refreshes_children_spawned_during_term() {
    let f = Fixture::new();
    let mut script=f.script("trap '(trap \"\" TERM; sleep 30) & echo $! > spawned-child' TERM; touch \"$1.ready\"; while true; do sleep 1; done");
    script.args = vec!["spawn".into()];
    let old = standalone_shell(&f, "spawn");
    fs::write(
        &script.file,
        "trap 'exit 0' TERM; echo NEW_READY; while true; do sleep 1; done",
    )
    .unwrap();
    let mut runtime = Runtime::default();
    let id = runtime
        .launch_checked(
            &fixture_project(&f, script.clone()),
            &script,
            f.0.join("logs"),
        )
        .unwrap();
    wait(&mut runtime, |r| {
        r.runs[0].state == State::Running && run_text(r, id).contains("NEW_READY")
    });
    let spawned = fs::read_to_string(f.0.join("spawned-child"))
        .unwrap()
        .trim()
        .parse::<u32>()
        .unwrap();
    assert!(crate::external::process(spawned).unwrap().is_none());
    assert!(crate::external::process(old.0.id()).unwrap().is_none());
    runtime.stop_all();
    wait(&mut runtime, |r| !r.runs[0].state.active());
}

#[test]
fn command_tooltips_bound_unicode_without_changing_execution() {
    use crate::presentation::{command_preview, run_tooltip};
    assert_eq!(
        command_preview("java com.example.Main test"),
        "java com.example.Main test"
    );
    let exact = "中".repeat(100);
    assert_eq!(command_preview(&exact), exact);
    let long = "中".repeat(101);
    assert_eq!(command_preview(&long), format!("{}…", "中".repeat(100)));
    assert_eq!(
        run_tooltip("Starter", &long, "~/Projects/example-java"),
        format!("Starter\n{}…\n~/Projects/example-java", "中".repeat(100))
    );
}

fn api_service(f: &Fixture) -> Service {
    Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store: Store::at(f.0.clone()).unwrap(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    }
}
fn api_call(s: &mut Service, method: &str, p: serde_json::Value) -> serde_json::Value {
    crate::api::dispatch(s, method, &p).unwrap_or_else(|e| panic!("{method}: {e}"))
}
#[test]
fn api_group_project_crud_has_default_group_and_atomic_metadata_validation() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("echo api");
    let mut s = api_service(&f);
    let p = api_call(
        &mut s,
        "projects.create",
        json!({"path":f.0,"description":"项目说明","name":"API 项目"}),
    );
    let pid = p["id"].as_u64().unwrap();
    assert_eq!(p["group"], 1);
    assert_eq!(p["description"], "项目说明");
    let group = api_call(&mut s, "groups.create", json!({"name":"开发"}))["id"]
        .as_u64()
        .unwrap();
    api_call(
        &mut s,
        "projects.update",
        json!({"id":pid,"patch":{"group":group,"favorite":true}}),
    );
    api_call(
        &mut s,
        "groups.update",
        json!({"id":group,"name":"开发环境"}),
    );
    api_call(&mut s, "groups.delete", json!({"id":group}));
    assert_eq!(s.catalog.projects[0].group, 1);
    assert!(crate::api::dispatch(&mut s, "groups.delete", &json!({"id":1})).is_err());
    api_call(
        &mut s,
        "projects.update",
        json!({"id":pid,"patch":{"group":null}}),
    );
    let before = serde_json::to_value(&s.catalog).unwrap();
    assert!(
        crate::api::dispatch(
            &mut s,
            "projects.update",
            &json!({"id":pid,"patch":{"group":999}})
        )
        .is_err()
    );
    assert_eq!(before, serde_json::to_value(&s.catalog).unwrap());
    api_call(&mut s, "projects.delete", json!({"id":pid}));
    assert!(s.catalog.projects.is_empty());
    assert!(f.0.join("dev.sh").exists());
}
#[test]
fn api_script_crud_persists_manifest_and_rolls_back_invalid_edits() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("echo api");
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    api_call(
        &mut s,
        "scripts.update",
        json!({"id":sid,"patch":{"name":"本地","description":"运行描述","display_command":"本地命令","enabled":false,"favorite":true,"args":["test"]}}),
    );
    assert_eq!(s.script(sid).unwrap().tab_command(), "本地命令");
    assert!(s.script(sid).unwrap().launch_block_reason().is_some());
    let blocked = s.launch(sid).unwrap();
    wait(&mut s.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == blocked && r.state == State::Failed)
    });
    assert!(run_text(&s.runtime, blocked).contains("未启用"));
    s.rescan(pid).unwrap();
    assert_eq!(s.script(sid).unwrap().description, "运行描述");
    assert!(!s.script(sid).unwrap().enabled);
    assert!(s.script(sid).unwrap().favorite);
    let original = fs::read(f.0.join(".dev-start.json")).unwrap();
    assert!(
        crate::api::dispatch(
            &mut s,
            "scripts.update",
            &json!({"id":sid,"patch":{"file":"missing.sh"}})
        )
        .is_err()
    );
    assert_eq!(original, fs::read(f.0.join(".dev-start.json")).unwrap());
    let added = api_call(&mut s,"scripts.create",json!({"project_id":pid,"spec":{"name":"测试","file":"dev.sh","entry":{"File":{"interpreter":"/bin/bash"}},"description":"另一个参数","args":["qa"]}}))["id"].as_u64().unwrap();
    s.rescan(pid).unwrap();
    assert_eq!(
        s.catalog.projects[0]
            .scripts
            .iter()
            .map(|s| s.id)
            .collect::<std::collections::HashSet<_>>()
            .len(),
        2
    );
    assert_eq!(s.script(added).unwrap().name, "测试");
    api_call(&mut s, "scripts.delete", json!({"id":added}));
    s.rescan(pid).unwrap();
    assert!(s.script(added).is_none());
    api_call(&mut s, "scripts.delete", json!({"id":sid}));
    s.rescan(pid).unwrap();
    assert!(s.catalog.projects[0].scripts.is_empty());
}
#[test]
fn api_runtime_logs_have_exact_byte_cursors_and_active_delete_guard() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("printf '中文日志\\n'; sleep 30");
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let rid = api_call(&mut s, "runs.start", json!({"script_id":sid}))["run_id"]
        .as_u64()
        .unwrap();
    wait(&mut s.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == rid && r.state == State::Running && r.lines.len() > 0)
    });
    assert!(crate::api::dispatch(&mut s, "projects.delete", &json!({"id":pid})).is_err());
    assert!(crate::api::dispatch(&mut s, "scripts.delete", &json!({"id":sid})).is_err());
    let mut offset = 0;
    let mut bytes = vec![];
    loop {
        let chunk = api_call(
            &mut s,
            "logs.read",
            json!({"run_id":rid,"offset":offset,"limit":7}),
        );
        let raw: Vec<u8> = serde_json::from_value(chunk["bytes"].clone()).unwrap();
        bytes.extend(raw);
        offset = chunk["next_offset"].as_u64().unwrap();
        if chunk["eof"] == true {
            break;
        }
    }
    assert_eq!(bytes, fs::read(&s.runtime.runs[0].log_path).unwrap());
    assert_eq!(
        api_call(&mut s, "logs.read", json!({"run_id":rid,"offset":u64::MAX}))["reset"],
        true
    );
    assert_eq!(
        api_call(&mut s, "logs.list", json!({}))["logs"][0]["run_id"],
        rid
    );
    api_call(&mut s, "runs.stop", json!({"id":rid}));
    wait(&mut s.runtime, |r| r.runs.iter().all(|r| !r.state.active()));
    assert_eq!(
        api_call(&mut s, "runs.get", json!({"id":rid}))["active"],
        false
    );
    assert!(
        api_call(&mut s, "logs.read", json!({"run_id":rid,"tail_bytes":20}))["bytes"]
            .as_array()
            .unwrap()
            .len()
            <= 20
    );
}
#[test]
fn api_jsonrpc_rejects_bad_methods_and_exposes_all_contracts() {
    use serde_json::json;
    let f = Fixture::new();
    let mut s = api_service(&f);
    assert_eq!(
        crate::api::response(&mut s, json!({"id":1,"method":"config.get"}))["error"]["code"],
        -32600
    );
    assert_eq!(
        crate::api::response(&mut s, json!({"jsonrpc":"2.0","id":2,"method":"missing"}))["error"]["code"],
        -32601
    );
    let schema = api_call(&mut s, "api.describe", json!({}));
    for method in crate::api::METHODS {
        assert!(
            schema["contracts"]["methods"].get(method).is_some(),
            "missing {method}"
        );
    }
    assert!(
        crate::api::dispatch(
            &mut s,
            "groups.create",
            &json!({"name":"QA","unexpected":true})
        )
        .is_err()
    );
}

#[test]
fn api_socket_permissions_roundtrip_and_invalid_json_are_bounded() {
    use serde_json::json;
    use std::{
        io::{Read, Write},
        os::unix::{fs::PermissionsExt, net::UnixStream},
    };
    let f = Fixture::new();
    let mut s = api_service(&f);
    let root = PathBuf::from(format!("/tmp/dsa-{}", crate::model::unique_id()));
    let server = crate::api_transport::Server::bind(&root).unwrap();
    let path = crate::api_transport::socket_path(&root);
    assert_eq!(
        fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let client_root = root.clone();
    let client = std::thread::spawn(move || {
        crate::api_transport::call(&client_root, "groups.create", json!({"name":"Socket组"}))
            .unwrap()
    });
    let started = Instant::now();
    while !client.is_finished() {
        server.poll(&mut s);
        assert!(started.elapsed() < Duration::from_secs(5));
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(client.join().unwrap()["result"]["name"], "Socket组");
    let mut stream = UnixStream::connect(&path).unwrap();
    std::thread::sleep(Duration::from_millis(100));
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    stream.write_all(b"bad json\n").unwrap();
    let mut response = String::new();
    stream.read_to_string(&mut response).unwrap();
    assert!(response.contains("error"));
    drop(server);
    assert!(!path.exists());
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn api_manifest_catalog_failure_restores_original_and_does_not_commit_memory() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("echo intact");
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    api_call(
        &mut s,
        "scripts.update",
        json!({"id":sid,"patch":{"description":"before"}}),
    );
    let manifest = fs::read(f.0.join(".dev-start.json")).unwrap();
    fs::create_dir(f.0.join("catalog.json.tmp")).unwrap();
    assert!(
        crate::api::dispatch(
            &mut s,
            "scripts.update",
            &json!({"id":sid,"patch":{"description":"after"}})
        )
        .is_err()
    );
    assert_eq!(fs::read(f.0.join(".dev-start.json")).unwrap(), manifest);
    assert_eq!(s.script(sid).unwrap().description, "before");
    fs::remove_dir(f.0.join("catalog.json.tmp")).unwrap();
    s.rescan(pid).unwrap();
    assert_eq!(s.script(sid).unwrap().description, "before");
}
#[test]
fn loading_legacy_or_orphaned_projects_rehomes_them_in_default_group() {
    let f = Fixture::new();
    let mut c = Catalog::default();
    c.groups.clear();
    c.next_id = 1;
    let project = fixture_project(&f, f.script("true"));
    c.projects.push(project);
    let store = Store::at(f.0.clone()).unwrap();
    store.save(&c).unwrap();
    let loaded = store.load().unwrap();
    assert_eq!(loaded.groups[0].id, 1);
    assert_eq!(loaded.groups[0].name, "默认分组");
    assert_eq!(loaded.projects[0].group, 1);
    assert!(loaded.next_id > 10);
}

#[test]
fn older_java_snapshot_defaults_do_not_invalidate_an_unchanged_source() {
    let f = Fixture::new();
    let manifest = native_project_fixture(&f);
    let mut entries = discovery::discover(&f.0).unwrap();
    if let Entry::Java { snapshot, .. } = &mut entries[0].entry {
        let mut old: serde_json::Value = serde_json::from_str(snapshot).unwrap();
        old.as_object_mut().unwrap().remove("enabled");
        old.as_object_mut().unwrap().remove("display_command");
        *snapshot = old.to_string();
    }
    assert!(crate::project_manifest::validate_launch(&entries[0]).is_ok());
    let mut changed: serde_json::Value =
        serde_json::from_slice(&fs::read(&manifest).unwrap()).unwrap();
    changed["launches"][0]["args"] = serde_json::json!(["different"]);
    fs::write(manifest, changed.to_string()).unwrap();
    assert!(crate::project_manifest::validate_launch(&entries[0]).is_err());
}

#[test]
fn fallback_discovery_reads_only_declared_startup_names_and_never_executes() {
    let f = Fixture::new();
    for name in [
        "dev.sh",
        "start-local.sh",
        "serve.py",
        "deploy.sh",
        "run-starter.sh",
        "upload.py",
        "helper.py",
        "dev-build.sh",
    ] {
        fs::write(f.0.join(name), "touch should-not-run").unwrap();
    }
    fs::write(f.0.join("package.json"),r#"{"scripts":{"dev":"vite","dev:h5:local":"vite","start":"node app","watch:api":"nodemon","build":"vite build","deploy":"upload","test":"vitest","dev:upload":"upload","predev":"setup"}}"#).unwrap();
    let scripts = discovery::discover(&f.0).unwrap();
    assert_eq!(scripts.len(), 7);
    assert!(!scripts.iter().any(|s| s.name.contains("build")
        || s.name.contains("deploy")
        || s.name.contains("upload")
        || s.name.contains("helper")
        || s.name.contains("run-starter")));
    assert!(!f.0.join("should-not-run").exists());
}

#[test]
fn appearance_settings_are_validated_atomic_and_persisted_without_changing_launches() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("echo ready");
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let before = serde_json::to_value(&s.catalog.projects).unwrap();
    let preferences = api_call(&mut s, "settings.get", json!({}));
    assert_eq!(preferences["theme"], "dark");
    api_call(
        &mut s,
        "settings.update",
        json!({"patch":{"theme":"light","console_font_size":16,"console_font":"menlo","density":"comfortable","sidebar_collapsed":true}}),
    );
    assert_eq!(s.store.load().unwrap().preferences.console_font_size, 16.0);
    assert_eq!(
        s.store.load().unwrap().preferences.theme,
        crate::preferences::Theme::Light
    );
    let stored = fs::read(f.0.join("catalog.json")).unwrap();
    for bad in [
        json!({"ui_font_size":3}),
        json!({"console_line_height":9}),
        json!({"theme":"neon"}),
        json!({"unknown":true}),
    ] {
        assert!(crate::api::dispatch(&mut s, "settings.update", &json!({"patch":bad})).is_err());
        assert_eq!(fs::read(f.0.join("catalog.json")).unwrap(), stored);
    }
    assert_eq!(serde_json::to_value(&s.catalog.projects).unwrap(), before);
    api_call(&mut s, "settings.reset", json!({}));
    assert_eq!(
        s.catalog.preferences,
        crate::preferences::Preferences::default()
    );
    let mut legacy = serde_json::to_value(&s.catalog).unwrap();
    legacy.as_object_mut().unwrap().remove("preferences");
    fs::write(f.0.join("catalog.json"), legacy.to_string()).unwrap();
    assert_eq!(
        s.store.load().unwrap().preferences,
        crate::preferences::Preferences::default()
    );
}

#[test]
fn console_search_incrementally_tracks_partial_lines_eviction_clear_and_navigation() {
    use crate::console_search::Query;
    let mut buffer = crate::console_buffer::ConsoleBuffer::default();
    buffer.append(false, "ready\nERR");
    let mut q = Query {
        keyword: "error".into(),
        only_matches: true,
        ..Default::default()
    };
    assert_eq!(buffer.query(&q, None, 0).unwrap().matched_lines, 0);
    buffer.append(true, "OR 中文\nnormal\nerror two\n");
    let view = buffer.query(&q, None, 0).unwrap();
    assert_eq!(view.rows, vec![1, 3]);
    assert_eq!(view.current, Some(0));
    assert_eq!(buffer.query(&q, Some(0), 1).unwrap().active_row, Some(3));
    assert_eq!(buffer.query(&q, Some(1), 1).unwrap().active_row, Some(1));
    assert_eq!(buffer.query(&q, None, -1).unwrap().active_row, Some(3));
    q.case_sensitive = true;
    assert_eq!(buffer.query(&q, None, 0).unwrap().rows, vec![3]);
    q.case_sensitive = false;
    buffer.evict_oldest();
    assert_eq!(buffer.query(&q, None, 0).unwrap().rows, vec![0, 2]);
    buffer.clear();
    assert_eq!(buffer.query(&q, Some(99), 0).unwrap().matched_lines, 0);
    buffer.append(false, "new ERROR\n");
    assert_eq!(buffer.query(&q, None, 0).unwrap().rows, vec![0]);
    q.keyword.clear();
    assert_eq!(buffer.query(&q, None, 0).unwrap().rows, vec![0]);
    assert_eq!(buffer.query(&q, None, 0).unwrap().matched_lines, 0);
}
#[test]
fn console_search_unicode_highlight_ranges_are_original_utf8_and_bounded() {
    use crate::console_search::{Query, spans};
    for (text, word, expected) in [
        ("中文错误日志", "错误", "错误"),
        ("İstanbul ERROR", "i", "İ"),
        ("前缀Error后缀", "ERROR", "Error"),
    ] {
        let q = Query {
            keyword: word.into(),
            ..Default::default()
        };
        let ranges = spans(text, &q);
        assert_eq!(&text[ranges[0].clone()], expected);
        for r in ranges {
            assert!(text.is_char_boundary(r.start));
            assert!(text.is_char_boundary(r.end));
        }
    }
    let q = Query {
        keyword: "a".into(),
        ..Default::default()
    };
    assert_eq!(spans(&"a".repeat(10000), &q).len(), 64);
    assert!(
        Query {
            keyword: "中".repeat(257),
            ..Default::default()
        }
        .validate()
        .is_err()
    );
    assert!(spans("anything", &Query::default()).is_empty());
}
#[test]
fn api_log_search_pages_only_the_bounded_console_and_preserves_raw_log() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("printf '中文 error one\\nnormal\\nERROR two\\n'");
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let rid = s.launch(sid).unwrap();
    wait(&mut s.runtime, |r| {
        r.runs.iter().any(|r| r.id == rid && !r.state.active())
    });
    let path = s
        .runtime
        .runs
        .iter()
        .find(|r| r.id == rid)
        .unwrap()
        .log_path
        .clone();
    let original = fs::read(&path).unwrap();
    let first = api_call(
        &mut s,
        "logs.search",
        json!({"run_id":rid,"keyword":"error","limit":1}),
    );
    assert_eq!(first["source"], "console_cache");
    assert_eq!(first["matched_lines"], 2);
    assert_eq!(first["rows"].as_array().unwrap().len(), 1);
    assert_eq!(first["has_more"], true);
    let second = api_call(
        &mut s,
        "logs.search",
        json!({"run_id":rid,"keyword":"error","offset":first["next_offset"]}),
    );
    assert_eq!(second["has_more"], false);
    assert!(
        second["rows"][0]["text"]
            .as_str()
            .unwrap()
            .contains("ERROR two")
    );
    assert_eq!(
        api_call(
            &mut s,
            "logs.search",
            json!({"run_id":rid,"keyword":"error","case_sensitive":true})
        )["matched_lines"],
        1
    );
    assert_eq!(
        api_call(
            &mut s,
            "logs.search",
            json!({"run_id":rid,"keyword":"不存在"})
        )["matched_lines"],
        0
    );
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn runtime_detects_multiple_real_child_ports_without_configuration_and_clears_on_stop() {
    use serde_json::json;
    let f = Fixture::new();
    let mut script=f.script("import socket,time,json,pathlib\nsockets=[socket.socket(),socket.socket()]\nfor s in sockets:\n s.bind(('127.0.0.1',0));s.listen()\npathlib.Path('ports.json').write_text(json.dumps(sorted(s.getsockname()[1] for s in sockets)))\nprint('PORTS_READY',flush=True)\ntime.sleep(30)\n");
    script.entry = Entry::File {
        interpreter: "python3".into(),
    };
    script.expected_port = Some(9);
    let mut service = api_service(&f);
    service
        .catalog
        .projects
        .push(fixture_project(&f, script.clone()));
    assert_eq!(service.running_port_cell(script.id).label, "9");
    let rid = service
        .runtime
        .start("Ports fixture", &script, f.0.join("logs"))
        .unwrap();
    wait(&mut service.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == rid && r.listening_ports.len() == 2)
    });
    let expected: Vec<u16> =
        serde_json::from_slice(&fs::read(f.0.join("ports.json")).unwrap()).unwrap();
    assert_eq!(service.running_ports(script.id), expected);
    assert!(!service.running_ports(script.id).contains(&9));
    let actual = api_call(&mut service, "runs.get", json!({"id":rid}));
    assert_eq!(actual["listening_ports"], json!(expected));
    assert_eq!(
        service.running_port_cell(script.id).label,
        expected
            .iter()
            .map(u16::to_string)
            .collect::<Vec<_>>()
            .join(", ")
    );
    assert!(
        service
            .running_port_cell(script.id)
            .tooltip
            .contains("实际 TCP")
    );
    assert!(
        service
            .running_port_cell(script.id)
            .tooltip
            .contains("配置端口：9")
    );
    service.runtime.stop(rid);
    wait(&mut service.runtime, |r| {
        r.runs.iter().all(|r| !r.state.active())
    });
    assert_eq!(service.running_port_cell(script.id).label, "9");
    assert_eq!(
        api_call(&mut service, "runs.get", json!({"id":rid}))["listening_ports"],
        json!([])
    );
}

#[test]
fn configured_port_is_visible_before_start_without_fabricating_observation() {
    let f = Fixture::new();
    let mut s = api_service(&f);
    let script = f.script("true");
    let sid = script.id;
    s.catalog.projects.push(fixture_project(&f, script));
    assert_eq!(s.running_port_cell(sid).label, "--");
    s.catalog.projects[0].scripts[0].expected_port = Some(8082);
    let cell = s.running_port_cell(sid);
    assert_eq!(cell.label, "8082");
    assert!(cell.tooltip.contains("配置端口：8082"));
    assert!(s.running_ports(sid).is_empty());
    assert!(s.runtime.runs.is_empty());
}

#[test]
fn configured_ports_read_flags_and_static_project_config_without_executing() {
    use crate::configured_ports::resolve;
    let f = Fixture::new();
    let mut script = f.script("touch must-not-run");
    script.args = vec!["--port".into(), "8123".into()];
    assert_eq!(resolve(&script).unwrap().unwrap().port, 8123);
    script.args = vec!["--port=$PORT".into()];
    assert!(resolve(&script).unwrap().is_none());
    script.args.clear();
    script.file = f.0.join("package.json");
    script.entry = Entry::Package {
        manager: "npm".into(),
        name: "dev".into(),
    };
    fs::write(&script.file, r#"{"scripts":{"dev":"vite --mode test"}}"#).unwrap();
    fs::write(
        f.0.join("vite.config.ts"),
        "export default { server: { port: 4321, proxy: { target: 'http://localhost:8082' } } }",
    )
    .unwrap();
    assert_eq!(resolve(&script).unwrap().unwrap().port, 4321);
    script.args = vec!["--port=9012".into()];
    assert_eq!(resolve(&script).unwrap().unwrap().port, 9012);
    script.args.clear();
    fs::write(
        &script.file,
        r#"{"scripts":{"dev":"uni -p mp-weixin --mode test"}}"#,
    )
    .unwrap();
    assert!(resolve(&script).unwrap().is_none());
    fs::write(&script.file, r#"{"scripts":{"dev":"vite --port $PORT"}}"#).unwrap();
    assert!(resolve(&script).unwrap().is_none());
    script.expected_port = Some(8082);
    assert_eq!(resolve(&script).unwrap().unwrap().port, 8082);
    assert!(!f.0.join("must-not-run").exists());
}

#[test]
fn port_details_api_reports_prelaunch_sources_without_changing_port_constraints() {
    use serde_json::json;
    let f = Fixture::new();
    f.script("#!/bin/bash\nexec uvicorn app:api --port 8765\n");
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let initial = api_call(&mut s, "scripts.ports", json!({"id":sid}));
    assert_eq!(initial["configured_port"], 8765);
    assert_eq!(initial["source"], "configured");
    assert_eq!(initial["actual_ports"], json!([]));
    assert_eq!(s.script(sid).unwrap().expected_port, None);
    assert!(s.runtime.runs.is_empty());
    api_call(
        &mut s,
        "scripts.update",
        json!({"id":sid,"patch":{"args":["--port=9876"]}}),
    );
    assert_eq!(
        api_call(&mut s, "scripts.ports", json!({"id":sid}))["display"],
        "9876"
    );
    s.rescan(pid).unwrap();
    assert_eq!(s.running_port_cell(sid).label, "9876");
    assert!(s.runtime.runs.is_empty());
}

#[test]
fn display_alias_changes_live_presentation_without_changing_execution() {
    let f = Fixture::new();
    f.script("trap 'exit 0' TERM\nwhile true; do sleep 0.1; done");
    let mut service = Service {
        editor: Default::default(),
        port_hints: Default::default(),
        store: Store::at(f.0.join("data")).unwrap(),
        catalog: Catalog::default(),
        runtime: Runtime::default(),
    };
    let pid = service.import(&f.0, 1).unwrap();
    let sid = service.catalog.projects[0].scripts[0].id;
    let original = service.script(sid).unwrap().clone();
    let rid = service.launch(sid).unwrap();
    let command = service.runtime.runs[0].command.clone();
    crate::api_config::handle(
        &mut service,
        "scripts.update",
        &serde_json::json!({"id":sid,"patch":{"display_name":"Java-Starter"}}),
    )
    .unwrap();
    assert_eq!(service.run_tabs()[0].label, "Java-Starter");
    assert_eq!(service.runtime.runs[0].id, rid);
    assert_eq!(service.runtime.runs[0].command, command);
    assert!(service.runtime.runs[0].state.active());
    assert_eq!(service.script(sid).unwrap().identity(), original.identity());
    assert_eq!(service.script(sid).unwrap().name, original.name);
    assert!(!f.0.join(crate::project_manifest::FILE_NAME).exists());
    service.rescan(pid).unwrap();
    assert_eq!(service.script(sid).unwrap().display_label(), "Java-Starter");
    assert_eq!(
        service.store.load().unwrap().projects[0].scripts[0].display_label(),
        "Java-Starter"
    );
    crate::api_config::handle(
        &mut service,
        "scripts.update",
        &serde_json::json!({"id":sid,"patch":{"display_name":null}}),
    )
    .unwrap();
    assert_eq!(service.script(sid).unwrap().display_label(), original.name);
    assert_eq!(service.run_tabs()[0].label, original.name);
    service.runtime.stop(rid);
}

#[test]
fn display_alias_limits_graphemes_and_truncates_only_presentation() {
    use unicode_segmentation::UnicodeSegmentation;
    assert!(Script::validate_display_name(Some("好".repeat(50))).is_ok());
    assert!(Script::validate_display_name(Some("好".repeat(51))).is_err());
    assert!(Script::validate_display_name(Some("👨‍👩‍👧‍👦".repeat(50))).is_ok());
    assert!(Script::validate_display_name(Some("a\nb".into())).is_err());
    assert!(Script::validate_display_name(Some("  ".into())).is_err());
    let label = crate::presentation::launch_label(&"好".repeat(51));
    assert_eq!(label.graphemes(true).count(), 50);
    assert!(label.ends_with("..."));
    assert_eq!(
        crate::presentation::launch_label(&"好".repeat(50)),
        "好".repeat(50)
    );
}

#[path = "editor_tests.rs"]
mod editor_tests;

#[test]
fn project_and_group_rename_preserve_live_startup_and_source_identity() {
    use serde_json::json;
    let f = Fixture::new();
    f.script(
        "#!/bin/bash\necho RENAME_READY\ntrap 'exit 0' TERM\nwhile true; do sleep 0.1; done\n",
    );
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let path = s.catalog.projects[0].path.clone();
    let scripts = serde_json::to_value(&s.catalog.projects[0].scripts).unwrap();
    let original_file = fs::read(&s.script(sid).unwrap().file).unwrap();
    let rid = s.launch(sid).unwrap();
    wait(&mut s.runtime, |r| {
        r.runs
            .iter()
            .any(|r| r.id == rid && r.state == State::Running)
    });
    let run = s.runtime.runs.iter().find(|r| r.id == rid).unwrap();
    let process = run.pid;
    let command = run.command.clone();
    api_call(
        &mut s,
        "projects.update",
        json!({"id":pid,"patch":{"name":"  example-java  "}}),
    );
    api_call(&mut s, "groups.update", json!({"id":1,"name":"未分组"}));
    let run = s.runtime.runs.iter().find(|r| r.id == rid).unwrap();
    assert_eq!(run.pid, process);
    assert_eq!(run.command, command);
    assert!(run.state.active());
    assert_eq!(s.runtime.runs.len(), 1);
    assert!(run.title.starts_with("example-java / "));
    assert_eq!(s.catalog.projects[0].path, path);
    assert_eq!(
        serde_json::to_value(&s.catalog.projects[0].scripts).unwrap(),
        scripts
    );
    assert_eq!(
        fs::read(&s.script(sid).unwrap().file).unwrap(),
        original_file
    );
    s.rescan(pid).unwrap();
    assert_eq!(s.catalog.projects[0].name, "example-java");
    let loaded = s.store.load().unwrap();
    assert_eq!(loaded.projects[0].name, "example-java");
    assert_eq!(loaded.groups[0].name, "未分组");
    for name in [" ", "bad\nname"] {
        assert!(
            crate::api::dispatch(
                &mut s,
                "projects.update",
                &json!({"id":pid,"patch":{"name":name}})
            )
            .is_err()
        );
        assert!(
            crate::api::dispatch(&mut s, "groups.update", &json!({"id":1,"name":name})).is_err()
        );
    }
    assert_eq!(s.catalog.projects[0].name, "example-java");
    s.runtime.stop(rid);
    wait(&mut s.runtime, |r| r.runs.iter().all(|r| !r.state.active()));
}

#[test]
fn runtime_duration_uses_day_hour_minute_units_and_always_seconds() {
    for (seconds, expected) in [
        (0, "0秒"),
        (59, "59秒"),
        (60, "1分钟0秒"),
        (61, "1分钟1秒"),
        (1279, "21分钟19秒"),
        (3599, "59分钟59秒"),
        (3600, "1小时0秒"),
        (3601, "1小时1秒"),
        (3661, "1小时1分钟1秒"),
        (86400, "1天0秒"),
        (90061, "1天1小时1分钟1秒"),
        (172861, "2天1分钟1秒"),
    ] {
        assert_eq!(crate::time_display::duration(seconds), expected);
    }
}
#[test]
fn external_process_start_date_is_validated_and_formatted_locally() {
    use crate::time_display::{local_timestamp, process_start};
    for (source, expected) in [
        ("Thu Oct 15 08:04:03 2026", "2026-10-15 08:04:03"),
        ("Tue Jan  6 00:00:00 2026", "2026-01-06 00:00:00"),
        ("Thu Feb 29 23:59:59 2024", "2024-02-29 23:59:59"),
    ] {
        let time = process_start(source)
            .unwrap()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap();
        assert_eq!(local_timestamp(time.as_secs()), expected);
    }
    for invalid in [
        "unknown",
        "Mon Feb 30 12:00:00 2026",
        "Mon Oct 15 24:00:00 2026",
        "Mon Oct 15 12:60:00 2026",
        "Mon Bad 15 12:00:00 2026",
    ] {
        assert!(
            process_start(invalid).is_err(),
            "accepted invalid start {invalid}"
        );
    }
}
#[test]
fn runtime_timing_has_real_spawn_timestamp_and_freezes_at_exit() {
    let f = Fixture::new();
    let script = f.script("echo TIMING_READY\n");
    let mut r = Runtime::default();
    let before = std::time::SystemTime::now();
    r.start("timing", &script, f.0.join("logs")).unwrap();
    assert!(r.runs[0].started_at.unwrap() >= before);
    wait(&mut r, |r| !r.runs[0].state.active());
    let run = &mut r.runs[0];
    run.elapsed = Duration::from_secs(1279);
    let timing = run.timing();
    assert_eq!(timing.elapsed_display, "21分钟19秒");
    assert!(timing.started_at.is_some());
    assert!(timing.started_at_unix_ms.is_some());
    assert!(timing.display.ends_with(" - 21分钟19秒"));
    run.started = Instant::now() - Duration::from_secs(600);
    assert_eq!(run.timing().elapsed_seconds, 1279);
    run.elapsed_offset = Duration::from_secs(86400);
    assert_eq!(run.timing().elapsed_display, "1天21分钟19秒");
}
#[test]
fn pending_or_failed_preflight_does_not_invent_a_start_timestamp() {
    let f = Fixture::new();
    f.script("echo unused\n");
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    s.catalog.projects[0].scripts[0].enabled = false;
    let id = s.launch(sid).unwrap();
    assert!(s.runtime.runs[0].timing().started_at.is_none());
    wait(&mut s.runtime, |r| !r.runs[0].state.active());
    let value = api_call(&mut s, "runs.get", serde_json::json!({"id":id}));
    assert!(value["started_at"].is_null());
    assert!(value["started_at_unix_ms"].is_null());
    assert!(
        value["timing_display"]
            .as_str()
            .unwrap()
            .starts_with("未启动 - ")
    );
}

#[test]
fn release_editions_use_separate_data_namespaces_with_an_empty_catalog() {
    let home = std::path::Path::new("/example-home");
    assert_eq!(
        crate::app_identity::default_data_root(home, true, true),
        home.join("Library/Application Support/dev-start")
    );
    assert_eq!(
        crate::app_identity::default_data_root(home, false, true),
        home.join("Library/Application Support/devstart")
    );
    assert_ne!(
        crate::app_identity::default_data_root(home, true, false),
        crate::app_identity::default_data_root(home, false, false)
    );
    let catalog = Catalog::default();
    assert!(catalog.projects.is_empty());
    assert_eq!(catalog.groups.len(), 1);
    assert_eq!(catalog.groups[0].name, "默认分组");
}
