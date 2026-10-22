//! First-run sample data for the public edition. Existing catalogs are never rewritten.
use crate::{model::Catalog, profile::Profile, store::Store};
use std::{fs, path::PathBuf};

const DEMO_PROFILE: &str = include_str!("../assets/demo-workspace/profile.json");
const DEMO_SERVER: &str = include_str!("../assets/demo-workspace/server.py");

pub fn load(store: &Store) -> Result<Catalog, String> {
    if store.root.join("catalog.json").exists() {
        return store.load();
    }

    let mut profile: Profile = serde_json::from_str(DEMO_PROFILE)
        .map_err(|error| format!("公开版演示工作区模板无效: {error}"))?;
    let projects_root = store.root.join("demo-projects");
    for project in &mut profile.projects {
        let project_root = projects_root.join(&project.path);
        fs::create_dir_all(&project_root)
            .map_err(|error| format!("创建演示项目目录 {}: {error}", project_root.display()))?;
        let server = project_root.join("server.py");
        if !server.exists() {
            fs::write(&server, DEMO_SERVER)
                .map_err(|error| format!("写入演示服务 {}: {error}", server.display()))?;
        }
        project.path = PathBuf::from(project_root);
    }

    let catalog = crate::profile::merge(&store.load()?, profile)?;
    store.save(&catalog)?;
    Ok(catalog)
}
