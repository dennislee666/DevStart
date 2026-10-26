//! Merge/rebase lifecycle and durable ownership of temporary update stashes.
use super::{actions, git, git_output, mutation_guard, root};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    #[default]
    Merge,
    Rebase,
}
impl Method {
    pub fn label(self) -> &'static str {
        match self {
            Self::Merge => "Merge",
            Self::Rebase => "Rebase",
        }
    }
}
#[derive(Clone, Debug, Default, Serialize)]
pub struct State {
    pub operation: Option<String>,
    pub conflicts: Vec<String>,
    pub recovering: bool,
    pub recovery_failed: bool,
    pub recovery_stash: Option<String>,
    pub can_continue: bool,
    pub can_abort: bool,
}
impl State {
    pub fn busy(&self) -> bool {
        self.operation.is_some() || self.recovering || !self.conflicts.is_empty()
    }
}
#[derive(Serialize, Deserialize)]
struct Recovery {
    stash: String,
    phase: String,
    branch: String,
    restore_head: Option<String>,
}
fn git_path(path: &Path, name: &str) -> Result<PathBuf, String> {
    Ok(path.join(git(path, &["rev-parse", "--git-path", name])?.trim()))
}
fn journal(path: &Path) -> Result<PathBuf, String> {
    git_path(path, "devstart-update.json")
}
fn load(path: &Path) -> Result<Option<Recovery>, String> {
    match std::fs::read(journal(path)?) {
        Ok(bytes) => serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| format!("Invalid update recovery journal: {e}")),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}
