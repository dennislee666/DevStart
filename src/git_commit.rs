//! Read-only commit inspection. Compare merges to their first parent and root commits to the empty tree.
use super::{Commit, git, root};
use serde::Serialize;
use std::path::Path;
#[derive(Clone, Serialize)]
pub struct Change {
    pub path: String,
    pub previous: Option<String>,
    pub status: String,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    pub file_name: String,
    pub full_path: String,
    pub tooltip: String,
}
#[derive(Clone, Serialize)]
pub struct Inspection {
    pub commit: Commit,
    pub parents: Vec<String>,
    pub base: Option<String>,
    pub files: Vec<Change>,
}
fn resolve(path: &Path, id: &str) -> Result<String, String> {
    if ![40, 64].contains(&id.len()) || !id.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("A full commit hash from git.history is required".into());
    }
    git(
        path,
        &["rev-parse", "--verify", &format!("{id}^{{commit}}")],
    )
}
pub fn files(path: &Path, id: &str) -> Result<Inspection, String> {
    let path = root(path)?;
    let id = resolve(&path, id)?;
    let text = git(
        &path,
        &[
            "show",
            "--no-patch",
            "--no-show-signature",
            "--format=%H%x00%an%x00%aI%x00%s%x00%P",
            &id,
            "--",
        ],
    )?;
    let parts: Vec<_> = text.splitn(5, '\0').collect();
    if parts.len() != 5 {
        return Err("Invalid Git commit metadata".into());
    }
    let parents: Vec<String> = parts[4].split_whitespace().map(str::to_owned).collect();
    if parts[0] != id
        || parents
            .iter()
            .any(|p| ![40, 64].contains(&p.len()) || !p.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err("Invalid Git commit identity or parent metadata".into());
    }
    let base = parents.first().cloned();
    let out = if let Some(base) = &base {
        git(
            &path,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--name-status",
                "--find-renames",
                "-z",
                base,
                &id,
                "--",
            ],
        )?
    } else {
        git(
            &path,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--name-status",
                "--find-renames",
                "-r",
                "-z",
                &id,
                "--",
            ],
        )?
    };
    // One numstat query for the entire commit; never launch Git once per file.
    let statistics = if let Some(base) = &base {
        git(
            &path,
            &[
                "diff",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "--find-renames",
                "-z",
                base,
                &id,
                "--",
            ],
        )?
    } else {
        git(
            &path,
            &[
                "diff-tree",
                "--root",
                "--no-commit-id",
                "--no-ext-diff",
                "--no-textconv",
                "--numstat",
                "--find-renames",
                "-r",
                "-z",
                &id,
                "--",
            ],
        )?
    };
    let statistics = parse_numstat(&statistics)?;
    let mut fields = out.split('\0').filter(|p| !p.is_empty());
    let mut files = vec![];
    while let Some(status) = fields.next() {
        let first = fields.next().ok_or("Invalid commit file record")?;
        let (previous, file) = if status.starts_with(['R', 'C']) {
            (
                Some(first.to_owned()),
                fields.next().ok_or("Invalid commit rename record")?,
            )
        } else {
            (None, first)
        };
        let full_path = path.join(file).display().to_string();
        let tooltip = previous
            .as_ref()
            .map(|old| format!("{} → {full_path}", path.join(old).display()))
            .unwrap_or_else(|| full_path.clone());
        let (additions, deletions) = statistics
            .get(file)
            .copied()
            .ok_or("Missing commit line statistics")?;
        files.push(Change {
            additions,
            deletions,
            file_name: Path::new(file)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned(),
            full_path,
            tooltip,
            path: file.into(),
            previous,
            status: status.into(),
        });
    }
    Ok(Inspection {
        commit: Commit {
            id: parts[0].into(),
            author: parts[1].into(),
            date: parts[2].into(),
            subject: parts[3].into(),
        },
        parents,
        base,
        files,
    })
}
pub fn diff(path: &Path, id: &str, file: &str) -> Result<String, String> {
    let path = root(path)?;
    let inspection = files(&path, id)?;
    let item = inspection
        .files
        .iter()
        .find(|c| c.path == file)
        .ok_or("File is not part of this commit")?;
    let paths: Vec<&str> = item
        .previous
        .iter()
        .map(String::as_str)
        .chain(std::iter::once(item.path.as_str()))
        .collect();
    let mut args = if let Some(base) = &inspection.base {
        vec![
            "diff",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
            base,
            &inspection.commit.id,
            "--",
        ]
    } else {
        vec![
            "show",
            "--format=",
            "--no-show-signature",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--find-renames",
            "--root",
            &inspection.commit.id,
            "--",
        ]
    };
    args.extend(paths);
    git(&path, &args)
}

/// NUL records preserve tabs/newlines in filenames. Renames contain separate old/new paths.
type LineStatistics = std::collections::HashMap<String, (Option<u64>, Option<u64>)>;
fn parse_numstat(text: &str) -> Result<LineStatistics, String> {
    let mut fields = text.split('\0').filter(|p| !p.is_empty());
    let mut result = std::collections::HashMap::new();
    while let Some(record) = fields.next() {
        let mut parts = record.splitn(3, '\t');
        let added = parts.next().ok_or("Invalid Git numstat record")?;
        let deleted = parts.next().ok_or("Invalid Git numstat record")?;
        let path = parts.next().ok_or("Invalid Git numstat path")?;
        let path = if path.is_empty() {
            fields.next().ok_or("Missing Git numstat rename source")?;
            fields
                .next()
                .ok_or("Missing Git numstat rename destination")?
        } else {
            path
        };
        let counts = if added == "-" && deleted == "-" {
            (None, None)
        } else {
            (
                Some(added.parse().map_err(|_| "Invalid Git added line count")?),
                Some(
                    deleted
                        .parse()
                        .map_err(|_| "Invalid Git deleted line count")?,
                ),
            )
        };
        if result.insert(path.to_owned(), counts).is_some() {
            return Err("Duplicate Git numstat path".into());
        }
    }
    Ok(result)
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn numstat_preserves_paths_and_binary_and_renames() {
        let rows =
            parse_numstat("12\t3\tsrc/a.java\0-\t-\timage.png\x000\t0\t\0old\tname\0new\nname\0")
                .unwrap();
        assert_eq!(rows["src/a.java"], (Some(12), Some(3)));
        assert_eq!(rows["image.png"], (None, None));
        assert_eq!(rows["new\nname"], (Some(0), Some(0)));
        assert!(parse_numstat("bad\t2\ta\0").is_err());
        assert!(parse_numstat("0\t0\t\0old\0").is_err());
    }
}
