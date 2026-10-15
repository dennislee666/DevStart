use crate::model::Catalog;
use std::{fs, io, path::PathBuf};

pub struct Store {
    pub root: PathBuf,
    _lock: fs::File,
}
impl Store {
    pub fn new() -> io::Result<Self> {
        let root = crate::app_identity::workspace()?;
        Self::at(root)
    }
    pub fn at(root: PathBuf) -> io::Result<Self> {
        use std::os::fd::AsRawFd;
        fs::create_dir_all(root.join("logs"))?;
        let lock = fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(root.join("instance.lock"))?;
        if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
            return Err(io::Error::other(
                "DevStart 已在运行；同一工作空间只允许一个实例",
            ));
        }
        Ok(Self { root, _lock: lock })
    }
    pub fn load(&self) -> Result<Catalog, String> {
        let path = self.root.join("catalog.json");
        if !path.exists() {
            return Ok(Catalog::default());
        }
        let data = fs::read(&path).map_err(|e| format!("读取 {}: {e}", path.display()))?;
        let mut catalog: Catalog = serde_json::from_slice(&data)
            .map_err(|e| format!("配置解析失败（原文件保留）: {e}"))?;
        if catalog.version != 1 {
            return Err(format!("不支持配置版本 {}", catalog.version));
        }
        catalog.preferences.validate()?;
        let before = serde_json::to_vec(&catalog).map_err(|e| e.to_string())?;
        if !catalog.groups.iter().any(|g| g.id == 1) {
            catalog.groups.insert(
                0,
                crate::model::Group {
                    id: 1,
                    name: "默认分组".into(),
                },
            );
        }
        let highest = catalog
            .groups
            .iter()
            .map(|g| g.id)
            .chain(catalog.projects.iter().map(|p| p.id))
            .chain(
                catalog
                    .projects
                    .iter()
                    .flat_map(|p| p.scripts.iter().map(|s| s.id)),
            )
            .max()
            .unwrap_or(1);
        catalog.next_id = catalog
            .next_id
            .max(highest.checked_add(1).ok_or("配置ID已耗尽")?);
        let group_ids: std::collections::HashSet<_> = catalog.groups.iter().map(|g| g.id).collect();
        for project in &mut catalog.projects {
            if !group_ids.contains(&project.group) {
                project.group = 1;
            }
        }
        if serde_json::to_vec(&catalog).map_err(|e| e.to_string())? != before {
            self.save(&catalog)?;
        }
        Ok(catalog)
    }
    pub fn save(&self, catalog: &Catalog) -> Result<(), String> {
        let path = self.root.join("catalog.json");
        let temp = self.root.join("catalog.json.tmp");
        let bytes = serde_json::to_vec_pretty(catalog).map_err(|e| e.to_string())?;
        use std::io::Write;
        let mut file = fs::File::create(&temp).map_err(|e| e.to_string())?;
        file.write_all(&bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(temp, path).map_err(|e| format!("配置写入失败: {e}"))
    }
}

/// Same-directory atomic replacement; never follow a manifest symlink or truncate the original.
pub fn atomic_write(path: &std::path::Path, bytes: &[u8]) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::OpenOptionsExt};
    if std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(format!("拒绝覆盖符号链接: {}", path.display()));
    }
    let temp = path.with_extension(format!("json.{}.tmp", crate::model::unique_id()));
    let result = (|| {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temp)
            .map_err(|e| e.to_string())?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|e| e.to_string())?;
        fs::rename(&temp, path).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