fn save(path: &Path, record: &Recovery) -> Result<(), String> {
    let file = journal(path)?;
    let temp = file.with_extension("json.tmp");
    std::fs::write(
        &temp,
        serde_json::to_vec(record).map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    std::fs::rename(temp, file).map_err(|e| e.to_string())
}
pub fn state(path: &Path) -> Result<State, String> {
    let path = root(path)?;
    let mut operation = None;
    for (marker, name) in [
        ("rebase-merge", "rebase"),
        ("rebase-apply", "rebase"),
        ("MERGE_HEAD", "merge"),
        ("CHERRY_PICK_HEAD", "cherry-pick"),
        ("REVERT_HEAD", "revert"),
    ] {
        if git_path(&path, marker)?.exists() {
            operation = Some(name.into());
            break;
        }
    }
    let recovery = load(&path)?;
    let recovering = recovery.is_some();
    let recovery_failed = recovery
        .as_ref()
        .is_some_and(|r| matches!(r.phase.as_str(), "failed" | "restoring"));
    let recovery_stash = recovery.as_ref().map(|r| r.stash.clone());
    let conflicts = actions::unresolved_conflicts(&path)?;
    let can_abort = matches!(operation.as_deref(), Some("merge" | "rebase"));
    let can_continue = (can_abort || recovering) && !recovery_failed && conflicts.is_empty();
    Ok(State {
        operation,
        conflicts,
        recovering,
        recovery_failed,
        recovery_stash,
        can_continue,
        can_abort,
    })
}
pub fn ensure_idle(path: &Path) -> Result<(), String> {
    if state(path)?.busy() {
        return Err("Git 操作或冲突处理尚未完成，请先继续或终止当前操作。".into());
    }
    Ok(())
}
pub fn preferred_method(path: &Path) -> Method {
    match git(path, &["config", "--get", "devstart.updateMethod"]).as_deref() {
        Ok("rebase") => Method::Rebase,
        _ => Method::Merge,
    }
}
fn finish_recovery(path: &Path, recovery: &Recovery) -> Result<String, String> {
    // A different program may have changed the stash stack. Drop only our current top entry.
    let top = git(path, &["rev-parse", "--verify", "refs/stash"])?;
    let note = if top.trim() == recovery.stash {
        git(path, &["stash", "drop", "stash@{0}"])?;
        "本地改动已恢复。".to_owned()
    } else {
        format!(
            "本地改动已恢复，恢复副本 {} 保留在 Git stash 中。",
            &recovery.stash[..12]
        )
    };
    std::fs::remove_file(journal(path)?).map_err(|e| e.to_string())?;
    Ok(note)
}
fn restore(path: &Path) -> Result<String, String> {
    let Some(mut recovery) = load(path)? else {
        return Ok(String::new());
    };
    if git(path, &["branch", "--show-current"])? != recovery.branch {
        return Err("保存本地改动后分支发生变化，恢复已暂停；请返回原分支。".into());
    }
    if let Some(expected) = &recovery.restore_head
        && git(path, &["rev-parse", "HEAD"])? != *expected
    {
        return Err("恢复期间 HEAD 已变化，请检查保留的 Git stash。".into());
    }
    if recovery.phase == "conflicts" {
        if !actions::unresolved_conflicts(path)?.is_empty() {
            return Err("请先解决所有恢复冲突。".into());
        }
        return finish_recovery(path, &recovery);
    }
    if matches!(recovery.phase.as_str(), "failed" | "restoring") {
        return Err(format!(
            "本地改动恢复未完成，恢复副本 {} 仍在 Git stash 中。请检查原始错误并手动恢复后再完成恢复。",
            recovery.stash
        ));
    }
    recovery.restore_head = Some(git(path, &["rev-parse", "HEAD"])?);
    recovery.phase = "restoring".into();
    save(path, &recovery)?;
    let mut kept_index = true;
    let mut applied = git_output(path, &["stash", "apply", "--index", &recovery.stash]);
    // An index patch can fail before Git attempts a three-way merge. Retry without
    // restoring staging only when the entire worktree/index is still untouched.
    if applied.is_err() && actions::changes(path)?.is_empty() {
        kept_index = false;
        applied = git_output(path, &["stash", "apply", &recovery.stash]);
    }
    match applied {
        Ok((out, err)) => Ok(format!(
            "{out}\n{err}\n{}{}",
            if kept_index {
                ""
            } else {
                "暂存状态无法原样恢复，改动已恢复到工作区。\n"
            },
            finish_recovery(path, &recovery)?
        )),
        Err(error) => {
            expose_untracked_collisions(path, &recovery.stash)?;
            recovery.phase = if actions::unresolved_conflicts(path)?.is_empty() {
                "failed"
            } else {
                "conflicts"
            }
            .into();
            save(path, &recovery)?;
            Err(format!(
                "恢复本地改动需要处理：{error}\n恢复副本 {} 已保留；请解决冲突后点击继续。",
                recovery.stash
            ))
        }
    }
}
/// Called only with the repository mutation lease held. Fetch happens before stashing.
pub(super) fn integrate(
    path: &Path,
    reference: &str,
    method: Method,
    preserve: bool,
) -> Result<String, String> {
    ensure_idle(path)?;
    if git(path, &["merge-base", "--is-ancestor", reference, "HEAD"]).is_ok() {
        return Ok("已经更新，没有任何变化。".into());
    }
    if !actions::changes(path)?.is_empty() {
        if !preserve {
            return Err("请先提交或暂存本地改动，再执行分支操作。".into());
        }
        git(
            path,
            &[
                "stash",
                "push",
                "--include-untracked",
                "-m",
                "DevStart update recovery",
            ],
        )?;
        let stash = git(path, &["rev-parse", "--verify", "refs/stash"])?;
        save(
            path,
            &Recovery {
                stash: stash.clone(),
                phase: "integrating".into(),
                branch: git(path, &["branch", "--show-current"])?,
                restore_head: None,
            },
        ).map_err(|error| format!("无法保存 Update 恢复记录：{error}。本地改动已安全保存在 Git stash {stash} 中，请先恢复该副本。"))?;
    }
    let result = match method {
        Method::Merge => git_output(
            path,
            &[
                "-c",
                "merge.autoStash=false",
                "merge",
                "--no-edit",
                "--",
                reference,
            ],
        ),
        Method::Rebase => git_output(path, &["-c", "rebase.autoStash=false", "rebase", reference]),
    };
    match result {
        Ok((out, err)) => Ok(format!("{out}\n{err}\n{}", restore(path)?)),
        Err(error) => {
            let current = state(path)?;
            if current.operation.is_some() || !current.conflicts.is_empty() {
                Err(format!(
                    "{error}\n操作已暂停，请处理冲突后继续，或终止操作。本地恢复副本会保留。"
                ))
            } else {
                let restored = restore(path);
                Err(format!("{error}\n{}", restored.unwrap_or_else(|e| e)))
            }
        }
    }
}
pub fn rebase(path: &Path, onto: &str) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    let current = git(&path, &["branch", "--show-current"])?;
    if current.is_empty() || current == onto {
        return Err("请选择与当前分支不同的 Rebase 目标。".into());
    }
    if !super::branches(&path)?.iter().any(|b| b == onto) {
        return Err("Rebase 目标分支已不存在，请刷新。".into());
    }
    integrate(&path, &format!("refs/heads/{onto}"), Method::Rebase, true)
}
pub fn continue_operation(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    let state = state(&path)?;
    if !state.conflicts.is_empty() {
        return Err("请先解决全部冲突文件，再继续操作。".into());
    }
    let output = match state.operation.as_deref() {
        Some("merge") => git(&path, &["commit", "--no-edit"])?,
        Some("rebase") => git(&path, &["rebase", "--continue"])?,
        Some(_) => return Err("请在发起该操作的工具中继续。".into()),
        None if state.recovering => String::new(),
        None => return Err("没有需要继续的 Git 操作。".into()),
    };
    Ok(format!("{output}\n{}", restore(&path)?))
}
pub fn abort(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    let current = state(&path)?;
    let output = match current.operation.as_deref() {
        Some("merge") => git(&path, &["merge", "--abort"])?,
        Some("rebase") => git(&path, &["rebase", "--abort"])?,
        _ => return Err("没有可以终止的 Merge/Rebase；恢复冲突需完成处理。".into()),
    };
    Ok(format!("{output}\n操作已终止。\n{}", restore(&path)?))
}

