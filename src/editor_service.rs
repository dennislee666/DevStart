//! Editor association and protocol adapter; file access is restricted to registered startup sources.
use crate::{editor, service::Service};
use serde_json::{Value, json};
use std::path::PathBuf;
impl Service {
    pub fn editor_source(&self, sid: u64) -> Result<PathBuf, String> {
        let file = &self.script(sid).ok_or("启动项不存在")?.file;
        editor::read_source(file)?;
        file.canonicalize()
            .map_err(|e| format!("启动文件路径无效: {e}"))
    }
    pub fn editor_open(&mut self, sid: u64) -> Result<editor::Document, String> {
        let source = self.editor_source(sid)?;
        self.editor.open(&self.store.root, sid, &source)
    }
    pub fn editor_list(&self) -> Vec<Value> {
        let mut records: Vec<_> = self.editor.visits.iter().filter_map(|visit| {
            let project = self.catalog.projects.iter().find(|p| p.scripts.iter().any(|s| s.id == visit.script_id))?;
            let script = project.scripts.iter().find(|s| s.id == visit.script_id)?;
            let doc = self.editor.documents.get(&visit.script_id);
            let name = visit.path.file_name().unwrap_or_default().to_string_lossy();
            Some((doc.is_none(), std::cmp::Reverse(visit.touched), json!({
                "script_id":script.id,"filename":name,"path":visit.path,"project":project.name,
                "launch":script.display_label(),"open":doc.is_some(),"dirty":doc.is_some_and(|d|d.dirty),
                "tooltip":format!("{} · {}\n{}", project.name, script.display_label(), crate::service::path_label(&visit.path))
            })))
        }).collect();
        records.sort_by_key(|r| (r.0, r.1));
        records.into_iter().map(|r| r.2).collect()
    }
    pub fn editor_action(&mut self, method: &str, p: &Value) -> Result<Value, String> {
        use crate::api_config::{fields, id, text};
        if method == "editor.list" {
            fields(p, &[])?;
            return Ok(json!(self.editor_list()));
        }
        let sid = id(p, "script_id")?;
        self.script(sid).ok_or("启动项不存在")?;
        let result = match method {
            "editor.open" => {
                fields(p, &["script_id"])?;
                json!(self.editor_open(sid)?)
            }
            "editor.read" => {
                fields(p, &["script_id"])?;
                json!(self.editor.read(sid)?)
            }
            "editor.versions" => {
                fields(p, &["script_id"])?;
                let path = self.editor_source(sid)?;
                let mut versions = editor::versions(&self.store.root, sid, &path)?;
                for v in &mut versions {
                    v.content = None;
                }
                json!(versions)
            }
            "editor.change" => {
                fields(p, &["script_id", "expected_revision", "content"])?;
                self.editor_verify_source(sid)?;
                json!(self.editor.change(
                    sid,
                    text(p, "expected_revision")?,
                    text(p, "content")?.into()
                )?)
            }
            "editor.save" => {
                fields(p, &["script_id", "expected_revision"])?;
                self.editor_verify_source(sid)?;
                let doc = self
                    .editor
                    .save(&self.store.root, sid, text(p, "expected_revision")?)?;
                // Port provenance can depend on edited package/config content.
                self.port_hints = crate::configured_ports::collect(&self.catalog.projects);
                json!(doc)
            }
            "editor.reload" | "editor.close" => {
                fields(p, &["script_id", "expected_revision", "discard"])?;
                let discard = p
                    .get("discard")
                    .map(|v| v.as_bool().ok_or("discard需要布尔值"))
                    .transpose()?
                    .unwrap_or(false);
                let rev = text(p, "expected_revision")?;
                if method == "editor.close" {
                    self.editor.close(&self.store.root, sid, rev, discard)?;
                    json!({"closed":sid})
                } else {
                    self.editor_verify_source(sid)?;
                    json!(self.editor.reload(sid, rev, discard)?)
                }
            }
            "editor.restore" => {
                fields(p, &["script_id", "expected_revision", "version_id"])?;
                self.editor_verify_source(sid)?;
                let doc = self.editor.restore(
                    &self.store.root,
                    sid,
                    text(p, "expected_revision")?,
                    if let Some(value) = p["version_id"].as_str() {
                        value
                            .parse::<u64>()
                            .map_err(|_| "version_id需要十进制版本ID")?
                    } else {
                        id(p, "version_id")?
                    },
                )?;
                self.port_hints = crate::configured_ports::collect(&self.catalog.projects);
                json!(doc)
            }
            _ => return Err("编辑器方法不存在".into()),
        };
        Ok(result)
    }
    fn editor_verify_source(&self, sid: u64) -> Result<(), String> {
        if self.editor_source(sid)? != self.editor.read(sid)?.path {
            return Err("启动项源文件已改变，请关闭并重新打开".into());
        }
        Ok(())
    }
    pub fn snapshot_launch(&self, sid: u64) -> Result<editor::Version, String> {
        let source = self.editor_source(sid)?;
        editor::snapshot(
            &self.store.root,
            sid,
            &source,
            &editor::read_source(&source)?,
            "launch",
        )
    }
}
