//! Integration tests use disposable local repositories; never the user's working copies.
use super::*;
use integration::Method;
struct Repo(PathBuf);
impl Repo {
    fn new() -> Self {
        let path =
            std::env::temp_dir().join(format!("devstart-workflow-{}", crate::model::unique_id()));
        std::fs::create_dir_all(&path).unwrap();
        git(&path, &["init", "--initial-branch=main"]).unwrap();
        for (key, value) in [
            ("user.name", "Test"),
            ("user.email", "test@example.invalid"),
            ("commit.gpgsign", "false"),
            ("receive.denyCurrentBranch", "ignore"),
            ("core.autocrlf", "false"),
            ("core.hooksPath", "/dev/null"),
        ] {
            git(&path, &["config", key, value]).unwrap();
        }
        Self(path)
    }
    fn write(&self, name: &str, text: &str) {
        std::fs::write(self.0.join(name), text).unwrap();
    }
    fn commit(&self, name: &str, text: &str) {
        self.write(name, text);
        git(&self.0, &["add", "--", name]).unwrap();
        git(&self.0, &["commit", "-m", name]).unwrap();
    }
    fn head(&self) -> String {
        git(&self.0, &["rev-parse", "HEAD"]).unwrap()
    }
    fn attach(&self, remote: &Repo) -> String {
        git(
            &self.0,
            &["remote", "add", "origin", remote.0.to_str().unwrap()],
        )
        .unwrap();
        git(&self.0, &["pull", "origin", "main"]).unwrap();
        remotes::list(&self.0).unwrap()[0].key.clone()
    }
    fn text(&self, name: &str) -> String {
        std::fs::read_to_string(self.0.join(name)).unwrap()
    }
}
impl Drop for Repo {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
#[test]
fn commit_is_offline_and_push_never_commits_working_files() {
    let remote = Repo::new();
    remote.commit("base", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.write("new", "local\n");
    local.write("unselected", "keep\n");
    actions::commit(&local.0, "new local file", &["new".into()]).unwrap();
    local.write("new", "pending\n");
    let before = local.head();
    remotes::push(&local.0, &key, "main").unwrap();
    assert_eq!(local.head(), before);
    assert_eq!(remote.head(), before);
    assert_eq!(local.text("new"), "pending\n");
    assert_eq!(local.text("unselected"), "keep\n");
    git(
        &local.0,
        &[
            "remote",
            "set-url",
            "origin",
            "/nonexistent-devstart-remote",
        ],
    )
    .unwrap();
    actions::commit(&local.0, "offline commit", &["new".into()]).unwrap();
    assert_ne!(local.head(), before);
}
#[test]
fn push_rejection_preserves_local_commit_and_requires_explicit_update() {
    let remote = Repo::new();
    remote.commit("base", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    remote.commit("remote", "remote\n");
    local.commit("local", "local\n");
    let before = local.head();
    let error = remotes::push(&local.0, &key, "main").unwrap_err();
    assert!(remotes::push_rejected(&error));
    assert_eq!(before, local.head());
    assert!(!local.0.join("remote").exists());
    remotes::update_with_method(&local.0, &key, "main", Method::Merge).unwrap();
    remotes::push(&local.0, &key, "main").unwrap();
    assert_eq!(local.head(), remote.head());
}
#[test]
fn no_op_update_and_failed_fetch_never_stash_local_files() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.write("file", "local\n");
    local.write("new", "untracked\n");
    assert!(
        remotes::update_with_method(&local.0, &key, "main", Method::Merge)
            .unwrap()
            .contains("没有任何变化")
    );
    assert!(git(&local.0, &["stash", "list"]).unwrap().is_empty());
    git(
        &local.0,
        &[
            "remote",
            "set-url",
            "origin",
            "/nonexistent-devstart-remote",
        ],
    )
    .unwrap();
    let key = remotes::list(&local.0).unwrap()[0].key.clone();
    assert!(remotes::update(&local.0, &key, "main").is_err());
    assert!(git(&local.0, &["stash", "list"]).unwrap().is_empty());
    assert_eq!(local.text("file"), "local\n");
    assert_eq!(local.text("new"), "untracked\n");
}
#[test]
fn update_restores_staged_unstaged_and_untracked_files_and_keeps_existing_stash() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.write("old", "old backup\n");
    git(&local.0, &["stash", "push", "-u", "-m", "existing"]).unwrap();
    let old = git(&local.0, &["rev-parse", "refs/stash"]).unwrap();
    local.write("file", "staged\n");
    git(&local.0, &["add", "file"]).unwrap();
    local.write("file", "unstaged\n");
    local.write("new", "new\n");
    remote.commit("remote", "remote\n");
    remotes::update_with_method(&local.0, &key, "main", Method::Merge).unwrap();
    assert_eq!(local.text("file"), "unstaged\n");
    assert_eq!(
        super::git_output_raw(&local.0, &["show", ":file"])
            .unwrap()
            .0,
        "staged\n"
    );
    assert_eq!(local.text("new"), "new\n");
    assert_eq!(local.text("remote"), "remote\n");
    assert_eq!(git(&local.0, &["rev-parse", "refs/stash"]).unwrap(), old);
    assert!(!integration::state(&local.0).unwrap().busy());
}
#[test]
fn merge_conflict_is_reviewed_without_overwriting_and_rejects_stale_edits() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.commit("file", "local\n\n");
    remote.commit("file", "remote\n\n");
    local.write("new", "keep me\n");
    assert!(remotes::update_with_method(&local.0, &key, "main", Method::Merge).is_err());
    let state = integration::state(&local.0).unwrap();
    assert_eq!(state.operation.as_deref(), Some("merge"));
    assert!(state.recovering);
    assert_eq!(state.conflicts, vec!["file"]);
    assert!(remotes::update(&local.0, &key, "main").is_err());
    let stale = conflicts::inspect(&local.0, "file").unwrap();
    assert_eq!(stale.current.as_deref(), Some("local\n\n"));
    assert_eq!(stale.incoming.as_deref(), Some("remote\n\n"));
    local.write("file", "manual outside editor\n");
    assert!(
        conflicts::resolve(
            &local.0,
            "file",
            &stale.token,
            conflicts::Resolution::Incoming
        )
        .is_err()
    );
    let doc = conflicts::inspect(&local.0, "file").unwrap();
    conflicts::resolve(
        &local.0,
        "file",
        &doc.token,
        conflicts::Resolution::Edited("merged\n\n".into()),
    )
    .unwrap();
    integration::continue_operation(&local.0).unwrap();
    assert_eq!(local.text("file"), "merged\n\n");
    assert_eq!(local.text("new"), "keep me\n");
    assert!(!integration::state(&local.0).unwrap().busy());
    assert!(git(&local.0, &["stash", "list"]).unwrap().is_empty());
}
#[test]
fn abort_merge_restores_original_head_and_local_work() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.commit("file", "local\n");
    let before = local.head();
    remote.commit("file", "remote\n");
    local.write("new", "recover\n");
    assert!(remotes::update_with_method(&local.0, &key, "main", Method::Merge).is_err());
    integration::abort(&local.0).unwrap();
    assert_eq!(before, local.head());
    assert_eq!(local.text("file"), "local\n");
    assert_eq!(local.text("new"), "recover\n");
    assert!(!integration::state(&local.0).unwrap().busy());
}
#[test]
fn staged_restore_conflict_uses_three_way_review_and_explicit_finish() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.write("file", "saved local\n");
    git(&local.0, &["add", "file"]).unwrap();
    remote.commit("file", "remote\n");
    assert!(remotes::update_with_method(&local.0, &key, "main", Method::Merge).is_err());
    let state = integration::state(&local.0).unwrap();
    assert!(state.operation.is_none());
    assert!(state.recovering);
    assert!(!state.recovery_failed);
    assert_eq!(state.conflicts, vec!["file"]);
    let doc = conflicts::inspect(&local.0, "file").unwrap();
    assert_eq!(doc.current.as_deref(), Some("remote\n"));
    assert_eq!(doc.incoming.as_deref(), Some("saved local\n"));
    conflicts::resolve(
        &local.0,
        "file",
        &doc.token,
        conflicts::Resolution::Incoming,
    )
    .unwrap();
    integration::continue_operation(&local.0).unwrap();
    assert_eq!(local.text("file"), "saved local\n");
    assert!(!integration::state(&local.0).unwrap().busy());
}
#[test]
fn rebase_conflict_labels_replayed_commit_and_can_continue_or_abort() {
    for abort in [false, true] {
        let remote = Repo::new();
        remote.commit("file", "base\n");
        let local = Repo::new();
        let key = local.attach(&remote);
        local.commit("file", "local commit\n");
        let old = local.head();
        remote.commit("file", "remote\n");
        local.write("new", "unstaged\n");
        assert!(remotes::update_with_method(&local.0, &key, "main", Method::Rebase).is_err());
        assert_eq!(
            integration::state(&local.0).unwrap().operation.as_deref(),
            Some("rebase")
        );
        if abort {
            integration::abort(&local.0).unwrap();
            assert_eq!(local.head(), old);
        } else {
            let doc = conflicts::inspect(&local.0, "file").unwrap();
            assert_eq!(doc.current.as_deref(), Some("remote\n"));
            assert_eq!(doc.incoming.as_deref(), Some("local commit\n"));
            assert!(doc.incoming_label.contains("重放"));
            conflicts::resolve(
                &local.0,
                "file",
                &doc.token,
                conflicts::Resolution::Incoming,
            )
            .unwrap();
            integration::continue_operation(&local.0).unwrap();
            assert_ne!(local.head(), old);
            assert_eq!(
                git(&local.0, &["rev-parse", "HEAD^@"]).unwrap(),
                remote.head()
            );
        }
        assert_eq!(local.text("new"), "unstaged\n");
        assert!(!integration::state(&local.0).unwrap().busy());
    }
}
#[test]
fn branch_rebase_does_not_update_remote_and_changes_local_history() {
    let repo = Repo::new();
    repo.commit("base", "base\n");
    git(&repo.0, &["switch", "-c", "feature"]).unwrap();
    repo.commit("feature", "feature\n");
    let original = repo.head();
    git(&repo.0, &["switch", "main"]).unwrap();
    repo.commit("main", "main\n");
    let main = repo.head();
    git(&repo.0, &["switch", "feature"]).unwrap();
    integration::rebase(&repo.0, "main").unwrap();
    assert_ne!(repo.head(), original);
    assert_eq!(git(&repo.0, &["rev-parse", "HEAD^"]).unwrap(), main);
    assert_eq!(
        git(&repo.0, &["branch", "--show-current"]).unwrap(),
        "feature"
    );
}
#[test]
fn conflict_resolution_supports_deletion_and_binary_versions() {
    let repo = Repo::new();
    repo.commit("file", "base\n");
    git(&repo.0, &["branch", "other"]).unwrap();
    repo.commit("file", "local\n");
    git(&repo.0, &["switch", "other"]).unwrap();
    git(&repo.0, &["rm", "file"]).unwrap();
    git(&repo.0, &["commit", "-m", "delete"]).unwrap();
    git(&repo.0, &["switch", "main"]).unwrap();
    assert!(git(&repo.0, &["merge", "other"]).is_err());
    let doc = conflicts::inspect(&repo.0, "file").unwrap();
    conflicts::resolve(&repo.0, "file", &doc.token, conflicts::Resolution::Incoming).unwrap();
    assert!(!repo.0.join("file").exists());
    integration::continue_operation(&repo.0).unwrap();
    std::fs::write(repo.0.join("binary"), [0, 1, 2]).unwrap();
    git(&repo.0, &["add", "binary"]).unwrap();
    git(&repo.0, &["commit", "-m", "binary"]).unwrap();
    git(&repo.0, &["branch", "binary-other"]).unwrap();
    std::fs::write(repo.0.join("binary"), [0, 3, 4]).unwrap();
    git(&repo.0, &["commit", "-am", "left"]).unwrap();
    git(&repo.0, &["switch", "binary-other"]).unwrap();
    std::fs::write(repo.0.join("binary"), [0, 5, 6]).unwrap();
    git(&repo.0, &["commit", "-am", "right"]).unwrap();
    git(&repo.0, &["switch", "main"]).unwrap();
    assert!(git(&repo.0, &["merge", "binary-other"]).is_err());
    let doc = conflicts::inspect(&repo.0, "binary").unwrap();
    assert!(!doc.editable);
    conflicts::resolve(
        &repo.0,
        "binary",
        &doc.token,
        conflicts::Resolution::Incoming,
    )
    .unwrap();
    assert_eq!(std::fs::read(repo.0.join("binary")).unwrap(), vec![0, 5, 6]);
}
#[test]
fn new_file_collision_becomes_reviewable_add_add_conflict() {
    let remote = Repo::new();
    remote.commit("base", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.write("new", "local new file\n");
    remote.commit("new", "remote new file\n");
    assert!(remotes::update_with_method(&local.0, &key, "main", Method::Merge).is_err());
    let state = integration::state(&local.0).unwrap();
    assert_eq!(state.conflicts, vec!["new"]);
    assert!(!state.recovery_failed);
    let doc = conflicts::inspect(&local.0, "new").unwrap();
    assert_eq!(doc.current.as_deref(), Some("remote new file\n"));
    assert_eq!(doc.incoming.as_deref(), Some("local new file\n"));
    conflicts::resolve(
        &local.0,
        "new",
        &doc.token,
        conflicts::Resolution::Edited("both versions merged\n".into()),
    )
    .unwrap();
    integration::continue_operation(&local.0).unwrap();
    assert_eq!(local.text("new"), "both versions merged\n");
    assert!(!integration::state(&local.0).unwrap().busy());
}
#[test]
fn skip_replayed_commit_is_explicit_and_restores_saved_local_work() {
    let remote = Repo::new();
    remote.commit("file", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    local.commit("file", "local\n");
    remote.commit("file", "remote\n");
    local.write("new", "preserve\n");
    assert!(remotes::update_with_method(&local.0, &key, "main", Method::Rebase).is_err());
    integration::skip_rebase_commit(&local.0).unwrap();
    assert_eq!(local.head(), remote.head());
    assert_eq!(local.text("new"), "preserve\n");
    assert!(!integration::state(&local.0).unwrap().busy());
}
#[test]
fn commit_and_push_rejection_keeps_the_completed_local_commit() {
    let remote = Repo::new();
    remote.commit("base", "base\n");
    let local = Repo::new();
    let key = local.attach(&remote);
    remote.commit("remote", "incoming\n");
    local.write("new", "selected\n");
    let before = local.head();
    let error =
        remotes::submit(&local.0, &key, "main", "local only", &["new".into()], true).unwrap_err();
    assert!(remotes::push_rejected(&error));
    assert_ne!(local.head(), before);
    assert_eq!(local.text("new"), "selected\n");
    assert!(!local.0.join("remote").exists());
    assert!(actions::changes(&local.0).unwrap().is_empty());
}
#[test]
fn interrupted_restore_does_not_reapply_or_delete_recovery_backups() {
    let repo = Repo::new();
    repo.commit("base", "base\n");
    repo.write("new", "saved\n");
    git(&repo.0, &["stash", "push", "-u"]).unwrap();
    let stash = git(&repo.0, &["rev-parse", "refs/stash"]).unwrap();
    let record = serde_json::json!({"stash":stash,"phase":"restoring","branch":"main","restore_head":repo.head()});
    std::fs::write(
        repo.0.join(".git/devstart-update.json"),
        serde_json::to_vec(&record).unwrap(),
    )
    .unwrap();
    let state = integration::state(&repo.0).unwrap();
    assert!(state.recovery_failed);
    assert!(!state.can_continue);
    assert!(integration::continue_operation(&repo.0).is_err());
    assert!(!repo.0.join("new").exists());
    assert_eq!(git(&repo.0, &["rev-parse", "refs/stash"]).unwrap(), stash);
    integration::finish_manual_recovery(&repo.0).unwrap();
    assert!(!integration::state(&repo.0).unwrap().busy());
    assert_eq!(git(&repo.0, &["rev-parse", "refs/stash"]).unwrap(), stash);
}

#[test]
fn ignore_context_action_is_exact_and_preserves_source_and_existing_rules() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "tracked");
    repo.write(".gitignore", "# existing\r\nold/");
    repo.write("[a]* space.md", "keep");
    repo.write("aX space.md", "other");
    actions::ignore(&repo.0, "[a]* space.md").unwrap();
    assert!(repo.0.join("[a]* space.md").exists());
    assert!(
        repo.text(".gitignore")
            .starts_with("# existing\r\nold/\r\n")
    );
    let files = actions::changes(&repo.0).unwrap();
    assert!(!files.iter().any(|f| f.path == "[a]* space.md"));
    assert!(files.iter().any(|f| f.path == "aX space.md"));
    repo.write("tracked.txt", "modified");
    assert!(actions::ignore(&repo.0, "tracked.txt").is_err());
    assert_eq!(repo.text("tracked.txt"), "modified");
    std::fs::create_dir(repo.0.join("cache")).unwrap();
    repo.write("cache/item", "keep");
    actions::ignore(&repo.0, "cache/").unwrap();
    assert!(repo.0.join("cache/item").exists());
}

