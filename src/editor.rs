//! Backend-owned source buffers and bounded, private revision storage.
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    fs,
    io::{Read, Write},
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
};

static VERSION_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub const MAX_BYTES: usize = 1_048_576;
pub const MAX_OPEN: usize = 16;
pub const MAX_HISTORY: usize = 10;
pub const MAX_VERSIONS: usize = 10;
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Visit {
    pub script_id: u64,
    pub path: PathBuf,
    pub touched: u64,
}
#[derive(Clone, Serialize)]
pub struct Document {
    pub script_id: u64,
    pub path: PathBuf,
    pub content: String,
    pub revision: String,
    pub disk_revision: String,
    pub dirty: bool,
    pub language: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Version {
    #[serde(with = "version_id_json")]
    pub id: u64,
    pub path: PathBuf,
    pub revision: String,
    pub reason: String,
    pub created_ms: u64,
    #[serde(default)]
    pub source_modified_ns: Option<u64>,
    pub bytes: usize,
    #[serde(skip)]
    pub created: bool,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub content: Option<String>,
}
#[derive(Default)]
pub struct Workspace {
    pub visits: Vec<Visit>,
    pub documents: HashMap<u64, Document>,
}
pub fn revision(text: &str) -> String {
    format!("{:x}", Sha256::digest(text.as_bytes()))
}
pub fn language(path: &Path) -> String {
    match path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
        .as_str()
    {
        "ts" | "tsx" | "jsx" | "mjs" | "cjs" => "js".into(),
        "vue" | "svelte" => "html".into(),
        "yml" => "yaml".into(),
        "bash" | "zsh" => "sh".into(),
        "pyw" => "py".into(),
        "" => "txt".into(),
        ext => ext.into(),
    }
}
pub fn read_source(path: &Path) -> Result<String, String> {
    let meta =
        fs::symlink_metadata(path).map_err(|e| format!("读取启动文件 {}: {e}", path.display()))?;
    if !meta.is_file() || meta.file_type().is_symlink() {
        return Err("启动文件必须为普通文件，不能是符号链接".into());
    }
    if meta.len() > MAX_BYTES as u64 {
        return Err("启动文件超过 1 MiB 编辑上限".into());
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(MAX_BYTES as u64 + 1)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > MAX_BYTES {
        return Err("读取时文件超过 1 MiB".into());
    }
    let text = String::from_utf8(data).map_err(|_| "启动文件不是 UTF-8 文本，未修改原文件")?;
    if text.contains('\0') {
        return Err("不支持编辑二进制文件".into());
    }
    Ok(text)
}
fn private_dir(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|e| e.to_string())?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())
}
fn version_dir(root: &Path, sid: u64) -> PathBuf {
    root.join("source-versions").join(sid.to_string())
}
pub fn versions(root: &Path, sid: u64, path: &Path) -> Result<Vec<Version>, String> {
    let _guard = VERSION_LOCK.lock().map_err(|_| "版本存储锁异常")?;
    read_versions(root, sid, path)
}
fn read_versions(root: &Path, sid: u64, path: &Path) -> Result<Vec<Version>, String> {
    let dir = version_dir(root, sid);
    if !dir.exists() {
        return Ok(vec![]);
    }
    let mut result = Vec::new();
    for entry in fs::read_dir(dir).map_err(|e| e.to_string())? {
        let file = entry.map_err(|e| e.to_string())?.path();
        if file.extension().is_some_and(|s| s == "json") {
            let data = fs::read(&file).map_err(|e| e.to_string())?;
            if data.len() > MAX_BYTES * 7 {
                return Err("版本文件超过限制".into());
            }
            let value: Version = serde_json::from_slice(&data)
                .map_err(|e| format!("版本损坏 {}: {e}", file.display()))?;
            if value.path == path {
                result.push(value);
            }
        }
    }
    result.sort_by_key(|v| std::cmp::Reverse(v.id));
    Ok(result)
}
/// Capture before destructive changes. The on-disk ring never exceeds ten successful snapshots.
pub fn snapshot(
    root: &Path,
    sid: u64,
    path: &Path,
    text: &str,
    reason: &str,
) -> Result<Version, String> {
    let _guard = VERSION_LOCK.lock().map_err(|_| "版本存储锁异常")?;
    if text.len() > MAX_BYTES {
        return Err("版本内容超过 1 MiB".into());
    }
    let dir = version_dir(root, sid);
    private_dir(&root.join("source-versions"))?;
    private_dir(&dir)?;
    let metadata = fs::symlink_metadata(path)
        .map_err(|e| format!("启动文件元数据 {}: {e}", path.display()))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("启动文件必须为普通文件，不能是符号链接".into());
    }
    let modified_ns = metadata
        .modified()
        .map_err(|e| e.to_string())?
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|e| e.to_string())?
        .as_nanos()
        .try_into()
        .map_err(|_| "启动文件修改时间超出范围")?;
    let source_bytes = metadata.len();
    let digest = revision(text);
    let existing = read_versions(root, sid, path)?;
    if reason == "launch" {
        if revision(&read_source(path)?) != digest {
            return Err("启动文件在备份检查期间发生修改，请重新启动".into());
        }
        if let Some(last) = existing.first()
            && (last.revision == digest
                || (last.source_modified_ns == Some(modified_ns)
                    && last.bytes as u64 == source_bytes))
        {
            let mut unchanged = last.clone();
            unchanged.created = false;
            return Ok(unchanged);
        }
    }
    if ["save", "restore"].contains(&reason) && path.extension().is_some_and(|s| s == "java") {
        crate::store::atomic_write(
            &dir.join("edited-source"),
            &serde_json::to_vec(path).map_err(|e| e.to_string())?,
        )?;
    }
    let id = crate::model::unique_id();
    let value = Version {
        id,
        path: path.to_owned(),
        revision: digest,
        reason: reason.into(),
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_millis() as u64,
        source_modified_ns: Some(modified_ns),
        bytes: usize::try_from(source_bytes).map_err(|_| "启动文件大小超出范围")?,
        created: true,
        content: Some(text.to_owned()),
    };
    crate::store::atomic_write(
        &dir.join(format!("{id}.json")),
        &serde_json::to_vec(&value).map_err(|e| e.to_string())?,
    )?;
    // Bound all files for this startup, including snapshots from a previously configured source.
    let mut files: Vec<_> = fs::read_dir(&dir)
        .map_err(|e| e.to_string())?
        .map(|entry| entry.map(|e| e.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("读取版本目录失败: {e}"))?
        .into_iter()
        .filter(|p| p.extension().is_some_and(|s| s == "json"))
        .collect();
    files.sort_by_key(|p| {
        std::cmp::Reverse(
            p.file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse::<u64>().ok())
                .unwrap_or(0),
        )
    });
    for file in files.into_iter().skip(MAX_VERSIONS) {
        fs::remove_file(file).map_err(|e| e.to_string())?;
    }
    Ok(value)
}
fn write_source(path: &Path, content: &str, expected: &str) -> Result<(), String> {
    if revision(&read_source(path)?) != expected {
        return Err("文件已被外部修改；请重新载入后再保存，当前草稿仍保留".into());
    }
    let mode = fs::metadata(path).map_err(|e| e.to_string())?.permissions();
    let temporary = path.with_extension(format!("devstart-{}.tmp", crate::model::unique_id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)
            .map_err(|e| e.to_string())?;
        file.write_all(content.as_bytes())
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        file.set_permissions(mode).map_err(|e| e.to_string())?;
        // Recheck after writing the temporary file, before the atomic replacement.
        if revision(&read_source(path)?) != expected {
            return Err("文件在保存期间被外部修改，拒绝覆盖".into());
        }
        fs::rename(&temporary, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result
}
impl Workspace {
    pub fn load(root: &Path) -> Result<Self, String> {
        let path = root.join("editor-history.json");
        let visits: Vec<Visit> = if path.exists() {
            serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("编辑历史损坏: {e}"))?
        } else {
            vec![]
        };
        let mut workspace = Self {
            visits,
            documents: HashMap::new(),
        };
        workspace.persist_visits(root, workspace.visits.clone(), None, None)?;
        Ok(workspace)
    }
    fn persist_visits(
        &mut self,
        root: &Path,
        mut next: Vec<Visit>,
        opening: Option<u64>,
        closing: Option<u64>,
    ) -> Result<(), String> {
        let is_open = |id: u64| {
            opening == Some(id) || (self.documents.contains_key(&id) && closing != Some(id))
        };
        next.sort_by_key(|v| (!is_open(v.script_id), std::cmp::Reverse(v.touched)));
        let mut seen = std::collections::HashSet::new();
        let mut history = 0;
        next.retain(|v| {
            if !seen.insert(v.script_id) {
                return false;
            }
            if is_open(v.script_id) {
                true
            } else {
                history += 1;
                history <= MAX_HISTORY
            }
        });
        if next != self.visits || !root.join("editor-history.json").exists() {
            crate::store::atomic_write(
                &root.join("editor-history.json"),
                &serde_json::to_vec(&next).map_err(|e| e.to_string())?,
            )?;
        }
        self.visits = next;
        Ok(())
    }
    fn touch(&mut self, root: &Path, sid: u64, path: &Path) -> Result<(), String> {
        let mut next = self.visits.clone();
        next.retain(|v| v.script_id != sid);
        next.push(Visit {
            script_id: sid,
            path: path.into(),
            touched: crate::model::unique_id(),
        });
        self.persist_visits(root, next, Some(sid), None)
    }
    pub fn open(&mut self, root: &Path, sid: u64, path: &Path) -> Result<Document, String> {
        if let Some(doc) = self.documents.get(&sid) {
            if doc.path != path {
                return Err("启动项源文件已变化，请先关闭旧编辑缓冲区".into());
            }
            let doc = doc.clone();
            self.touch(root, sid, path)?;
            return Ok(doc);
        }
        if self.documents.len() >= MAX_OPEN {
            return Err("最多同时编辑 16 个文件，请先关闭部分文件".into());
        }
        let content = read_source(path)?;
        if versions(root, sid, path)?.is_empty() {
            snapshot(root, sid, path, &content, "baseline")?;
        }
        self.touch(root, sid, path)?;
        let rev = revision(&content);
        let doc = Document {
            script_id: sid,
            path: path.into(),
            content,
            revision: rev.clone(),
            disk_revision: rev,
            dirty: false,
            language: language(path),
        };
        self.documents.insert(sid, doc.clone());
        Ok(doc)
    }
    pub fn read(&self, sid: u64) -> Result<&Document, String> {
        self.documents
            .get(&sid)
            .ok_or("文件未打开，请先打开编辑器".into())
    }
    fn check(&self, sid: u64, expected: &str) -> Result<&Document, String> {
        let doc = self.read(sid)?;
        if doc.revision != expected {
            return Err("编辑缓冲区已变化，请重新读取后重试".into());
        }
        Ok(doc)
    }
    pub fn change(
        &mut self,
        sid: u64,
        expected: &str,
        content: String,
    ) -> Result<Document, String> {
        self.check(sid, expected)?;
        if content.len() > MAX_BYTES || content.contains('\0') {
            return Err("内容需为不超过 1 MiB 的 UTF-8 文本".into());
        }
        let doc = self.documents.get_mut(&sid).unwrap();
        doc.revision = revision(&content);
        doc.content = content;
        doc.dirty = doc.revision != doc.disk_revision;
        Ok(doc.clone())
    }
    pub fn save(&mut self, root: &Path, sid: u64, expected: &str) -> Result<Document, String> {
        let doc = self.check(sid, expected)?.clone();
        self.replace(root, sid, &doc, doc.content.clone(), "save")
    }
    fn replace(
        &mut self,
        root: &Path,
        sid: u64,
        doc: &Document,
        content: String,
        reason: &str,
    ) -> Result<Document, String> {
        let current = read_source(&doc.path)?;
        if revision(&current) != doc.disk_revision {
            return Err("文件已被外部修改；请重新载入，当前草稿仍保留".into());
        }
        let existing = versions(root, sid, &doc.path)?;
        if existing
            .first()
            .is_none_or(|v| v.revision != doc.disk_revision)
        {
            snapshot(root, sid, &doc.path, &current, "before-change")?;
        }
        if content != current {
            let version = snapshot(root, sid, &doc.path, &content, reason)?;
            if let Err(e) = write_source(&doc.path, &content, &doc.disk_revision) {
                fs::remove_file(version_dir(root, sid).join(format!("{}.json", version.id)))
                    .map_err(|cleanup| format!("{e}; 清理未提交版本失败: {cleanup}"))?;
                return Err(e);
            }
        }
        let revision = revision(&content);
        let saved = Document {
            content,
            revision: revision.clone(),
            disk_revision: revision,
            dirty: false,
            ..doc.clone()
        };
        self.documents.insert(sid, saved.clone());
        Ok(saved)
    }
    pub fn reload(&mut self, sid: u64, expected: &str, discard: bool) -> Result<Document, String> {
        let doc = self.check(sid, expected)?.clone();
        if doc.dirty && !discard {
            return Err("存在未保存修改，请选择保存或放弃草稿".into());
        }
        let content = read_source(&doc.path)?;
        let rev = revision(&content);
        let loaded = Document {
            content,
            revision: rev.clone(),
            disk_revision: rev,
            dirty: false,
            ..doc
        };
        self.documents.insert(sid, loaded.clone());
        Ok(loaded)
    }
    pub fn close(
        &mut self,
        root: &Path,
        sid: u64,
        expected: &str,
        discard: bool,
    ) -> Result<(), String> {
        if self.check(sid, expected)?.dirty && !discard {
            return Err("文件有未保存修改，请先保存或明确放弃".into());
        }
        let mut next = self.visits.clone();
        if let Some(visit) = next.iter_mut().find(|v| v.script_id == sid) {
            visit.touched = crate::model::unique_id();
        }
        self.persist_visits(root, next, None, Some(sid))?;
        self.documents.remove(&sid);
        Ok(())
    }
    pub fn restore(
        &mut self,
        root: &Path,
        sid: u64,
        expected: &str,
        version: u64,
    ) -> Result<Document, String> {
        let doc = self.check(sid, expected)?.clone();
        if doc.dirty {
            return Err("请先保存或放弃未保存修改，再回退版本".into());
        }
        let target = versions(root, sid, &doc.path)?
            .into_iter()
            .find(|v| v.id == version)
            .ok_or("版本不存在或已超出最近 10 次")?;
        let content = target.content.ok_or("版本缺少内容")?;
        if revision(&content) != target.revision {
            return Err("版本内容校验失败，拒绝回退".into());
        }
        self.replace(root, sid, &doc, content, "restore")
    }
}

pub fn reason_label(reason: &str) -> &str {
    match reason {
        "baseline" => "初始版本",
        "save" => "保存",
        "launch" => "启动备份",
        "restore" => "回退",
        "before-change" => "修改前",
        _ => reason,
    }
}
pub fn display_time(ms: u64) -> String {
    crate::time_display::local_timestamp(ms / 1000)
}

// JSON strings preserve 64-bit version IDs through JavaScript/AI clients without rounding.
mod version_id_json {
    use serde::{Deserialize, Deserializer, Serializer};
    pub fn serialize<S: Serializer>(value: &u64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_string())
    }
    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Id {
            Text(String),
            Integer(u64),
        }
        match Id::deserialize(deserializer)? {
            Id::Text(value) => value.parse().map_err(serde::de::Error::custom),
            Id::Integer(value) => Ok(value),
        }
    }
}
