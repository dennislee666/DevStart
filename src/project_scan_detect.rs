//! Deterministic launch adapters: existing package scripts and explicit framework/tool entry points.
use crate::model::{Entry, Script};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};
pub(super) fn read(path: &Path) -> Result<String, String> {
    let meta = std::fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    if !meta.is_file() || meta.len() > 2_000_000 {
        return Err(format!(
            "Not a regular text source or exceeds 2 MB: {}",
            path.display()
        ));
    }
    std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
}
fn json(root: &Path, name: &str) -> Result<serde_json::Value, String> {
    serde_json::from_str(&read(&root.join(name))?).map_err(|e| e.to_string())
}
fn script(root: &Path, file: PathBuf, name: &str, entry: Entry, args: &[&str]) -> Script {
    Script {
        id: 0,
        display_name: None,
        description_override: None,
        enabled: true,
        display_command: None,
        name: name.into(),
        file,
        cwd: root.into(),
        entry,
        args: args.iter().map(|s| (*s).into()).collect(),
        description: "Detected from existing project source".into(),
        favorite: false,
        environment: BTreeMap::new(),
        expected_port: None,
        custom: true,
        resource_key: None,
    }
}
fn command(root: &Path, file: &str, name: &str, program: &str, args: &[&str]) -> Script {
    script(
        root,
        root.join(file),
        name,
        Entry::Command {
            program: program.into(),
        },
        args,
    )
}
fn value<'a>(source: &'a str, key: &str) -> Option<&'a str> {
    source
        .lines()
        .filter_map(|line| line.trim().split_once('='))
        .find(|(k, _)| k.trim() == key)
        .map(|(_, v)| v.trim().trim_matches(['\'', '"']))
}
fn xml(source: &str, key: &str) -> Option<String> {
    source
        .split_once(&format!("<{key}>"))?
        .1
        .split_once(&format!("</{key}>"))
        .map(|(v, _)| v.trim().to_owned())
}
pub(super) fn project(root: &Path) -> Result<(String, String, Vec<Script>), String> {
    let mut name = root
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let mut stacks = vec![];
    let mut scripts = crate::discovery::discover(root)?;
    if root.join("package.json").is_file() {
        let data = json(root, "package.json")?;
        if let Some(n) = data["name"].as_str() {
            name = n.chars().take(100).collect();
        }
        let deps = format!("{} {}", data["dependencies"], data["devDependencies"]);
        stacks.push(if deps.contains("@dcloudio/") {
            "UniApp"
        } else if deps.contains("\"next\"") {
            "Next.js"
        } else if deps.contains("\"vue\"") {
            "Vue"
        } else {
            "Node.js"
        });
        for item in scripts
            .iter_mut()
            .filter(|s| s.file == root.join("package.json"))
        {
            if let Entry::Package { name, .. } = &item.entry {
                item.name = name.clone();
            }
        }
        if !scripts.iter().any(|s| s.file == root.join("package.json"))
            && let Some(main) = data["main"].as_str()
        {
            let file = root.join(main);
            if file.is_file()
                && file
                    .canonicalize()
                    .map_err(|e| e.to_string())?
                    .starts_with(root)
            {
                scripts.push(script(
                    root,
                    file,
                    "Node.js",
                    Entry::File {
                        interpreter: "node".into(),
                    },
                    &[],
                ));
            }
        }
    }
    if root.join("pyproject.toml").is_file() {
        stacks.push("Python");
        let text = read(&root.join("pyproject.toml"))?;
        let mut section = "";
        for line in text.lines().map(str::trim) {
            if line.starts_with('[') {
                section = line;
                continue;
            }
            if section == "[project]"
                && let Some(v) = value(line, "name")
            {
                name = v.chars().take(100).collect();
            }
            if ["[project.scripts]", "[tool.poetry.scripts]"].contains(&section)
                && let Some((key, _)) = line.split_once('=')
            {
                let key = key.trim().trim_matches(['\'', '"']);
                if !key.is_empty()
                    && key
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || "_-".contains(c))
                {
                    scripts.push(command(root, "pyproject.toml", key, key, &[]));
                }
            }
        }
    }
    if root.join("requirements.txt").is_file() {
        stacks.push("Python");
    }
    let python = [root.join(".venv/bin/python"), root.join("venv/bin/python")]
        .into_iter()
        .find(|p| p.is_file())
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "python3".into());
    if root.join("manage.py").is_file() {
        stacks.push("Django");
        scripts.push(script(
            root,
            root.join("manage.py"),
            "Django",
            Entry::File {
                interpreter: python.clone(),
            },
            &["runserver"],
        ));
    }
    for file in ["app.py", "main.py", "serve.py"] {
        if !root.join(file).is_file() {
            continue;
        }
        let source = read(&root.join(file))?;
        if source
            .lines()
            .filter_map(|line| line.split_once('='))
            .any(|(lhs, rhs)| lhs.trim() == "app" && rhs.trim_start().starts_with("FastAPI("))
        {
            stacks.push("FastAPI");
            let target = format!("{}:app", file.trim_end_matches(".py"));
            scripts.push(command(
                root,
                file,
                "FastAPI",
                &python,
                &["-m", "uvicorn", &target],
            ));
        } else if source.contains("Flask(") && !source.contains("__main__") {
            stacks.push("Flask");
            scripts.push(command(
                root,
                file,
                "Flask",
                &python,
                &["-m", "flask", "--app", file, "run"],
            ));
        } else if source.contains("__main__") && !scripts.iter().any(|s| s.file == root.join(file))
        {
            stacks.push("Python");
            scripts.push(script(
                root,
                root.join(file),
                file,
                Entry::File {
                    interpreter: python.clone(),
                },
                &[],
            ));
        }
    }
    if root.join("composer.json").is_file() {
        stacks.push("PHP");
        let data = json(root, "composer.json")?;
        if let Some(n) = data["name"].as_str() {
            name = n.chars().take(100).collect();
        }
        if let Some(items) = data["scripts"].as_object() {
            for key in items.keys().filter(|n| {
                ["dev", "start", "serve", "watch"]
                    .iter()
                    .any(|prefix| n.as_str() == *prefix || n.starts_with(&format!("{prefix}:")))
            }) {
                scripts.push(command(
                    root,
                    "composer.json",
                    key,
                    "composer",
                    &["run-script", key],
                ));
            }
        }
    }
    if root.join("artisan").is_file() {
        stacks.push("Laravel");
        scripts.push(script(
            root,
            root.join("artisan"),
            "Laravel",
            Entry::File {
                interpreter: "php".into(),
            },
            &["serve"],
        ));
    }
    if root.join("pom.xml").is_file() {
        stacks.push("Java / Maven");
        let text = read(&root.join("pom.xml"))?;
        if let Some(n) = xml(&text, "name")
            && !n.contains("${")
        {
            name = n.chars().take(100).collect();
        }
        let program: String = if root.join("mvnw").is_file() {
            "./mvnw".into()
        } else {
            "mvn".into()
        };
        if text.contains("spring-boot-maven-plugin") {
            scripts.push(command(
                root,
                "pom.xml",
                "Spring Boot",
                &program,
                &["spring-boot:run"],
            ));
        }
        if text.contains("exec-maven-plugin") && text.contains("<mainClass>") {
            scripts.push(command(
                root,
                "pom.xml",
                "Java application",
                &program,
                &["exec:java"],
            ));
        }
    }
    for file in ["build.gradle", "build.gradle.kts"] {
        if !root.join(file).is_file() {
            continue;
        }
        stacks.push("Java / Gradle");
        let text = read(&root.join(file))?;
        let program: String = if root.join("gradlew").is_file() {
            "./gradlew".into()
        } else {
            "gradle".into()
        };
        if text.contains("org.springframework.boot") {
            scripts.push(command(root, file, "Spring Boot", &program, &["bootRun"]));
        } else if text.contains("application") {
            scripts.push(command(root, file, "Java application", &program, &["run"]));
        }
    }
    if root.join("Cargo.toml").is_file() && root.join("src/main.rs").is_file() {
        stacks.push("Rust");
        scripts.push(command(root, "Cargo.toml", "Rust", "cargo", &["run"]));
    }
    if root.join(".vscode/launch.json").is_file() {
        stacks.push("VS Code");
        scripts.extend(ide_launches(root)?);
    }
    let java_port = if stacks.iter().any(|s| s.starts_with("Java")) {
        spring_port(root)?
    } else {
        None
    };
    for item in &mut scripts {
        if item.cwd == root
            && java_port.is_some()
            && matches!(&item.entry,Entry::Command{program} if program.ends_with("mvn")||program.ends_with("mvnw")||program.ends_with("gradle")||program.ends_with("gradlew"))
        {
            item.expected_port = java_port;
        }
    }
    stacks.sort();
    stacks.dedup();
    let stack = if stacks.is_empty() {
        "Scripts".into()
    } else {
        stacks.join(" / ")
    };
    Ok((name, stack, scripts))
}
fn spring_port(root: &Path) -> Result<Option<u16>, String> {
    let mut ports = std::collections::HashSet::new();
    for base in [root.to_path_buf(), root.join("src/main/resources")] {
        let properties = base.join("application.properties");
        if properties.is_file()
            && let Some(v) = value(&read(&properties)?, "server.port")
            && let Ok(p) = v.parse::<u16>()
            && p > 0
        {
            ports.insert(p);
        }
        for name in ["application.yml", "application.yaml"] {
            let path = base.join(name);
            if !path.is_file() {
                continue;
            }
            let text = read(&path)?;
            let mut server = false;
            for line in text.lines().filter(|l| !l.trim().starts_with('#')) {
                if !line.starts_with(char::is_whitespace) {
                    server = line.trim() == "server:";
                }
                if server
                    && let Some(v) = line.trim().strip_prefix("port:")
                    && let Ok(p) = v.trim().parse::<u16>()
                    && p > 0
                {
                    ports.insert(p);
                }
            }
        }
    }
    Ok(if ports.len() == 1 {
        ports.into_iter().next()
    } else {
        None
    })
}

