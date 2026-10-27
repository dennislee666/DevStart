//! Real lifecycle regressions for one visible Console per stable launch/service identity.
use super::*;

#[test]
fn completed_closed_and_failed_launches_keep_one_console_beyond_history_limit() {
    let fixture = Fixture::new();
    fixture.script("echo GENERATION_DONE\n");
    let mut service = api_service(&fixture);
    service.import(&fixture.0, 1).unwrap();
    let script = service.catalog.projects[0].scripts[0].id;
    assert_eq!(service.editor_console_run(script), None);
    let mut console = None;
    let mut previous = None;
    for pass in 0..12 {
        service.catalog.projects[0].scripts[0].enabled = pass != 4;
        let run = service.launch(script).unwrap();
        assert_eq!(service.editor_console_run(script), Some(run));
        let stable = *console.get_or_insert(run);
        assert_eq!(service.run_tabs().len(), 1);
        assert_eq!(service.session_tabs().len(), 1);
        assert_eq!(service.run_tabs()[0].console_id, stable);
        if let Some(old) = previous {
            assert_ne!(old, run);
            assert_eq!(service.current_console_run(old), Some(run));
        }
        wait(&mut service.runtime, |runtime| {
            runtime
                .runs
                .iter()
                .any(|r| r.id == run && !r.state.active())
        });
        let current = service.runtime.runs.iter().find(|r| r.id == run).unwrap();
        assert_eq!(
            current.state,
            if pass == 4 {
                State::Failed
            } else {
                State::Succeeded
            }
        );
        assert!(current.log_path.is_file());
        assert_eq!(service.current_console_generation(stable), Some(run));
        assert_eq!(service.editor_console_run(script), Some(run));
        if pass == 2 {
            service.runtime.close_tab(run, false).unwrap();
            assert_eq!(service.editor_console_run(script), None);
        }
        previous = Some(run);
    }
    assert_eq!(service.run_tabs().len(), 1);
    assert_eq!(service.session_tabs().len(), 1);
    assert!(service.runtime.runs.len() <= 9);
}

#[test]
fn ordinary_start_after_stop_and_while_running_reuses_one_console() {
    let fixture = Fixture::new();
    fixture.script("echo SLOT_READY; trap 'exit 0' TERM; while true; do sleep 1; done");
    let mut service = api_service(&fixture);
    service.import(&fixture.0, 1).unwrap();
    let script = service.catalog.projects[0].scripts[0].id;
    let first = service.launch(script).unwrap();
    wait(&mut service.runtime, |runtime| {
        runtime
            .runs
            .iter()
            .any(|run| run.id == first && run.state == State::Running)
    });
    let mut old = first;
    for stop_first in [true, false, true] {
        let old_pid = service
            .runtime
            .runs
            .iter()
            .find(|run| run.id == old)
            .unwrap()
            .pid;
        if stop_first {
            service.runtime.stop(old);
            wait(&mut service.runtime, |runtime| {
                runtime
                    .runs
                    .iter()
                    .any(|run| run.id == old && !run.state.active())
            });
        }
        let current = service.launch(script).unwrap();
        // Repeated clicks during preflight still share the same pending generation.
        assert_eq!(service.launch(script).unwrap(), current);
        assert_eq!(service.run_tabs().len(), 1);
        assert_eq!(service.run_tabs()[0].console_id, first);
        assert_eq!(
            crate::windows::viewport_id(first),
            crate::windows::viewport_id(service.run_tabs()[0].console_id)
        );
        wait(&mut service.runtime, |runtime| {
            runtime
                .runs
                .iter()
                .any(|run| run.id == current && run.state == State::Running)
        });
        assert_eq!(service.active_count(), 1);
        assert_eq!(unsafe { libc::kill(-(old_pid as i32), 0) }, -1);
        old = current;
    }
}

#[test]
fn failed_new_launch_restores_the_original_live_console() {
    let fixture = Fixture::new();
    fixture.script("echo ORIGINAL_READY; while true; do sleep 1; done");
    let mut service = api_service(&fixture);
    service.import(&fixture.0, 1).unwrap();
    let script = service.catalog.projects[0].scripts[0].id;
    let old = service.launch(script).unwrap();
    wait(&mut service.runtime, |runtime| {
        runtime
            .runs
            .iter()
            .any(|run| run.id == old && run.state == State::Running)
    });
    let pid = service
        .runtime
        .runs
        .iter()
        .find(|run| run.id == old)
        .unwrap()
        .pid;
    service.catalog.projects[0].scripts[0].enabled = false;
    let failed = service.launch(script).unwrap();
    wait(&mut service.runtime, |runtime| {
        runtime
            .runs
            .iter()
            .any(|run| run.id == failed && run.state == State::Failed)
    });
    assert_eq!(service.run_tabs().len(), 1);
    assert_eq!(service.run_tabs()[0].id, old);
    assert_eq!(service.run_tabs()[0].console_id, old);
    assert_eq!(service.active_count(), 1);
    assert_eq!(unsafe { libc::kill(-(pid as i32), 0) }, 0);
}

