//! Durable all-or-nothing updates for the workspace index and managed launch files.
//! A pending journal is rolled back after a crash before configuration is loaded.
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};

#[derive(Serialize, Deserialize)]
struct PreviousFile {
    path: PathBuf,
    bytes: Option<Vec<u8>>,
}
fn journal(root: &Path) -> PathBuf {
    root.join("config-transaction.json")
}
fn destination(root: &Path, relative: &Path) -> Result<PathBuf, String> {
    if relative
        .components()
        .any(|c| !matches!(c, Component::Normal(_)))
    {
        return Err("配置事务包含非法路径".into());
    }
    let mut path = root.to_path_buf();
    for component in relative.components() {
        path.push(component);
        if std::fs::symlink_metadata(&path).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err(format!("配置路径不允许符号链接: {}", path.display()));
        }
    }
    Ok(path)
}
fn clear_journal(root: &Path) -> Result<(), String> {
    std::fs::remove_file(journal(root)).map_err(|e| e.to_string())?;
    std::fs::File::open(root)
        .and_then(|directory| directory.sync_all())
        .map_err(|e| e.to_string())
}
fn restore(root: &Path, previous: &[PreviousFile]) -> Result<(), String> {
    for file in previous.iter().rev() {
        let path = destination(root, &file.path)?;
        if let Some(bytes) = &file.bytes {
            crate::store::atomic_write(&path, bytes)?;
        } else if path.exists() {
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
    }
    clear_journal(root)
}
pub fn recover(root: &Path) -> Result<(), String> {
    let path = journal(root);
    if !path.exists() {
        return Ok(());
    }
    let previous: Vec<PreviousFile> =
        serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("配置事务日志损坏，原文件保留: {e}"))?;
    restore(root, &previous).map_err(|e| format!("恢复未完成的配置事务失败: {e}"))
}
pub fn commit(root: &Path, files: Vec<(PathBuf, Vec<u8>)>) -> Result<(), String> {
    recover(root)?;
    let mut changes = vec![];
    let mut previous = vec![];
    for (relative, bytes) in files {
        let path = destination(root, &relative)?;
        let old = match std::fs::read(&path) {
            Ok(value) => Some(value),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => None,
            Err(e) => return Err(format!("读取配置 {}: {e}", path.display())),
        };
        if old.as_ref() == Some(&bytes) {
            continue;
        }
        std::fs::create_dir_all(path.parent().ok_or("配置父目录缺失")?)
            .map_err(|e| e.to_string())?;
        previous.push(PreviousFile {
            path: relative,
            bytes: old,
        });
        changes.push((path, bytes));
    }
    if changes.is_empty() {
        return Ok(());
    }
    crate::store::atomic_write(
        &journal(root),
        &serde_json::to_vec(&previous).map_err(|e| e.to_string())?,
    )?;
    for (path, bytes) in changes {
        if let Err(error) = crate::store::atomic_write(&path, &bytes) {
            return Err(match restore(root, &previous) {
                Ok(()) => error,
                Err(rollback) => format!("{error}; 配置回滚失败: {rollback}"),
            });
        }
    }
    clear_journal(root)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_commit_restores_all_files_and_removes_new_files() {
        let root = std::env::temp_dir().join(format!(
            "devstart-transaction-{}",
            crate::model::unique_id()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let previous = vec![
            PreviousFile {
                path: "catalog.json".into(),
                bytes: Some(b"old index".to_vec()),
            },
            PreviousFile {
                path: "new.json".into(),
                bytes: None,
            },
        ];
        std::fs::write(root.join("catalog.json"), b"new index").unwrap();
        std::fs::write(root.join("new.json"), b"new manifest").unwrap();
        std::fs::write(journal(&root), serde_json::to_vec(&previous).unwrap()).unwrap();
        recover(&root).unwrap();
        assert_eq!(
            std::fs::read(root.join("catalog.json")).unwrap(),
            b"old index"
        );
        assert!(!root.join("new.json").exists());
        assert!(!journal(&root).exists());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn managed_paths_cannot_write_through_a_symlink_into_a_project() {
        let root =
            std::env::temp_dir().join(format!("devstart-symlink-{}", crate::model::unique_id()));
        std::fs::create_dir_all(root.join("source")).unwrap();
        std::os::unix::fs::symlink(root.join("source"), root.join("projects")).unwrap();
        assert!(
            commit(
                &root,
                vec![("projects/file.json".into(), b"unsafe".to_vec())]
            )
            .is_err()
        );
        assert!(!root.join("source/file.json").exists());
        std::fs::remove_dir_all(root).unwrap();
    }
}
