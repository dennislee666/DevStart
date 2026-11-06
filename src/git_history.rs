//! Current-branch commit history. Fetch only the upstream ref; never change HEAD/index/worktree.
use super::{Commit, git, mutation_guard, root};
use serde::Serialize;
use std::path::Path;
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Source {
    #[default]
    Remote,
    Local,
}
#[derive(Clone, Serialize)]
pub struct Page {
    pub source: Source,
    pub branch: String,
    pub reference: String,
    pub commits: Vec<Commit>,
    pub offset: usize,
    /// Commit divergence from the configured upstream: local-only, then remote-only.
    pub local_ahead: usize,
    pub remote_ahead: usize,
}

fn divergence(path: &Path, branch: &str) -> Result<(usize, usize), String> {
    if branch.is_empty() {
        return Ok((0, 0));
    }
    let tracking = git(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    if tracking.is_empty() {
        return Ok((0, 0));
    }
    if git(
        path,
        &[
            "rev-parse",
            "--verify",
            "--quiet",
            &format!("{tracking}^{{commit}}"),
        ],
    )
    .is_err()
    {
        // A local-only branch may not have fetched its upstream yet. Local history remains usable.
        return Ok((0, 0));
    }
    let local = format!("refs/heads/{branch}");
    let counts = git(
        path,
        &[
            "rev-list",
            "--left-right",
            "--count",
            &format!("{local}...{tracking}"),
        ],
    )?;
    let mut values = counts
        .split_whitespace()
        .filter_map(|value| value.parse::<usize>().ok());
    match (values.next(), values.next()) {
        (Some(local_ahead), Some(remote_ahead)) => Ok((local_ahead, remote_ahead)),
        _ => Err(format!("无法解析本地与远程分支差异: {counts}")),
    }
}
fn branch_context(path: &Path) -> Result<(String, String), String> {
    let branch = git(path, &["branch", "--show-current"])?;
    let label = if branch.is_empty() {
        format!("detached {}", git(path, &["rev-parse", "--short", "HEAD"])?)
    } else {
        branch.clone()
    };
    Ok((branch, label))
}
type Upstream = (String, String, String);

fn upstream(path: &Path, branch: &str) -> Result<Option<Upstream>, String> {
    if branch.is_empty() {
        return Ok(None);
    }
    let tracking = git(
        path,
        &[
            "for-each-ref",
            "--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
            &format!("refs/heads/{branch}"),
        ],
    )?;
    if tracking.is_empty() {
        return Ok(None);
    }
    let fields: Vec<_> = tracking.split('\0').collect();
    let [remote, remote_ref, tracking_ref] = fields.as_slice() else {
        return Ok(None);
    };
    if remote.is_empty()
        || *remote == "."
        || !remote_ref.starts_with("refs/heads/")
        || !tracking_ref.starts_with("refs/remotes/")
    {
        return Ok(None);
    }
    Ok(Some((
        (*remote).into(),
        (*remote_ref).into(),
        (*tracking_ref).into(),
    )))
}

fn fetch_upstream(path: &Path, upstream: &Upstream) -> Result<(), String> {
    let (remote, remote_ref, tracking_ref) = upstream;
    let _guard = mutation_guard(path)?;
    let refspec = format!("+{remote_ref}:{tracking_ref}");
    git(
        path,
        &[
            "fetch",
            "--no-tags",
            "--no-recurse-submodules",
            "--no-write-fetch-head",
            "--",
            remote,
            &refspec,
        ],
    )
    .map_err(|e| format!("读取远程提交历史失败: {e}"))?;
    Ok(())
}

pub fn read(
    path: &Path,
    expected_branch: Option<&str>,
    source: Source,
    offset: usize,
    limit: usize,
    refresh: bool,
) -> Result<Page, String> {
    if limit == 0 || limit > 100 || offset > 100_000 {
        return Err("history requires limit 1..100 and offset <= 100000".into());
    }
    root(path)?;
    let (branch, label) = branch_context(path)?;
    if expected_branch.is_some_and(|expected| expected != label) {
        return Err("当前分支已变化，请刷新项目后查看提交历史".into());
    }
    let started = std::time::Instant::now();
    let result = (|| {
        let upstream = upstream(path, &branch)?;
        // Refresh upstream state for both history sources so local view buttons stay current too.
        if refresh && let Some(upstream) = &upstream {
            fetch_upstream(path, upstream)?;
        }
        let (reference, commits) = match source {
            Source::Local => {
                let reference = if branch.is_empty() {
                    "HEAD".into()
                } else {
                    format!("refs/heads/{branch}")
                };
                (reference, super::history(path, offset, limit)?)
            }
            Source::Remote => {
                let Some((_, _, tracking_ref)) = &upstream else {
                    return Err(if branch.is_empty() {
                        "当前 HEAD 未关联分支，无法查看远程历史".into()
                    } else {
                        "当前分支未配置远程上游分支".into()
                    });
                };
                let text = git(
                    path,
                    &[
                        "log",
                        "--no-show-signature",
                        "--format=%H%x00%an%x00%aI%x00%s",
                        &format!("--skip={offset}"),
                        &format!("--max-count={limit}"),
                        tracking_ref,
                        "--",
                    ],
                )?;
                (tracking_ref.clone(), super::parse_history(&text)?)
            }
        };
        let (local_ahead, remote_ahead) = divergence(path, &branch)?;
        if branch_context(path)?.1 != label {
            return Err("查询期间当前分支已变化，请刷新项目".into());
        }
        Ok(Page {
            source,
            branch: label.clone(),
            reference,
            commits,
            offset,
            local_ahead,
            remote_ahead,
        })
    })();
    eprintln!(
        "[dev start git] history source={source:?} branch={label} offset={offset} refresh={refresh} elapsed_ms={} success={}",
        started.elapsed().as_millis(),
        result.is_ok()
    );
    result
}
