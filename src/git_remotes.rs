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
/// Resolve conflicted paths to the selected side, without touching unrelated files.
fn resolve_conflicts(path: &Path, prefer_remote: bool) -> Result<Vec<String>, String> {
    let conflicts = super::actions::unresolved_conflicts(path)?;
    let side = if prefer_remote { "theirs" } else { "ours" };
    let stage = if prefer_remote { "3" } else { "2" };
    for file in &conflicts {
        let index = git(path, &["ls-files", "-u", "--", file])?;
        let has_preferred_stage = index
            .lines()
            .any(|line| line.split_whitespace().nth(2) == Some(stage));
        if !has_preferred_stage {
            // A missing side represents a deletion; stage that deletion as the chosen version.
            git(path, &["rm", "-f", "--", file])?;
        } else {
            git(path, &["checkout", &format!("--{side}"), "--", file])?;
            git(path, &["add", "-A", "--", file])?;
        }
    }
    Ok(conflicts)
}
/// Finish a previous interrupted update using the explicit remote-wins policy.
fn settle_previous_update_conflicts(path: &Path) -> Result<Option<String>, String> {
    let conflicts = super::actions::unresolved_conflicts(path)?;
    if conflicts.is_empty() {
        return Ok(None);
    }
    if git(path, &["rev-parse", "--verify", "-q", "CHERRY_PICK_HEAD"]).is_ok()
        || git(path, &["rev-parse", "--verify", "-q", "REVERT_HEAD"]).is_ok()
    {
        return Err(format!(
            "发现非 Update 操作产生的冲突，请先完成该操作：{}",
            conflicts.join("、")
        ));
    }
    let merging = git(path, &["rev-parse", "--verify", "-q", "MERGE_HEAD"]).is_ok();
    let resolved = resolve_conflicts(path, merging)?;
    let note = if merging {
        git(path, &["commit", "--no-edit"]).map_err(|error| {
            format!(
                "冲突文件已采用远程版本，但 Git 未能完成合并提交：{error}。文件：{}",
                resolved.join("、")
            )
        })?;
        format!(
            "已完成上次未结束的合并；冲突文件采用远程版本：{}。",
            resolved.join("、")
        )
    } else {
        format!(
            "已处理上次 Update 的恢复冲突，冲突文件采用当前远程版本：{}。原本地改动保留在 Git stash 中。",
            resolved.join("、")
        )
    };
    Ok(Some(note))
}
/// Preserve tracked, staged, and untracked work around a merge. A conflicted stash is
/// deliberately retained so the user can resolve it without losing the recovery copy.
fn update_preserving_work(path: &Path, target: &Target, branch: &str) -> Result<String, String> {
    let dirty = !super::actions::changes(path)?.is_empty();
    let stash_hash = if dirty {
        git(
            path,
            &[
                "stash",
                "push",
                "--include-untracked",
                "--message",
                "DevStart auto-update",
            ],
        )?;
        let hash = git(path, &["rev-parse", "--verify", "refs/stash"])?;
        Some(hash.trim().to_owned())
    } else {
        None
    };
    let pull_result = git_output(
        path,
        &[
            "pull",
            "--no-rebase",
            "--no-edit",
            "--strategy-option=theirs",
            "--",
            &target.url,
            branch,
        ],
    )
    .map(|(out, err)| format!("{out}\n{err}").replace(&target.url, &target.remote.url))
    .map_err(|error| error.replace(&target.url, &target.remote.url));

    let mut output = match pull_result {
        Ok(output) => output,
        Err(error) => {
            let conflicts = super::actions::unresolved_conflicts(path)?;
            if !conflicts.is_empty() {
                let resolved = resolve_conflicts(path, true)?;
                if git(path, &["rev-parse", "--verify", "-q", "MERGE_HEAD"]).is_ok() {
                    git(path, &["commit", "--no-edit"]).map_err(|commit_error| format!(
                    "{error}\n远程优先的冲突文件已解决，但 Git 无法完成合并提交：{commit_error}"
                ))?;
                }
                format!(
                    "{error}\n无法自动合并的文件已按规则采用远程版本：{}。",
                    resolved.join("、")
                )
            } else {
                if let Some(hash) = &stash_hash {
                    let restore = git(path, &["stash", "apply", "--index", hash]);
                    let restore_conflicts = super::actions::unresolved_conflicts(path)?;
                    if !restore_conflicts.is_empty() {
                        let resolved = resolve_conflicts(path, false)?;
                        return Err(format!(
                            "{error}\n本地改动与当前版本冲突，冲突文件按远程版本处理：{}。本地完整改动仍保存在 Git stash 中。",
                            resolved.join("、")
                        ));
                    }
                    if let Err(restore_error) = restore {
                        return Err(format!(
                            "{error}\n恢复本地改动失败：{restore_error}。本地改动仍保存在 Git stash 中。"
                        ));
                    }
                }
                return Err(format!(
                    "{error}\n本地改动已保留；自动恢复副本仍在 Git stash 中。"
                ));
            }
        }
    };
    if output.contains("Already up to date") || output.contains("Already up-to-date") {
        output.push_str("\n远程已是最新，本次没有新增变更。");
    }
    if let Some(hash) = stash_hash {
        let apply = git_output(path, &["stash", "apply", "--index", &hash])
            .map(|(out, err)| format!("{out}\n{err}"));
        match apply {
            Ok(restored) => {
                let conflicts = super::actions::unresolved_conflicts(path)?;
                if !conflicts.is_empty() {
                    let resolved = resolve_conflicts(path, false)?;
                    output.push_str(&format!("\n本地改动无法自动合并的文件已采用远程版本：{}。完整本地改动仍保存在 Git stash 中。", resolved.join("、")));
                    output.push_str(&format!("\n{restored}"));
                    return Ok(output);
                }
                output.push_str(&format!("\n本地改动已恢复。\n{restored}"));
                // Drop only the exact stash entry created above; keep it if its identity changed.
                let entries = git(path, &["stash", "list", "--format=%H"])?;
                if let Some(index) = entries.lines().position(|entry| entry == hash) {
                    let selector = format!("stash@{{{index}}}");
                    if git(path, &["stash", "drop", &selector]).is_ok() {
                        output.push_str("\n已清理本次自动恢复副本。");
                    } else {
                        output.push_str("\n本地改动已恢复；Git stash 中保留了一份恢复副本。");
                    }
                }
            }
            Err(error) => {
                let conflicts = super::actions::unresolved_conflicts(path)?;
                if conflicts.is_empty() {
                    return Err(format!(
                        "{output}\n恢复本地改动失败：{error}。本地改动仍保存在 Git stash 中。"
                    ));
                }
                let resolved = resolve_conflicts(path, false)?;
                output.push_str(&format!("\n本地改动无法自动合并的文件已采用远程版本：{}。完整本地改动仍保存在 Git stash 中。\n{error}", resolved.join("、")));
                return Ok(output);
            }
        }
    }
    Ok(output)
}
/// Merge exactly the selected URL's current branch, without implicit upstream selection or rebase.
pub fn update(path: &Path, key: &str, branch: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let target = current_target(&root, key, branch)?;
    let resolution_note = settle_previous_update_conflicts(&root)?;
    let mut output = update_preserving_work(&root, &target, branch)?;
    if let Some(note) = resolution_note {
        output = format!("{note}\n{output}");
    }
    Ok(output)
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
    let resolution_note = settle_previous_update_conflicts(&root)?;
    if !files.is_empty() && message.trim().is_empty() {
        return Err("Commit message is required".into());
    }
    let target = current_target(&root, key, branch)?;
    if files.is_empty() && !super::actions::changes(&root)?.is_empty() {
        return Err("Select the files to commit before submitting".into());
    }
    let mut progress = String::new();
    if let Some(note) = resolution_note {
        progress.push_str(&format!("{note}\n"));
    }
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
    let update_result = update_preserving_work(&root, &target, branch);
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
        let current = super::actions::changes(&root)?;
        let remaining: Vec<_> = files
            .iter()
            .filter(|file| current.iter().any(|change| &change.path == *file))
            .cloned()
            .collect();
        if remaining.is_empty() {
            progress.push_str("Commit: SKIPPED (selected files now match the remote version)\n");
        } else {
            execute(
                &mut progress,
                "Commit",
                super::actions::commit_unlocked(&root, message, &remaining),
            )?;
        }
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
