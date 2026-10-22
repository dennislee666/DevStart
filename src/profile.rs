use crate::model::{Catalog, Entry, Group, Project, Script};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, HashSet},
    path::{Path, PathBuf},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    pub group: String,
    pub projects: Vec<ProfileProject>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileProject {
    pub name: String,
    pub path: PathBuf,
    pub scripts: Vec<ProfileScript>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProfileScript {
    #[serde(default = "crate::model::enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub display_command: Option<String>,
    pub name: String,
    pub file: PathBuf,
    pub entry: Entry,
    pub description: String,
    #[serde(default)]
    pub cwd: Option<PathBuf>,
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
    #[serde(default)]
    pub expected_port: Option<u16>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub resource_key: Option<String>,
}
/// Validate all references before committing. Import is declarative and never runs a script.
pub fn merge(catalog: &Catalog, profile: Profile) -> Result<Catalog, String> {
    if profile.group.trim().is_empty() {
        return Err("启动 profile 分组名称为空".into());
    }
    let mut next = catalog.clone();
    let group = if let Some(g) = next.groups.iter().find(|g| g.name == profile.group) {
        g.id
    } else {
        let id = next.id();
        next.groups.push(Group {
            id,
            name: profile.group,
        });
        id
    };
    let mut paths = HashSet::new();
    for project in profile.projects {
        let path = project
            .path
            .canonicalize()
            .map_err(|e| format!("项目目录 {}: {e}", project.path.display()))?;
        if !path.is_dir() || !paths.insert(path.clone()) {
            return Err("profile 项目目录无效或重复".into());
        }
        let old = next.projects.iter().find(|p| p.path == path).cloned();
        let id = old.as_ref().map(|p| p.id).unwrap_or_else(|| next.id());
        let mut scripts = vec![];
        let mut identities = HashSet::new();
        for spec in project.scripts {
            let file = resolve(&path, &spec.file)?;
            let cwd = spec
                .cwd
                .as_ref()
                .map(|p| resolve(&path, p))
                .transpose()?
                .unwrap_or_else(|| path.clone());
            let mut script = Script {
                display_name: None,
                description_override: None,
                enabled: spec.enabled,
                display_command: spec.display_command,
                id: 0,
                name: spec.name,
                file,
                cwd,
                entry: spec.entry,
                description: spec.description,
                favorite: false,
                environment: spec.environment,
                expected_port: spec.expected_port,
                args: spec.args,
                custom: true,
                resource_key: spec.resource_key,
            };
            validate(&script)?;
            if !identities.insert(script.identity()) {
                return Err("profile 包含重复脚本入口".into());
            }
            if let Some(prev) = old
                .as_ref()
                .and_then(|p| p.scripts.iter().find(|s| s.name == script.name))
            {
                script.id = prev.id;
                script.favorite = prev.favorite;
                script.display_name = prev.display_name.clone();
            } else {
                script.id = next.id();
            }
            scripts.push(script);
        }
        let configured = Project {
            description: String::new(),
            id,
            name: project.name,
            path,
            group,
            favorite: old.as_ref().is_some_and(|p| p.favorite),
            scripts,
            explicit_scripts: true,
        };
        if let Some(index) = next.projects.iter().position(|p| p.id == id) {
            next.projects[index] = configured;
        } else {
            next.projects.push(configured);
        }
    }
    Ok(next)
}
fn resolve(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let full = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    full.canonicalize()
        .map_err(|e| format!("入口路径 {}: {e}", full.display()))
}
pub fn validate(script: &Script) -> Result<(), String> {
    if script
        .resource_key
        .as_ref()
        .is_some_and(|key| key.trim().is_empty())
    {
        return Err("共享调试资源不能为空".into());
    }
    if script.expected_port == Some(0) {
        return Err("启动端口不能为 0".into());
    }
    if script.name.trim().is_empty() || !script.file.is_file() || !script.cwd.is_dir() {
        return Err(format!("无效启动入口: {}", script.file.display()));
    }
    for key in script.environment.keys() {
        let mut chars = key.chars();
        if !chars
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
            || !chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
        {
            return Err(format!("无效环境变量名称: {key}"));
        }
    }
    match &script.entry {
        Entry::Flutter { launch } => launch.validate()?,
        Entry::Java {
            main_class,
            program,
            ..
        } => {
            if main_class.trim().is_empty() || program.trim().is_empty() {
                return Err("Java 启动协议不完整".into());
            }
        }
        Entry::File { interpreter } => {
            if interpreter.trim().is_empty() {
                return Err("脚本解释器为空".into());
            }
        }
        Entry::Command { program } => {
            if program.trim().is_empty() {
                return Err("执行程序为空".into());
            }
        }
        Entry::Package { manager, name } => {
            if manager.trim().is_empty() {
                return Err("包管理器为空".into());
            }
            let json: serde_json::Value =
                serde_json::from_slice(&std::fs::read(&script.file).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            if !json
                .get("scripts")
                .and_then(|v| v.get(name))
                .is_some_and(|v| v.is_string())
            {
                return Err(format!("{} 不包含运行入口 {name}", script.file.display()));
            }
        }
    }
    Ok(())
}
