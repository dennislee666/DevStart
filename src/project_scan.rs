//! Bounded, read-only workspace discovery. Reports are immutable import plans, never executable requests.
#[path = "project_scan_detect.rs"]
mod detect;
use crate::model::{Catalog, Project, Script};
pub(crate) use detect::{jsonc as parse_jsonc, read as read_source};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
#[derive(Clone, Serialize)]
pub struct Launch {
    pub id: u64,
    pub script: Script,
    pub command: String,
    pub port: Option<u16>,
    pub port_source: Option<String>,
    pub existing: bool,
    fingerprint: String,
}
#[derive(Clone, Serialize)]
pub struct Candidate {
    pub name: String,
    pub path: PathBuf,
    pub group: String,
    pub stack: String,
    pub launches: Vec<Launch>,
}
#[derive(Clone, Serialize)]
pub struct Report {
    pub root: PathBuf,
    pub projects: Vec<Candidate>,
    pub warnings: Vec<String>,
    pub visited: usize,
    pub truncated: bool,
}
#[derive(Clone, Serialize)]
pub struct Job {
    pub id: u64,
    pub state: String,
    pub report: Option<Report>,
    pub error: Option<String>,
}
struct Task {
    state: Arc<Mutex<Job>>,
    cancel: Arc<AtomicBool>,
}
#[derive(Default)]
pub struct Manager {
    tasks: HashMap<u64, Task>,
}
impl Manager {
    pub fn start(&mut self, root: &Path, catalog: &Catalog) -> Result<u64, String> {
        if self
            .tasks
            .values()
            .any(|t| t.state.lock().is_ok_and(|j| j.state == "running"))
        {
            return Err("A project scan is already running".into());
        }
        let root = if let Ok(relative) = root.strip_prefix("~") {
            PathBuf::from(std::env::var_os("HOME").ok_or("Home directory is unavailable")?)
                .join(relative)
        } else {
            root.to_path_buf()
        };
        let root = root.canonicalize().map_err(|e| e.to_string())?;
        if !root.is_dir() {
            return Err("Scan directory does not exist".into());
        }
        if self.tasks.len() >= 4
            && let Some(id) = self.tasks.keys().copied().min()
        {
            self.tasks.remove(&id);
        }
        let id = crate::model::unique_id();
        let cancel = Arc::new(AtomicBool::new(false));
        let state = Arc::new(Mutex::new(Job {
            id,
            state: "running".into(),
            report: None,
            error: None,
        }));
        self.tasks.insert(
            id,
            Task {
                state: state.clone(),
                cancel: cancel.clone(),
            },
        );
        let catalog = catalog.clone();
        std::thread::spawn(move || {
            let started = Instant::now();
            let result = scan(&root, &catalog, &cancel);
            if let Ok(mut j) = state.lock() {
                match result {
                    Ok(report) => {
                        j.state = "completed".into();
                        j.report = Some(report);
                    }
                    Err(e) => {
                        eprintln!("[dev start scan] {e}");
                        j.state = if cancel.load(Ordering::Relaxed) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                        j.error = Some(e);
                    }
                }
            }
            eprintln!(
                "[dev start scan] job={id} elapsed_ms={}",
                started.elapsed().as_millis()
            );
        });
        Ok(id)
    }
    pub fn get(&self, id: u64) -> Result<Job, String> {
        self.tasks
            .get(&id)
            .ok_or("Scan report expired; scan again")?
            .state
            .lock()
            .map(|v| v.clone())
            .map_err(|e| e.to_string())
    }
    pub fn cancel(&self, id: u64) -> Result<(), String> {
        self.tasks
            .get(&id)
            .ok_or("Scan job not found")?
            .cancel
            .store(true, Ordering::Relaxed);
        Ok(())
    }
}
impl Drop for Manager {
    fn drop(&mut self) {
        for task in self.tasks.values() {
            task.cancel.store(true, Ordering::Relaxed);
        }
    }
}
fn fingerprint(path: &Path) -> Result<String, String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(detect::read(path)?.as_bytes())
    ))
}
fn ignored(name: &str) -> bool {
    name.starts_with('.')
        || [
            "node_modules",
            "vendor",
            "target",
            "dist",
            "build",
            "Pods",
            "venv",
            "env",
            "__pycache__",
            "coverage",
        ]
        .contains(&name)
}
fn marker(path: &Path) -> bool {
    [
        "pubspec.yaml",
        "package.json",
        "composer.json",
        "pyproject.toml",
        "requirements.txt",
        "pom.xml",
        "build.gradle",
        "build.gradle.kts",
        "Cargo.toml",
        "manage.py",
    ]
    .iter()
    .any(|n| path.join(n).is_file())
        || path.join(".vscode/launch.json").is_file()
}
fn scan(root: &Path, catalog: &Catalog, cancel: &AtomicBool) -> Result<Report, String> {
    let started = Instant::now();
    let mut report = Report {
        root: root.into(),
        projects: vec![],
        warnings: vec![],
        visited: 0,
        truncated: false,
    };
    let mut queue = vec![(root.to_path_buf(), 0)];
    let mut roots = vec![];
    while let Some((dir, depth)) = queue.pop() {
        if cancel.load(Ordering::Relaxed) {
            return Err("Scan cancelled".into());
        }
        if report.visited >= 30000 || started.elapsed() > Duration::from_secs(30) {
            report.truncated = true;
            break;
        }
        let entries = match std::fs::read_dir(&dir) {
            Ok(v) => v,
            Err(e) => {
                report.warnings.push(format!("{}: {e}", dir.display()));
                continue;
            }
        };
        let mut conventional = false;
        for entry in entries {
            report.visited += 1;
            if report.visited >= 30000 {
                report.truncated = true;
                break;
            }
            let entry = match entry {
                Ok(v) => v,
                Err(e) => {
                    report.warnings.push(e.to_string());
                    continue;
                }
            };
            let ty = entry.file_type().map_err(|e| e.to_string())?;
            let name = entry.file_name().to_string_lossy().into_owned();
            if ty.is_symlink() {
                continue;
            }
            if ty.is_dir()
                && !ignored(&name)
                && !crate::flutter_launch::is_platform_directory(&dir, &name)
            {
                if depth < 8 {
                    queue.push((entry.path(), depth + 1));
                } else {
                    report.truncated = true;
                }
            }
            if ty.is_file()
                && [
                    "serve.py", "main.py", "app.py", "start.sh", "dev.sh", "serve.sh", "artisan",
                ]
                .contains(&name.as_str())
            {
                conventional = true;
            }
        }
        if marker(&dir) || conventional {
            roots.push(dir);
        }
    }
    roots.sort();
    // Explicit nested manifests form distinct projects; loose source files belong to the nearest project.
    let explicit: Vec<_> = roots.iter().filter(|p| marker(p)).cloned().collect();
    let paths: Vec<_> = roots
        .into_iter()
        .filter(|p| {
            marker(p)
                || !explicit
                    .iter()
                    .any(|parent| p != parent && p.starts_with(parent))
        })
        .collect();
    let mut launch_count = 0;
    let mut remaining_bytes = 4_000_000usize;
    for path in &paths {
        if cancel.load(Ordering::Relaxed) {
            return Err("Scan cancelled".into());
        }
        if report.projects.len() >= 200
            || launch_count >= 2000
            || remaining_bytes < 65536
            || started.elapsed() > Duration::from_secs(30)
        {
            report.truncated = true;
            break;
        }
        let (name, stack, mut scripts) = match detect::project(path) {
            Ok(v) => v,
            Err(e) => {
                if report.warnings.len() < 100 {
                    report.warnings.push(format!("{}: {e}", path.display()));
                }
                continue;
            }
        };
        scripts.retain(|s| {
            !paths.iter().any(|nested| {
                nested != path && nested.starts_with(path) && s.file.starts_with(nested)
            })
        });
        let mut identities = HashSet::new();
        let mut names = HashSet::new();
        let mut launches = vec![];
        for mut script in scripts {
            if !identities.insert(script.identity()) {
                continue;
            }
            if launch_count >= 2000 {
                report.truncated = true;
                break;
            }
            script.name = unique_name(&script.name, &mut names);
            let hint = match crate::configured_ports::resolve(&script) {
                Ok(v) => v,
                Err(e) => {
                    report.warnings.push(e);
                    None
                }
            };
            let port = hint.as_ref().map(|h| h.port).or(script.expected_port);
            let command = script.tab_command();
            let fingerprint = match fingerprint(&script.file) {
                Ok(v) => v,
                Err(e) => {
                    report.warnings.push(e);
                    continue;
                }
            };
            let existing = catalog
                .projects
                .iter()
                .filter(|p| p.path == *path)
                .flat_map(|p| &p.scripts)
                .any(|s| same_launch(s, &script));
            script.expected_port = port;
            let launch = Launch {
                id: crate::model::unique_id(),
                script,
                command,
                port,
                port_source: hint.map(|h| h.source),
                existing,
                fingerprint,
            };
            let size = serde_json::to_vec(&launch)
                .map_err(|e| e.to_string())?
                .len();
            if size > remaining_bytes {
                report.truncated = true;
                break;
            }
            remaining_bytes -= size;
            launches.push(launch);
            launch_count += 1;
        }

        let relative = path.strip_prefix(root).unwrap_or(path);
        let components: Vec<_> = relative.components().collect();
        let group = if components.len() > 1 {
            components[0].as_os_str().to_string_lossy().into_owned()
        } else {
            root.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        };
        report.projects.push(Candidate {
            name,
            path: path.clone(),
            group,
            stack,
            launches,
        });
    }
    report.warnings.truncate(100);
    Ok(report)
}
fn unique_name(base: &str, names: &mut HashSet<String>) -> String {
    if names.insert(base.to_owned()) {
        return base.to_owned();
    }
    for suffix in 2.. {
        let name = format!("{base} ({suffix})");
        if names.insert(name.clone()) {
            return name;
        }
    }
    unreachable!("unbounded suffix sequence")
}
fn same_launch(a: &Script, b: &Script) -> bool {
    a.file == b.file
        && a.invocation() == b.invocation()
        && a.cwd == b.cwd
        && a.environment == b.environment
}
/// Validate the immutable report and every selected source before one service-owned commit.
pub fn apply(catalog: &Catalog, job: Job, ids: &[u64]) -> Result<(Catalog, usize), String> {
    if job.state != "completed" {
        return Err("Scan has not completed".into());
    }
    let report = job.report.ok_or("Missing scan report")?;
    let selected: HashSet<_> = ids.iter().copied().collect();
    if selected.is_empty() || selected.len() != ids.len() {
        return Err("Select unique launch IDs from the scan report".into());
    }
    let known: HashSet<_> = report
        .projects
        .iter()
        .flat_map(|p| &p.launches)
        .map(|l| l.id)
        .collect();
    if !selected.is_subset(&known) {
        return Err("Unknown scan launch ID".into());
    }
    let mut next = catalog.clone();
    let mut added = 0;
    for project in report.projects {
        let mut scripts = vec![];
        let mut names: HashSet<_> = next
            .projects
            .iter()
            .filter(|p| p.path == project.path)
            .flat_map(|p| &p.scripts)
            .map(|s| s.name.clone())
            .collect();
        for launch in project
            .launches
            .into_iter()
            .filter(|l| selected.contains(&l.id))
        {
            if fingerprint(&launch.script.file)? != launch.fingerprint {
                return Err("Launch file changed after scanning; scan again".into());
            }
            if !launch
                .script
                .file
                .canonicalize()
                .map_err(|e| e.to_string())?
                .starts_with(&project.path)
            {
                return Err("Launch source escaped the project directory".into());
            }
            crate::profile::validate(&launch.script)?;
            if next
                .projects
                .iter()
                .filter(|p| p.path == project.path)
                .flat_map(|p| &p.scripts)
                .any(|s| same_launch(s, &launch.script))
            {
                continue;
            }
            let mut script = launch.script;
            script.name = unique_name(&script.name, &mut names);
            script.id = next.id();
            script.custom = true;
            scripts.push(script);
            added += 1;
        }
        if scripts.is_empty() {
            continue;
        }
        if let Some(existing) = next.projects.iter_mut().find(|p| p.path == project.path) {
            existing.scripts.extend(scripts);
            existing.explicit_scripts = true;
            continue;
        }
        let group = if let Some(group) = next.groups.iter().find(|g| g.name == project.group) {
            group.id
        } else {
            let id = next.id();
            next.groups.push(crate::model::Group {
                id,
                name: project.group,
            });
            id
        };
        let id = next.id();
        next.projects.push(Project {
            id,
            display_name: None,
            name: project.name,
            path: project.path,
            group,
            favorite: false,
            scripts,
            description: String::new(),
            explicit_scripts: true,
        });
    }
    Ok((next, added))
}

