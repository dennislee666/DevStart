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
                if branch.is_empty() {
                    return Err("当前 HEAD 未关联分支，无法查看远程历史".into());
                }
                let tracking = git(
                    path,
                    &[
                        "for-each-ref",
                        "--format=%(upstream:remotename)%00%(upstream:remoteref)%00%(upstream)",
                        &format!("refs/heads/{branch}"),
                    ],
                )?;
                let fields: Vec<_> = tracking.split('\0').collect();
                let [remote, remote_ref, tracking_ref] = fields.as_slice() else {
                    return Err("当前分支未配置远程上游分支".into());
                };
                if remote.is_empty()
                    || *remote == "."
                    || !remote_ref.starts_with("refs/heads/")
                    || !tracking_ref.starts_with("refs/remotes/")
                {
                    return Err("当前分支未配置远程上游分支".into());
                }
                if refresh {
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
                }
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
                ((*tracking_ref).into(), super::parse_history(&text)?)
            }
        };
        if branch_context(path)?.1 != label {
            return Err("查询期间当前分支已变化，请刷新项目".into());
        }
        Ok(Page {
            source,
            branch: label.clone(),
            reference,
            commits,
            offset,
        })
    })();
    eprintln!(
        "[dev start git] history source={source:?} branch={label} offset={offset} refresh={refresh} elapsed_ms={} success={}",
        started.elapsed().as_millis(),
        result.is_ok()
    );
    result
}
