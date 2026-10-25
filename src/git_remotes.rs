//! Existing remote targets are selected by stable keys; URLs are never rewritten on selection.
use super::{git, git_output, mutation_guard, root};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::path::Path;
#[derive(Clone, Serialize)]
pub struct Remote {
    pub key: String,
    pub name: String,
    pub url: String,
    pub preferred: bool,
}
struct Target {
    remote: Remote,
    url: String,
}
fn display_url(url: &str) -> String {
    let mut value = url.to_owned();
    if let Some((scheme, rest)) = url.split_once("://") {
        let authority = rest.split('/').next().unwrap_or(rest);
        if let Some((_, host)) = authority.rsplit_once('@') {
            value = format!("{scheme}://***@{host}{}", &rest[authority.len()..]);
        }
    }
    if let Some(i) = value.find('?') {
        value.truncate(i);
        value.push_str("?***");
    }
    value
}
fn targets(path: &Path) -> Result<Vec<Target>, String> {
    let root = root(path)?;
    let branch = git(&root, &["branch", "--show-current"])?;
    let tracking = if branch.is_empty() {
        String::new()
    } else {
        git(
            &root,
            &[
                "for-each-ref",
                "--format=%(upstream:remotename)",
                &format!("refs/heads/{branch}"),
            ],
        )?
    };
    let mut result = vec![];
    for name in git(&root, &["remote"])?.lines() {
        for url in git(&root, &["remote", "get-url", "--all", name])?.lines() {
            let digest = Sha256::digest(format!("{name}\0{url}").as_bytes());
            let key = format!("{digest:x}");
            result.push(Target {
                remote: Remote {
                    key,
                    name: name.into(),
                    url: display_url(url),
                    preferred: false,
                },
                url: url.into(),
            });
        }
    }
    let preferred = result
        .iter()
        .position(|r| !tracking.is_empty() && r.remote.name == tracking)
        .or_else(|| result.iter().position(|r| r.remote.name == "origin"))
        .unwrap_or(0);
    if let Some(item) = result.get_mut(preferred) {
        item.remote.preferred = true;
    }
    Ok(result)
}
pub fn list(path: &Path) -> Result<Vec<Remote>, String> {
    Ok(targets(path)?.into_iter().map(|t| t.remote).collect())
}
fn current_target(path: &Path, key: &str, branch: &str) -> Result<Target, String> {
    let actual = git(path, &["branch", "--show-current"])?;
    if branch.is_empty() || actual != branch {
        return Err("Current branch changed or HEAD is detached; refresh before continuing".into());
    }
    targets(path)?
        .into_iter()
        .find(|r| r.remote.key == key)
        .ok_or("Remote address changed or was removed; refresh before continuing".into())
}
/// Merge exactly the selected URL's current branch, without implicit upstream selection or rebase.
pub fn update(path: &Path, key: &str, branch: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let target = current_target(&root, key, branch)?;
    let conflicts = super::actions::unresolved_conflicts(&root)?;
    if !conflicts.is_empty() {
        return Err(format!(
            "{}",
            crate::i18n::tf(
                "存在未解决的冲突，请先处理：{files}",
                &[("files", conflicts.join("、"))]
            )
        ));
    }
    if !super::actions::changes(&root)?.is_empty() {
        return Err("Commit or stash local changes before updating".into());
    }
    let result = git_output(
        &root,
        &[
            "pull",
            "--no-rebase",
            "--no-edit",
            "--",
            &target.url,
            branch,
        ],
    );
    result
        .map(|(out, err)| format!("{out}\n{err}").replace(&target.url, &target.remote.url))
        .map_err(|e| e.replace(&target.url, &target.remote.url))
}
pub fn push(path: &Path, key: &str, branch: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    push_unlocked(path, key, branch)
}
fn push_unlocked(path: &Path, key: &str, branch: &str) -> Result<String, String> {
    let root = root(path)?;
    let target = current_target(&root, key, branch)?;
    let result = git_output(
        &root,
        &[
            "push",
            "--",
            &target.url,
            &format!("HEAD:refs/heads/{branch}"),
        ],
    );
    result
        .map(|(out, err)| format!("{out}\n{err}").replace(&target.url, &target.remote.url))
        .map_err(|e| e.replace(&target.url, &target.remote.url))
}

/// One repository write lease covers update, selected-file commit and optional push.
/// Git autostash preserves tracked edits during merge; untracked files are never removed.
pub fn submit(
    path: &Path,
    key: &str,
    branch: &str,
    message: &str,
    files: &[String],
    push_remote: bool,
) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let conflicts = super::actions::unresolved_conflicts(&root)?;
    if !conflicts.is_empty() {
        return Err(format!(
            "{}",
            crate::i18n::tf(
                "存在未解决的冲突，请先处理：{files}",
                &[("files", conflicts.join("、"))]
            )
        ));
    }
    if !files.is_empty() && message.trim().is_empty() {
        return Err("Commit message is required".into());
    }
    let target = current_target(&root, key, branch)?;
    if files.is_empty() && !super::actions::changes(&root)?.is_empty() {
        return Err("Select the files to commit before submitting".into());
    }
    let mut progress = String::new();
    let execute =
        |progress: &mut String, step: &str, result: Result<String, String>| -> Result<(), String> {
            match result {
                Ok(output) => {
                    progress.push_str(&format!("{step}: OK\n{output}\n"));
                    Ok(())
                }
                Err(error) => Err(format!("{progress}{step}: FAILED\n{error}")),
            }
        };
    let update_result = git_output(
        &root,
        &[
            "pull",
            "--no-rebase",
            "--no-edit",
            "--autostash",
            "--",
            &target.url,
            branch,
        ],
    )
    .map(|(out, err)| format!("{out}\n{err}").replace(&target.url, &target.remote.url))
    .map_err(|error| error.replace(&target.url, &target.remote.url));
    match update_result {
        Ok(output) => progress.push_str(&format!("Update Project: OK\n{output}\n")),
        Err(error) => {
            let conflicts = super::actions::unresolved_conflicts(&root)?;
            if !conflicts.is_empty() {
                return Err(format!(
                    "Update Project: FAILED\n{error}\n{}",
                    crate::i18n::tf(
                        "自动暂存恢复发生冲突；本地改动仍保存在 Git stash 中。冲突文件：{files}",
                        &[("files", conflicts.join("、"))]
                    )
                ));
            }
            return Err(format!("Update Project: FAILED\n{error}"));
        }
    }
    let conflicts = super::actions::unresolved_conflicts(&root)?;
    if !conflicts.is_empty() {
        return Err(format!(
            "{progress}Update Project: FAILED\n{}",
            crate::i18n::tf(
                "自动暂存恢复发生冲突；本地改动仍保存在 Git stash 中。冲突文件：{files}",
                &[("files", conflicts.join("、"))]
            )
        ));
    }
    if files.is_empty() {
        progress.push_str("Commit: SKIPPED (no selected changes)\n");
    } else {
        execute(
            &mut progress,
            "Commit",
            super::actions::commit_unlocked(&root, message, files),
        )?;
    }
    if push_remote {
        execute(&mut progress, "Push", push_unlocked(&root, key, branch))?;
    }
    Ok(progress)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn displayed_urls_keep_repository_identity_without_secrets() {
        assert_eq!(
            display_url("https://user:secret@example.com/team/repo.git?token=private"),
            "https://***@example.com/team/repo.git?***"
        );
        assert_eq!(
            display_url("git@example.com:team/repo.git"),
            "git@example.com:team/repo.git"
        );
    }
}
