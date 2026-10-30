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
    /// True when Git reported a collapsed untracked directory entry.
    pub directory: bool,
}
impl Change {
    /// Resolve repository-relative Git paths for presentation without exposing porcelain codes.
    pub fn preview_path(&self, repository: &Path) -> String {
        repository.join(&self.path).display().to_string()
    }

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
    // Match Git's normal directory collapsing instead of enumerating every untracked descendant.
    let out = git(
        &root,
        &["status", "--porcelain=v1", "-z", "--untracked-files=normal"],
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
        let path = record[3..].to_owned();
        result.push(Change {
            directory: status == "??" && path.ends_with('/'),
            path,
            previous,
            status,
        });
    }
    Ok(result)
}
/// List paths with unresolved index stages before any action can stage or update them.
pub fn unresolved_conflicts(path: &Path) -> Result<Vec<String>, String> {
    let root = root(path)?;
    Ok(
        git(&root, &["diff", "--name-only", "--diff-filter=U", "-z"])?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect(),
    )
}
fn conflict_error(paths: &[String]) -> String {
    format!(
        "存在未解决的合并冲突：{}。请先解决冲突并暂存文件，再继续操作。",
        paths.join("、")
    )
}
fn selected(path: &Path, files: &[String]) -> Result<Vec<String>, String> {
    if files.len() > 10000 {
        return Err("Select at most 10000 files".into());
    }
    if files.is_empty() {
        return Err("Select at least one changed file".into());
    }
    let changes = changes(path)?;
    selected_from_changes(&changes, files)
}
fn selected_from_changes(changes: &[Change], files: &[String]) -> Result<Vec<String>, String> {
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
        if item.directory {
            return Err("未跟踪目录包含多个文件，无法作为单文件预览".into());
        }
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
struct CommitPathspec(PathBuf);
impl CommitPathspec {
    fn new(files: &[String]) -> Result<Self, String> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let path = std::env::temp_dir().join(format!(
            "devstart-commit-paths-{}",
            crate::model::unique_id()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let guard = Self(path);
        for path in files {
            file.write_all(path.as_bytes())
                .and_then(|_| file.write_all(&[0]))
                .map_err(|e| e.to_string())?;
        }
        Ok(guard)
    }
}
impl Drop for CommitPathspec {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}
struct TemporaryIndex(PathBuf);
impl Drop for TemporaryIndex {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
        let _ = std::fs::remove_file(self.0.with_extension("lock"));
    }
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
    let root = root(path)?;
    super::integration::ensure_idle(&root)?;
    let conflicts = unresolved_conflicts(&root)?;
    if !conflicts.is_empty() {
        return Err(conflict_error(&conflicts));
    }
    if message.trim().is_empty() || message.len() > 16384 {
        return Err("Commit message must contain 1..16384 bytes".into());
    }
    let changes = changes(&root)?;
    let files = selected_from_changes(&changes, files)?;
    // An index deletion may retain an ignored working copy after `rm --cached`.
    // Preserve that deletion instead of staging or committing the working copy again.
    let removed: std::collections::HashSet<_> = changes
        .iter()
        .filter(|c| c.status.starts_with('D'))
        .map(|c| c.path.as_str())
        .collect();
    let untracked: Vec<_> = files
        .iter()
        .filter(|p| {
            !removed.contains(p.as_str())
                && changes.iter().any(|c| c.path == **p && c.status == "??")
        })
        .cloned()
        .collect();
    let untracked_set: std::collections::HashSet<_> = untracked.iter().collect();
    let mut indexed = std::collections::HashSet::new();
    for chunk in files.chunks(200) {
        let mut args = vec!["ls-files", "-z", "--"];
        args.extend(chunk.iter().map(String::as_str));
        let output = super::git_output_raw(&root, &args)?.0;
        indexed.extend(
            output
                .split('\0')
                .filter(|p| !p.is_empty())
                .map(str::to_owned),
        );
    }
    let tracked: Vec<_> = files
        .iter()
        .filter(|p| {
            indexed.contains(*p) && !removed.contains(p.as_str()) && !untracked_set.contains(p)
        })
        .cloned()
        .collect();
    for (paths, mode) in [(&tracked, "-u"), (&untracked, "-A")] {
        if !paths.is_empty() {
            let pathspec = CommitPathspec::new(paths)?;
            let argument = format!("--pathspec-from-file={}", pathspec.0.display());
            git(&root, &["add", mode, &argument, "--pathspec-file-nul"])?;
        }
    }
    // Build the commit from HEAD plus selected index entries. `commit --only`
    // reads working files and would undo an intentional index-only removal.
    let index_path = PathBuf::from(git(&root, &["rev-parse", "--git-path", "index"])?);
    let index_path = if index_path.is_absolute() {
        index_path
    } else {
        root.join(index_path)
    };
    let temporary = TemporaryIndex(index_path.with_file_name(format!(
        "devstart-commit-index-{}",
        crate::model::unique_id()
    )));
    let head = git(&root, &["rev-parse", "--verify", "--quiet", "HEAD"]).ok();
    super::git_output_index(
        &root,
        if head.is_some() {
            &["read-tree", "HEAD"]
        } else {
            &["read-tree", "--empty"]
        },
        None,
        Some(&temporary.0),
    )?;
    let zero = match git(&root, &["rev-parse", "--show-object-format"])?.as_str() {
        "sha1" => "0".repeat(40),
        "sha256" => "0".repeat(64),
        _ => return Err("Unsupported Git object format".into()),
    };
    let mut input = Vec::new();
    for file in files.iter().filter(|p| !p.ends_with('/')) {
        input.extend_from_slice(format!("0 {zero}\t{file}\0").as_bytes());
    }
    for chunk in files.chunks(200) {
        let mut args = vec!["ls-files", "--stage", "-z", "--"];
        args.extend(chunk.iter().map(String::as_str));
        input.extend_from_slice(super::git_output_raw(&root, &args)?.0.as_bytes());
    }
    super::git_output_index(
        &root,
        &["update-index", "-z", "--index-info"],
        Some(&input),
        Some(&temporary.0),
    )?;
    let (out, err) =
        super::git_output_index(&root, &["commit", "-m", message], None, Some(&temporary.0))
            .map_err(|e| {
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
    let root = root(path)?;
    let branch = git(&root, &["branch", "--show-current"])?;
    let remotes = super::remotes::list(&root)?;
    let remote = remotes
        .iter()
        .find(|r| r.preferred)
        .ok_or("No remote configured")?;
    super::remotes::update(&root, &remote.key, &branch)
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

/// Ignore exactly a current untracked status entry; never remove tracked files from the index.
pub fn ignore(path: &Path, file: &str) -> Result<String, String> {
    let path = root(path)?;
    let _guard = mutation_guard(&path)?;
    let entry = changes(&path)?
        .into_iter()
        .find(|c| c.path == file && c.status == "??")
        .ok_or("只能忽略当前未跟踪的文件或目录，请刷新后重试")?;
    add_ignore_rule(&path, file, entry.directory)
}
fn add_ignore_rule(path: &Path, file: &str, directory: bool) -> Result<String, String> {
    if file.chars().any(char::is_control)
        || Path::new(file).is_absolute()
        || Path::new(file)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("无法安全表示此文件的忽略规则".into());
    }
    let target = path.join(".gitignore");
    if let Ok(meta) = std::fs::symlink_metadata(&target) {
        if !meta.is_file() || meta.file_type().is_symlink() {
            return Err(".gitignore 必须为普通文件".into());
        }
    }
    let mut data = match std::fs::read(&target) {
        Ok(data) => data,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Vec::new(),
        Err(e) => return Err(e.to_string()),
    };
    let mut rule = String::from("/");
    for ch in file.chars() {
        if matches!(ch, '\\' | '*' | '?' | '[' | ']' | ' ' | '#' | '!') {
            rule.push('\\');
        }
        rule.push(ch);
    }
    if directory && !rule.ends_with('/') {
        rule.push('/');
    }
    let newline = if data.windows(2).any(|w| w == b"\r\n") {
        b"\r\n".as_slice()
    } else {
        b"\n".as_slice()
    };
    if !data.is_empty() && !data.ends_with(b"\n") {
        data.extend_from_slice(newline);
    }
    data.extend_from_slice(rule.as_bytes());
    data.extend_from_slice(newline);
    // A new sibling temporary file avoids partial .gitignore contents on write failure.
    let temp = path.join(format!(".gitignore.devstart-{}", crate::model::unique_id()));
    let result = (|| {
        use std::io::Write;
        let mut out = std::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        out.write_all(&data)
            .and_then(|_| out.sync_all())
            .map_err(|e| e.to_string())?;
        if let Ok(meta) = std::fs::metadata(&target) {
            std::fs::set_permissions(&temp, meta.permissions()).map_err(|e| e.to_string())?;
        }
        std::fs::rename(&temp, &target).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(format!("已添加忽略规则：{rule}"))
}

/// Remove a single tracked path from the index, preserving the working file.
/// Never force removal when Git detects unique staged content.
pub fn untrack(path: &Path, file: &str, ignore_file: bool) -> Result<String, String> {
    let path = root(path)?;
    let _guard = mutation_guard(&path)?;
    let change = changes(&path)?
        .into_iter()
        .find(|c| c.path == file)
        .ok_or("文件状态已变化，请刷新后重试")?;
    if change.status == "??"
        || change.status == "!!"
        || change.directory
        || change
            .status
            .chars()
            .any(|c| !matches!(c, ' ' | 'M' | 'A' | 'T'))
    {
        return Err("仅支持撤销跟踪已修改或新增的非冲突文件".into());
    }
    if file.chars().any(char::is_control)
        || Path::new(file).is_absolute()
        || Path::new(file)
            .components()
            .any(|c| !matches!(c, std::path::Component::Normal(_)))
    {
        return Err("无效文件路径".into());
    }
    std::fs::symlink_metadata(path.join(file)).map_err(|e| format!("本地文件不可用：{e}"))?;
    git(&path, &["rm", "--cached", "--dry-run", "--", file])?;
    let ignore_path = path.join(".gitignore");
    let before = if ignore_file {
        match std::fs::read(&ignore_path) {
            Ok(bytes) => Some(bytes),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(e.to_string()),
        }
    } else {
        None
    };
    if ignore_file {
        add_ignore_rule(&path, file, false)?;
    }
    if let Err(error) = git(&path, &["rm", "--cached", "--", file]) {
        if ignore_file {
            let rollback = match before {
                Some(bytes) => std::fs::write(&ignore_path, bytes),
                None => std::fs::remove_file(&ignore_path),
            };
            if let Err(rollback) = rollback {
                return Err(format!("{error}\n恢复 .gitignore 失败：{rollback}"));
            }
        }
        return Err(error);
    }
    Ok(format!(
        "已撤销跟踪{}，本地文件保留：{file}",
        if ignore_file { "并忽略" } else { "" }
    ))
}