#[derive(Serialize)]
pub struct Imported {
    pub added: usize,
    pub project_ids: Vec<u64>,
}
/// An explicit import destination; omitted targets preserve scan-inferred groups.
#[derive(Clone, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum ImportGroup {
    Existing { id: u64 },
    New { name: String },
}
/// Resolve grouping on a cloned catalog, so validation failures never persist empty groups.
fn apply_group(catalog: &mut Catalog, projects: &[u64], target: ImportGroup) -> Result<(), String> {
    let group = match target {
        ImportGroup::Existing { id } => {
            if !catalog.groups.iter().any(|g| g.id == id) {
                return Err("Unknown destination group".into());
            }
            id
        }
        ImportGroup::New { name } => {
            let name = name.trim();
            if name.is_empty() {
                return Err("请输入分组名称".into());
            }
            if catalog.groups.iter().any(|g| g.name == name) {
                return Err("分组已存在".into());
            }
            let id = catalog.id();
            catalog.groups.push(crate::model::Group {
                id,
                name: name.into(),
            });
            id
        }
    };
    for project in catalog
        .projects
        .iter_mut()
        .filter(|p| projects.contains(&p.id))
    {
        project.group = group;
    }
    Ok(())
}
impl crate::service::Service {
    pub fn import_scan_grouped(
        &mut self,
        id: u64,
        launch_ids: &[u64],
        target: Option<ImportGroup>,
    ) -> Result<Imported, String> {
        let job = self.scans.get(id)?;
        let (mut next, added) = apply(&self.catalog, job, launch_ids)?;
        let project_ids: Vec<u64> = next
            .projects
            .iter()
            .filter(|p| {
                self.catalog
                    .projects
                    .iter()
                    .find(|old| old.id == p.id)
                    .is_none_or(|old| p.scripts.len() > old.scripts.len())
            })
            .map(|p| p.id)
            .collect();
        if added > 0 {
            if let Some(target) = target {
                // Inferred groups created by apply are not needed for an explicit destination.
                let original: HashSet<_> = self.catalog.groups.iter().map(|g| g.id).collect();
                next.groups.retain(|g| original.contains(&g.id));
                apply_group(&mut next, &project_ids, target)?;
            }
            self.commit(next)?;
        }
        Ok(Imported { added, project_ids })
    }
}

