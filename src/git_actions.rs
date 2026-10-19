//! Explicit repository mutations and diff operations. Paths come from Git status, never a shell.
use super::{branches, git, git_output, mutation_guard, root};
use serde::Serialize;
use std::{
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub previous: Option<String>,
    pub status: String,
}
impl Change {
    /// Translate porcelain states for display; retain the original code for API consumers.
    pub fn status_label(&self) -> &'static str {
        match self.status.as_str() {
            "??" => "未跟踪",
            "!!" => "已忽略",
            "DD" | "AU" | "UD" | "UA" | "DU" | "AA" | "UU" => "冲突",
            code if code.contains('R') => "已重命名",
            code if code.contains('C') => "已复制",
            code if code.contains('D') => "已删除",
            code if code.contains('A') => "已新增",
            code if code.contains('M') || code.contains('T') => "已修改",
            _ => "文件变更",
        }
    }
}
pub fn changes(path: &Path) -> Result<Vec<Change>, String> {
    let root = root(path)?;
    let out = git(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    )?;
    let mut parts = out.split('\0').filter(|v| !v.is_empty());
    let mut result = vec![];
    while let Some(record) = parts.next() {
        if record.len() < 4 || !record.is_char_boundary(3) {
            return Err("Invalid Git status record".into());
        }
        let status = record[..2].to_owned();
        let previous = if status.contains('R') || status.contains('C') {
            Some(
                parts
                    .next()
                    .ok_or("Invalid renamed file record")?
                    .to_owned(),
            )
        } else {
            None
        };
        result.push(Change {
            path: record[3..].into(),
            previous,
            status,
        });
    }
    Ok(result)
}
fn selected(path: &Path, files: &[String]) -> Result<Vec<String>, String> {
    if files.len() > 1000 {
        return Err("Select at most 1000 files".into());
    }
    if files.is_empty() {
        return Err("Select at least one changed file".into());
    }
    let changes = changes(path)?;
    let mut result = vec![];
    for file in files {
        let item = changes
            .iter()
            .find(|c| &c.path == file)
            .ok_or_else(|| format!("File is no longer changed: {file}"))?;
        if !result.contains(file) {
            result.push(file.clone());
        }
        if let Some(old) = &item.previous
            && !result.contains(old)
        {
            result.push(old.clone());
        }
    }
    Ok(result)
}
pub fn diff(path: &Path, file: &str) -> Result<String, String> {
    let root = root(path)?;
    let items = changes(&root)?;
    let item = items
        .iter()
        .find(|c| c.path == file)
        .ok_or("File is no longer changed")?;
    if item.status == "??" {
        let mut bytes = Vec::new();
        std::fs::File::open(root.join(file))
            .map_err(|e| e.to_string())?
            .take(262145)
            .read_to_end(&mut bytes)
            .map_err(|e| e.to_string())?;
        if bytes.len() > 262144 {
            return Err("Untracked file exceeds preview limit (256 KiB)".into());
        }
        if bytes.contains(&0) {
            return Err("此文件不支持查看预览".into());
        }
        return String::from_utf8(bytes)
            .map(|s| {
                let lines = s
                    .lines()
                    .map(|line| format!("+{line}\n"))
                    .collect::<String>();
                format!(
                    "--- /dev/null\n+++ {file}\n@@ -0,0 +1,{} @@\n{lines}",
                    s.lines().count()
                )
            })
            .map_err(|_| "此文件不支持查看预览".into());
    }
    let files = selected(&root, &[file.into()])?;
    let mut args = vec![
        "diff",
        "--no-ext-diff",
        "--no-textconv",
        "--no-color",
        "HEAD",
        "--",
    ];
    args.extend(files.iter().map(String::as_str));
    // An unborn branch has only an index diff.
    let branch = git(&root, &["branch", "--show-current"])?;
    if !branch.is_empty() && !branches(&root)?.contains(&branch) {
        return text_patch(git(
            &root,
            &[
                "diff",
                "--cached",
                "--no-ext-diff",
                "--no-textconv",
                "--no-color",
                "--",
                file,
            ],
        )?);
    }
    text_patch(git(&root, &args)?)
}
fn text_patch(patch: String) -> Result<String, String> {
    if patch
        .lines()
        .any(|line| line.starts_with("Binary files ") || line == "GIT binary patch")
    {
        return Err("此文件不支持查看预览".into());
    }
    Ok(patch)
}
pub fn commit(path: &Path, message: &str, files: &[String]) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    commit_unlocked(path, message, files)
}
pub(super) fn commit_unlocked(
    path: &Path,
    message: &str,
    files: &[String],
) -> Result<String, String> {
    if message.trim().is_empty() || message.len() > 16384 {
        return Err("Commit message must contain 1..16384 bytes".into());
    }
    let root = root(path)?;
    let files = selected(&root, files)?;
    let mut add = vec!["add", "-A", "--"];
    add.extend(files.iter().map(String::as_str));
    git(&root, &add)?;
    let mut args = vec!["commit", "--only", "-m", message, "--"];
    args.extend(files.iter().map(String::as_str));
    let (out, err) = git_output(&root, &args).map_err(|e| {
        format!("{e}\nSelected files remain staged; inspect status before retrying.")
    })?;
    Ok(format!("{out}\n{err}"))
}
pub fn push(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let (out, err) = git_output(&root, &["push"])?;
    Ok(format!("{out}\n{err}"))
}
pub fn pull(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    if !changes(&root)?.is_empty() {
        return Err("Commit or stash local changes before pulling".into());
    }
    let (out, err) = git_output(&root, &["pull", "--no-rebase", "--no-edit"])?;
    Ok(format!("{out}\n{err}"))
}
/// Store reverse patches before restoring both index and worktree from HEAD. Keep ten snapshots.
pub fn restore(path: &Path, file: &str, backup_root: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let items = changes(&root)?;
    let item = items
        .iter()
        .find(|c| c.path == file)
        .ok_or("File is no longer changed")?;
    if item.status == "??" {
        return Err("Untracked files have no HEAD version; restore is unavailable".into());
    }
    let files = selected(&root, &[file.into()])?;
    let mut args = vec![
        "diff",
        "--binary",
        "--no-ext-diff",
        "--no-textconv",
        "HEAD",
        "--",
    ];
    args.extend(files.iter().map(String::as_str));
    let working = git(&root, &args)?;
    let mut args = vec![
        "diff",
        "--cached",
        "--binary",
        "--no-ext-diff",
        "--no-textconv",
        "HEAD",
        "--",
    ];
    args.extend(files.iter().map(String::as_str));
    let index = git(&root, &args)?;
    std::fs::create_dir_all(backup_root).map_err(|e| e.to_string())?;
    let backup = backup_root.join(crate::model::unique_id().to_string());
    std::fs::create_dir(&backup).map_err(|e| e.to_string())?;
    std::fs::write(backup.join("worktree.patch"), format!("{working}\n"))
        .map_err(|e| e.to_string())?;
    std::fs::write(backup.join("index.patch"), format!("{index}\n")).map_err(|e| e.to_string())?;
    std::fs::write(
        backup.join("metadata.json"),
        serde_json::to_vec(&serde_json::json!({"repository":root,"files":files}))
            .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let mut args = vec!["restore", "--source=HEAD", "--staged", "--worktree", "--"];
    args.extend(files.iter().map(String::as_str));
    git(&root, &args)?;
    let mut snapshots: Vec<PathBuf> = std::fs::read_dir(backup_root)
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.is_dir()
                && p.file_name()
                    .is_some_and(|s| s.to_string_lossy().chars().all(|c| c.is_ascii_digit()))
        })
        .collect();
    snapshots.sort();
    let excess = snapshots.len().saturating_sub(10);
    for old in snapshots.into_iter().take(excess) {
        std::fs::remove_dir_all(old).map_err(|e| e.to_string())?;
    }
    Ok(format!("Restored {file}\nBackup: {}", backup.display()))
}
/// A Git argument console, not a system shell. Quoting is supported; shell operators have no effect.
pub fn command(path: &Path, input: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    if input.len() > 8192 {
        return Err("Command exceeds 8192 bytes".into());
    }
    let mut args = vec![];
    let mut token = String::new();
    let mut quote = None;
    let mut escape = false;
    let mut started = false;
    for c in input.chars() {
        if escape {
            token.push(c);
            escape = false;
            started = true;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escape = true;
            started = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                token.push(c);
            }
            started = true;
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
            started = true;
        } else if c.is_whitespace() {
            if started {
                args.push(std::mem::take(&mut token));
                started = false;
            }
        } else {
            token.push(c);
            started = true;
        }
    }
    if quote.is_some() || escape {
        return Err("Unclosed command quote or escape".into());
    }
    if started {
        args.push(token);
    }
    if args.first().map(String::as_str) != Some("git") || args.len() < 2 {
        return Err("Enter a git command, for example: git status".into());
    }
    let root = root(path)?;
    let refs: Vec<_> = args[1..].iter().map(String::as_str).collect();
    let (out, err) = git_output(&root, &refs)?;
    Ok(format!("{out}\n{err}"))
}
