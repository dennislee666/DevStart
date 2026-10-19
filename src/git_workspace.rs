//! Git operations shared by UI workers and API. Never infer repository state in the UI.
use serde::Serialize;
use std::os::unix::process::CommandExt;
#[path = "git_actions.rs"]
pub mod actions;
#[path = "git_commit.rs"]
pub mod commits;
#[path = "git_remotes.rs"]
pub mod remotes;
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize)]
pub struct Repository {
    pub project_id: u64,
    pub name: String,
    pub path: PathBuf,
    pub root: Option<PathBuf>,
    pub branch: String,
    pub version: String,
    pub dirty: bool,
    pub latest: String,
    pub author: String,
    pub date: String,
    pub error: Option<String>,
}
#[derive(Clone, Serialize)]
pub struct Commit {
    pub id: String,
    pub author: String,
    pub date: String,
    pub subject: String,
}
impl Commit {
    pub fn display_date(&self) -> String {
        crate::time_display::git_date(&self.date)
    }
}
/// Bound both duration and captured bytes; prompts and optional read locks are disabled.
fn git(path: &Path, args: &[&str]) -> Result<String, String> {
    Ok(git_output(path, args)?.0)
}
fn git_output(path: &Path, args: &[&str]) -> Result<(String, String), String> {
    let mut child = Command::new("git")
        .args(["--literal-pathspecs", "-c", "core.fsmonitor=false", "-C"])
        .arg(path)
        .args(args)
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_EDITOR", "false")
        .env("GIT_SEQUENCE_EDITOR", "false")
        .process_group(0)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Git: {e}"))?;
    let (tx, rx) = std::sync::mpsc::channel();
    for (index, pipe) in [
        (
            0,
            child
                .stdout
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
        (
            1,
            child
                .stderr
                .take()
                .map(|p| Box::new(p) as Box<dyn Read + Send>),
        ),
    ] {
        let tx = tx.clone();
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe.unwrap().take(2_097_153).read_to_end(&mut bytes);
            let _ = tx.send((index, result.map(|_| bytes)));
        });
    }
    drop(tx);
    let deadline = Instant::now() + Duration::from_secs(120);
    let mut wait_ms = 1;
    let status = loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        if Instant::now() >= deadline {
            // Git may own SSH, hooks, or filter children; stop its dedicated group.
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err("Git command timed out (120s)".into());
        }
        // Fast local commands should not pay a fixed 20 ms minimum on every spawn.
        std::thread::sleep(Duration::from_millis(wait_ms));
        wait_ms = (wait_ms * 2).min(20);
    };
    let mut output = [String::new(), String::new()];
    for _ in 0..2 {
        let (index, bytes) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .map_err(|e| format!("Git output: {e}"))?;
        let bytes = bytes.map_err(|e| e.to_string())?;
        if bytes.len() > 2_097_152 {
            return Err("Git output exceeds 2 MiB".into());
        }
        output[index] = String::from_utf8(bytes).map_err(|_| {
            "Git returned non-UTF-8 output; no lossy file paths are accepted".to_owned()
        })?;
    }
    if !status.success() {
        let error = format!(
            "git {}: {}{}",
            args.first().copied().unwrap_or(""),
            output[1],
            output[0]
        );
        eprintln!("[dev start git] {error}");
        return Err(error.trim().to_owned());
    }
    Ok((
        output[0].trim_end_matches('\n').to_owned(),
        output[1].clone(),
    ))
}
/// One write operation per actual repository, shared by GUI workers and API jobs.
struct MutationGuard(PathBuf);
fn active_writes() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static ACTIVE: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    ACTIVE.get_or_init(|| std::sync::Mutex::new(Default::default()))
}
impl Drop for MutationGuard {
    fn drop(&mut self) {
        if let Ok(mut active) = active_writes().lock() {
            active.remove(&self.0);
        }
    }
}
fn mutation_guard(path: &Path) -> Result<MutationGuard, String> {
    let root = root(path)?;
    if !active_writes()
        .lock()
        .map_err(|e| e.to_string())?
        .insert(root.clone())
    {
        return Err("Another Git write operation is running in this repository".into());
    }
    Ok(MutationGuard(root))
}
fn root(path: &Path) -> Result<PathBuf, String> {
    git(path, &["rev-parse", "--show-toplevel"]).map(PathBuf::from)
}
pub fn inspect(id: u64, name: String, path: PathBuf) -> Repository {
    let mut r = Repository {
        project_id: id,
        name,
        path,
        root: None,
        branch: String::new(),
        version: String::new(),
        dirty: false,
        latest: String::new(),
        author: String::new(),
        date: String::new(),
        error: None,
    };
    let result = (|| -> Result<(), String> {
        r.root = Some(root(&r.path)?);
        r.branch = git(&r.path, &["branch", "--show-current"])?;
        if r.branch.is_empty() {
            r.branch = format!(
                "detached {}",
                git(&r.path, &["rev-parse", "--short", "HEAD"])?
            );
        }
        r.dirty = !git(
            &r.path,
            &["status", "--porcelain=v1", "--untracked-files=normal"],
        )?
        .is_empty();
        // An unborn branch has no commit; it is a valid repository, not a command failure.
        if let Some(commit) = history(&r.path, 0, 1)?.first() {
            r.latest = commit.subject.clone();
            r.author = commit.author.clone();
            r.date = commit.date.clone();
            r.version = commit.id.chars().take(10).collect();
        }
        Ok(())
    })();
    if let Err(error) = result {
        r.error = Some(error);
    }
    r
}
/// One bounded snapshot for the project workspace. Independent reads overlap;
/// status/history also supply the summary, avoiding duplicate repository scans.
#[derive(Clone)]
pub struct Snapshot {
    pub repository: Repository,
    pub remotes: Vec<remotes::Remote>,
    pub branches: Vec<String>,
    pub files: Vec<actions::Change>,
    pub history: Vec<Commit>,
    pub offset: usize,
}
impl Snapshot {
    /// Group porcelain changes centrally; UI only renders the ordered, non-empty groups.
    pub fn change_groups(&self) -> Vec<(&'static str, Vec<&actions::Change>)> {
        let mut groups: [Vec<&actions::Change>; 4] = Default::default();
        for file in &self.files {
            let index = match file.status_label() {
                "未跟踪" => 0,
                "已新增" => 2,
                "已删除" => 3,
                _ => 1,
            };
            groups[index].push(file);
        }
        ["未跟踪", "有变化", "新文件", "删除"]
            .into_iter()
            .zip(groups)
            .filter(|(_, files)| !files.is_empty())
            .collect()
    }
}
pub fn snapshot(project: &crate::model::Project, offset: usize) -> Result<Box<Snapshot>, String> {
    let started = Instant::now();
    let result = std::thread::scope(|scope| {
        let path = &project.path;
        let metadata = scope.spawn(|| -> Result<_, String> {
            let root = root(path)?;
            let mut branch = git(path, &["branch", "--show-current"])?;
            if branch.is_empty() {
                branch = format!("detached {}", git(path, &["rev-parse", "--short", "HEAD"])?);
            }
            Ok((root, branch))
        });
        let branches = scope.spawn(|| branches(path));
        let remotes = scope.spawn(|| remotes::list(path));
        let files = scope.spawn(|| actions::changes(path));
        // Keep HEAD metadata correct while browsing later history pages.
        let history = scope.spawn(|| history(path, offset, 50));
        let latest = scope.spawn(|| {
            if offset == 0 {
                Ok(vec![])
            } else {
                self::history(path, 0, 1)
            }
        });
        let join_error = |_| "Git snapshot worker interrupted".to_owned();
        let (root, branch) = metadata.join().map_err(join_error)??;
        let branches = branches.join().map_err(join_error)??;
        let remotes = remotes.join().map_err(join_error)??;
        let files = files.join().map_err(join_error)??;
        let history = history.join().map_err(join_error)??;
        let latest = latest.join().map_err(join_error)??;
        let head = if offset == 0 {
            history.first()
        } else {
            latest.first()
        };
        let repository = Repository {
            project_id: project.id,
            name: project.name.clone(),
            path: path.clone(),
            root: Some(root),
            branch,
            version: head
                .map(|c| c.id.chars().take(10).collect())
                .unwrap_or_default(),
            dirty: !files.is_empty(),
            latest: head.map(|c| c.subject.clone()).unwrap_or_default(),
            author: head.map(|c| c.author.clone()).unwrap_or_default(),
            date: head.map(|c| c.date.clone()).unwrap_or_default(),
            error: None,
        };
        Ok(Box::new(Snapshot {
            repository,
            branches,
            remotes,
            files,
            history,
            offset,
        }))
    });
    eprintln!(
        "[dev start git] snapshot project={} offset={} elapsed_ms={} success={}",
        project.id,
        offset,
        started.elapsed().as_millis(),
        result.is_ok()
    );
    result
}
pub fn list(projects: &[crate::model::Project]) -> Vec<Repository> {
    projects
        .iter()
        .map(|p| inspect(p.id, p.name.clone(), p.path.clone()))
        .collect()
}
pub fn branches(path: &Path) -> Result<Vec<String>, String> {
    root(path)?;
    Ok(git(
        path,
        &["for-each-ref", "--format=%(refname:short)", "refs/heads/"],
    )?
    .lines()
    .map(str::to_owned)
    .collect())
}
pub fn history(path: &Path, offset: usize, limit: usize) -> Result<Vec<Commit>, String> {
    if limit == 0 || limit > 100 || offset > 100_000 {
        return Err("history requires limit 1..100 and offset <= 100000".into());
    }
    root(path)?;
    // symbolic HEAD can be unborn even when other branches have commits.
    if git(path, &["rev-list", "--all", "--max-count=1"])?.is_empty() {
        return Ok(vec![]);
    }
    let branch = git(path, &["branch", "--show-current"])?;
    if !branch.is_empty() && !branches(path)?.contains(&branch) {
        return Ok(vec![]);
    }
    let text = git(
        path,
        &[
            "log",
            "--no-show-signature",
            "--format=%H%x00%an%x00%aI%x00%s",
            &format!("--skip={offset}"),
            &format!("--max-count={limit}"),
        ],
    )?;
    text.lines()
        .map(|line| {
            let f: Vec<_> = line.splitn(4, '\0').collect();
            if f.len() != 4 {
                return Err("Invalid Git log record".into());
            }
            Ok(Commit {
                id: f[0].into(),
                author: f[1].into(),
                date: f[2].into(),
                subject: f[3].into(),
            })
        })
        .collect()
}
/// Switch the actual working tree; Git protects local changes. Never force, stash, or fetch.
pub fn switch(path: &Path, branch: &str) -> Result<(), String> {
    let _guard = mutation_guard(path)?;
    if !branches(path)?.iter().any(|b| b == branch) {
        return Err("Unknown local branch".into());
    }
    // Git preserves non-conflicting local changes and reports the exact files that block checkout.
    git(path, &["switch", "--no-guess", "--", branch])?;
    let actual = git(path, &["branch", "--show-current"])?;
    if actual != branch {
        return Err(format!(
            "Branch verification failed: expected {branch}, actual {actual}"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn workspace_snapshot_matches_reads_and_keeps_head_on_later_pages() {
        let repo = Repo::new();
        let project = crate::model::Project {
            id: 1,
            name: "Demo".into(),
            path: repo.0.clone(),
            group: 1,
            favorite: false,
            scripts: vec![],
            explicit_scripts: false,
            description: String::new(),
        };
        assert!(snapshot(&project, 0).unwrap().history.is_empty());
        repo.commit("first");
        repo.commit("second");
        std::fs::write(repo.0.join("new.txt"), "untracked").unwrap();
        let start = Instant::now();
        let old = inspect(1, "Demo".into(), repo.0.clone());
        let old_branches = branches(&repo.0).unwrap();
        let old_remotes = remotes::list(&repo.0).unwrap();
        let old_files = actions::changes(&repo.0).unwrap();
        let old_history = history(&repo.0, 0, 50).unwrap();
        let sequential_ms = start.elapsed().as_millis();
        let start = Instant::now();
        let fresh = snapshot(&project, 0).unwrap();
        eprintln!(
            "snapshot benchmark sequential_ms={sequential_ms} parallel_ms={}",
            start.elapsed().as_millis()
        );
        assert_eq!(fresh.repository.version, old.version);
        assert_eq!(fresh.repository.dirty, old.dirty);
        assert_eq!(fresh.branches, old_branches);
        assert_eq!(fresh.remotes.len(), old_remotes.len());
        assert_eq!(fresh.files.len(), old_files.len());
        assert_eq!(fresh.history[0].id, old_history[0].id);
        let page = snapshot(&project, 1).unwrap();
        assert_eq!(page.repository.version, fresh.repository.version);
        assert_eq!(page.history[0].id, fresh.history[1].id);
        assert_eq!(git(&repo.0, &["branch", "--show-current"]).unwrap(), "main");
    }
    struct Repo(PathBuf);
    impl Repo {
        fn new() -> Self {
            let p = std::env::temp_dir().join(format!(
                "devstart-git-{}-{}",
                std::process::id(),
                crate::model::unique_id()
            ));
            std::fs::create_dir_all(&p).unwrap();
            git(&p, &["init", "--initial-branch=main"]).unwrap();
            Self(p)
        }
        fn commit(&self, text: &str) {
            std::fs::write(self.0.join("file.txt"), text).unwrap();
            git(&self.0, &["add", "--", "file.txt"]).unwrap();
            git(
                &self.0,
                &[
                    "-c",
                    "user.name=Test",
                    "-c",
                    "user.email=test@example.invalid",
                    "-c",
                    "commit.gpgsign=false",
                    "commit",
                    "-m",
                    text,
                ],
            )
            .unwrap();
        }
    }
    impl Drop for Repo {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn unborn_repository_and_non_repository_have_distinct_states() {
        let repo = Repo::new();
        let r = inspect(1, "Example".into(), repo.0.clone());
        assert!(r.error.is_none());
        assert_eq!(r.branch, "main");
        assert!(r.version.is_empty());
        assert!(history(&repo.0, 0, 50).unwrap().is_empty());
        let child = repo.0.join("outside");
        std::fs::create_dir(&child).unwrap();
        let missing = inspect(2, "Missing".into(), child.join("missing"));
        assert!(missing.error.is_some());
    }
    #[test]
    fn branch_switch_preserves_unrelated_files_and_history_paginates() {
        let repo = Repo::new();
        repo.commit("first commit");
        repo.commit("second commit");
        git(&repo.0, &["branch", "feature/demo"]).unwrap();
        assert_eq!(history(&repo.0, 1, 1).unwrap()[0].subject, "first commit");
        std::fs::write(repo.0.join("untracked.txt"), "keep").unwrap();
        switch(&repo.0, "feature/demo").unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.0.join("untracked.txt")).unwrap(),
            "keep"
        );
        switch(&repo.0, "feature/demo").unwrap();
        assert_eq!(
            inspect(1, "Example".into(), repo.0.clone()).branch,
            "feature/demo"
        );
        assert!(switch(&repo.0, "--discard-changes").is_err());
        assert!(history(&repo.0, 0, 101).is_err());
    }
    #[test]
    fn submit_stops_on_autostash_conflicts_before_commit_or_push() {
        let remote = Repo::new();
        remote.commit("base");
        let local = Repo::new();
        git(
            &local.0,
            &["remote", "add", "origin", remote.0.to_str().unwrap()],
        )
        .unwrap();
        git(&local.0, &["pull", "origin", "main"]).unwrap();
        remote.commit("remote edit");
        std::fs::write(local.0.join("file.txt"), "local edit").unwrap();
        let key = remotes::list(&local.0).unwrap()[0].key.clone();
        let error = remotes::submit(
            &local.0,
            &key,
            "main",
            "must not commit",
            &["file.txt".into()],
            true,
        )
        .unwrap_err();
        assert!(error.contains("conflicts"));
        assert!(!error.contains("Commit: OK"));
        assert!(
            !git(&local.0, &["diff", "--name-only", "--diff-filter=U"])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            git(&local.0, &["rev-parse", "HEAD"]).unwrap(),
            git(&remote.0, &["rev-parse", "HEAD"]).unwrap()
        );
    }
    #[test]
    fn submit_updates_then_commits_and_pushes_without_losing_unselected_edits() {
        let remote = Repo::new();
        remote.commit("base");
        git(
            &remote.0,
            &["config", "receive.denyCurrentBranch", "ignore"],
        )
        .unwrap();
        let local = Repo::new();
        git(
            &local.0,
            &["remote", "add", "origin", remote.0.to_str().unwrap()],
        )
        .unwrap();
        git(&local.0, &["pull", "origin", "main"]).unwrap();
        git(&local.0, &["config", "user.name", "Test"]).unwrap();
        git(&local.0, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&local.0, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(remote.0.join("remote.txt"), "remote update").unwrap();
        git(&remote.0, &["add", "remote.txt"]).unwrap();
        git(
            &remote.0,
            &[
                "-c",
                "user.name=Test",
                "-c",
                "user.email=test@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "remote update",
            ],
        )
        .unwrap();
        std::fs::write(local.0.join("file.txt"), "local edit").unwrap();
        std::fs::write(local.0.join("keep.txt"), "unselected").unwrap();
        let key = remotes::list(&local.0).unwrap()[0].key.clone();
        let output = remotes::submit(
            &local.0,
            &key,
            "main",
            "local message",
            &["file.txt".into()],
            true,
        )
        .unwrap();
        assert!(output.contains("Push: OK"));
        assert_eq!(
            std::fs::read_to_string(local.0.join("remote.txt")).unwrap(),
            "remote update"
        );
        assert_eq!(
            git(&remote.0, &["rev-parse", "HEAD"]).unwrap(),
            git(&local.0, &["rev-parse", "HEAD"]).unwrap()
        );
        assert!(
            actions::changes(&local.0)
                .unwrap()
                .iter()
                .any(|f| f.path == "keep.txt")
        );
        std::fs::write(local.0.join("file.txt"), "still here").unwrap();
        let before = git(&local.0, &["rev-parse", "HEAD"]).unwrap();
        git(
            &local.0,
            &["remote", "set-url", "origin", "/does/not/exist"],
        )
        .unwrap();
        let key = remotes::list(&local.0).unwrap()[0].key.clone();
        assert!(
            remotes::submit(
                &local.0,
                &key,
                "main",
                "must not commit",
                &["file.txt".into()],
                true
            )
            .is_err()
        );
        assert_eq!(git(&local.0, &["rev-parse", "HEAD"]).unwrap(), before);
        assert_eq!(
            std::fs::read_to_string(local.0.join("file.txt")).unwrap(),
            "still here"
        );
    }
    #[test]
    fn working_diff_previews_text_and_rejects_binary_files() {
        let repo = Repo::new();
        repo.commit("base");
        std::fs::write(repo.0.join("new.txt"), "hello\nworld\n").unwrap();
        let patch = actions::diff(&repo.0, "new.txt").unwrap();
        assert!(patch.contains("@@ -0,0 +1,2 @@\n+hello\n+world"));
        std::fs::write(repo.0.join("image.bin"), b"abc\0def").unwrap();
        assert!(
            actions::diff(&repo.0, "image.bin")
                .unwrap_err()
                .contains("不支持")
        );
        git(&repo.0, &["add", "--", "image.bin"]).unwrap();
        assert!(
            actions::diff(&repo.0, "image.bin")
                .unwrap_err()
                .contains("不支持")
        );
        std::fs::write(repo.0.join("file.txt"), "updated\n").unwrap();
        assert!(
            actions::diff(&repo.0, "file.txt")
                .unwrap()
                .contains("+updated")
        );
    }
    #[test]
    fn branch_switch_reports_conflicts_without_discarding_changes() {
        let repo = Repo::new();
        repo.commit("base");
        git(&repo.0, &["branch", "other"]).unwrap();
        repo.commit("main change");
        std::fs::write(repo.0.join("file.txt"), "local work").unwrap();
        let error = switch(&repo.0, "other").unwrap_err();
        assert!(error.contains("file.txt"));
        assert_eq!(git(&repo.0, &["branch", "--show-current"]).unwrap(), "main");
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "local work"
        );
    }
    #[test]
    fn nested_project_uses_repo_root_and_detached_head_keeps_history() {
        let repo = Repo::new();
        repo.commit("initial");
        let child = repo.0.join("nested");
        std::fs::create_dir(&child).unwrap();
        git(&repo.0, &["switch", "--detach"]).unwrap();
        let r = inspect(3, "Nested".into(), child);
        assert!(r.error.is_none());
        assert!(r.branch.starts_with("detached "));
        assert_eq!(
            r.root.unwrap().canonicalize().unwrap(),
            repo.0.canonicalize().unwrap()
        );
        assert_eq!(r.version.len(), 10);
    }
    #[test]
    fn selected_commit_excludes_other_staged_files_and_restore_saves_backup() {
        let repo = Repo::new();
        repo.commit("initial");
        git(&repo.0, &["config", "user.name", "Test"]).unwrap();
        git(&repo.0, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&repo.0, &["config", "commit.gpgsign", "false"]).unwrap();
        std::fs::write(repo.0.join("other.txt"), "staged separately").unwrap();
        git(&repo.0, &["add", "--", "other.txt"]).unwrap();
        std::fs::write(repo.0.join("file.txt"), "selected change").unwrap();
        actions::commit(&repo.0, "selected commit", &["file.txt".into()]).unwrap();
        assert_eq!(
            git(&repo.0, &["show", "HEAD:file.txt"]).unwrap(),
            "selected change"
        );
        assert!(
            git(&repo.0, &["ls-tree", "--name-only", "HEAD"])
                .unwrap()
                .lines()
                .all(|p| p != "other.txt")
        );
        assert!(
            actions::changes(&repo.0)
                .unwrap()
                .iter()
                .any(|c| c.path == "other.txt")
        );
        std::fs::write(repo.0.join("file.txt"), "discard me").unwrap();
        assert!(
            actions::diff(&repo.0, "file.txt")
                .unwrap()
                .contains("+discard me")
        );
        let storage = Repo::new();
        let backups = storage.0.join("backups");
        actions::restore(&repo.0, "file.txt", &backups).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "selected change"
        );
        assert_eq!(std::fs::read_dir(&backups).unwrap().count(), 1);
        assert!(actions::restore(&repo.0, "../outside", &backups).is_err());
    }
    #[test]
    fn git_console_quotes_and_renamed_file_status_are_lossless() {
        let repo = Repo::new();
        repo.commit("initial");
        git(&repo.0, &["mv", "file.txt", "renamed file.txt"]).unwrap();
        let files = actions::changes(&repo.0).unwrap();
        assert_eq!(files[0].path, "renamed file.txt");
        assert_eq!(files[0].previous.as_deref(), Some("file.txt"));
        assert!(
            actions::command(&repo.0, "git status --short")
                .unwrap()
                .contains("renamed")
        );
        assert!(actions::command(&repo.0, "echo hello").is_err());
        assert!(actions::command(&repo.0, "git log '").is_err());
        let storage = Repo::new();
        actions::restore(&repo.0, "renamed file.txt", &storage.0.join("backups")).unwrap();
        assert!(repo.0.join("file.txt").exists());
        assert!(!repo.0.join("renamed file.txt").exists());
    }
    #[test]
    fn local_remote_push_and_pull_merge_are_real_git_operations() {
        let repo = Repo::new();
        repo.commit("initial");
        let storage = Repo::new();
        let remote = storage.0.join("remote.git");
        let peer = storage.0.join("peer");
        git(&storage.0, &["init", "--bare", remote.to_str().unwrap()]).unwrap();
        git(
            &repo.0,
            &["remote", "add", "origin", remote.to_str().unwrap()],
        )
        .unwrap();
        git(&repo.0, &["push", "-u", "origin", "main"]).unwrap();
        git(
            &storage.0,
            &[
                "clone",
                "--branch",
                "main",
                remote.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        )
        .unwrap();
        std::fs::write(peer.join("file.txt"), "remote update").unwrap();
        git(&peer, &["add", "."]).unwrap();
        git(
            &peer,
            &[
                "-c",
                "user.name=Peer",
                "-c",
                "user.email=peer@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "peer update",
            ],
        )
        .unwrap();
        actions::push(&peer).unwrap();
        actions::pull(&repo.0).unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "remote update"
        );
        std::fs::write(repo.0.join("file.txt"), "uncommitted").unwrap();
        assert!(actions::pull(&repo.0).is_err());
    }
    #[test]
    fn update_uses_selected_address_and_branch_instead_of_implicit_upstream() {
        let repo = Repo::new();
        repo.commit("initial");
        let storage = Repo::new();
        let a = storage.0.join("a.git");
        let b = storage.0.join("b.git");
        let peer = storage.0.join("peer");
        for remote in [&a, &b] {
            git(&storage.0, &["init", "--bare", remote.to_str().unwrap()]).unwrap();
        }
        git(&repo.0, &["remote", "add", "origin", a.to_str().unwrap()]).unwrap();
        git(&repo.0, &["remote", "add", "mirror", b.to_str().unwrap()]).unwrap();
        git(&repo.0, &["push", "-u", "origin", "main"]).unwrap();
        git(&repo.0, &["push", "mirror", "main"]).unwrap();
        git(
            &storage.0,
            &[
                "clone",
                "--branch",
                "main",
                b.to_str().unwrap(),
                peer.to_str().unwrap(),
            ],
        )
        .unwrap();
        std::fs::write(peer.join("file.txt"), "selected remote update").unwrap();
        git(&peer, &["add", "."]).unwrap();
        git(
            &peer,
            &[
                "-c",
                "user.name=Peer",
                "-c",
                "user.email=peer@example.invalid",
                "-c",
                "commit.gpgsign=false",
                "commit",
                "-m",
                "selected remote commit",
            ],
        )
        .unwrap();
        actions::push(&peer).unwrap();
        git(
            &repo.0,
            &["config", "--add", "remote.mirror.url", a.to_str().unwrap()],
        )
        .unwrap();
        let targets = remotes::list(&repo.0).unwrap();
        assert_eq!(targets.len(), 3);
        assert!(
            targets
                .iter()
                .find(|r| r.preferred)
                .is_some_and(|r| r.name == "origin")
        );
        let selected = targets
            .iter()
            .find(|r| r.url == b.to_str().unwrap())
            .unwrap();
        remotes::update(&repo.0, &selected.key, "main").unwrap();
        assert_eq!(
            std::fs::read_to_string(repo.0.join("file.txt")).unwrap(),
            "selected remote update"
        );
        assert_eq!(
            git(&repo.0, &["config", "branch.main.remote"]).unwrap(),
            "origin"
        );
        assert!(remotes::update(&repo.0, &selected.key, "other-branch").is_err());
        assert!(remotes::update(&repo.0, "stale-key", "main").is_err());
    }
    #[test]
    fn commit_browser_root_diff_is_immutable_and_rejects_unknown_files() {
        let repo = Repo::new();
        repo.commit("initial");
        let id = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let inspection = commits::files(&repo.0, &id).unwrap();
        assert!(inspection.base.is_none());
        assert_eq!(inspection.files[0].status, "A");
        assert_eq!(
            (inspection.files[0].additions, inspection.files[0].deletions),
            (Some(1), Some(0))
        );
        std::fs::write(repo.0.join("file.txt"), "uncommitted change").unwrap();
        let before = git(&repo.0, &["status", "--porcelain"]).unwrap();
        let diff = commits::diff(&repo.0, &id, "file.txt").unwrap();
        assert!(diff.contains("+initial"));
        assert!(!diff.contains("uncommitted change"));
        assert_eq!(before, git(&repo.0, &["status", "--porcelain"]).unwrap());
        assert!(commits::diff(&repo.0, &id, "../outside").is_err());
        assert!(commits::files(&repo.0, "HEAD").is_err());
    }
    #[test]
    fn commit_browser_preserves_renames_deletions_and_binary_changes() {
        let repo = Repo::new();
        repo.commit("initial");
        git(&repo.0, &["config", "user.name", "Test"]).unwrap();
        git(&repo.0, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&repo.0, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&repo.0, &["mv", "file.txt", "renamed file.txt"]).unwrap();
        std::fs::write(repo.0.join("binary.bin"), [0, 255, 0]).unwrap();
        git(&repo.0, &["add", "."]).unwrap();
        git(&repo.0, &["commit", "-m", "rename and binary"]).unwrap();
        let id = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let inspection = commits::files(&repo.0, &id).unwrap();
        let renamed = inspection
            .files
            .iter()
            .find(|c| c.path == "renamed file.txt")
            .unwrap();
        assert!(renamed.status.starts_with('R'));
        assert_eq!((renamed.additions, renamed.deletions), (Some(0), Some(0)));
        let binary = inspection
            .files
            .iter()
            .find(|c| c.path == "binary.bin")
            .unwrap();
        assert_eq!((binary.additions, binary.deletions), (None, None));
        assert_eq!(renamed.previous.as_deref(), Some("file.txt"));
        assert!(
            commits::diff(&repo.0, &id, "renamed file.txt")
                .unwrap()
                .contains("rename from file.txt")
        );
        assert!(
            commits::diff(&repo.0, &id, "binary.bin")
                .unwrap()
                .contains("Binary files")
        );
        git(&repo.0, &["rm", "renamed file.txt"]).unwrap();
        git(&repo.0, &["commit", "-m", "delete"]).unwrap();
        let id = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let deleted = commits::files(&repo.0, &id).unwrap();
        assert_eq!(deleted.files[0].status, "D");
        assert_eq!(
            (deleted.files[0].additions, deleted.files[0].deletions),
            (Some(0), Some(1))
        );
        assert!(
            commits::diff(&repo.0, &id, "renamed file.txt")
                .unwrap()
                .contains("-initial")
        );
    }
    #[test]
    fn commit_browser_merge_uses_first_parent() {
        let repo = Repo::new();
        repo.commit("initial");
        git(&repo.0, &["config", "user.name", "Test"]).unwrap();
        git(&repo.0, &["config", "user.email", "test@example.invalid"]).unwrap();
        git(&repo.0, &["config", "commit.gpgsign", "false"]).unwrap();
        git(&repo.0, &["switch", "-c", "feature"]).unwrap();
        std::fs::write(repo.0.join("feature.txt"), "feature").unwrap();
        git(&repo.0, &["add", "."]).unwrap();
        git(&repo.0, &["commit", "-m", "feature"]).unwrap();
        git(&repo.0, &["switch", "main"]).unwrap();
        std::fs::write(repo.0.join("main.txt"), "main").unwrap();
        git(&repo.0, &["add", "."]).unwrap();
        git(&repo.0, &["commit", "-m", "main change"]).unwrap();
        let base = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        git(&repo.0, &["merge", "--no-ff", "--no-edit", "feature"]).unwrap();
        let id = git(&repo.0, &["rev-parse", "HEAD"]).unwrap();
        let inspection = commits::files(&repo.0, &id).unwrap();
        assert_eq!(inspection.parents.len(), 2);
        assert_eq!(inspection.base.as_deref(), Some(base.as_str()));
        assert_eq!(inspection.files.len(), 1);
        assert_eq!(inspection.files[0].path, "feature.txt");
        assert!(
            commits::diff(&repo.0, &id, "feature.txt")
                .unwrap()
                .contains("+feature")
        );
        assert_eq!(git(&repo.0, &["branch", "--show-current"]).unwrap(), "main");
    }
}
