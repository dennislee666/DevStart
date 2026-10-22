//! Versioned JSON-RPC commands; GUI and headless mode execute these on their service thread.
use crate::{api_config as config, service::Service};
use serde_json::{Value, json};

pub const METHODS: &[&str] = &[
    "scan.start",
    "scan.get",
    "scan.cancel",
    "scan.import",
    "terminal.open",
    "terminal.read",
    "terminal.line",
    "terminal.interrupt",
    "terminal.clear",
    "terminal.close",
    "git.commit_files",
    "git.commit_diff",
    "git.remotes",
    "git.update",
    "git.push_target",
    "git.job",
    "git.changes",
    "git.diff",
    "git.commit",
    "git.push",
    "git.pull",
    "git.restore",
    "git.command",
    "git.list",
    "git.branches",
    "git.history",
    "git.switch",
    "git.merge",
    "git.restore_version",
    "editor.list",
    "editor.open",
    "editor.read",
    "editor.change",
    "editor.save",
    "editor.reload",
    "editor.close",
    "editor.versions",
    "editor.restore",
    "api.describe",
    "api.docs",
    "config.get",
    "settings.get",
    "settings.update",
    "settings.reset",
    "groups.list",
    "groups.create",
    "groups.update",
    "groups.delete",
    "groups.refresh",
    "projects.list",
    "projects.get",
    "projects.create",
    "projects.update",
    "projects.delete",
    "projects.refresh",
    "scripts.list",
    "scripts.get",
    "scripts.ports",
    "scripts.status",
    "scripts.create",
    "scripts.update",
    "scripts.delete",
    "runs.list",
    "runs.get",
    "runs.start",
    "runs.restart",
    "runs.stop",
    "runs.open",
    "runs.close",
    "runs.stop_all",
    "runs.bulk_plan",
    "runs.bulk_apply",
    "logs.search",
    "logs.open_target",
    "logs.read",
    "logs.list",
];
pub fn response(s: &mut Service, request: Value) -> Value {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    if request.get("id").is_none()
        || request["jsonrpc"] != "2.0"
        || !request["method"].is_string()
        || (!id.is_null() && !id.is_string() && !id.is_number())
    {
        return error(id, -32600, "无效JSON-RPC请求");
    }
    let method = request["method"].as_str().unwrap();
    if !METHODS.contains(&method) {
        return error(id, -32601, "方法不存在");
    }
    let params = request.get("params").cloned().unwrap_or_else(|| json!({}));
    if !params.is_object() {
        return error(id, -32602, "params必须为对象");
    }
    match dispatch(s, method, &params) {
        Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
        Err(message) => {
            eprintln!("[dev start api] {method}: {message}");
            error(id, -32000, &message)
        }
    }
}
pub fn error(id: Value, code: i64, message: &str) -> Value {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}})
}
pub fn dispatch(s: &mut Service, method: &str, p: &Value) -> Result<Value, String> {
    match method {
        "scan.start" => {
            config::fields(p, &["path"])?;
            let id = s
                .scans
                .start(std::path::Path::new(config::text(p, "path")?), &s.catalog)?;
            Ok(json!({"job_id":id,"state":"running"}))
        }
        "scan.get" | "scan.cancel" => {
            config::fields(p, &["id"])?;
            let id = config::id(p, "id")?;
            if method == "scan.cancel" {
                s.scans.cancel(id)?;
            }
            Ok(json!(s.scans.get(id)?))
        }
        "scan.import" => {
            config::fields(p, &["id", "launch_ids", "group"])?;
            let ids: Vec<u64> =
                serde_json::from_value(p["launch_ids"].clone()).map_err(|e| e.to_string())?;
            let group = p
                .get("group")
                .map(|value| serde_json::from_value(value.clone()))
                .transpose()
                .map_err(|e| format!("Invalid group: {e}"))?;
            Ok(json!(s.import_scan_grouped(
                config::id(p, "id")?,
                &ids,
                group
            )?))
        }

        m if m.starts_with("editor.") => s.editor_action(m, p),
        "git.list" => {
            config::fields(p, &[])?;
            Ok(json!(crate::git_workspace::list(&s.catalog.projects)))
        }
        "terminal.open" => {
            config::fields(p, &["project_id"])?;
            let id = config::id(p, "project_id")?;
            let path = config::project(s, id)?.path.clone();
            Ok(json!({"id":s.terminals.open(id,&path)?}))
        }
        "terminal.read" | "terminal.line" | "terminal.interrupt" | "terminal.clear"
        | "terminal.close" => {
            config::fields(
                p,
                if method == "terminal.line" {
                    &["id", "text"]
                } else {
                    &["id"]
                },
            )?;
            let id = config::id(p, "id")?;
            match method {
                "terminal.read" => Ok(json!(s.terminals.read(id)?)),
                "terminal.line" => {
                    s.terminals.line(id, config::text(p, "text")?)?;
                    Ok(json!({"sent":true}))
                }
                "terminal.interrupt" => {
                    s.terminals.interrupt(id)?;
                    Ok(json!({"sent":true}))
                }
                "terminal.clear" => {
                    s.terminals.clear(id)?;
                    Ok(json!({"cleared":true}))
                }
                _ => {
                    s.terminals.close(id)?;
                    Ok(json!({"closed":true}))
                }
            }
        }
        "git.commit_files" | "git.commit_diff" => {
            config::fields(
                p,
                if method == "git.commit_files" {
                    &["project_id", "commit"]
                } else {
                    &["project_id", "commit", "file"]
                },
            )?;
            let path = &config::project(s, config::id(p, "project_id")?)?.path;
            let commit = config::text(p, "commit")?;
            if method == "git.commit_files" {
                Ok(json!(crate::git_workspace::commits::files(path, commit)?))
            } else {
                Ok(
                    json!({"output":crate::git_workspace::commits::diff(path,commit,config::text(p,"file")?)?}),
                )
            }
        }
        "git.remotes" => {
            config::fields(p, &["project_id"])?;
            let path = &config::project(s, config::id(p, "project_id")?)?.path;
            Ok(json!(crate::git_workspace::remotes::list(path)?))
        }
        "git.update" | "git.push_target" => {
            config::fields(p, &["project_id", "remote_key", "branch"])?;
            let path = config::project(s, config::id(p, "project_id")?)?
                .path
                .clone();
            let key = config::text(p, "remote_key")?.to_owned();
            let branch = config::text(p, "branch")?.to_owned();
            let update = method == "git.update";
            let id = crate::git_jobs::start(move || {
                if update {
                    crate::git_workspace::remotes::update(&path, &key, &branch)
                } else {
                    crate::git_workspace::remotes::push(&path, &key, &branch)
                }
            })?;
            Ok(json!({"job_id":id,"state":"running"}))
        }
        "git.job" => {
            config::fields(p, &["id"])?;
            Ok(json!(crate::git_jobs::get(config::id(p, "id")?)?))
        }
        "git.changes" | "git.diff" | "git.commit" | "git.push" | "git.pull" | "git.restore"
        | "git.command" => {
            use crate::git_workspace::actions;
            let allowed = match method {
                "git.diff" | "git.restore" => vec!["project_id", "file"],
                "git.commit" => vec!["project_id", "message", "files"],
                "git.command" => vec!["project_id", "command"],
                _ => vec!["project_id"],
            };
            config::fields(p, &allowed)?;
            let path = config::project(s, config::id(p, "project_id")?)?
                .path
                .clone();
            if method == "git.changes" {
                return Ok(json!(actions::changes(&path)?));
            }
            if method == "git.diff" {
                return Ok(json!({"output":actions::diff(&path,config::text(p,"file")?)?}));
            }
            let file = if matches!(method, "git.restore") {
                Some(config::text(p, "file")?.to_owned())
            } else {
                None
            };
            let text = match method {
                "git.commit" => config::text(p, "message")?,
                "git.command" => config::text(p, "command")?,
                _ => "",
            }
            .to_owned();
            let files = if method == "git.commit" {
                let values = p["files"].as_array().ok_or("files must be an array")?;
                if values.len() > 10000 {
                    return Err("Select at most 10000 files".into());
                }
                values
                    .iter()
                    .map(|v| {
                        v.as_str()
                            .map(str::to_owned)
                            .ok_or("files must contain strings".to_owned())
                    })
                    .collect::<Result<Vec<_>, _>>()?
            } else {
                vec![]
            };
            let backup = s.store.root.join("git-backups");
            let method = method.to_owned();
            let id = crate::git_jobs::start(move || match method.as_str() {
                "git.commit" => actions::commit(&path, &text, &files),
                "git.push" => actions::push(&path),
                "git.pull" => actions::pull(&path),
                "git.restore" => actions::restore(&path, file.as_deref().unwrap(), &backup),
                _ => actions::command(&path, &text),
            })?;
            Ok(json!({"job_id":id,"state":"running"}))
        }
        "git.restore_version" => {
            config::fields(p, &["project_id", "commit", "branch", "expected_head"])?;
            let path = config::project(s, config::id(p, "project_id")?)?
                .path
                .clone();
            let commit = config::text(p, "commit")?.to_owned();
            let branch = config::text(p, "branch")?.to_owned();
            let head = config::text(p, "expected_head")?.to_owned();
            let id = crate::git_jobs::start(move || {
                crate::git_workspace::version_restore::restore(&path, &commit, &branch, &head)
            })?;
            Ok(json!({"job_id":id,"state":"running"}))
        }
        "git.merge" => {
            config::fields(p, &["project_id", "source", "target"])?;
            let project = config::project(s, config::id(p, "project_id")?)?;
            let path = project.path.clone();
            let source = config::text(p, "source")?.to_owned();
            let target = config::text(p, "target")?.to_owned();
            let id = crate::git_jobs::start(move || {
                crate::git_workspace::merging::merge(&path, &source, &target)
            })?;
            Ok(json!({"job_id":id,"state":"running"}))
        }
        "git.branches" | "git.history" | "git.switch" => {
            let allowed = match method {
                "git.branches" => vec!["project_id"],
                "git.history" => vec!["project_id", "offset", "limit"],
                _ => vec!["project_id", "branch"],
            };
            config::fields(p, &allowed)?;
            let project = config::project(s, config::id(p, "project_id")?)?;
            match method {
                "git.branches" => Ok(json!(crate::git_workspace::branches(&project.path)?)),
                "git.history" => {
                    let number = |key: &str, default: u64| -> Result<usize, String> {
                        let n = match p.get(key) {
                            Some(v) => v
                                .as_u64()
                                .ok_or_else(|| format!("{key} must be an unsigned integer"))?,
                            None => default,
                        };
                        usize::try_from(n).map_err(|e| e.to_string())
                    };
                    Ok(json!(crate::git_workspace::history(
                        &project.path,
                        number("offset", 0)?,
                        number("limit", 50)?
                    )?))
                }
                _ => {
                    let path = project.path.clone();
                    let branch = config::text(p, "branch")?.to_owned();
                    let id = crate::git_jobs::start(move || {
                        crate::git_workspace::switch(&path, &branch)
                            .map(|_| format!("Switched to {branch}"))
                    })?;
                    Ok(json!({"job_id":id,"state":"running"}))
                }
            }
        }
        "api.describe" => {
            config::fields(p, &[])?;
            Ok(
                json!({"name":"DevStart","edition":crate::app_identity::edition(),"api_version":1,"transport":"unix-jsonrpc-ndjson","methods":METHODS,
                "default_group_id":1,"max_request_bytes":2097152,"max_response_bytes":8388608,
                "max_log_chunk_bytes":65536,"log_retention":{
                    "max_lines_per_file":crate::log_store::MAX_LINES,"max_bytes_per_file":crate::log_store::MAX_BYTES,
                    "max_total_bytes":crate::log_store::MAX_TOTAL_BYTES,"max_files":crate::log_store::MAX_FILES,
                    "max_age_days":7},"documentation":"docs/api.md","contracts":serde_json::from_str::<Value>(include_str!("../docs/api-schema.json")).map_err(|e|e.to_string())?, "launch_spec":{
                    "required":["name","file","entry","description"],
                    "optional":["cwd","args","environment","expected_port","resource_key","enabled","display_command"]}}),
            )
        }
        "api.docs" => {
            config::fields(p, &[])?;
            Ok(json!({"api_version":1,"format":"markdown","text":include_str!("../docs/api.md")}))
        }
        "settings.get" => {
            config::fields(p, &[])?;
            Ok(json!(s.catalog.preferences))
        }
        "settings.update" => {
            config::fields(p, &["patch"])?;
            Ok(json!(s.update_preferences(p["patch"].clone())?))
        }
        "settings.reset" => {
            config::fields(p, &[])?;
            Ok(json!(
                s.update_preferences(
                    serde_json::to_value(crate::preferences::Preferences::default())
                        .map_err(|e| e.to_string())?
                )?
            ))
        }
        "scripts.status" => {
            config::fields(p, &["id"])?;
            let id = config::id(p, "id")?;
            s.script(id).ok_or("启动项不存在")?;
            Ok(json!({"id":id,"label":s.script_status_label(id)}))
        }
        "scripts.ports" => {
            config::fields(p, &["id"])?;
            let id = config::id(p, "id")?;
            if s.script(id).is_none() {
                return Err("启动项不存在".into());
            }
            let hint = s.configured_port_hint(id);
            let actual = s.running_ports(id);
            let cell = s.running_port_cell(id);
            let configured = hint.as_ref().ok().and_then(|v| v.as_ref());
            Ok(
                json!({"script_id":id,"configured_port":configured.map(|h|h.port),"configuration_source":configured.map(|h|h.source.clone()),
                "configuration_error":hint.as_ref().err(),"actual_ports":actual,"display":cell.label,"tooltip":cell.tooltip,
                "source":if !actual.is_empty(){"actual"}else if configured.is_some(){"configured"}else{"unknown"}}),
            )
        }
        "config.get" => {
            config::fields(p, &[])?;
            Ok(json!(s.catalog))
        }
        "runs.list" => {
            config::fields(p, &[])?;
            Ok(json!(
                s.runtime.runs.iter().map(run_value).collect::<Vec<_>>()
            ))
        }
        "runs.get" => {
            config::fields(p, &["id"])?;
            Ok(run_value(run(s, config::id(p, "id")?)?))
        }
        "runs.start" | "runs.restart" => {
            config::fields(
                p,
                if method == "runs.restart" {
                    &["script_id", "id"]
                } else {
                    &["script_id"]
                },
            )?;
            let sid = config::id(p, "script_id")?;
            let id = if method == "runs.restart" && p.get("id").is_some() {
                s.restart_run(config::id(p, "id")?, sid)?
            } else if method == "runs.restart"
                && s.runtime
                    .runs
                    .iter()
                    .any(|r| r.associated_scripts.contains(&sid) && !r.superseded)
            {
                s.restart(sid)?
            } else {
                s.launch(sid)?
            };
            let console_id = run(s, id)?.console_id;
            Ok(json!({"run_id":id,"console_id":console_id,"state":"starting","asynchronous":true}))
        }
        "runs.stop" => {
            config::fields(p, &["id"])?;
            let id = config::id(p, "id")?;
            let active = run(s, id)?.state.active();
            if active {
                s.runtime.stop(id);
            }
            Ok(json!({"run_id":id,"stop_requested":active}))
        }
        "runs.open" | "runs.close" => {
            config::fields(
                p,
                if method == "runs.close" {
                    &["id", "stop"]
                } else {
                    &["id"]
                },
            )?;
            let id = config::id(p, "id")?;
            if method == "runs.open" {
                s.runtime.open_tab(id)?;
            } else {
                let stop = p
                    .get("stop")
                    .map(|v| v.as_bool().ok_or("stop需要布尔值"))
                    .transpose()?
                    .unwrap_or(false);
                s.runtime.close_tab(id, stop)?;
            }
            Ok(run_value(run(s, id)?))
        }
        "runs.bulk_plan" => {
            config::fields(p, &["action"])?;
            let action: crate::bulk::Action =
                serde_json::from_value(p.get("action").ok_or("缺少批量动作")?.clone())
                    .map_err(|e| format!("无效批量动作：{e}"))?;
            Ok(json!(s.bulk_plan(action)))
        }
        "runs.bulk_apply" => {
            config::fields(p, &["plan", "confirmed"])?;
            let plan: crate::bulk::Plan =
                serde_json::from_value(p.get("plan").ok_or("缺少批量计划")?.clone())
                    .map_err(|e| format!("无效批量计划：{e}"))?;
            let confirmed = p
                .get("confirmed")
                .and_then(Value::as_bool)
                .ok_or("confirmed需要布尔值")?;
            Ok(json!(s.apply_bulk(&plan, confirmed)?))
        }
        "runs.stop_all" => {
            config::fields(p, &[])?;
            s.runtime.stop_all();
            Ok(json!({"stop_requested":true}))
        }
        "logs.open_target" => {
            config::fields(p, &["run_id", "target"])?;
            let run = run(s, config::id(p, "run_id")?)?;
            crate::console_text::open(config::text(p, "target")?, &run.cwd)?;
            Ok(json!({"opened":true}))
        }
        "logs.search" => {
            config::fields(
                p,
                &["run_id", "keyword", "case_sensitive", "offset", "limit"],
            )?;
            let id = config::id(p, "run_id")?;
            let query = crate::console_search::Query {
                keyword: config::text(p, "keyword")?.into(),
                case_sensitive: p
                    .get("case_sensitive")
                    .map(|v| v.as_bool().ok_or("case_sensitive需要布尔值"))
                    .transpose()?
                    .unwrap_or(false),
                only_matches: true,
            };
            let offset = p
                .get("offset")
                .map(|_| config::id(p, "offset"))
                .transpose()?
                .unwrap_or(0)
                .min(usize::MAX as u64) as usize;
            let limit = p
                .get("limit")
                .map(|_| config::id(p, "limit"))
                .transpose()?
                .unwrap_or(100)
                .clamp(1, 200) as usize;
            let run = s
                .runtime
                .runs
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("当前服务中没有该运行记录，历史日志请用logs.read")?;
            let view = run.lines.query(&query, None, 0)?;
            let rows:Vec<_>=view.rows.iter().skip(offset).take(limit).filter_map(|row|run.lines.get(*row).map(|(stderr,text)|json!({"cached_row":row,"line_number":run.lines.dropped+row+1,"stderr":stderr,"text":text,"highlights":crate::console_search::spans(text,&query).iter().map(|r|json!({"start":r.start,"end":r.end})).collect::<Vec<_>>()}))).collect();
            let next = offset.saturating_add(rows.len());
            Ok(
                json!({"run_id":id,"source":"console_cache","cached_lines":run.lines.len(),"dropped_lines":run.lines.dropped,"matched_lines":view.matched_lines,"rows":rows,"next_offset":next,"has_more":next<view.rows.len()}),
            )
        }
        "logs.list" => {
            config::fields(p, &["after_id", "limit"])?;
            let after = p
                .get("after_id")
                .map(|_| config::id(p, "after_id"))
                .transpose()?
                .unwrap_or(0);
            let limit = p
                .get("limit")
                .map(|_| config::id(p, "limit"))
                .transpose()?
                .unwrap_or(100)
                .clamp(1, 1000) as usize;
            let mut files = std::collections::BTreeMap::new();
            for entry in std::fs::read_dir(s.store.root.join("logs")).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                if !entry.file_type().map_err(|e| e.to_string())?.is_file() {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if let Some(id) = name
                    .strip_prefix("run-")
                    .and_then(|v| v.strip_suffix(".log"))
                    .and_then(|v| v.parse::<u64>().ok())
                    .filter(|id| *id > after)
                {
                    files.insert(id, entry.metadata().map_err(|e| e.to_string())?.len());
                    if files.len() > limit + 1 {
                        files.pop_last();
                    }
                }
            }
            let more = files.len() > limit;
            if more {
                files.pop_last();
            }
            let files: Vec<_> = files.into_iter().collect();
            Ok(
                json!({"logs":files.iter().map(|(id,size)|json!({"run_id":id,"size":size})).collect::<Vec<_>>(),"next_after_id":files.last().map(|v|v.0).unwrap_or(after),"has_more":more}),
            )
        }
        "logs.read" => {
            config::fields(p, &["run_id", "offset", "limit", "tail_bytes"])?;
            let run_id = config::id(p, "run_id")?;
            let active = s
                .runtime
                .runs
                .iter()
                .find(|r| r.id == run_id)
                .is_some_and(|r| r.state.active());
            let path = s.store.root.join("logs").join(format!("run-{run_id}.log"));
            if std::fs::symlink_metadata(&path).is_ok_and(|m| !m.file_type().is_file()) {
                return Err("日志路径不是普通文件".into());
            }
            if p.get("offset").is_some() && p.get("tail_bytes").is_some() {
                return Err("offset和tail_bytes不能同时指定".into());
            }
            let requested = p
                .get("offset")
                .map(|_| config::id(p, "offset"))
                .transpose()?;
            let tail = p
                .get("tail_bytes")
                .map(|_| config::id(p, "tail_bytes"))
                .transpose()?;
            let limit = p
                .get("limit")
                .map(|_| config::id(p, "limit"))
                .transpose()?
                .unwrap_or(16384)
                .clamp(1, 65536);
            let mut page = match crate::log_store::read(&path, requested, tail, limit) {
                Ok(page) => serde_json::to_value(page).map_err(|e| e.to_string())?,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound && active => json!({
                    "offset":0,"next_offset":0,"size":0,"base_offset":0,"retained_bytes":0,
                    "text":"","bytes":[],"reset":true,"eof":true}),
                Err(e) => return Err(e.to_string()),
            };
            page["run_id"] = json!(run_id);
            page["active"] = json!(active);
            Ok(page)
        }
        _ => config::handle(s, method, p),
    }
}
fn run(s: &Service, id: u64) -> Result<&crate::runtime::Run, String> {
    s.runtime
        .runs
        .iter()
        .find(|r| r.id == id)
        .ok_or("运行实例不存在".into())
}
fn run_value(run: &crate::runtime::Run) -> Value {
    let timing = run.timing();
    json!({"id":run.id,"script_id":run.script_id,"associated_scripts":run.associated_scripts,
        "console_id":run.console_id,"superseded":run.superseded,"tab_open":run.tab_open,"close_requested":run.close_requested,"title":run.title,"launch_name":run.launch_name,"display_command":run.display_command,
        "state":match run.state { crate::runtime::State::Starting=>"starting",crate::runtime::State::Running=>"running",crate::runtime::State::Stopping=>"stopping",crate::runtime::State::Succeeded=>"succeeded",crate::runtime::State::Stopped=>"stopped",crate::runtime::State::Failed=>"failed" },
        "state_display":crate::presentation::run_state_display(run),"pid":run.pid,"listening_ports":if run.state.active(){run.listening_ports.clone()}else{vec![]},"active":run.state.active(),"external":run.external.is_some(),"exit_code":run.exit,
        "elapsed_seconds":timing.elapsed_seconds,"started_at":timing.started_at,"started_at_unix_ms":timing.started_at_unix_ms,
        "elapsed_display":timing.elapsed_display,"timing_display":timing.display,
        "log_path":run.log_path,"cwd":run.cwd})
}
