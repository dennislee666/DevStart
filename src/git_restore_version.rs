//! Restore a historical tree as a new local commit, keeping history pushable and remote refs untouched.
use super::{git, mutation_guard, root};
use std::{collections::HashSet, path::Path};

pub fn restore(
    path: &Path,
    commit: &str,
    branch: &str,
    expected_head: &str,
) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let target = super::commits::files(&root, commit)?.commit.id;
    let actual_branch = git(&root, &["branch", "--show-current"])?;
    let head = git(&root, &["rev-parse", "HEAD"])?;
    if branch.is_empty()
        || actual_branch != branch
        || expected_head.len() < 10
        || !head.starts_with(expected_head)
    {
        return Err("Branch or HEAD changed after selection; refresh before restoring".into());
    }
    if !super::actions::changes(&root)?.is_empty() {
        return Err(
            "Commit or stash local changes before restoring a version; no files were changed"
                .into(),
        );
    }
    for state in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        let state_path = git(&root, &["rev-parse", "--git-path", state])?;
        if root.join(state_path).exists() {
            return Err(format!("Git operation in progress: {state}"));
        }
    }
    let current = git(&root, &["ls-files", "-z"])?;
    let tracked: HashSet<_> = current.split('\0').collect();
    let tree = git(&root, &["ls-tree", "-r", "--name-only", "-z", &target])?;
    for file in tree.split('\0').filter(|p| !p.is_empty()) {
        // Ignored local files are outside status. Never overwrite an ignored collision silently.
        if !tracked.contains(file) && root.join(file).symlink_metadata().is_ok() {
            return Err(format!(
                "Local untracked or ignored path would be overwritten: {file}"
            ));
        }
    }
    let target_tree = git(&root, &["rev-parse", &format!("{target}^{{tree}}")])?;
    if git(&root, &["rev-parse", "HEAD^{tree}"])? == target_tree {
        return Ok(format!(
            "Already matches {target}; no new commit required. Remote unchanged."
        ));
    }
    let backup = format!(
        "refs/devstart/version-backups/{}",
        crate::model::unique_id()
    );
    git(&root, &["update-ref", &backup, &head])?;
    git(
        &root,
        &[
            "restore",
            &format!("--source={target}"),
            "--staged",
            "--worktree",
            "--",
            ".",
        ],
    )?;
    let message = format!("Restore project to {}", &target[..10]);
    git(&root, &["commit", "-m", &message]).map_err(|error| {
        format!("{error}\nRestored files remain staged; original HEAD backup: {backup}")
    })?;
    let restored_tree = git(&root, &["rev-parse", "HEAD^{tree}"])?;
    if restored_tree != target_tree {
        return Err(format!(
            "Restore commit tree differs from selected version (possibly changed by a hook). Original HEAD backup: {backup}"
        ));
    }
    Ok(format!(
        "Restored {target} as latest local commit on {branch}.\nOriginal HEAD backup: {backup}\nRemote unchanged; push separately to synchronize."
    ))
}