fn visible(runtime: &Runtime) -> Vec<&crate::runtime::Run> {
    runtime
        .runs
        .iter()
        .filter(|run| run.tab_open && !run.superseded)
        .collect()
}
fn replace_outside(server: &mut ExternalServer, runtime: &mut Runtime, old: u64) {
    server.child.kill().unwrap();
    server.child.wait().unwrap();
    wait(runtime, |runtime| {
        runtime
            .runs
            .iter()
            .any(|run| run.id == old && !run.state.active())
    });
    server.child = std::process::Command::new("python3")
        .arg(&server.script.file)
        .args(&server.script.args)
        .current_dir(&server.project.path)
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now();
    while std::net::TcpStream::connect(("127.0.0.1", server.script.expected_port.unwrap())).is_err()
    {
        assert!(deadline.elapsed() < Duration::from_secs(5));
        assert!(server.child.try_wait().unwrap().is_none());
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn external_restarts_reuse_the_slot_and_old_generation_events_remain_isolated() {
    let fixture = Fixture::new();
    let mut server = ExternalServer::new(&fixture);
    let mut alternative = server.script.clone();
    alternative.id += 1;
    server.project.scripts.push(alternative);
    let mut runtime = Runtime::default();
    let first = runtime
        .adopt(server.candidate(), &fixture.0.join("logs"))
        .unwrap();
    let mut previous = first;
    for _ in 0..3 {
        replace_outside(&mut server, &mut runtime, previous);
        let current = runtime
            .adopt(server.candidate(), &fixture.0.join("logs"))
            .unwrap();
        assert_ne!(previous, current);
        assert_eq!(visible(&runtime).len(), 1);
        assert_eq!(visible(&runtime)[0].id, current);
        assert_eq!(visible(&runtime)[0].console_id, first);
        assert_eq!(visible(&runtime)[0].associated_scripts, vec![10, 11]);
        assert_ne!(
            runtime
                .runs
                .iter()
                .find(|run| run.id == previous)
                .unwrap()
                .log_path,
            visible(&runtime)[0].log_path
        );
        runtime.note(previous, "ONLY_OLD_GENERATION");
        runtime.poll();
        assert!(
            !visible(&runtime)[0]
                .lines
                .iter()
                .any(|(_, line)| line.contains("ONLY_OLD_GENERATION"))
        );
        runtime.stop(previous);
        assert!(
            crate::external::same_process(visible(&runtime)[0].external.as_ref().unwrap()).unwrap()
        );
        previous = current;
    }
}

#[test]
fn rediscovering_the_same_external_pid_is_idempotent() {
    let fixture = Fixture::new();
    let server = ExternalServer::new(&fixture);
    let candidate = server.candidate();
    let mut runtime = Runtime::default();
    let first = runtime
        .adopt(candidate.clone(), &fixture.0.join("logs"))
        .unwrap();
    assert_eq!(
        runtime.adopt(candidate, &fixture.0.join("logs")).unwrap(),
        first
    );
    assert_eq!(runtime.runs.len(), 1);
    assert_eq!(visible(&runtime).len(), 1);
}

#[test]
fn external_services_on_different_ports_keep_separate_consoles() {
    let fixture = Fixture::new();
    let mut first = ExternalServer::new(&fixture);
    fs::remove_file(fixture.0.join("bound-port")).unwrap();
    let second = ExternalServer::new(&fixture);
    assert_ne!(first.script.expected_port, second.script.expected_port);
    let mut runtime = Runtime::default();
    let a = runtime
        .adopt(first.candidate(), &fixture.0.join("logs"))
        .unwrap();
    first.child.kill().unwrap();
    first.child.wait().unwrap();
    wait(&mut runtime, |runtime| {
        runtime
            .runs
            .iter()
            .any(|run| run.id == a && !run.state.active())
    });
    let b = runtime
        .adopt(second.candidate(), &fixture.0.join("logs"))
        .unwrap();
    assert_eq!(visible(&runtime).len(), 2);
    assert_ne!(
        runtime
            .runs
            .iter()
            .find(|run| run.id == a)
            .unwrap()
            .console_id,
        runtime
            .runs
            .iter()
            .find(|run| run.id == b)
            .unwrap()
            .console_id
    );
}