#[test]
fn untrack_preserves_worktree_and_respects_staged_content() {
    let repo = Repo::new();
    repo.commit("tracked.txt", "original");
    repo.write("tracked.txt", "local edit");
    actions::untrack(&repo.0, "tracked.txt", false).unwrap();
    assert_eq!(repo.text("tracked.txt"), "local edit");
    assert!(
        git(&repo.0, &["ls-files", "--", "tracked.txt"])
            .unwrap()
            .is_empty()
    );
    assert!(!repo.0.join(".gitignore").exists());
    git(&repo.0, &["add", "--", "tracked.txt"]).unwrap();
    actions::untrack(&repo.0, "tracked.txt", true).unwrap();
    assert_eq!(repo.text("tracked.txt"), "local edit");
    assert!(repo.text(".gitignore").contains("/tracked.txt"));
    assert!(
        !actions::changes(&repo.0)
            .unwrap()
            .iter()
            .any(|c| c.path == "tracked.txt" && c.status == "??")
    );
    repo.commit("other.txt", "base");
    repo.write("other.txt", "staged only");
    git(&repo.0, &["add", "--", "other.txt"]).unwrap();
    repo.write("other.txt", "working only");
    let before = repo.text(".gitignore");
    assert!(actions::untrack(&repo.0, "other.txt", true).is_err());
    assert_eq!(
        git(&repo.0, &["show", ":other.txt"]).unwrap(),
        "staged only"
    );
    assert_eq!(repo.text("other.txt"), "working only");
    assert_eq!(repo.text(".gitignore"), before);
}

