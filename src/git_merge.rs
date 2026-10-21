//! Local branch merge holds one repository lease across target checkout and merge.
use super::{branches, git, mutation_guard, root};
use std::path::Path;

/// Reject dirty or in-progress work before checkout. Conflicts remain available for resolution.
pub fn merge(path: &Path, source: &str, target: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    if source == target {
        return Err("Source and destination branches must differ".into());
    }
    let local = branches(&path)?;
    if !local.iter().any(|b| b == source) || !local.iter().any(|b| b == target) {
        return Err(
            "Source or destination local branch no longer exists; refresh and select again".into(),
        );
    }
    if !git(
        &path,
        &["status", "--porcelain=v1", "--untracked-files=normal"],
    )?
    .is_empty()
    {
        return Err(
            "Commit or stash local changes before merging branches; no branch was switched".into(),
        );
    }
    for state in [
        "MERGE_HEAD",
        "CHERRY_PICK_HEAD",
        "REVERT_HEAD",
        "rebase-merge",
        "rebase-apply",
    ] {
        let state_path = git(&path, &["rev-parse", "--git-path", state])?;
        if path.join(state_path).exists() {
            return Err(format!(
                "A Git operation is already in progress ({state}); resolve it before merging"
            ));
        }
    }
    // Git's own merge-state validation remains authoritative; never force or discard edits.
    git(&path, &["switch", "--no-guess", "--", target])?;
    let actual = git(&path, &["branch", "--show-current"])?;
    if actual != target {
        return Err(format!(
            "Expected destination {target}, actual branch {actual}"
        ));
    }
    let reference = format!("refs/heads/{source}");
    match git(&path, &["merge", "--no-edit", "--", &reference]) {
        Ok(output) => Ok(format!(
            "{source} → {target}\nCurrent branch: {target}\n{output}"
        )),
        Err(error) => Err(format!(
            "{source} → {target}\nCurrent branch: {target}\n{error}"
        )),
    }
}
