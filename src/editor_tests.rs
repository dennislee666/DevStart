use super::*;
use crate::editor::{Workspace, revision};
use std::os::unix::fs::PermissionsExt;
#[test]
fn source_editor_retains_ten_versions_and_restores_actual_next_launch_source() {
    let f = Fixture::new();
    let script = f.script("#!/bin/bash\necho BASELINE\n");
    fs::set_permissions(&script.file, fs::Permissions::from_mode(0o755)).unwrap();
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let mut doc = s.editor_open(sid).unwrap();
    for n in 0..13 {
        doc = s
            .editor
            .change(
                sid,
                &doc.revision,
                format!("#!/bin/bash\necho VERSION_{n}\n"),
            )
            .unwrap();
        doc = s.editor.save(&s.store.root, sid, &doc.revision).unwrap();
    }
    let versions = crate::editor::versions(&s.store.root, sid, &doc.path).unwrap();
    assert_eq!(versions.len(), 10);
    let selected = versions
        .iter()
        .find(|v| {
            v.content
                .as_ref()
                .is_some_and(|c| c.contains("VERSION_5\n"))
        })
        .unwrap()
        .clone();
    doc = s
        .editor
        .restore(&s.store.root, sid, &doc.revision, selected.id)
        .unwrap();
    assert!(doc.content.contains("VERSION_5\n"));
    assert_eq!(
        fs::metadata(&script.file).unwrap().permissions().mode() & 0o777,
        0o755
    );
    let run = s.launch(sid).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        s.poll();
        let text = fs::read_to_string(
            &s.runtime
                .runs
                .iter()
                .find(|r| r.id == run)
                .unwrap()
                .log_path,
        )
        .unwrap();
        if text.contains("VERSION_5") {
            assert!(text.contains("启动文件已备份"));
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(
        crate::editor::versions(&s.store.root, sid, &doc.path)
            .unwrap()
            .len(),
        10
    );
    s.editor
        .close(&s.store.root, sid, &doc.revision, false)
        .unwrap();
    assert!(!s.editor_list()[0]["open"].as_bool().unwrap());
    s.rescan(pid).unwrap();
    assert_eq!(
        Workspace::load(&s.store.root).unwrap().visits[0].script_id,
        sid
    );
}
#[test]
fn source_editor_detects_external_and_buffer_conflicts_and_protects_dirty_close() {
    let f = Fixture::new();
    let script = f.script("original\r\n");
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let doc = s.editor_open(sid).unwrap();
    let changed = s
        .editor
        .change(sid, &doc.revision, "draft\r\n".into())
        .unwrap();
    assert!(s.editor.change(sid, &doc.revision, "stale".into()).is_err());
    assert!(
        s.editor
            .close(&s.store.root, sid, &changed.revision, false)
            .is_err()
    );
    assert!(s.editor.reload(sid, &changed.revision, false).is_err());
    fs::write(&script.file, "external\r\n").unwrap();
    assert!(
        s.editor
            .save(&s.store.root, sid, &changed.revision)
            .err()
            .unwrap()
            .contains("外部修改")
    );
    assert_eq!(fs::read_to_string(&script.file).unwrap(), "external\r\n");
    assert_eq!(s.editor.read(sid).unwrap().content, "draft\r\n");
    let loaded = s.editor.reload(sid, &changed.revision, true).unwrap();
    assert_eq!(loaded.content, "external\r\n");
    assert!(!loaded.dirty);
    assert_eq!(loaded.revision, revision("external\r\n"));
    s.editor
        .close(&s.store.root, sid, &loaded.revision, false)
        .unwrap();
    assert!(s.editor.read(sid).is_err());
}
#[test]
fn source_editor_api_is_associated_bounded_and_has_safe_versions() {
    let f = Fixture::new();
    f.script("echo original\n");
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let open = api_call(&mut s, "editor.open", serde_json::json!({"script_id":sid}));
    let baseline = api_call(
        &mut s,
        "editor.versions",
        serde_json::json!({"script_id":sid}),
    );
    assert!(baseline[0].get("content").is_none());
    let changed = api_call(
        &mut s,
        "editor.change",
        serde_json::json!({"script_id":sid,"expected_revision":open["revision"],"content":"echo changed\n"}),
    );
    let saved = api_call(
        &mut s,
        "editor.save",
        serde_json::json!({"script_id":sid,"expected_revision":changed["revision"]}),
    );
    let restored = api_call(
        &mut s,
        "editor.restore",
        serde_json::json!({"script_id":sid,"expected_revision":saved["revision"],"version_id":baseline[0]["id"]}),
    );
    assert_eq!(restored["content"], "echo original\n");
    assert!(
        s.editor_action(
            "editor.open",
            &serde_json::json!({"script_id":sid,"path":"/etc/passwd"})
        )
        .is_err()
    );
    assert!(
        s.editor_action("editor.open", &serde_json::json!({"script_id":999}))
            .is_err()
    );
    assert!(
        s.editor
            .change(
                sid,
                restored["revision"].as_str().unwrap(),
                "x".repeat(crate::editor::MAX_BYTES + 1)
            )
            .is_err()
    );
    assert!(
        s.editor
            .change(sid, restored["revision"].as_str().unwrap(), "a\0b".into())
            .is_err()
    );
    let path = s.editor.read(sid).unwrap().path.clone();
    for v in crate::editor::versions(&s.store.root, sid, &path).unwrap() {
        let backup = s
            .store
            .root
            .join("source-versions")
            .join(sid.to_string())
            .join(format!("{}.json", v.id));
        assert_eq!(
            fs::metadata(backup).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}
#[test]
fn source_editor_rejects_reconfigured_source_and_keeps_history_order() {
    let f = Fixture::new();
    f.script("echo a");
    fs::write(f.0.join("start.sh"), "echo b").unwrap();
    let mut s = api_service(&f);
    s.import(&f.0, 1).unwrap();
    let ids: Vec<_> = s.catalog.projects[0].scripts.iter().map(|s| s.id).collect();
    assert_eq!(ids.len(), 2);
    let first = s.editor_open(ids[0]).unwrap();
    let second = s.editor_open(ids[1]).unwrap();
    assert_eq!(s.editor_list()[0]["script_id"], ids[1]);
    s.editor_open(ids[0]).unwrap();
    assert_eq!(s.editor_list()[0]["script_id"], ids[0]);
    s.editor
        .close(&s.store.root, ids[0], &first.revision, false)
        .unwrap();
    assert_eq!(s.editor_list()[0]["script_id"], ids[1]);
    s.catalog.projects[0].scripts[1].file = f.0.join("other.sh");
    fs::write(f.0.join("other.sh"), "echo other").unwrap();
    assert!(
        s.editor_action(
            "editor.save",
            &serde_json::json!({"script_id":ids[1],"expected_revision":second.revision})
        )
        .is_err()
    );
}
#[test]
fn syntect_highlights_mainstream_source_types_without_execution() {
    let ctx = eframe::egui::Context::default();
    let style = ctx.style();
    let theme = egui_extras::syntax_highlighting::CodeTheme::dark(12.0);
    for (lang, code) in [
        ("rs", "fn main() { let n = 1; }"),
        (
            "java",
            "public class Main { public static void main(String[] args) {} }",
        ),
        ("py", "def main():\n    print('hello')"),
        ("js", "const x = 'hello'; // comment"),
        ("html", "<div class=\"x\">hello</div>"),
        ("css", "body { color: red; }"),
        ("json", "{\"name\": \"test\"}"),
        ("sh", "#!/bin/bash\necho \"hello\" # comment"),
    ] {
        let job = egui_extras::syntax_highlighting::highlight(&ctx, &style, &theme, code, lang);
        let colors: std::collections::HashSet<_> =
            job.sections.iter().map(|s| s.format.color).collect();
        assert!(colors.len() > 1, "no syntax colors for {lang}");
    }
}
#[test]
fn editor_java_compiles_only_registered_saved_source_and_preserves_old_classes_on_error() {
    let f = Fixture::new();
    let mut script = f.script("echo placeholder");
    let java = f.0.join("bin/java");
    fs::create_dir_all(java.parent().unwrap()).unwrap();
    let javac = f.0.join("bin/javac");
    fs::write(&javac,"#!/bin/bash\nwhile [ \"$1\" != \"-d\" ]; do shift; done\nshift\nmkdir -p \"$1/example\"\nprintf newclass > \"$1/example/Main.class\"\n").unwrap();
    fs::set_permissions(&javac, fs::Permissions::from_mode(0o755)).unwrap();
    let classes = f.0.join("classes");
    fs::create_dir_all(classes.join("example")).unwrap();
    fs::write(classes.join("example/Main.class"), "oldclass").unwrap();
    let source = f.0.join("Main.java");
    fs::write(&source, "class Main {}\n").unwrap();
    script.file = source.clone();
    script.entry = crate::model::Entry::Java {
        program: java.display().to_string(),
        configuration: "main".into(),
        main_class: "example.Main".into(),
        vm_args: vec![],
        class_paths: vec![classes.clone()],
        classpath_file: None,
        enabled: true,
        manifest: f.0.join("manifest.json"),
        snapshot: String::new(),
    };
    let log = f.0.join("logs/test.log");
    fs::create_dir_all(log.parent().unwrap()).unwrap();
    fs::write(&log, "").unwrap();
    let journal = crate::lifecycle::Journal::new(log, |_, _| {});
    let control = crate::lifecycle::Control::default();
    assert!(
        crate::java_edit::prepare(&script, &f.0, &control, &journal)
            .unwrap()
            .is_none()
    );
    crate::editor::snapshot(
        &f.0,
        script.id,
        &source.canonicalize().unwrap(),
        &fs::read_to_string(&source).unwrap(),
        "save",
    )
    .unwrap();
    let compilation = crate::java_edit::prepare(&script, &f.0, &control, &journal)
        .unwrap()
        .unwrap();
    assert_eq!(
        fs::read_to_string(classes.join("example/Main.class")).unwrap(),
        "oldclass"
    );
    compilation.install(&journal).unwrap();
    assert_eq!(
        fs::read_to_string(classes.join("example/Main.class")).unwrap(),
        "newclass"
    );
    fs::write(&javac, "#!/bin/bash\necho invalid >&2\nexit 1\n").unwrap();
    assert!(crate::java_edit::prepare(&script, &f.0, &control, &journal).is_err());
    assert_eq!(
        fs::read_to_string(classes.join("example/Main.class")).unwrap(),
        "newclass"
    );
}

#[test]
fn config_clients_cannot_delete_unsaved_editor_drafts() {
    let f = Fixture::new();
    f.script("echo original");
    let mut s = api_service(&f);
    let pid = s.import(&f.0, 1).unwrap();
    let sid = s.catalog.projects[0].scripts[0].id;
    let doc = s.editor_open(sid).unwrap();
    let doc = s
        .editor
        .change(sid, &doc.revision, "echo unsaved".into())
        .unwrap();
    assert!(
        crate::api_config::handle(&mut s, "projects.delete", &serde_json::json!({"id":pid}))
            .is_err()
    );
    assert!(s.script(sid).is_some());
    assert_eq!(s.editor.read(sid).unwrap().content, "echo unsaved");
    s.editor
        .close(&s.store.root, sid, &doc.revision, true)
        .unwrap();
    crate::api_config::handle(&mut s, "projects.delete", &serde_json::json!({"id":pid})).unwrap();
}

#[test]
fn editor_soft_wrap_keeps_real_line_numbers_and_exact_source() {
    use eframe::egui::{Color32, FontId, text::LayoutJob};
    let source = format!("\n{}\n\n中文终点\n", "long_token_".repeat(60));
    run_editor_layout_test(|ui| {
        let mut job = LayoutJob::simple(
            source.clone(),
            FontId::monospace(12.0),
            Color32::WHITE,
            f32::INFINITY,
        );
        crate::editor_layout::wrap(&mut job, 120.0);
        let narrow = ui.fonts_mut(|f| f.layout_job(job.clone()));
        assert_eq!(narrow.job.text, source);
        let numbers = crate::editor_layout::line_numbers(&narrow);
        assert_eq!(
            numbers.iter().map(|(n, _)| *n).collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert!(narrow.rows.len() > numbers.len());
        assert!(numbers.windows(2).all(|w| w[1].1 > w[0].1));
        assert!(numbers[2].1 - numbers[1].1 > narrow.rows[0].size.y * 2.0);
        assert!(narrow.rows.iter().all(|r| r.size.x <= 120.5));
        crate::editor_layout::wrap(&mut job, 480.0);
        let wide = ui.fonts_mut(|f| f.layout_job(job));
        assert!(wide.rows.len() < narrow.rows.len());
        assert_eq!(
            crate::editor_layout::line_numbers(&wide)
                .iter()
                .map(|(n, _)| *n)
                .collect::<Vec<_>>(),
            vec![1, 2, 3, 4, 5]
        );
        assert_eq!(wide.job.text, source);
    });
}
#[test]
fn editor_gutter_handles_empty_crlf_and_digit_boundaries() {
    use eframe::egui::{Color32, FontId, text::LayoutJob};
    for (source, count) in [("", 1), ("\n", 2), ("a\r\nb\r\n", 3), ("中文\n\nend", 3)] {
        run_editor_layout_test(|ui| {
            let mut job = LayoutJob::simple(
                source.into(),
                FontId::monospace(12.0),
                Color32::WHITE,
                100.0,
            );
            crate::editor_layout::wrap(&mut job, 100.0);
            let galley = ui.fonts_mut(|f| f.layout_job(job));
            assert_eq!(crate::editor_layout::line_numbers(&galley).len(), count);
            assert_eq!(galley.job.text, source);
        });
    }
    assert_eq!(crate::editor_layout::gutter_digits(""), 2);
    assert_eq!(crate::editor_layout::gutter_digits(&"\n".repeat(98)), 2);
    assert_eq!(crate::editor_layout::gutter_digits(&"\n".repeat(99)), 3);
}

// egui's doctest helper deliberately installs empty fonts, which cannot verify real wrapping.
fn run_editor_layout_test(mut draw: impl FnMut(&mut eframe::egui::Ui)) {
    let ctx = eframe::egui::Context::default();
    let _ = ctx.run(Default::default(), |ctx| {
        eframe::egui::CentralPanel::default().show(ctx, |ui| draw(ui));
    });
}

#[test]
fn editor_history_retains_ten_closed_files_without_evicting_open_documents() {
    let f = Fixture::new();
    let root = f.0.join("state");
    fs::create_dir_all(&root).unwrap();
    let mut workspace = Workspace::default();
    for id in 1..=13 {
        let file = f.0.join(format!("source-{id}.rs"));
        fs::write(&file, "fn main() {}\n").unwrap();
        let doc = workspace.open(&root, id, &file).unwrap();
        workspace.close(&root, id, &doc.revision, false).unwrap();
    }
    assert_eq!(workspace.visits.len(), 10);
    assert_eq!(
        workspace
            .visits
            .iter()
            .map(|v| v.script_id)
            .collect::<Vec<_>>(),
        (4..=13).rev().collect::<Vec<_>>()
    );
    for id in 100..112 {
        let file = f.0.join(format!("source-{id}.py"));
        fs::write(&file, "print('open')\n").unwrap();
        workspace.open(&root, id, &file).unwrap();
    }
    assert_eq!(workspace.documents.len(), 12);
    assert_eq!(workspace.visits.len(), 22);
    assert!(
        workspace
            .documents
            .keys()
            .all(|id| workspace.visits.iter().any(|v| v.script_id == *id))
    );
    let doc = workspace.read(100).unwrap().clone();
    workspace.close(&root, 100, &doc.revision, false).unwrap();
    assert_eq!(workspace.documents.len(), 11);
    let history: Vec<_> = workspace
        .visits
        .iter()
        .filter(|v| !workspace.documents.contains_key(&v.script_id))
        .map(|v| v.script_id)
        .collect();
    assert_eq!(history.len(), 10);
    assert_eq!(history[0], 100);
    assert!(!history.contains(&4));
    let reloaded = Workspace::load(&root).unwrap();
    assert_eq!(reloaded.visits.len(), 10);
    assert!(reloaded.documents.is_empty());
    assert_eq!(reloaded.visits[0].script_id, 100);
}
#[test]
fn editor_history_migrates_old_records_by_recency_and_persists_the_limit() {
    let f = Fixture::new();
    let visits: Vec<_> = (1..=25)
        .map(|id| crate::editor::Visit {
            script_id: id,
            path: f.0.join(format!("{id}.rs")),
            touched: id,
        })
        .collect();
    fs::write(
        f.0.join("editor-history.json"),
        serde_json::to_vec(&visits).unwrap(),
    )
    .unwrap();
    let loaded = Workspace::load(&f.0).unwrap();
    assert_eq!(
        loaded
            .visits
            .iter()
            .map(|v| v.script_id)
            .collect::<Vec<_>>(),
        (16..=25).rev().collect::<Vec<_>>()
    );
    let saved: Vec<crate::editor::Visit> =
        serde_json::from_slice(&fs::read(f.0.join("editor-history.json")).unwrap()).unwrap();
    assert_eq!(saved.len(), 10);
}
