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
        crate::config_transaction::recover(&root).map_err(io::Error::other)?;
        crate::log_store::maintain(&root.join("logs"), true)?;
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
        if catalog.version == 1 {
            return self.migrate(catalog, &data);
        }
        if catalog.version != 2 {
            return Err(format!("不支持配置版本 {}", catalog.version));
        }
        self.finish_migration(&catalog)?;
        catalog.preferences.validate()?;
        for project in &mut catalog.projects {
            let path = self.manifest_path(project.id);
            let bytes = fs::read(&path)
                .map_err(|e| format!("读取DevStart启动配置 {}: {e}", path.display()))?;
            crate::project_manifest::validate_document(&bytes)?;
        }
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
    /// Project IDs keep managed configuration independent of visible project names.
    pub fn manifest_path(&self, project_id: u64) -> PathBuf {
        self.root
            .join("projects")
            .join(project_id.to_string())
            .join(crate::project_manifest::FILE_NAME)
    }
    pub fn normalize(&self, mut catalog: Catalog) -> Result<Catalog, String> {
        catalog.version = 2;
        for project in &mut catalog.projects {
            let path = self.manifest_path(project.id);
            let specs = crate::project_manifest::storage_specifications(project, &path)?;
            crate::project_manifest::bind_snapshots(project, &path, specs)?;
        }
        Ok(catalog)
    }
    pub fn save(&self, catalog: &Catalog) -> Result<(), String> {
        let catalog = self.normalize(catalog.clone())?;
        let mut files = vec![];
        for project in &catalog.projects {
            let relative = PathBuf::from("projects")
                .join(project.id.to_string())
                .join(crate::project_manifest::FILE_NAME);
            files.push((
                relative,
                crate::project_manifest::encode(crate::project_manifest::storage_specifications(
                    project,
                    &self.manifest_path(project.id),
                )?)?,
            ));
        }
        files.push((
            PathBuf::from("catalog.json"),
            serde_json::to_vec_pretty(&catalog).map_err(|e| e.to_string())?,
        ));
        crate::config_transaction::commit(&self.root, files)
    }
    /// One-time format conversion, never a runtime fallback to project-owned files.
    /// Archive exact bytes first; publish the new workspace before removing originals.
    fn migrate(&self, mut catalog: Catalog, original: &[u8]) -> Result<Catalog, String> {
        let backup = self
            .root
            .join("migration-backups")
            .join(format!("managed-config-{}", crate::model::unique_id()));
        fs::create_dir_all(&backup).map_err(|e| e.to_string())?;
        atomic_write(&backup.join("catalog.json"), original)?;
        let mut legacy = vec![];
        for project in &mut catalog.projects {
            let path = project.path.join(crate::project_manifest::FILE_NAME);
            match fs::read(&path) {
                Ok(bytes) => {
                    project.explicit_scripts = true;
                    atomic_write(&backup.join(format!("project-{}.json", project.id)), &bytes)?;
                    legacy.push((path, format!("project-{}.json", project.id)));
                }
                Err(e) if e.kind() == io::ErrorKind::NotFound => (),
                Err(e) => return Err(format!("备份旧启动清单 {}: {e}", path.display())),
            }
        }
        let catalog = self.normalize(catalog)?;
        let plan = serde_json::json!({"backup":backup,"files":legacy});
        atomic_write(
            &backup.join("plan.json"),
            &serde_json::to_vec_pretty(&plan).map_err(|e| e.to_string())?,
        )?;
        atomic_write(
            &self.root.join("config-migration.json"),
            &serde_json::to_vec(&plan).map_err(|e| e.to_string())?,
        )?;
        self.save(&catalog)?;
        self.finish_migration(&catalog)?;
        Ok(catalog)
    }
    /// The cleanup plan survives a crash between publication and legacy removal.
    /// Only byte-identical originals named by the registered projects are removed.
    fn finish_migration(&self, catalog: &Catalog) -> Result<(), String> {
        let plan_path = self.root.join("config-migration.json");
        if !plan_path.exists() {
            return Ok(());
        }
        #[derive(serde::Deserialize)]
        struct Plan {
            backup: PathBuf,
            files: Vec<(PathBuf, String)>,
        }
        let plan: Plan = serde_json::from_slice(&fs::read(&plan_path).map_err(|e| e.to_string())?)
            .map_err(|e| format!("迁移清理计划无效: {e}"))?;
        if plan.backup.parent() != Some(self.root.join("migration-backups").as_path()) {
            return Err("迁移备份位置无效".into());
        }
        for (path, archived) in &plan.files {
            if !catalog
                .projects
                .iter()
                .any(|p| p.path.join(crate::project_manifest::FILE_NAME) == *path)
                || std::path::Path::new(archived).components().count() != 1
            {
                return Err("迁移清理文件不属于已登记项目".into());
            }
            let bytes = match fs::read(path) {
                Ok(bytes) => bytes,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e.to_string()),
            };
            let backup = fs::read(plan.backup.join(archived)).map_err(|e| e.to_string())?;
            if bytes != backup {
                return Err(format!("迁移中旧清单被修改，未删除: {}", path.display()));
            }
            fs::remove_file(path)
                .map_err(|e| format!("配置已迁移，删除旧清单失败 {}: {e}", path.display()))?;
        }
        atomic_write(
            &plan.backup.join("completed.json"),
            b"{\"version\":2,\"status\":\"complete\"}",
        )?;
        fs::remove_file(plan_path).map_err(|e| e.to_string())
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
        fs::rename(&temp, path).map_err(|e| e.to_string())?;
        fs::File::open(path.parent().ok_or("配置父目录缺失")?)
            .and_then(|dir| dir.sync_all())
            .map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(temp);
    }
    result
}
