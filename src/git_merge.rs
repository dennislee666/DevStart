//! Local branch merge holds one repository lease across target checkout and merge.
use super::{branches, git, mutation_guard, root};
use std::path::Path;

/// Shared deterministic validation for UI confirmation and API execution.
pub fn validate_pair(source: &str, target: &str) -> Result<(), String> {
    if source == target {
        return Err("合并分支不能两个相同的分支合并".into());
    }
    Ok(())
}
/// Reject dirty or in-progress work before checkout. Conflicts remain available for resolution.
pub fn merge(path: &Path, source: &str, target: &str) -> Result<String, String> {
    validate_pair(source, target)?;
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    super::integration::ensure_idle(&path)?;
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

#[cfg(test)]
mod tests {
    #[test]
    fn same_branch_rejected_before_any_repository_operation() {
        let error = super::merge(
            std::path::Path::new("/nonexistent-devstart-test-repo"),
            "main",
            "main",
        )
        .unwrap_err();
        assert_eq!(error, "合并分支不能两个相同的分支合并");
        assert!(super::validate_pair("main", "feature").is_ok());
    }
}
