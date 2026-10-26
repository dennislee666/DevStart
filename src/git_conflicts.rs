//! Version-checked conflict review and resolution shared by UI/API.
use super::{git, mutation_guard, root};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Write,
    path::{Component, Path, PathBuf},
};
const LIMIT: usize = 1_048_576;
#[derive(Clone, Serialize)]
pub struct Document {
    pub file: String,
    pub token: String,
    pub base: Option<String>,
    pub current: Option<String>,
    pub incoming: Option<String>,
    pub result: String,
    pub editable: bool,
    pub current_label: String,
    pub incoming_label: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "choice", content = "text", rename_all = "snake_case")]
pub enum Resolution {
    Current,
    Incoming,
    Edited(String),
}
fn path_for(root: &Path, file: &str) -> Result<PathBuf, String> {
    if Path::new(file)
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("Invalid repository-relative conflict path".into());
    }
    let path = root.join(file);
    let mut parent = path.parent();
    while let Some(dir) = parent {
        if dir == root {
            break;
        }
        if std::fs::symlink_metadata(dir).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("Conflict parent directory is a symbolic link".into());
        }
        parent = dir.parent();
    }
    Ok(path)
}
fn working(path: &Path) -> Result<Vec<u8>, String> {
    match std::fs::symlink_metadata(path) {
        Ok(m) if m.file_type().is_symlink() => Ok(std::fs::read_link(path)
            .map_err(|e| e.to_string())?
            .as_os_str()
            .as_encoded_bytes()
            .to_vec()),
        Ok(m) if m.is_file() && m.len() <= LIMIT as u64 => {
            std::fs::read(path).map_err(|e| e.to_string())
        }
        Ok(_) => Err("冲突文件过大或不是普通文件，请在外部编辑器处理。".into()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(vec![]),
        Err(e) => Err(e.to_string()),
    }
}
fn text_blob(path: &Path, hash: Option<&str>) -> Option<String> {
    let hash = hash?;
    let size: usize = git(path, &["cat-file", "-s", hash]).ok()?.parse().ok()?;
    if size > LIMIT {
        return None;
    }
    let text = super::git_output_raw(path, &["cat-file", "blob", hash])
        .ok()?
        .0;
    if text.contains('\0') {
        None
    } else {
        Some(text)
    }
}
pub fn inspect(path: &Path, file: &str) -> Result<Document, String> {
    let root = root(path)?;
    let path = path_for(&root, file)?;
    let stages = git(&root, &["ls-files", "-u", "--", file])?;
    if stages.is_empty() {
        return Err("文件已不再处于冲突状态，请刷新。".into());
    }
    let mut hashes: [Option<String>; 3] = [None, None, None];
    for line in stages.lines() {
        let mut fields = line.split_whitespace();
        let _mode = fields.next();
        let hash = fields.next().ok_or("Invalid Git conflict stage")?;
        let stage: usize = fields
            .next()
            .ok_or("Invalid Git conflict stage")?
            .parse::<usize>()
            .map_err(|e| e.to_string())?;
        if (1..=3).contains(&stage) {
            hashes[stage - 1] = Some(hash.into());
        }
    }
    let bytes = working(&path)?;
    let token = format!(
        "{:x}",
        Sha256::digest([stages.as_bytes(), &[0], &bytes].concat())
    );
    let base = text_blob(&root, hashes[0].as_deref());
    let current = text_blob(&root, hashes[1].as_deref());
    let incoming = text_blob(&root, hashes[2].as_deref());
    let regular = std::fs::symlink_metadata(&path).map_or(true, |m| m.is_file());
    let editable = regular
        && !bytes.contains(&0)
        && String::from_utf8(bytes.clone()).is_ok()
        && (hashes[1].is_none() || current.is_some())
        && (hashes[2].is_none() || incoming.is_some());
    let state = super::integration::state(&root)?;
    let (current_label, incoming_label) = if state.operation.as_deref() == Some("rebase") {
        ("目标分支（Rebase 基线）", "正在重放的本地提交")
    } else if state.recovering && state.operation.is_none() {
        ("更新后的分支", "保存的本地改动")
    } else {
        ("当前分支", "传入分支")
    };
    Ok(Document {
        file: file.into(),
        token,
        base,
        current,
        incoming,
        result: String::from_utf8(bytes).unwrap_or_default(),
        editable,
        current_label: current_label.into(),
        incoming_label: incoming_label.into(),
    })
}
pub fn resolve(
    path: &Path,
    file: &str,
    token: &str,
    resolution: Resolution,
) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let doc = inspect(&root, file)?;
    if doc.token != token {
        return Err("冲突内容已变化，请重新打开文件后再应用。".into());
    }
    let work = path_for(&root, file)?;
    // Save the exact pre-resolution worktree and index stages before writing anything.
    let backup = root
        .join(git(
            &root,
            &["rev-parse", "--git-path", "devstart-conflict-backups"],
        )?)
        .join(crate::model::unique_id().to_string());
    std::fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
    std::fs::write(backup.join("working-copy"), working(&work)?).map_err(|e| e.to_string())?;
    std::fs::write(
        backup.join("conflict.json"),
        serde_json::to_vec(&doc).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    match resolution {
        Resolution::Edited(text) => {
            if !doc.editable || text.len() > LIMIT || text.contains('\0') {
                return Err("此文件不能在内置合并编辑器中保存。".into());
            }
            if text.lines().any(|line| {
                line.starts_with("<<<<<<< ") || line.starts_with(">>>>>>> ") || line == "======="
            }) {
                return Err("结果中仍包含冲突标记，请处理后再应用。".into());
            }
            let temporary =
                work.with_file_name(format!(".devstart-merge-{}", crate::model::unique_id()));
            let written = (|| -> Result<(), String> {
                let mut output = std::fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&temporary)
                    .map_err(|e| e.to_string())?;
                output
                    .write_all(text.as_bytes())
                    .and_then(|_| output.sync_all())
                    .map_err(|e| e.to_string())?;
                if let Ok(metadata) = std::fs::metadata(&work) {
                    std::fs::set_permissions(&temporary, metadata.permissions())
                        .map_err(|e| e.to_string())?;
                }
                std::fs::rename(&temporary, &work).map_err(|e| e.to_string())
            })();
            if written.is_err() {
                let _ = std::fs::remove_file(&temporary);
            }
            written?;
            git(&root, &["add", "-A", "--", file])?;
        }
        side => {
            let (stage, flag) = match side {
                Resolution::Current => ("2", "--ours"),
                _ => ("3", "--theirs"),
            };
            let index = git(&root, &["ls-files", "-u", "--", file])?;
            if index
                .lines()
                .any(|line| line.split_whitespace().nth(2) == Some(stage))
            {
                git(&root, &["checkout", flag, "--", file])?;
                git(&root, &["add", "-A", "--", file])?;
            } else {
                git(&root, &["rm", "-f", "--", file])?;
            }
        }
    }
    Ok(format!("已解决并暂存：{file}"))
}
