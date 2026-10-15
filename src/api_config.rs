//! Configuration commands shared by local automation; all mutations stay in the service owner.
use crate::{model::Project, profile::ProfileScript, service::Service};
use serde_json::{Value, json};
use std::path::Path;

pub fn fields(params: &Value, allowed: &[&str]) -> Result<(), String> {
    let object = params.as_object().ok_or("params必须为对象")?;
    if let Some(key) = object.keys().find(|k| !allowed.contains(&k.as_str())) {
        return Err(format!("未知字段: {key}"));
    }
    Ok(())
}
pub fn id(p: &Value, key: &str) -> Result<u64, String> {
    p[key].as_u64().ok_or_else(|| format!("{key}需要非负整数"))
}
pub fn text<'a>(p: &'a Value, key: &str) -> Result<&'a str, String> {
    p[key].as_str().ok_or_else(|| format!("{key}需要字符串"))
}
pub fn project(s: &Service, id: u64) -> Result<&Project, String> {
    s.catalog
        .projects
        .iter()
        .find(|p| p.id == id)
        .ok_or("项目不存在".into())
}
pub fn idle(s: &Service, ids: &[u64]) -> Result<(), String> {
    if s.runtime
        .runs
        .iter()
        .any(|r| r.state.active() && r.associated_scripts.iter().any(|id| ids.contains(id)))
    {
        return Err("配置仍有运行实例，请先停止并确认退出".into());
    }
    Ok(())
}
fn patch<T: serde::Serialize + serde::de::DeserializeOwned>(
    value: &T,
    changes: &Value,
) -> Result<T, String> {
    let mut value = serde_json::to_value(value).map_err(|e| e.to_string())?;
    for (key, val) in changes.as_object().ok_or("patch需要对象")? {
        value[key] = val.clone();
    }
    serde_json::from_value(value).map_err(|e| e.to_string())
}
/// The project manifest is authoritative. Validate it before committing the catalog and roll back failures.
#[derive(Default)]
struct PresentationPatch {
    name: Option<(u64, Option<String>)>,
    description: Option<(u64, String)>,
}
fn persist(
    s: &mut Service,
    pid: u64,
    specs: Vec<ProfileScript>,
    ids: Vec<u64>,
    expected: Option<Vec<u8>>,
    favorite: Option<(u64, bool)>,
    presentation: PresentationPatch,
) -> Result<(), String> {
    let old = project(s, pid)?.clone();
    let path = old.path.join(crate::project_manifest::FILE_NAME);
    let current = std::fs::read(&path).ok();
    if current != expected {
        return Err("项目清单被外部修改，请重新读取后重试".into());
    }
    let bytes = crate::project_manifest::encode(specs)?;
    crate::store::atomic_write(&path, &bytes)?;
    let result = (|| {
        let mut scripts = crate::project_manifest::discover(&old.path)?;
        for (script, id) in scripts.iter_mut().zip(ids) {
            script.id = id;
            script.display_name = presentation
                .name
                .clone()
                .filter(|(sid, _)| *sid == id)
                .map(|(_, value)| value)
                .unwrap_or_else(|| {
                    old.scripts
                        .iter()
                        .find(|v| v.id == id)
                        .and_then(|v| v.display_name.clone())
                });
            script.description_override = presentation
                .description
                .as_ref()
                .filter(|(sid, _)| *sid == id)
                .map(|(_, text)| text.clone())
                .or_else(|| {
                    old.scripts
                        .iter()
                        .find(|v| v.id == id)
                        .and_then(|v| v.description_override.clone())
                });
            if let Some(text) = &script.description_override {
                script.description = text.clone();
            }
            script.favorite = favorite
                .filter(|(sid, _)| *sid == id)
                .map(|(_, v)| v)
                .unwrap_or_else(|| old.scripts.iter().any(|v| v.id == id && v.favorite));
        }
        let mut next = s.catalog.clone();
        next.next_id = next
            .next_id
            .max(scripts.iter().map(|v| v.id + 1).max().unwrap_or(2));
        next.projects
            .iter_mut()
            .find(|p| p.id == pid)
            .ok_or("项目不存在")?
            .scripts = scripts;
        s.commit(next)
    })();
    if let Err(error) = result {
        let rollback = if let Some(bytes) = expected {
            crate::store::atomic_write(&path, &bytes)
        } else {
            std::fs::remove_file(&path).map_err(|e| e.to_string())
        };
        return Err(match rollback {
            Ok(()) => error,
            Err(e) => format!("{error}; 清单回滚失败: {e}"),
        });
    }
    Ok(())
}
pub fn handle(s: &mut Service, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "groups.list" => {
            fields(p, &[])?;
            Ok(json!(s.catalog.groups))
        }
        "groups.create" => {
            fields(p, &["name"])?;
            s.add_group(text(p, "name")?)?;
            Ok(json!(s.catalog.groups.last()))
        }
        "groups.update" => {
            fields(p, &["id", "name"])?;
            let gid = id(p, "id")?;
            let name = text(p, "name")?.trim();
            if name.chars().any(char::is_control) {
                return Err("分组名称不能包含换行或控制字符".into());
            }
            if name.is_empty()
                || s.catalog
                    .groups
                    .iter()
                    .any(|g| g.id != gid && g.name == name)
            {
                return Err("分组名称为空或重复".into());
            }
            let mut next = s.catalog.clone();
            next.groups
                .iter_mut()
                .find(|g| g.id == gid)
                .ok_or("分组不存在")?
                .name = name.into();
            s.commit(next)?;
            Ok(json!({"id":gid}))
        }
        "groups.delete" => {
            fields(p, &["id"])?;
            let gid = id(p, "id")?;
            if gid == 1 {
                return Err("默认分组不可删除".into());
            }
            let mut next = s.catalog.clone();
            if !next.groups.iter().any(|g| g.id == gid) {
                return Err("分组不存在".into());
            }
            next.groups.retain(|g| g.id != gid);
            for project in &mut next.projects {
                if project.group == gid {
                    project.group = 1;
                }
            }
            s.commit(next)?;
            Ok(json!({"deleted":gid,"projects_moved_to":1}))
        }
        "projects.list" => {
            fields(p, &[])?;
            Ok(json!(s.catalog.projects))
        }
        "projects.get" => {
            fields(p, &["id"])?;
            Ok(json!(project(s, id(p, "id")?)?))
        }
        "projects.create" => {
            fields(p, &["path", "group", "name", "description", "favorite"])?;
            let group = if p.get("group").is_none_or(Value::is_null) {
                1
            } else {
                id(p, "group")?
            };
            // Validate metadata before import so bad requests never leave partial projects.
            for key in ["name", "description"] {
                if p.get(key).is_some() {
                    text(p, key)?;
                }
            }
            if p.get("name")
                .is_some_and(|v| v.as_str().is_some_and(|n| n.trim().is_empty()))
            {
                return Err("项目名不能为空".into());
            }
            if p.get("favorite").is_some_and(|v| !v.is_boolean()) {
                return Err("favorite需要布尔值".into());
            }
            let pid = s.import_configured(
                Path::new(text(p, "path")?),
                group,
                p.get("name").and_then(Value::as_str),
                p.get("description").and_then(Value::as_str).unwrap_or(""),
                p.get("favorite").and_then(Value::as_bool).unwrap_or(false),
            )?;
            Ok(json!(project(s, pid)?))
        }
        "projects.update" => {
            fields(p, &["id", "patch"])?;
            let pid = id(p, "id")?;
            let changes = &p["patch"];
            fields(changes, &["name", "description", "group", "favorite"])?;
            let mut normalized = changes.clone();
            if normalized.get("group").is_some_and(Value::is_null) {
                normalized["group"] = json!(1);
            }
            let mut item: Project = patch(project(s, pid)?, &normalized)?;
            item.name = item.name.trim().to_owned();
            if item.name.chars().any(char::is_control) {
                return Err("项目名称不能包含换行或控制字符".into());
            }
            if item.name.is_empty() {
                return Err("项目名不能为空".into());
            }
            if !s.catalog.groups.iter().any(|g| g.id == item.group) {
                return Err("分组不存在".into());
            }
            let mut next = s.catalog.clone();
            *next
                .projects
                .iter_mut()
                .find(|v| v.id == pid)
                .ok_or("项目不存在")? = item;
            s.commit(next)?;
            s.sync_display_names();
            Ok(json!(project(s, pid)?))
        }
        "projects.delete" => {
            fields(p, &["id"])?;
            let pid = id(p, "id")?;
            idle(
                s,
                &project(s, pid)?
                    .scripts
                    .iter()
                    .map(|v| v.id)
                    .collect::<Vec<_>>(),
            )?;
            let mut next = s.catalog.clone();
            next.projects.retain(|p| p.id != pid);
            s.commit(next)?;
            Ok(json!({"deleted":pid,"source_files_deleted":false}))
        }
        "projects.refresh" => {
            fields(p, &["id"])?;
            let pid = id(p, "id")?;
            s.rescan(pid)?;
            Ok(json!(project(s, pid)?))
        }
        "scripts.list" => {
            fields(p, &["project_id"])?;
            if p.get("project_id").is_some() {
                Ok(json!(project(s, id(p, "project_id")?)?.scripts))
            } else {
                Ok(json!(
                    s.catalog
                        .projects
                        .iter()
                        .flat_map(|p| &p.scripts)
                        .collect::<Vec<_>>()
                ))
            }
        }
        "scripts.get" => {
            fields(p, &["id"])?;
            Ok(json!(s.script(id(p, "id")?).ok_or("启动项不存在")?))
        }
        "scripts.create" | "scripts.update" | "scripts.delete" => {
            fields(
                p,
                if method == "scripts.create" {
                    &["project_id", "spec"]
                } else if method == "scripts.update" {
                    &["id", "patch"]
                } else {
                    &["id"]
                },
            )?;
            let sid = if method == "scripts.create" {
                s.catalog.next_id
            } else {
                id(p, "id")?
            };
            let pid = if method == "scripts.create" {
                id(p, "project_id")?
            } else {
                s.catalog
                    .projects
                    .iter()
                    .find(|v| v.scripts.iter().any(|v| v.id == sid))
                    .ok_or("启动项不存在")?
                    .id
            };
            let display_name =
                if method == "scripts.update" && p["patch"].get("display_name").is_some() {
                    let value: Option<String> =
                        serde_json::from_value(p["patch"]["display_name"].clone())
                            .map_err(|e| e.to_string())?;
                    Some((sid, crate::model::Script::validate_display_name(value)?))
                } else {
                    None
                };
            if method == "scripts.update"
                && p["patch"].as_object().is_some_and(|v| {
                    !v.is_empty()
                        && v.keys()
                            .all(|key| matches!(key.as_str(), "display_name" | "description"))
                })
            {
                let name = display_name.as_ref().map(|(_, value)| value.clone());
                let description = p["patch"]
                    .get("description")
                    .map(|v| v.as_str().map(str::to_owned).ok_or("description需要字符串"))
                    .transpose()?;
                if description.is_none() {
                    s.set_display_name(sid, name.expect("display-name metadata patch"))?;
                } else {
                    s.update_script_info(sid, name, description)?;
                }
                return Ok(json!(s.script(sid)));
            }
            let proj = project(s, pid)?;
            let path = proj.path.join(crate::project_manifest::FILE_NAME);
            let expected = match std::fs::read(&path) {
                Ok(v) => Some(v),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
                Err(e) => return Err(e.to_string()),
            };
            let mut specs = crate::project_manifest::specifications(proj)?;
            let mut ids: Vec<_> = proj.scripts.iter().map(|v| v.id).collect();
            if method == "scripts.create" {
                let spec: ProfileScript =
                    serde_json::from_value(p["spec"].clone()).map_err(|e| e.to_string())?;
                specs.push(spec);
                ids.push(sid);
            } else {
                idle(s, &[sid])?;
                let index = ids.iter().position(|v| *v == sid).ok_or("启动项不存在")?;
                if method == "scripts.delete" {
                    specs.remove(index);
                    ids.remove(index);
                } else {
                    fields(
                        &p["patch"],
                        &[
                            "display_name",
                            "name",
                            "file",
                            "entry",
                            "description",
                            "cwd",
                            "environment",
                            "expected_port",
                            "args",
                            "resource_key",
                            "enabled",
                            "display_command",
                            "favorite",
                        ],
                    )?;
                    let mut changes = p["patch"].clone();
                    changes.as_object_mut().unwrap().remove("display_name");
                    if let Some(value) = changes.as_object_mut().unwrap().remove("favorite")
                        && !value.is_boolean()
                    {
                        return Err("favorite需要布尔值".into());
                    }
                    specs[index] = patch(&specs[index], &changes)?;
                }
            }
            let favorite = p["patch"]
                .get("favorite")
                .and_then(Value::as_bool)
                .map(|v| (sid, v));
            persist(
                s,
                pid,
                specs,
                ids,
                expected,
                favorite,
                PresentationPatch {
                    name: display_name,
                    description: p["patch"]
                        .get("description")
                        .and_then(Value::as_str)
                        .map(|text| (sid, text.to_owned())),
                },
            )?;
            s.sync_display_names();
            if method == "scripts.delete" {
                Ok(json!({"deleted":sid}))
            } else {
                Ok(json!(s.script(sid)))
            }
        }
        _ => Err("方法不存在".into()),
    }
}