#[test]
fn selected_commit_preserves_untrack_ignore_and_unselected_index_for_both_actions() {
    for push in [false, true] {
        let remote = Repo::new();
        remote.commit("base.txt", "base\n");
        let local = Repo::new();
        let target = local.attach(&remote);
        local.commit("verification.md", "original\n");
        local.commit("unselected.txt", "base\n");
        local.write("verification.md", "keep local verification\n");
        actions::untrack(&local.0, "verification.md", true).unwrap();
        local.write("unselected.txt", "unselected staged change\n");
        git(&local.0, &["add", "--", "unselected.txt"]).unwrap();
        local.write("new.txt", "selected new file\n");
        let files = vec![
            "verification.md".into(),
            ".gitignore".into(),
            "new.txt".into(),
        ];
        if push {
            remotes::submit(
                &local.0,
                &target,
                "main",
                "untrack and ignore selected files",
                &files,
                true,
            )
            .unwrap();
        } else {
            actions::commit(&local.0, "untrack and ignore selected files", &files).unwrap();
        }
        assert!(git(&local.0, &["cat-file", "-e", "HEAD:verification.md"]).is_err());
        assert_eq!(local.text("verification.md"), "keep local verification\n");
        assert!(
            git(&local.0, &["ls-files", "--", "verification.md"])
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            git(&local.0, &["show", "HEAD:unselected.txt"]).unwrap(),
            "base"
        );
        assert_eq!(
            git(&local.0, &["show", ":unselected.txt"]).unwrap(),
            "unselected staged change"
        );
        assert_eq!(
            git(&local.0, &["show", "HEAD:new.txt"]).unwrap(),
            "selected new file"
        );
        assert!(
            git(&local.0, &["show", "HEAD:.gitignore"])
                .unwrap()
                .contains("/verification.md")
        );
        if push {
            assert_eq!(local.head(), remote.head());
        }
    }
}

