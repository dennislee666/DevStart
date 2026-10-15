use crate::model::{Entry, Script};
use std::{fs, path::Path};

/// Discovery only reads files. Importing a project never executes its code.
pub fn discover(root: &Path) -> Result<Vec<Script>, String> {
    if root.join(crate::project_manifest::FILE_NAME).exists() {
        return crate::project_manifest::discover(root);
    }
    let mut result = vec![];
    let mut budget = 5000;
    scan(root, root, 0, &mut budget, &mut result)?;
    result.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(result)
}
fn scan(
    root: &Path,
    dir: &Path,
    depth: u8,
    budget: &mut usize,
    result: &mut Vec<Script>,
) -> Result<(), String> {
    for entry in fs::read_dir(dir).map_err(|e| format!("扫描 {}: {e}", dir.display()))? {
        if *budget == 0 {
            return Err("目录超过 5000 个条目，请导入更具体的项目目录".into());
        }
        *budget -= 1;
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        let ty = entry.file_type().map_err(|e| e.to_string())?;
        if ty.is_symlink() {
            continue;
        }
        if ty.is_dir() {
            if depth < 3
                && !name.starts_with('.')
                && ![
                    "node_modules",
                    "target",
                    "dist",
                    "build",
                    "vendor",
                    "Pods",
                    "venv",
                    "__pycache__",
                ]
                .contains(&name.as_str())
            {
                scan(root, &path, depth + 1, budget, result)?;
            }
        } else if ty.is_file() {
            if name == "package.json" {
                package(root, &path, result)?;
            }
            let interpreter = match path.extension().and_then(|x| x.to_str()) {
                Some("sh" | "command") => Some("/bin/bash"),
                Some("py") => Some("python3"),
                _ => None,
            };
            if let Some(interpreter) = interpreter
                && path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .is_some_and(is_launch_name)
            {
                let source = read_small(&path)?;
                let description = source
                    .lines()
                    .take(20)
                    .filter_map(|l| l.trim().strip_prefix('#'))
                    .map(str::trim)
                    .find(|l| !l.is_empty() && !l.starts_with('!'))
                    .unwrap_or("执行项目原有脚本")
                    .to_string();
                result.push(Script {
                    display_name: None,
                    description_override: None,
                    enabled: true,
                    display_command: None,
                    id: 0,
                    name: path
                        .strip_prefix(root)
                        .unwrap_or(&path)
                        .display()
                        .to_string(),
                    file: path.clone(),
                    cwd: root.to_path_buf(),
                    entry: Entry::File {
                        interpreter: source
                            .lines()
                            .next()
                            .and_then(|line| line.strip_prefix("#!"))
                            .map(str::trim)
                            .filter(|line| !line.is_empty())
                            .unwrap_or(interpreter)
                            .into(),
                    },
                    description,
                    favorite: false,
                    environment: Default::default(),
                    expected_port: None,
                    args: vec![],
                    custom: false,
                    resource_key: None,
                });
            }
        }
    }
    Ok(())
}
fn read_small(path: &Path) -> Result<String, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 2_000_000 {
        return Err(format!("脚本或清单超过 2 MB: {}", path.display()));
    }
    fs::read_to_string(path).map_err(|e| format!("读取 {}: {e}", path.display()))
}
fn package(root: &Path, path: &Path, result: &mut Vec<Script>) -> Result<(), String> {
    let json: serde_json::Value =
        serde_json::from_str(&read_small(path)?).map_err(|e| format!("{}: {e}", path.display()))?;
    let cwd = path.parent().ok_or("package.json 缺少父目录")?;
    let manager = if cwd.join("pnpm-lock.yaml").exists() {
        "pnpm"
    } else if cwd.join("yarn.lock").exists() {
        "yarn"
    } else {
        "npm"
    };
    if let Some(scripts) = json.get("scripts").and_then(|v| v.as_object()) {
        for (name, value) in scripts {
            if let Some(command) = value.as_str()
                && is_launch_name(name)
            {
                result.push(Script {
                    display_name: None,
                    description_override: None,
                    enabled: true,
                    display_command: None,
                    id: 0,
                    name: format!(
                        "{} · {name}",
                        cwd.strip_prefix(root).unwrap_or(cwd).display()
                    ),
                    file: path.to_path_buf(),
                    cwd: cwd.to_path_buf(),
                    entry: Entry::Package {
                        manager: manager.into(),
                        name: name.clone(),
                    },
                    description: command.into(),
                    favorite: false,
                    environment: Default::default(),
                    expected_port: None,
                    args: vec![],
                    custom: false,
                    resource_key: None,
                });
            }
        }
    }
    Ok(())
}

/// Explicit fallback naming protocol, not a guess about the source code's behavior.
/// Other tasks require a project-owned manifest or a manually selected entry.
fn is_launch_name(name: &str) -> bool {
    let normalized = name.to_ascii_lowercase();
    let parts: Vec<_> = normalized.split([':', '-', '_']).collect();
    matches!(parts.first(), Some(&"dev" | &"start" | &"serve" | &"watch"))
        && !parts.iter().any(|p| {
            [
                "build", "deploy", "upload", "publish", "release", "test", "lint", "format",
                "install",
            ]
            .contains(p)
        })
}