/// Explicit acknowledgment after manual recovery: keep the backup rather than guessing what was restored.
pub fn finish_manual_recovery(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    let current = state(&path)?;
    if current.operation.is_some() || !current.conflicts.is_empty() {
        return Err("请先完成当前操作并解决冲突。".into());
    }
    let record = load(&path)?.ok_or("没有恢复记录")?;
    std::fs::remove_file(journal(&path)?).map_err(|e| e.to_string())?;
    Ok(format!(
        "已结束手动恢复。完整恢复副本 {} 仍保留在 Git stash 中。",
        record.stash
    ))
}

/// Skipping discards only the currently replayed commit and always requires an explicit UI choice.
pub fn skip_rebase_commit(path: &Path) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let path = root(path)?;
    if state(&path)?.operation.as_deref() != Some("rebase") {
        return Err("没有正在暂停的 Rebase。".into());
    }
    let output = git(&path, &["rebase", "--skip"])?;
    Ok(format!("{output}\n{}", restore(&path)?))
}

/// Stash cannot merge an untracked file when the updated branch now tracks the
/// same path. Represent that add/add collision in Git's index for normal review.
fn expose_untracked_collisions(path: &Path, stash: &str) -> Result<(), String> {
    let Ok(entries) = git(path, &["ls-tree", "-r", "-z", &format!("{stash}^3")]) else {
        return Ok(());
    };
    for entry in entries.split('\0').filter(|entry| !entry.is_empty()) {
        let Some((metadata, file)) = entry.split_once('\t') else {
            return Err("Invalid stash tree entry".into());
        };
        if actions::unresolved_conflicts(path)?
            .iter()
            .any(|p| p == file)
        {
            continue;
        }
        let saved: Vec<_> = metadata.split_whitespace().collect();
        if saved.len() != 3 {
            return Err("Invalid stash tree metadata".into());
        }
        let tracked = git(path, &["ls-files", "--stage", "-z", "--", file])?;
        let Some((index, _)) = tracked.split_once('\t') else {
            continue;
        };
        let current: Vec<_> = index.split_whitespace().collect();
        if current.len() != 3 || current[2] != "0" {
            continue;
        }
        let data = format!(
            "0 {}\t{file}\0{} {} 2\t{file}\0{} {} 3\t{file}\0",
            "0".repeat(current[1].len()),
            current[0],
            current[1],
            saved[0],
            saved[2]
        );
        super::git_output_input(
            path,
            &["update-index", "-z", "--index-info"],
            Some(data.as_bytes()),
        )?;
    }
    Ok(())
}