#[test]
fn commit_of_index_only_removal_keeps_unignored_local_copy() {
    let local = Repo::new();
    local.commit("local.txt", "original\n");
    local.write("local.txt", "keep this version\n");
    actions::untrack(&local.0, "local.txt", false).unwrap();
    actions::commit(&local.0, "stop tracking local file", &["local.txt".into()]).unwrap();
    assert_eq!(local.text("local.txt"), "keep this version\n");
    assert!(git(&local.0, &["cat-file", "-e", "HEAD:local.txt"]).is_err());
    assert!(
        git(&local.0, &["ls-files", "--", "local.txt"])
            .unwrap()
            .is_empty()
    );
}

#[test]
fn isolated_selected_commit_supports_first_commit_and_untracked_directories() {
    let local = Repo::new();
    std::fs::create_dir(local.0.join("new-dir")).unwrap();
    local.write("new-dir/one.txt", "one\n");
    local.write("new-dir/two.txt", "two\n");
    local.write("not-selected.txt", "keep untracked\n");
    actions::commit(&local.0, "first selected directory", &["new-dir/".into()]).unwrap();
    assert_eq!(
        git(&local.0, &["show", "HEAD:new-dir/one.txt"]).unwrap(),
        "one"
    );
    assert_eq!(
        git(&local.0, &["show", "HEAD:new-dir/two.txt"]).unwrap(),
        "two"
    );
    assert!(git(&local.0, &["cat-file", "-e", "HEAD:not-selected.txt"]).is_err());
}
