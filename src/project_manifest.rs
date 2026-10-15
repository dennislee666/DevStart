//! Project-owned launch definitions. No IDE configuration or build task is read.
use crate::{
    model::{Entry, Script},
    profile::ProfileScript,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
pub const FILE_NAME: &str = ".dev-start.json";
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    version: u32,
    launches: Vec<ProfileScript>,
}
fn read(path: &Path) -> Result<Manifest, String> {
    let data =
        std::fs::read(path).map_err(|e| format!("读取项目启动清单 {}: {e}", path.display()))?;
    if data.len() > 2_000_000 {
        return Err("项目启动清单超过2MiB".into());
    }
    let m: Manifest =
        serde_json::from_slice(&data).map_err(|e| format!("{}: {e}", path.display()))?;
    if m.version != 1 {
        return Err(format!("不支持的项目清单版本 {}", m.version));
    }
    Ok(m)
}
fn resolve(root: &Path, path: &Path) -> Result<PathBuf, String> {
    let p = if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    };
    p.canonicalize()
        .map_err(|e| format!("项目入口路径 {}: {e}", p.display()))
}
pub fn discover(root: &Path) -> Result<Vec<Script>, String> {
    let manifest = resolve(root, Path::new(FILE_NAME))?;
    let mut result = vec![];
    let mut names = HashSet::new();
    for spec in read(&manifest)?.launches {
        if !names.insert(spec.name.clone()) {
            return Err(format!("项目启动项名称重复: {}", spec.name));
        }
        let snapshot = serde_json::to_string(&spec).map_err(|e| e.to_string())?;
        let file = resolve(root, &spec.file)?;
        let cwd = resolve(root, spec.cwd.as_deref().unwrap_or(Path::new(".")))?;
        let mut entry = spec.entry;
        if let Entry::Java {
            configuration,
            program,
            main_class,
            vm_args,
            class_paths,
            classpath_file,
            manifest: origin,
            snapshot: stored,
            ..
        } = &mut entry
        {
            *configuration = spec.name.clone();
            *stored = snapshot;
            *origin = manifest.clone();
            if main_class.is_empty()
                || main_class.starts_with('-')
                || main_class.contains('/')
                || main_class.chars().any(char::is_whitespace)
            {
                return Err("Java主类必须为完整类名".into());
            }
            if program.is_empty() {
                return Err("Java执行程序为空".into());
            }
            if vm_args.iter().any(|a| {
                !a.starts_with("-D")
                    && !a.starts_with("-X")
                    && ![
                        "-ea",
                        "-da",
                        "-esa",
                        "-dsa",
                        "-server",
                        "-client",
                        "--enable-preview",
                        "-verbose",
                    ]
                    .contains(&a.as_str())
            }) {
                return Err("包含未支持的JVM参数，拒绝替换主类或classpath".into());
            }
            for p in class_paths.iter_mut() {
                *p = resolve(root, p)?;
            }
            if let Some(file) = classpath_file {
                *file = resolve(root, file)?;
                let raw = std::fs::read_to_string(&*file).map_err(|e| e.to_string())?;
                if raw.len() > 2_000_000 {
                    return Err("classpath文件超过2MiB".into());
                }
                for p in std::env::split_paths(raw.trim()) {
                    class_paths.push(resolve(root, &p)?);
                }
            }
            if class_paths.is_empty() {
                return Err("Java classpath为空，不会自动构建".into());
            }
        }
        let script = Script {
            display_name: None,
            description_override: None,
            enabled: spec.enabled,
            display_command: spec.display_command,
            id: 0,
            name: spec.name,
            file,
            cwd,
            entry,
            description: spec.description,
            favorite: false,
            environment: spec.environment,
            expected_port: spec.expected_port,
            args: spec.args,
            custom: false,
            resource_key: spec.resource_key,
        };
        crate::profile::validate(&script)?;
        result.push(script);
    }
    Ok(result)
}
pub fn validate_launch(script: &Script) -> Result<(), String> {
    if let Some(reason) = script.launch_block_reason() {
        return Err(reason.to_string());
    }
    if let Entry::Java {
        configuration,
        main_class,
        class_paths,
        manifest,
        snapshot,
        ..
    } = &script.entry
    {
        let matching: Vec<_> = read(manifest)?
            .launches
            .into_iter()
            .filter(|s| s.name == *configuration)
            .collect();
        // Decode snapshots before comparison so newly defaulted presentation fields do not
        // falsely invalidate an unchanged launch definition after an application upgrade.
        let stored: ProfileScript =
            serde_json::from_str(snapshot).map_err(|e| format!("启动快照无效，请重新读取: {e}"))?;
        if matching.len() != 1
            || serde_json::to_value(&matching[0]).map_err(|e| e.to_string())?
                != serde_json::to_value(&stored).map_err(|e| e.to_string())?
        {
            return Err("项目启动清单已变化，请重新读取后再启动".into());
        }
        if class_paths.iter().any(|p| !p.exists()) {
            return Err("Java依赖缺失，不会自动构建".into());
        }
        let class = main_class.replace('.', "/") + ".class";
        if !class_paths
            .iter()
            .any(|p| p.is_dir() && p.join(&class).is_file())
        {
            return Err(format!("缺少已编译主类 {main_class}，不会自动构建"));
        }
    }
    Ok(())
}

/// Read editable definitions, preserving relative source paths and classpath-file references.
pub fn specifications(project: &crate::model::Project) -> Result<Vec<ProfileScript>, String> {
    let path = project.path.join(FILE_NAME);
    let existing = if path.exists() {
        read(&path)?.launches
    } else {
        vec![]
    };
    project
        .scripts
        .iter()
        .map(|script| {
            if let Some(spec) = existing.iter().find(|s| s.name == script.name) {
                return Ok(spec.clone());
            }
            let mut entry = script.entry.clone();
            if let Entry::Java { classpath_file, .. } = &mut entry {
                // Resolved class_paths already contain classpath_file entries.
                *classpath_file = None;
            }
            Ok(ProfileScript {
                enabled: script.enabled,
                display_command: script.display_command.clone(),
                name: script.name.clone(),
                file: script.file.clone(),
                entry,
                description: script.description.clone(),
                cwd: Some(script.cwd.clone()),
                environment: script.environment.clone(),
                expected_port: script.expected_port,
                args: script.args.clone(),
                resource_key: script.resource_key.clone(),
            })
        })
        .collect()
}
pub fn encode(specs: Vec<ProfileScript>) -> Result<Vec<u8>, String> {
    let bytes = serde_json::to_vec_pretty(&Manifest {
        version: 1,
        launches: specs,
    })
    .map_err(|e| e.to_string())?;
    if bytes.len() > 2_000_000 {
        return Err("项目启动清单超过2MiB".into());
    }
    Ok(bytes)
}