/// Shared authoritative discovery used by group refresh and workspace scanning.
pub(crate) fn rediscover(root: &Path) -> Result<(String, Vec<Script>), String> {
    let (name, _, scripts) = detect::project(root)?;
    Ok((name, scripts))
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Fixture(PathBuf);
    impl Fixture {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!("ds-scan-{}", crate::model::unique_id()));
            std::fs::create_dir_all(&p).unwrap();
            Self(p.canonicalize().unwrap())
        }
        fn write(&self, file: &str, text: &str) {
            let path = self.0.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn flutter_platforms_scan_as_one_app_and_roundtrip_import() {
        let f = Fixture::new();
        f.write(
            "app/pubspec.yaml",
            "name: mobile_demo\ndependencies:\n  flutter:\n    sdk: flutter\n",
        );
        f.write("app/lib/main.dart", "void main() {}\n");
        for platform in ["ios", "android", "ohos", "web"] {
            f.write(&format!("app/{platform}/marker"), "");
        }
        f.write(
            "app/android/app/build.gradle",
            "plugins { id 'com.android.application' }\n",
        );
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(report.projects.len(), 1);
        let app = &report.projects[0];
        assert_eq!(app.name, "mobile_demo");
        assert!(app.stack.contains("Flutter"));
        assert_eq!(app.launches.len(), 7);
        assert!(
            app.launches
                .iter()
                .any(|l| l.script.name.contains("鸿蒙") && l.script.name.contains("真机"))
        );
        assert!(
            app.launches
                .iter()
                .any(|l| l.script.name.contains("iOS") && l.script.name.contains("模拟器"))
        );
        let ids: Vec<_> = app.launches.iter().map(|l| l.id).collect();
        let (catalog, count) = apply(&Catalog::default(), completed(report), &ids).unwrap();
        assert_eq!(count, 7);
        let encoded = serde_json::to_vec(&catalog).unwrap();
        let decoded: Catalog = serde_json::from_slice(&encoded).unwrap();
        assert!(
            decoded.projects[0]
                .scripts
                .iter()
                .all(|s| matches!(s.entry, crate::model::Entry::Flutter { .. }))
        );
        assert!(!f.0.join("app/pubspec.lock").exists());
        assert!(!f.0.join("app/.dart_tool").exists());
    }
    #[test]
    fn android_gradle_is_not_a_java_application_launcher() {
        let f = Fixture::new();
        f.write(
            "native/build.gradle",
            "plugins { id 'com.android.application' }\n",
        );
        f.write("java/build.gradle", "plugins { id 'application' }\n");
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        assert!(
            report
                .projects
                .iter()
                .find(|p| p.path.ends_with("native"))
                .unwrap()
                .launches
                .is_empty()
        );
        assert_eq!(
            report
                .projects
                .iter()
                .find(|p| p.path.ends_with("java"))
                .unwrap()
                .launches
                .len(),
            1
        );
    }
    #[test]
    fn custom_frontend_script_names_keep_declared_commands() {
        let f = Fixture::new();
        f.write("vue/package.json", r#"{"name":"web-demo","dependencies":{"vue":"^3.5.0"},"scripts":{"local":"vite --port 4567","bundle":"vite build","preview":"vite preview"}}"#);
        f.write("uni/package.json", r#"{"name":"uni-demo","dependencies":{"@dcloudio/uni-app":"1"},"scripts":{"h5":"uni -p h5","dev:mp-weixin":"uni -p mp-weixin","build:h5":"uni build -p h5"}}"#);
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        let vue = report
            .projects
            .iter()
            .find(|p| p.name == "web-demo")
            .unwrap();
        assert_eq!(vue.stack, "Vue 3");
        assert_eq!(vue.launches.len(), 2);
        assert!(vue.launches.iter().any(|l| l.port == Some(4567)));
        let uni = report
            .projects
            .iter()
            .find(|p| p.name == "uni-demo")
            .unwrap();
        assert_eq!(uni.stack, "UniApp");
        assert_eq!(uni.launches.len(), 2);
    }
    fn completed(report: Report) -> Job {
        Job {
            id: 1,
            state: "completed".into(),
            report: Some(report),
            error: None,
        }
    }
    #[test]
    fn explicit_grouping_validates_and_moves_only_imported_projects() {
        let mut catalog = Catalog::default();
        let original = catalog.clone();
        assert!(apply_group(&mut catalog, &[], ImportGroup::Existing { id: u64::MAX }).is_err());
        assert!(apply_group(&mut catalog, &[], ImportGroup::New { name: "  ".into() }).is_err());
        assert_eq!(catalog.groups.len(), original.groups.len());
        apply_group(
            &mut catalog,
            &[],
            ImportGroup::New {
                name: " Demo ".into(),
            },
        )
        .unwrap();
        assert!(catalog.groups.iter().any(|g| g.name == "Demo"));
        assert!(
            apply_group(
                &mut catalog,
                &[],
                ImportGroup::New {
                    name: "Demo".into()
                }
            )
            .is_err()
        );
    }
    #[test]
    fn workspace_scan_discovers_stacks_ports_groups_and_skips_dependencies() {
        let f = Fixture::new();
        f.write("business/web/package.json",r#"{"name":"web","dependencies":{"vue":"3"},"scripts":{"dev":"vite --port 5173","build":"vite build"}}"#);
        f.write("business/mobile/package.json",r#"{"name":"mobile","dependencies":{"@dcloudio/uni-app":"3"},"scripts":{"dev:h5":"uni --port 8000"}}"#);
        f.write(
            "business/next/package.json",
            r#"{"name":"site","dependencies":{"next":"15"},"scripts":{"dev":"next dev -p 3001"}}"#,
        );
        f.write(
            "business/api/pom.xml",
            "<project><name>API</name><artifactId>spring-boot-maven-plugin</artifactId></project>",
        );
        f.write(
            "business/api/src/main/resources/application.properties",
            "server.port=8082\n",
        );
        f.write("python/requirements.txt", "fastapi\n");
        f.write(
            "python/main.py",
            "from fastapi import FastAPI\napp = FastAPI()\n",
        );
        f.write(
            "php/composer.json",
            r#"{"name":"demo/php","scripts":{"dev":"php artisan serve"}}"#,
        );
        f.write(
            "business/web/node_modules/hidden/package.json",
            r#"{"scripts":{"dev":"node bad.js"}}"#,
        );
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(report.projects.len(), 6);
        let web = report.projects.iter().find(|p| p.name == "web").unwrap();
        assert_eq!(web.group, "business");
        assert_eq!(web.launches.len(), 1);
        assert_eq!(web.launches[0].port, Some(5173));
        let java = report.projects.iter().find(|p| p.name == "API").unwrap();
        assert_eq!(java.launches[0].port, Some(8082));
        assert!(report.projects.iter().any(|p| p.stack.contains("UniApp")));
        assert!(report.projects.iter().any(|p| p.stack.contains("Next.js")));
        assert!(report.projects.iter().any(|p| p.stack.contains("FastAPI")));
        let ids = vec![web.launches[0].id];
        let job = completed(report);
        let (catalog, n) = apply(&Catalog::default(), job.clone(), &ids).unwrap();
        assert_eq!(n, 1);
        assert_eq!(catalog.projects.len(), 1);
        let (_, n) = apply(&catalog, job, &ids).unwrap();
        assert_eq!(n, 0);
        assert!(!f.0.join("business/web/.dev-start.json").exists());
    }
    #[test]
    fn changed_source_and_unknown_ids_reject_entire_import() {
        let f = Fixture::new();
        f.write("package.json", r#"{"scripts":{"dev":"vite --port 3000"}}"#);
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        let id = report.projects[0].launches[0].id;
        let job = completed(report);
        assert!(apply(&Catalog::default(), job.clone(), &[0]).is_err());
        f.write("package.json", r#"{"scripts":{"dev":"vite --port 3001"}}"#);
        assert!(
            apply(&Catalog::default(), job, &[id])
                .err()
                .unwrap()
                .contains("changed")
        );
        assert!(scan(&f.0, &Catalog::default(), &AtomicBool::new(true)).is_err());
    }
    #[test]
    fn launch_jsonc_preserves_python_args_and_java_classpaths() {
        let f = Fixture::new();
        f.write("serve.py", "print('not executed')\n");
        f.write(".vscode/launch.json",r#"{
            // IDE startup declarations are parsed, not evaluated.
            "configurations":[
                {"type":"debugpy","request":"launch","name":"Preview","program":"${workspaceFolder}/serve.py","args":["--port","4180"],},
                {"type":"java","request":"launch","name":"Java API","mainClass":"demo.Main","classPaths":["${workspaceFolder}/classes"],"vmArgs":"-Dserver.port=8088"},
            ],
        }"#);
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        let launches = &report.projects[0].launches;
        assert_eq!(
            launches
                .iter()
                .find(|l| l.script.name == "Preview")
                .unwrap()
                .port,
            Some(4180)
        );
        assert_eq!(
            launches
                .iter()
                .find(|l| l.script.name == "Java API")
                .unwrap()
                .port,
            Some(8088)
        );
    }
    #[test]
    fn nested_projects_and_symbolic_links_are_not_imported_twice() {
        let f = Fixture::new();
        f.write("package.json", r#"{"scripts":{"dev":"node root.js"}}"#);
        f.write("apps/site/package.json", r#"{"scripts":{"dev":"vite"}}"#);
        let outside = Fixture::new();
        outside.write("package.json", r#"{"scripts":{"dev":"node external.js"}}"#);
        std::os::unix::fs::symlink(&outside.0, f.0.join("linked")).unwrap();
        let report = scan(&f.0, &Catalog::default(), &AtomicBool::new(false)).unwrap();
        assert_eq!(report.projects.len(), 2);
        assert!(report.projects.iter().all(|p| p.launches.len() == 1));
    }
}
