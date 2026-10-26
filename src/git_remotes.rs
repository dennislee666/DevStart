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
/// Update is independent from commit and push. Conflicts always remain for review.
pub fn update(path: &Path, key: &str, branch: &str) -> Result<String, String> {
    update_with_method(
        path,
        key,
        branch,
        super::integration::preferred_method(path),
    )
}
pub fn update_with_method(
    path: &Path,
    key: &str,
    branch: &str,
    method: super::integration::Method,
) -> Result<String, String> {
    let _guard = mutation_guard(path)?;
    let root = root(path)?;
    let target = current_target(&root, key, branch)?;
    super::integration::ensure_idle(&root)?;
    git(
        &root,
        &[
            "config",
            "--local",
            "devstart.updateMethod",
            match method {
                super::integration::Method::Merge => "merge",
                super::integration::Method::Rebase => "rebase",
            },
        ],
    )?;
    let fetched = git_output(&root, &["fetch", "--no-tags", "--", &target.url, branch])
        .map_err(|error| error.replace(&target.url, &target.remote.url))?;
    let reference = git(&root, &["rev-parse", "--verify", "FETCH_HEAD^{commit}"])?;
    let output = super::integration::integrate(&root, &reference, method, true)?;
    Ok(format!("{}\n{}\n{output}", fetched.0, fetched.1).replace(&target.url, &target.remote.url))
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

/// Explicit Commit and Push: commit selected changes locally, then push existing commits.
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
    super::integration::ensure_idle(&root)?;
    if push_remote {
        current_target(&root, key, branch)?;
    }
    let mut output = super::actions::commit_unlocked(&root, message, files)?;
    if push_remote {
        output.push_str(&format!(
            "\nPush: OK\n{}",
            push_unlocked(&root, key, branch)
                .map_err(|error| format!("本地提交已完成，推送失败：{error}"))?
        ));
    }
    Ok(output)
}
pub fn push_rejected(error: &str) -> bool {
    ["non-fast-forward", "fetch first", "[rejected]"]
        .iter()
        .any(|marker| error.contains(marker))
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
