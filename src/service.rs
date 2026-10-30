use crate::{
    discovery,
    model::{Catalog, Group, Project, Script},
    runtime::{Runtime, State},
    store::Store,
};
use std::{path::Path, process::Command};

#[derive(Clone)]
pub struct LaunchTableRow {
    pub run_id: Option<u64>,
    pub group: Group,
    pub project_id: u64,
    pub project_name: String,
    pub project_path: std::path::PathBuf,
    pub project_favorite: bool,
    pub script: Option<Script>,
}

/// Filtered project views keep project identity and ordering in the service layer.
pub struct LaunchProjectSection {
    pub group: Group,
    pub project_favorite: bool,
    pub project_id: u64,
    pub project_name: String,
    pub project_path: std::path::PathBuf,
    pub rows: Vec<LaunchTableRow>,
}
impl LaunchProjectSection {
    pub fn action_target(&self) -> LaunchTableRow {
        LaunchTableRow {
            run_id: None,
            group: self.group.clone(),
            project_id: self.project_id,
            project_name: self.project_name.clone(),
            project_path: self.project_path.clone(),
            project_favorite: self.project_favorite,
            script: None,
        }
    }
}
pub enum LaunchTableContent {
    Flat(Vec<LaunchTableRow>),
    Projects(Vec<LaunchProjectSection>),
}
impl LaunchTableContent {
    pub fn is_empty(&self) -> bool {
        match self {
            Self::Flat(rows) => rows.is_empty(),
            Self::Projects(projects) => projects.is_empty(),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum LaunchSortColumn {
    Group,
    Project,
    Status,
    Port,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Ascending,
    Descending,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct LaunchSort {
    pub column: LaunchSortColumn,
    pub direction: SortDirection,
}

impl LaunchSort {
    /// The selected group is constant, so its default ordering becomes project name.
    pub fn for_project_sections(self) -> Self {
        if self.column == LaunchSortColumn::Group {
            Self {
                column: LaunchSortColumn::Project,
                direction: SortDirection::Ascending,
            }
        } else {
            self
        }
    }
}
impl Default for LaunchSort {
    fn default() -> Self {
        Self {
            column: LaunchSortColumn::Group,
            direction: SortDirection::Ascending,
        }
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum Filter {
    All,
    Favorites,
    Git,
    Running,
    Terminal,
    Settings,
    Editor,
}
pub struct Service {
    pub scans: crate::project_scan::Manager,
    pub desktop_terminals: crate::desktop_terminal::Manager,
    pub terminals: crate::project_terminal::Manager,
    pub editor: crate::editor::Workspace,
    pub catalog: Catalog,
    pub runtime: Runtime,
    pub store: Store,
    pub port_hints: crate::configured_ports::Cache,
}
impl Service {
    pub fn load() -> Result<Self, String> {
        let store = Store::new().map_err(|e| e.to_string())?;
        #[cfg(feature = "personal")]
        let catalog = store.load()?;
        #[cfg(not(feature = "personal"))]
        let catalog = crate::demo_workspace::load(&store)?;
        let editor = crate::editor::Workspace::load(&store.root)?;
        let port_hints = crate::configured_ports::collect(&catalog.projects);
        Ok(Self {
            scans: Default::default(),
            terminals: Default::default(),
            desktop_terminals: Default::default(),
            editor,
            port_hints,
            catalog,
            runtime: Runtime::default(),
            store,
        })
    }
    /// Commit configuration only after durable storage succeeds.
    pub(crate) fn commit(&mut self, next: Catalog) -> Result<(), String> {
        self.commit_config(next, None)
    }
    fn commit_config(
        &mut self,
        mut next: Catalog,
        refreshed_project: Option<u64>,
    ) -> Result<(), String> {
        // Refuse stale-cache overwrites. Explicit refresh is the only operation
        // that accepts a user-edited managed document as the new definition.
        for project in &self.catalog.projects {
            if refreshed_project == Some(project.id)
                || !next.projects.iter().any(|p| p.id == project.id)
            {
                continue;
            }
            let path = self.store.manifest_path(project.id);
            let actual: serde_json::Value = serde_json::from_slice(
                &std::fs::read(&path)
                    .map_err(|e| format!("读取DevStart启动配置 {}: {e}", path.display()))?,
            )
            .map_err(|e| format!("启动配置解析失败: {e}"))?;
            let expected: serde_json::Value =
                serde_json::from_slice(&crate::project_manifest::encode(
                    crate::project_manifest::storage_specifications(project, &path)?,
                )?)
                .map_err(|e| e.to_string())?;
            if actual != expected {
                return Err(format!(
                    "DevStart启动配置已被外部修改，请先重新读取启动脚本: {}",
                    path.display()
                ));
            }
        }
        // Configuration clients must not remove or remap a source with an unsaved editor draft.
        let detached: Vec<_> = self
            .editor
            .documents
            .keys()
            .copied()
            .filter(|id| {
                let before = self.script(*id);
                let after = next
                    .projects
                    .iter()
                    .flat_map(|p| &p.scripts)
                    .find(|s| s.id == *id);
                match (before, after) {
                    (Some(a), Some(b)) => a.file != b.file,
                    _ => true,
                }
            })
            .collect();
        if detached
            .iter()
            .any(|id| self.editor.documents.get(id).is_some_and(|d| d.dirty))
        {
            return Err("启动文件有未保存的编辑，请先保存或关闭再删除/变更源文件".into());
        }
        for project in &mut next.projects {
            let unchanged_execution = self
                .catalog
                .projects
                .iter()
                .find(|old| old.id == project.id)
                .is_some_and(|old| {
                    old.scripts.len() == project.scripts.len()
                        && old
                            .scripts
                            .iter()
                            .zip(&project.scripts)
                            .all(|(a, b)| a.name == b.name && a.identity() == b.identity())
                });
            if !unchanged_execution {
                let bytes = crate::project_manifest::encode(
                    crate::project_manifest::specifications(project)?,
                )?;
                crate::project_manifest::bind(
                    project,
                    &self.store.manifest_path(project.id),
                    &bytes,
                )?;
            }
        }
        let next = self.store.normalize(next)?;
        let hints = crate::configured_ports::collect(&next.projects);
        self.store.save(&next)?;
        for id in detached {
            self.editor.documents.remove(&id);
        }
        self.catalog = next;
        self.port_hints = hints;
        Ok(())
    }
    pub fn update_preferences(
        &mut self,
        patch: serde_json::Value,
    ) -> Result<crate::preferences::Preferences, String> {
        let mut value =
            serde_json::to_value(&self.catalog.preferences).map_err(|e| e.to_string())?;
        for (key, val) in patch.as_object().ok_or("设置patch需要对象")? {
            value[key] = val.clone();
        }
        let preferences: crate::preferences::Preferences =
            serde_json::from_value(value).map_err(|e| e.to_string())?;
        preferences.validate()?;
        let mut next = self.catalog.clone();
        next.preferences = preferences.clone();
        self.commit(next)?;
        Ok(preferences)
    }
    /// First-discovery learning never overrides a later explicit user preference or Auto mode.
    pub fn remember_commit_agent(
        &mut self,
        requested: crate::preferences::CommitAgent,
        used: crate::preferences::CommitAgent,
    ) -> Result<bool, String> {
        use crate::preferences::CommitAgent;
        if requested != CommitAgent::Discover || self.catalog.preferences.commit_agent != requested
        {
            return Ok(false);
        }
        if matches!(used, CommitAgent::Discover | CommitAgent::Auto) {
            return Err("A concrete successful agent is required".into());
        }
        self.update_preferences(serde_json::json!({"commit_agent":used}))?;
        Ok(true)
    }
    pub fn import_profile(&mut self, path: &Path) -> Result<(), String> {
        let profile: crate::profile::Profile =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("profile 解析失败: {e}"))?;
        let next = crate::profile::merge(&self.catalog, profile)?;
        self.commit(next)
    }
    pub fn add_group(&mut self, name: &str) -> Result<(), String> {
        let name = name.trim();
        if name.is_empty() {
            return Err("请输入分组名称".into());
        }
        if self.catalog.groups.iter().any(|g| g.name == name) {
            return Err("分组已存在".into());
        }
        let mut next = self.catalog.clone();
        let id = next.id();
        next.groups.push(Group {
            id,
            name: name.into(),
        });
        self.commit(next)
    }
    pub fn import(&mut self, path: &Path, group: u64) -> Result<u64, String> {
        self.import_configured(path, group, None, "", false)
    }
    pub(crate) fn import_configured(
        &mut self,
        path: &Path,
        group: u64,
        label: Option<&str>,
        description: &str,
        favorite: bool,
    ) -> Result<u64, String> {
        if label.is_some_and(|v| v.trim().is_empty()) {
            return Err("项目名不能为空".into());
        }
        if !self.catalog.groups.iter().any(|g| g.id == group) {
            return Err("分组不存在".into());
        }
        let path = path
            .canonicalize()
            .map_err(|e| format!("项目目录 {}: {e}", path.display()))?;
        if self.catalog.projects.iter().any(|p| p.path == path) {
            return Err("项目已导入".into());
        }
        let mut scripts = discovery::discover(&path)?;
        let mut next = self.catalog.clone();
        for script in &mut scripts {
            script.id = next.id();
        }
        let id = next.id();
        let name = path
            .file_name()
            .ok_or("无效项目目录")?
            .to_string_lossy()
            .into_owned();
        next.projects.push(Project {
            description: description.into(),
            display_name: None,
            id,
            name: label.unwrap_or(&name).into(),
            path,
            group,
            favorite,
            scripts,
            explicit_scripts: false,
        });
        self.commit(next)?;
        Ok(id)
    }
    pub fn rescan(&mut self, id: u64) -> Result<(), String> {
        let old = self
            .catalog
            .projects
            .iter()
            .find(|p| p.id == id)
            .ok_or("项目不存在")?
            .clone();
        // Both modes require the app-owned manifest. Native discovery is an
        // explicit mode, never a fallback when managed configuration is missing.
        let path = self.store.manifest_path(id);
        let bytes = std::fs::read(&path)
            .map_err(|e| format!("读取DevStart启动配置 {}: {e}", path.display()))?;
        crate::project_manifest::validate_document(&bytes)?;
        let has_manifest = old.explicit_scripts;
        let mut scripts = if has_manifest {
            crate::project_manifest::discover(&old.path, &path)?
        } else {
            discovery::discover(&old.path)?
        };
        let mut next = self.catalog.clone();
        for s in &mut scripts {
            if let Some(prev) = old.scripts.iter().find(|p| {
                if has_manifest {
                    p.name == s.name
                } else {
                    !p.custom && p.source_identity() == s.source_identity()
                }
            }) {
                s.id = prev.id;
                s.custom = prev.custom;
                s.favorite = prev.favorite;
                s.display_name = prev.display_name.clone();
                s.description_override = prev.description_override.clone();
                if let Some(text) = &s.description_override {
                    s.description = text.clone();
                } else if !has_manifest {
                    s.description = prev.description.clone();
                }
                if !has_manifest {
                    s.args = prev.args.clone();
                    s.environment = prev.environment.clone();
                }
                if !has_manifest {
                    s.expected_port = prev.expected_port;
                }
            } else {
                s.id = next.id();
            }
        }
        for configured in old.scripts.iter().filter(|_| !has_manifest) {
            if configured.custom
                && !scripts
                    .iter()
                    .any(|s| s.identity() == configured.identity())
            {
                scripts.push(configured.clone());
            }
        }
        next.projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("项目不存在")?
            .scripts = scripts;
        self.commit_config(next, Some(id))
    }
    pub fn project_favorite(&mut self, id: u64) -> Result<(), String> {
        let mut next = self.catalog.clone();
        let p = next
            .projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("项目不存在")?;
        p.favorite = !p.favorite;
        self.commit(next)
    }
    pub fn script_favorite(&mut self, id: u64) -> Result<(), String> {
        let mut next = self.catalog.clone();
        let s = next
            .projects
            .iter_mut()
            .flat_map(|p| &mut p.scripts)
            .find(|s| s.id == id)
            .ok_or("脚本不存在")?;
        s.favorite = !s.favorite;
        self.commit(next)
    }
    pub fn move_project(&mut self, id: u64, group: u64) -> Result<(), String> {
        let mut next = self.catalog.clone();
        if !next.groups.iter().any(|g| g.id == group) {
            return Err("分组不存在".into());
        }
        next.projects
            .iter_mut()
            .find(|p| p.id == id)
            .ok_or("项目不存在")?
            .group = group;
        self.commit(next)
    }
    pub fn describe(&mut self, id: u64, text: String) -> Result<(), String> {
        self.update_script_info(id, None, Some(text))
    }
    /// Atomically update presentation metadata without modifying launch definitions.
    pub fn update_script_info(
        &mut self,
        id: u64,
        name: Option<Option<String>>,
        description: Option<String>,
    ) -> Result<(), String> {
        let name = name.map(Script::validate_display_name).transpose()?;
        let mut next = self.catalog.clone();
        let script = next
            .projects
            .iter_mut()
            .flat_map(|p| &mut p.scripts)
            .find(|s| s.id == id)
            .ok_or("启动项不存在")?;
        if let Some(name) = name {
            script.display_name = name;
        }
        if let Some(text) = description {
            script.description = text.clone();
            script.description_override = Some(text);
        }
        self.commit(next)?;
        self.sync_display_names();
        Ok(())
    }
    pub fn update_launch_info(
        &mut self,
        id: u64,
        name: Option<Option<String>>,
        project_name: Option<Option<String>>,
        description: String,
    ) -> Result<(), String> {
        let name = name.map(Script::validate_display_name).transpose()?;
        let project_name = project_name
            .map(Project::validate_display_name)
            .transpose()?;
        let mut next = self.catalog.clone();
        let project = next
            .projects
            .iter_mut()
            .find(|p| p.scripts.iter().any(|s| s.id == id))
            .ok_or("启动项不存在")?;
        if let Some(project_name) = project_name {
            project.display_name = project_name;
        }
        let script = project
            .scripts
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("启动项不存在")?;
        if let Some(name) = name {
            script.display_name = name;
        }
        script.description = description.clone();
        script.description_override = Some(description);
        self.commit(next)?;
        self.sync_display_names();
        Ok(())
    }
    pub fn generate_description(&mut self, id: u64) -> Result<(), String> {
        let script = self.script(id).ok_or("脚本不存在")?;
        let source = std::fs::read_to_string(&script.file).map_err(|e| e.to_string())?;
        let comment = source
            .lines()
            .take(24)
            .filter_map(|l| l.trim().strip_prefix('#'))
            .map(str::trim)
            .filter(|l| !l.is_empty() && !l.starts_with('!'))
            .take(3)
            .collect::<Vec<_>>()
            .join("；");
        let intro = if comment.is_empty() {
            format!("运行 {}", script.invocation())
        } else {
            comment
        };
        let evidence = self
            .runtime
            .runs
            .iter()
            .rev()
            .find(|r| !r.superseded && r.script_id == id)
            .map(|r| {
                if r.state.active() {
                    format!("；最近执行：{}", r.state.label())
                } else {
                    format!(
                        "；最近执行：{}，{}",
                        r.state.label(),
                        crate::runtime::exit_label(r.exit)
                    )
                }
            })
            .unwrap_or_default();
        self.describe(id, format!("{intro}{evidence}"))
    }
    pub fn add_script(
        &mut self,
        project_id: u64,
        path: &Path,
        interpreter: &str,
    ) -> Result<(), String> {
        let file = path
            .canonicalize()
            .map_err(|e| format!("项目目录 {}: {e}", path.display()))?;
        if !file.is_file() {
            return Err("请选择脚本文件".into());
        }
        if interpreter.trim().is_empty() {
            return Err("请填写解释器".into());
        }
        let mut next = self.catalog.clone();
        let id = next.id();
        let p = next
            .projects
            .iter_mut()
            .find(|p| p.id == project_id)
            .ok_or("项目不存在")?;
        if p.scripts.iter().any(|s| s.file == file) {
            return Err("脚本已存在".into());
        }
        p.scripts.push(Script {
            display_name: None,
            description_override: None,
            enabled: true,
            display_command: None,
            id,
            name: file
                .file_name()
                .ok_or("无效路径")?
                .to_string_lossy()
                .into_owned(),
            cwd: p.path.clone(),
            file,
            entry: crate::model::Entry::File {
                interpreter: interpreter.trim().into(),
            },
            description: "执行项目原有脚本".into(),
            favorite: false,
            environment: Default::default(),
            expected_port: None,
            args: vec![],
            custom: false,
            resource_key: None,
        });
        self.commit(next)
    }
    pub fn script(&self, id: u64) -> Option<&Script> {
        self.catalog
            .projects
            .iter()
            .flat_map(|p| &p.scripts)
            .find(|s| s.id == id)
    }
    pub fn launch(&mut self, id: u64) -> Result<u64, String> {
        let p = self
            .catalog
            .projects
            .iter()
            .find(|p| p.scripts.iter().any(|s| s.id == id))
            .ok_or("项目不存在")?;
        let mut s = p
            .scripts
            .iter()
            .find(|s| s.id == id)
            .ok_or("脚本不存在")?
            .clone();
        // Apply statically inferred ports to this launch only; project config stays authoritative and unchanged.
        if s.expected_port.is_none()
            && let Ok(Some(hint)) = self.configured_port_hint(id)
        {
            s.expected_port = Some(hint.port);
        }
        let version = if s.launch_block_reason().is_none() {
            Some(self.snapshot_launch(id)?)
        } else {
            None
        };
        let run = self
            .runtime
            .launch_checked(p, &s, self.store.root.join("logs"))?;
        if let Some(version) = version {
            let note = if version.created {
                format!(
                    "启动文件已备份 · 版本 {} · {}",
                    version.id,
                    version.path.display()
                )
            } else {
                format!("启动文件未变化，跳过备份 · 版本 {}", version.id)
            };
            self.runtime.note(run, note);
        }
        Ok(run)
    }
    /// Resolve terminal ownership from registered launch entries, then exact known project paths.
    /// Historical runs whose projects were removed fail explicitly instead of opening another directory.
    pub fn run_project_context(&self, id: u64) -> Result<(u64, std::path::PathBuf), String> {
        let run = self
            .runtime
            .runs
            .iter()
            .find(|r| r.id == id)
            .ok_or("启动记录不存在")?;
        let project = self
            .catalog
            .projects
            .iter()
            .find(|p| {
                p.scripts
                    .iter()
                    .any(|s| s.id == run.script_id || run.associated_scripts.contains(&s.id))
            })
            .or_else(|| self.catalog.projects.iter().find(|p| p.path == run.cwd))
            .ok_or("启动记录对应的项目不存在")?;
        Ok((project.id, project.path.clone()))
    }
    pub fn poll(&mut self) {
        self.desktop_terminals.poll();
        self.terminals.retain_projects(
            &self
                .catalog
                .projects
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
        );
        if let Err(error) = crate::log_store::maintain(&self.store.root.join("logs"), false) {
            self.runtime.error = Some(format!("日志保留策略维护失败: {error}"));
        }
        self.runtime.poll();
        self.runtime
            .refresh_external(&self.catalog.projects, self.store.root.join("logs"));
    }
    pub fn can_restart_target(&self, run_id: u64, script_id: u64) -> bool {
        self.runtime
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .is_some_and(|run| {
                !run.superseded
                    && !run.close_requested
                    && !matches!(run.state, State::Stopping | State::Starting)
                    && run.associated_scripts.contains(&script_id)
                    && self
                        .script(script_id)
                        .is_some_and(|s| s.launch_block_reason().is_none())
            })
    }
    pub fn can_restart(&self, run_id: u64) -> bool {
        self.runtime
            .runs
            .iter()
            .find(|r| r.id == run_id)
            .is_some_and(|run| {
                run.associated_scripts
                    .iter()
                    .any(|id| self.can_restart_target(run_id, *id))
            })
    }
    pub fn can_restart_script(&self, script_id: u64) -> bool {
        self.runtime
            .runs
            .iter()
            .rev()
            .find(|r| !r.superseded && r.associated_scripts.contains(&script_id))
            .is_some_and(|r| self.can_restart_target(r.id, script_id))
    }
    pub fn restart(&mut self, script_id: u64) -> Result<u64, String> {
        let run = self
            .active_run(script_id)
            .or_else(|| {
                self.runtime
                    .runs
                    .iter()
                    .rev()
                    .find(|r| !r.superseded && r.associated_scripts.contains(&script_id))
                    .map(|r| r.id)
            })
            .ok_or("暂无启动记录")?;
        self.restart_run(run, script_id)
    }
    pub fn restart_run(&mut self, run: u64, script_id: u64) -> Result<u64, String> {
        let mut script = self.script(script_id).ok_or("启动项不存在")?.clone();
        if script.expected_port.is_none()
            && let Ok(Some(hint)) = self.configured_port_hint(script_id)
        {
            script.expected_port = Some(hint.port);
        }
        let project = self
            .catalog
            .projects
            .iter()
            .find(|p| p.scripts.iter().any(|s| s.id == script_id))
            .ok_or("项目不存在")?;
        let target = self
            .runtime
            .runs
            .iter()
            .find(|r| r.id == run)
            .ok_or("运行实例不存在")?;
        if !target.associated_scripts.contains(&script_id)
            || !self.can_restart_target(run, script_id)
        {
            return Err("该运行记录当前不可重启，请操作最新实例".into());
        }
        let version = self.snapshot_launch(script_id)?;
        let id =
            self.runtime
                .restart_checked(run, project, &script, self.store.root.join("logs"))?;
        let note = if version.created {
            format!(
                "启动文件已备份 · 版本 {} · {}",
                version.id,
                version.path.display()
            )
        } else {
            format!("启动文件未变化，跳过备份 · 版本 {}", version.id)
        };
        self.runtime.note(id, note);
        Ok(id)
    }
    pub fn create_preset(
        &mut self,
        source: u64,
        name: &str,
        args: &str,
        port: &str,
    ) -> Result<u64, String> {
        let port = if port.trim().is_empty() {
            None
        } else {
            Some(
                port.trim()
                    .parse::<u16>()
                    .map_err(|_| "端口需为 1–65535 的整数".to_string())?,
            )
        };
        if port == Some(0) {
            return Err("端口需为 1–65535 的整数".into());
        }
        self.add_preset(
            source,
            name,
            args.lines().map(str::to_owned).collect(),
            port,
        )
    }
    pub fn add_preset(
        &mut self,
        source_id: u64,
        name: &str,
        args: Vec<String>,
        expected_port: Option<u16>,
    ) -> Result<u64, String> {
        if name.trim().is_empty() {
            return Err("请输入启动项名称".into());
        }
        let mut next = self.catalog.clone();
        let mut preset = self.script(source_id).ok_or("启动项不存在")?.clone();
        preset.name = name.trim().into();
        preset.display_name = None;
        preset.args = args;
        preset.expected_port = expected_port;
        preset.custom = true;
        crate::profile::validate(&preset)?;
        let project = next
            .projects
            .iter()
            .find(|p| p.scripts.iter().any(|s| s.id == source_id))
            .ok_or("项目不存在")?;
        if project
            .scripts
            .iter()
            .any(|s| s.identity() == preset.identity())
        {
            return Err("相同命令和参数的启动项已存在".into());
        }
        preset.id = next.id();
        let id = preset.id;
        next.projects
            .iter_mut()
            .find(|p| p.scripts.iter().any(|s| s.id == source_id))
            .ok_or("项目不存在")?
            .scripts
            .push(preset);
        self.commit(next)?;
        Ok(id)
    }
    pub fn set_display_name(&mut self, id: u64, name: Option<String>) -> Result<(), String> {
        self.update_script_info(id, Some(name), None)
    }
    pub fn sync_display_names(&mut self) {
        for run in &mut self.runtime.runs {
            if let Some((project, script)) = self.catalog.projects.iter().find_map(|p| {
                p.scripts
                    .iter()
                    .find(|s| s.id == run.script_id)
                    .map(|s| (p, s))
            }) {
                if run.external.is_some() {
                    run.launch_name = format!("{} · 外部运行", project.display_label());
                    run.title = format!("{} / 外部运行（参数未识别）", project.display_label());
                } else {
                    run.launch_name = script.display_label().to_owned();
                    run.title = format!("{} / {}", project.display_label(), script.display_label());
                }
            }
        }
    }
    pub fn session_tabs(&self) -> Vec<crate::presentation::RunTab> {
        self.runtime
            .runs
            .iter()
            .filter(|run| !run.superseded)
            .map(|run| crate::presentation::RunTab {
                console_id: run.console_id,
                id: run.id,
                label: crate::presentation::tab_label(&run.launch_name),
                full_name: run.launch_name.clone(),
                tooltip: crate::presentation::run_tooltip(
                    &run.launch_name,
                    &run.display_command,
                    &path_label(
                        self.catalog
                            .projects
                            .iter()
                            .find(|p| {
                                p.scripts
                                    .iter()
                                    .any(|s| run.associated_scripts.contains(&s.id))
                            })
                            .map(|p| p.path.as_path())
                            .unwrap_or(&run.cwd),
                    ),
                ),
                state: run.state,
            })
            .collect()
    }
    /// Locale-specific DTOs keep user names, arguments, identifiers and API defaults stable.
    pub fn localize_tabs(
        &self,
        mut tabs: Vec<crate::presentation::RunTab>,
        locale: crate::i18n::Locale,
    ) -> Vec<crate::presentation::RunTab> {
        for tab in &mut tabs {
            let Some(run) = self
                .runtime
                .runs
                .iter()
                .find(|run| run.id == tab.id && run.external.is_some())
            else {
                continue;
            };
            if let Some(project) = self
                .catalog
                .projects
                .iter()
                .find(|p| p.scripts.iter().any(|s| s.id == run.script_id))
            {
                tab.full_name = locale.format(
                    "{project} · 外部运行",
                    &[("project", project.display_label().to_owned())],
                );
                tab.label = crate::presentation::tab_label(&tab.full_name);
                tab.tooltip = crate::presentation::run_tooltip(
                    &tab.full_name,
                    &run.display_command,
                    &path_label(&project.path),
                );
            }
        }
        tabs
    }
    /// Navigation keeps active sessions above history without reordering workspace tabs.
    pub fn sidebar_sessions(&self) -> Vec<crate::presentation::RunTab> {
        let mut sessions = self.session_tabs();
        sessions.sort_by_key(|tab| (!tab.state.active(), std::cmp::Reverse(tab.id)));
        let mut history = 0;
        sessions.retain(|tab| {
            if tab.state.active() {
                true
            } else {
                history += 1;
                history <= 8
            }
        });
        sessions
    }
    pub fn run_tabs(&self) -> Vec<crate::presentation::RunTab> {
        let open: std::collections::HashSet<_> = self
            .runtime
            .runs
            .iter()
            .filter(|r| r.tab_open)
            .map(|r| r.id)
            .collect();
        self.session_tabs()
            .into_iter()
            .filter(|t| open.contains(&t.id))
            .collect()
    }
    /// Keep editor output attached to the latest generation of its launch item.
    pub fn editor_console_run(&self, script_id: u64) -> Option<u64> {
        self.runtime
            .runs
            .iter()
            .rev()
            .find(|run| {
                !run.superseded
                    && run.tab_open
                    && (run.script_id == script_id || run.associated_scripts.contains(&script_id))
            })
            .map(|run| run.id)
    }
    pub fn current_console_generation(&self, console_id: u64) -> Option<u64> {
        self.runtime
            .runs
            .iter()
            .find(|run| run.console_id == console_id && run.tab_open && !run.superseded)
            .map(|run| run.id)
    }
    pub fn current_console_run(&self, id: u64) -> Option<u64> {
        let console_id = self.runtime.runs.iter().find(|r| r.id == id)?.console_id;
        self.current_console_generation(console_id)
    }
    pub fn selected_run(&self, selected: Option<u64>) -> Option<u64> {
        selected
            .and_then(|id| self.current_console_run(id))
            .or_else(|| {
                self.runtime
                    .runs
                    .iter()
                    .rev()
                    .find(|r| r.tab_open)
                    .map(|r| r.id)
            })
    }
    pub fn has_external_run(&self, project_id: u64) -> bool {
        self.catalog
            .projects
            .iter()
            .find(|p| p.id == project_id)
            .is_some_and(|p| {
                self.runtime.runs.iter().any(|r| {
                    r.state.active()
                        && r.external.is_some()
                        && p.scripts
                            .iter()
                            .any(|s| r.associated_scripts.contains(&s.id))
                })
            })
    }
    pub fn configured_port_hint(
        &self,
        script_id: u64,
    ) -> Result<Option<crate::configured_ports::Hint>, String> {
        if let Some(hint) = self.port_hints.get(&script_id) {
            return hint.clone();
        }
        Ok(self
            .script(script_id)
            .and_then(|s| s.expected_port)
            .map(|port| crate::configured_ports::Hint {
                port,
                source: "启动清单 expected_port".into(),
            }))
    }
    pub fn running_port_cell(&self, script_id: u64) -> crate::presentation::PortCell {
        self.running_port_cell_for_locale(script_id, crate::i18n::Locale::Chinese)
    }
    pub fn running_port_cell_for_locale(
        &self,
        script_id: u64,
        locale: crate::i18n::Locale,
    ) -> crate::presentation::PortCell {
        let ports = self.running_ports(script_id);
        let configured = self.configured_port_hint(script_id);
        if !ports.is_empty() {
            let label = ports
                .iter()
                .map(u16::to_string)
                .collect::<Vec<_>>()
                .join(", ");
            let mut tooltip = locale.format(
                "实际 TCP 监听端口（含子进程）：{label}",
                &[("label", label.clone())],
            );
            if let Ok(Some(hint)) = configured {
                tooltip.push_str(&locale.format(
                    "\n配置端口：{0}\n来源：{1}",
                    &[
                        ("0", hint.port.to_string()),
                        ("1", locale.text(&hint.source).to_owned()),
                    ],
                ));
            }
            crate::presentation::PortCell { label, tooltip }
        } else if let Ok(Some(hint)) = configured {
            crate::presentation::PortCell {
                label: hint.port.to_string(),
                tooltip: locale.format(
                    "配置端口：{0}\n来源：{1}\n尚未确认实际监听",
                    &[
                        ("0", hint.port.to_string()),
                        ("1", locale.text(&hint.source).to_owned()),
                    ],
                ),
            }
        } else {
            crate::presentation::PortCell {
                label: "--".into(),
                tooltip: configured
                    .err()
                    .map(|e| locale.format("读取端口配置失败：{e}", &[("e",e)]))
                    .unwrap_or_else(|| {
                        locale.text("无法静态确定启动端口，且尚未确认实际监听；可在启动清单中声明 expected_port").to_owned()
                    }),
            }
        }
    }
    pub fn running_ports(&self, script_id: u64) -> Vec<u16> {
        self.runtime
            .runs
            .iter()
            .filter(|r| r.state.active() && r.associated_scripts.contains(&script_id))
            .flat_map(|r| r.listening_ports.iter().copied())
            .collect::<std::collections::BTreeSet<_>>()
            .into_iter()
            .collect()
    }
    pub fn script_status_label(&self, id: u64) -> &'static str {
        if self.runtime.runs.iter().any(|r| {
            !r.superseded
                && r.state == State::Running
                && r.associated_scripts.contains(&id)
                && (r.external.is_none() || r.associated_scripts.len() == 1)
        }) {
            "运行中"
        } else {
            "--"
        }
    }
    pub fn active_run(&self, id: u64) -> Option<u64> {
        self.runtime
            .runs
            .iter()
            .rev()
            .find(|r| !r.superseded && r.associated_scripts.contains(&id) && r.state.active())
            .map(|r| r.id)
    }
    pub fn grouped_projects(&self, filter: Filter, query: &str) -> Vec<(Group, Vec<Project>)> {
        let visible = self.visible_projects(filter, query);
        self.catalog
            .groups
            .iter()
            .filter_map(|group| {
                let projects: Vec<_> = visible
                    .iter()
                    .filter(|p| p.group == group.id)
                    .cloned()
                    .collect();
                if projects.is_empty()
                    && (group.id == 1 || !query.is_empty() || filter == Filter::Favorites)
                {
                    None
                } else {
                    Some((group.clone(), projects))
                }
            })
            .collect()
    }
    pub fn visible_projects(&self, filter: Filter, query: &str) -> Vec<Project> {
        let q = query.trim().to_lowercase();
        self.catalog
            .projects
            .iter()
            .filter_map(|p| {
                let project_match = p.name.to_lowercase().contains(&q)
                    || p.display_label().to_lowercase().contains(&q)
                    || p.path.to_string_lossy().to_lowercase().contains(&q);
                let mut visible = p.clone();
                visible.scripts.retain(|s| {
                    let filter_match = match filter {
                        Filter::Favorites => p.favorite || s.favorite,
                        Filter::Running => self.active_run(s.id).is_some(),
                        _ => true,
                    };
                    filter_match
                        && (project_match
                            || s.name.to_lowercase().contains(&q)
                            || s.display_label().to_lowercase().contains(&q)
                            || s.description.to_lowercase().contains(&q)
                            || s.invocation().to_lowercase().contains(&q))
                });
                let keep = match filter {
                    Filter::Favorites => p.favorite || !visible.scripts.is_empty(),
                    Filter::Running => !visible.scripts.is_empty(),
                    _ => project_match || !visible.scripts.is_empty(),
                };
                keep.then_some(visible)
            })
            .collect()
    }
    /// Project views are ordered by group/project; favorites retain their flat table.
    pub fn launch_table_content(
        &self,
        filter: Filter,
        query: &str,
        sort: LaunchSort,
        group_filter: Option<u64>,
    ) -> Result<LaunchTableContent, String> {
        let grouped = filter == Filter::All;
        let sort = if grouped && group_filter.is_some() {
            sort.for_project_sections()
        } else {
            sort
        };
        let rows = self.launch_table_rows(filter, query, sort, group_filter)?;
        if !grouped {
            return Ok(LaunchTableContent::Flat(rows));
        }
        let mut sections = std::collections::BTreeMap::new();
        for row in rows {
            let project = sections
                .entry(row.project_id)
                .or_insert_with(|| LaunchProjectSection {
                    group: row.group.clone(),
                    project_favorite: row.project_favorite,
                    project_id: row.project_id,
                    project_name: row.project_name.clone(),
                    project_path: row.project_path.clone(),
                    rows: vec![],
                });
            // Empty projects have a title row only, without a placeholder launch/action row.
            if row.script.is_some() {
                project.rows.push(row);
            }
        }
        let mut sections: Vec<_> = sections.into_values().collect();
        sections.sort_by(|left, right| {
            let names = left
                .project_name
                .to_lowercase()
                .cmp(&right.project_name.to_lowercase());
            let names = if sort.column == LaunchSortColumn::Project
                && sort.direction == SortDirection::Descending
            {
                names.reverse()
            } else {
                names
            };
            let groups = if group_filter.is_none() {
                let groups = crate::i18n::group_name(&left.group)
                    .to_lowercase()
                    .cmp(&crate::i18n::group_name(&right.group).to_lowercase());
                if sort.column == LaunchSortColumn::Group
                    && sort.direction == SortDirection::Descending
                {
                    groups.reverse()
                } else {
                    groups
                }
            } else {
                std::cmp::Ordering::Equal
            };
            groups
                .then_with(|| left.group.id.cmp(&right.group.id))
                .then(names)
                .then_with(|| left.project_path.cmp(&right.project_path))
                .then_with(|| left.project_id.cmp(&right.project_id))
        });
        Ok(LaunchTableContent::Projects(sections))
    }
    /// Return table rows in the requested view and sort order.
    pub fn running_table_rows(&self, sort: LaunchSort) -> Vec<LaunchTableRow> {
        let mut seen = std::collections::HashSet::new();
        let mut rows = Vec::new();
        for run in self
            .runtime
            .runs
            .iter()
            .rev()
            .filter(|r| !r.superseded && matches!(r.state, State::Starting | State::Running))
        {
            // PID identifies live external processes; managed preflight has no PID yet.
            let key = if run.pid != 0 {
                (true, run.pid as u64)
            } else {
                (false, run.console_id)
            };
            if !seen.insert(key) {
                continue;
            }
            let script = if run.external.is_none() {
                self.script(run.script_id)
            } else if run.associated_scripts.len() == 1 {
                self.script(run.associated_scripts[0])
            } else {
                None
            };
            let project = script
                .and_then(|script| {
                    self.catalog
                        .projects
                        .iter()
                        .find(|p| p.scripts.iter().any(|s| s.id == script.id))
                })
                .or_else(|| self.catalog.projects.iter().find(|p| p.path == run.cwd));
            let group = project
                .and_then(|p| self.catalog.groups.iter().find(|g| g.id == p.group))
                .cloned()
                .unwrap_or(Group {
                    id: 0,
                    name: String::new(),
                });
            rows.push(LaunchTableRow {
                run_id: Some(run.id),
                group,
                project_id: project.map_or(0, |p| p.id),
                project_name: project
                    .map_or_else(|| run.title.clone(), |p| p.display_label().to_owned()),
                project_path: project.map_or_else(|| run.cwd.clone(), |p| p.path.clone()),
                project_favorite: project.is_some_and(|p| p.favorite),
                script: script.cloned(),
            });
        }
        rows.sort_by(|a, b| {
            let run = |row: &LaunchTableRow| {
                self.runtime
                    .runs
                    .iter()
                    .find(|r| Some(r.id) == row.run_id)
                    .unwrap()
            };
            let cmp = match sort.column {
                LaunchSortColumn::Group => a
                    .group
                    .name
                    .to_lowercase()
                    .cmp(&b.group.name.to_lowercase()),
                LaunchSortColumn::Project => a
                    .project_name
                    .to_lowercase()
                    .cmp(&b.project_name.to_lowercase()),
                LaunchSortColumn::Status => run(a).state.label().cmp(run(b).state.label()),
                LaunchSortColumn::Port => run(a)
                    .listening_ports
                    .first()
                    .or(run(a).expected_port.as_ref())
                    .cmp(
                        &run(b)
                            .listening_ports
                            .first()
                            .or(run(b).expected_port.as_ref()),
                    ),
            };
            let cmp = if sort.direction == SortDirection::Descending {
                cmp.reverse()
            } else {
                cmp
            };
            cmp.then_with(|| a.run_id.cmp(&b.run_id))
        });
        rows
    }
    pub fn launch_table_rows(
        &self,
        filter: Filter,
        query: &str,
        sort: LaunchSort,
        group_filter: Option<u64>,
    ) -> Result<Vec<LaunchTableRow>, String> {
        if !matches!(filter, Filter::All | Filter::Favorites) {
            return Err("启动项表格只支持项目和收藏视图".into());
        }
        let mut rows = Vec::new();
        for project in self
            .visible_projects(filter, query)
            .into_iter()
            .filter(|project| group_filter.is_none_or(|group_id| project.group == group_id))
        {
            let group = self
                .catalog
                .groups
                .iter()
                .find(|group| group.id == project.group)
                .cloned()
                .ok_or_else(|| format!("项目 {} 所属分组不存在", project.name))?;
            if project.scripts.is_empty() {
                if filter == Filter::All {
                    rows.push(LaunchTableRow {
                        run_id: None,
                        group,
                        project_id: project.id,
                        project_name: project.display_label().to_owned(),
                        project_path: project.path,
                        project_favorite: project.favorite,
                        script: None,
                    });
                }
                continue;
            }
            let project_name = project.display_label().to_owned();
            rows.extend(project.scripts.into_iter().map(|script| LaunchTableRow {
                run_id: None,
                group: group.clone(),
                project_id: project.id,
                project_name: project_name.clone(),
                project_path: project.path.clone(),
                project_favorite: project.favorite,
                script: Some(script),
            }));
        }
        rows.sort_by(|left, right| {
            let cmp_text = |a: &str, b: &str| a.to_lowercase().cmp(&b.to_lowercase());
            let left_status = left
                .script
                .as_ref()
                .map(|script| self.script_status_label(script.id))
                .unwrap_or("--");
            let right_status = right
                .script
                .as_ref()
                .map(|script| self.script_status_label(script.id))
                .unwrap_or("--");
            let port_key = |row: &LaunchTableRow| {
                row.script.as_ref().and_then(|script| {
                    self.running_ports(script.id)
                        .into_iter()
                        .next()
                        .or_else(|| {
                            self.configured_port_hint(script.id)
                                .ok()
                                .flatten()
                                .map(|hint| hint.port)
                        })
                })
            };
            let ports = (port_key(left), port_key(right));
            let primary = match sort.column {
                LaunchSortColumn::Group => cmp_text(
                    crate::i18n::group_name(&left.group),
                    crate::i18n::group_name(&right.group),
                ),
                LaunchSortColumn::Project => cmp_text(&left.project_name, &right.project_name),
                LaunchSortColumn::Status => cmp_text(left_status, right_status),
                LaunchSortColumn::Port => match ports {
                    (Some(a), Some(b)) if sort.direction == SortDirection::Descending => b.cmp(&a),
                    (Some(a), Some(b)) => a.cmp(&b),
                    (Some(_), None) => match sort.direction {
                        SortDirection::Ascending => std::cmp::Ordering::Less,
                        SortDirection::Descending => std::cmp::Ordering::Greater,
                    },
                    (None, Some(_)) => match sort.direction {
                        SortDirection::Ascending => std::cmp::Ordering::Greater,
                        SortDirection::Descending => std::cmp::Ordering::Less,
                    },
                    (None, None) => std::cmp::Ordering::Equal,
                },
            };
            let primary = if sort.direction == SortDirection::Descending
                && sort.column != LaunchSortColumn::Port
            {
                primary.reverse()
            } else {
                primary
            };
            primary
                .then_with(|| {
                    cmp_text(
                        crate::i18n::group_name(&left.group),
                        crate::i18n::group_name(&right.group),
                    )
                })
                .then_with(|| cmp_text(&left.project_name, &right.project_name))
                .then_with(|| {
                    cmp_text(
                        left.script
                            .as_ref()
                            .map(Script::display_label)
                            .unwrap_or(""),
                        right
                            .script
                            .as_ref()
                            .map(Script::display_label)
                            .unwrap_or(""),
                    )
                })
                .then_with(|| left.project_id.cmp(&right.project_id))
                .then_with(|| {
                    left.script
                        .as_ref()
                        .map(|script| script.id)
                        .cmp(&right.script.as_ref().map(|script| script.id))
                })
        });
        Ok(rows)
    }
    /// Return groups ordered by the names shown in the launch table filter bar.
    pub fn groups_for_launch_filter(&self) -> Vec<Group> {
        let mut groups = self.catalog.groups.clone();
        groups.sort_by_cached_key(|group| crate::i18n::group_name(group).to_lowercase());
        groups
    }
    pub fn active_count(&self) -> usize {
        self.runtime
            .runs
            .iter()
            .filter(|r| r.state.active() && !r.superseded)
            .count()
    }
    pub fn latest_state(&self, id: u64) -> Option<State> {
        self.runtime
            .runs
            .iter()
            .rev()
            .find(|r| !r.superseded && r.script_id == id)
            .map(|r| r.state)
    }
}
/// Open the original file in the system's associated editor; never copy or rewrite it.
pub fn open_file(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Err(format!("文件不存在: {}", path.display()));
    }
    #[cfg(target_os = "macos")]
    let mut cmd = {
        let mut c = Command::new("open");
        if path.is_file() {
            c.arg("-t");
        }
        c
    };
    #[cfg(not(target_os = "macos"))]
    let mut cmd = Command::new("xdg-open");
    let output = cmd.arg(path).output().map_err(|e| e.to_string())?;
    if output.status.success() {
        Ok(())
    } else {
        Err(format!(
            "打开文件失败: {}",
            String::from_utf8_lossy(&output.stderr)
        ))
    }
}
pub fn path_label(path: &Path) -> String {
    let text = path.display().to_string();
    if let Ok(home) = std::env::var("HOME") {
        text.replacen(&home, "~", 1)
    } else {
        text
    }
}