/// JSONC comments/trailing commas are removed only outside strings; configuration is never evaluated.
fn jsonc(source: &str) -> Result<serde_json::Value, String> {
    let mut out = String::new();
    let mut chars = source.chars().peekable();
    let (mut quoted, mut escaped) = (false, false);
    while let Some(c) = chars.next() {
        if quoted {
            out.push(c);
            if escaped {
                escaped = false;
            } else if c == '\\' {
                escaped = true;
            } else if c == '"' {
                quoted = false;
            }
            continue;
        }
        if c == '"' {
            quoted = true;
            out.push(c);
            continue;
        }
        if c == '/' && chars.peek() == Some(&'/') {
            chars.next();
            for v in chars.by_ref() {
                if v == '\n' {
                    out.push(v);
                    break;
                }
            }
            continue;
        }
        if c == '/' && chars.peek() == Some(&'*') {
            chars.next();
            let mut closed = false;
            while let Some(v) = chars.next() {
                if v == '*' && chars.peek() == Some(&'/') {
                    chars.next();
                    closed = true;
                    break;
                }
            }
            if !closed {
                return Err("Unterminated JSONC comment".into());
            }
            out.push(' ');
            continue;
        }
        if c == ','
            && chars
                .clone()
                .find(|c| !c.is_whitespace())
                .is_some_and(|c| c == '}' || c == ']')
        {
            continue;
        }
        out.push(c);
    }
    serde_json::from_str(&out).map_err(|e| format!("VS Code launch.json: {e}"))
}
fn ide_launches(root: &Path) -> Result<Vec<Script>, String> {
    let source = root.join(".vscode/launch.json");
    if !source
        .canonicalize()
        .map_err(|e| e.to_string())?
        .starts_with(root)
    {
        return Err("IDE configuration escapes project directory".into());
    }
    let data = jsonc(&read(&source)?)?;
    let mut scripts = vec![];
    let expand = |value: &str| -> Option<String> {
        let value = value.replace("${workspaceFolder}", &root.display().to_string());
        if value.contains("${") {
            None
        } else {
            Some(value)
        }
    };
    for config in data["configurations"].as_array().into_iter().flatten() {
        if config["request"].as_str() != Some("launch") {
            continue;
        }
        let name = config["name"].as_str().unwrap_or("Launch");
        let arguments = match &config["args"] {
            serde_json::Value::Array(items) => items
                .iter()
                .map(|v| v.as_str().and_then(&expand))
                .collect::<Option<Vec<_>>>(),
            serde_json::Value::String(s) => crate::configured_ports::words(s)
                .and_then(|v| v.iter().map(|s| expand(s)).collect()),
            serde_json::Value::Null => Some(vec![]),
            _ => None,
        };
        let Some(args) = arguments else {
            continue;
        };
        let mut launch = match config["type"].as_str() {
            Some("python" | "debugpy" | "node" | "pwa-node") => {
                let python = matches!(config["type"].as_str(), Some("python" | "debugpy"));
                let interpreter = config[if python {
                    "python"
                } else {
                    "runtimeExecutable"
                }]
                .as_str()
                .unwrap_or(if python { "python3" } else { "node" });
                let Some(interpreter) = expand(interpreter) else {
                    continue;
                };
                if let Some(program) = config["program"].as_str().and_then(&expand) {
                    let path = if Path::new(&program).is_absolute() {
                        PathBuf::from(program)
                    } else {
                        root.join(program)
                    };
                    if !path.is_file()
                        || !path
                            .canonicalize()
                            .map_err(|e| e.to_string())?
                            .starts_with(root)
                    {
                        continue;
                    }
                    script(root, path, name, Entry::File { interpreter }, &[])
                } else if python && let Some(module) = config["module"].as_str().and_then(&expand) {
                    command(
                        root,
                        ".vscode/launch.json",
                        name,
                        &interpreter,
                        &["-m", &module],
                    )
                } else {
                    continue;
                }
            }
            Some("java") => {
                let Some(main_class) = config["mainClass"].as_str().and_then(&expand) else {
                    continue;
                };
                let Some(paths) = config["classPaths"].as_array() else {
                    continue;
                };
                let Some(paths) = paths
                    .iter()
                    .map(|v| {
                        v.as_str().and_then(&expand).map(|p| {
                            if Path::new(&p).is_absolute() {
                                PathBuf::from(p)
                            } else {
                                root.join(p)
                            }
                        })
                    })
                    .collect::<Option<Vec<_>>>()
                else {
                    continue;
                };
                if paths.is_empty() {
                    continue;
                }
                let vm_args = config["vmArgs"]
                    .as_str()
                    .and_then(crate::configured_ports::words)
                    .unwrap_or_default();
                script(
                    root,
                    source.clone(),
                    name,
                    Entry::Java {
                        configuration: name.into(),
                        program: config["javaExec"].as_str().unwrap_or("java").into(),
                        main_class,
                        vm_args,
                        class_paths: paths,
                        enabled: true,
                        snapshot: String::new(),
                        manifest: PathBuf::new(),
                        classpath_file: None,
                    },
                    &[],
                )
            }
            _ => continue,
        };
        launch.args.extend(args);
        if let Some(cwd) = config["cwd"].as_str().and_then(&expand) {
            let cwd = if Path::new(&cwd).is_absolute() {
                PathBuf::from(cwd)
            } else {
                root.join(cwd)
            };
            let cwd = cwd.canonicalize().map_err(|e| e.to_string())?;
            if !cwd.starts_with(root) {
                continue;
            }
            launch.cwd = cwd;
        }
        if let Some(env) = config["env"].as_object() {
            for (k, v) in env {
                if let Some(v) = v.as_str().and_then(&expand) {
                    launch.environment.insert(k.clone(), v);
                }
            }
        }
        scripts.push(launch);
    }
    Ok(scripts)
}
