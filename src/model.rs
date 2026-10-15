use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: u64,
    pub name: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub enum Entry {
    File {
        interpreter: String,
    },
    Package {
        manager: String,
        name: String,
    },
    Command {
        program: String,
    },
    Java {
        #[serde(default)]
        configuration: String,
        program: String,
        main_class: String,
        vm_args: Vec<String>,
        class_paths: Vec<PathBuf>,
        enabled: bool,
        #[serde(default)]
        snapshot: String,
        #[serde(default)]
        manifest: PathBuf,
        #[serde(default)]
        classpath_file: Option<PathBuf>,
    },
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Script {
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub description_override: Option<String>,
    #[serde(default = "enabled_default")]
    pub enabled: bool,
    #[serde(default)]
    pub display_command: Option<String>,
    pub id: u64,
    pub name: String,
    pub file: PathBuf,
    pub cwd: PathBuf,
    pub entry: Entry,
    pub description: String,
    pub favorite: bool,
    #[serde(default)]
    pub environment: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    pub expected_port: Option<u16>,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub custom: bool,
    #[serde(default)]
    pub resource_key: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Project {
    #[serde(default)]
    pub description: String,
    pub id: u64,
    pub name: String,
    pub path: PathBuf,
    pub group: u64,
    pub favorite: bool,
    pub scripts: Vec<Script>,
    #[serde(default)]
    pub explicit_scripts: bool,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Catalog {
    #[serde(default)]
    pub preferences: crate::preferences::Preferences,
    pub version: u32,
    pub next_id: u64,
    pub groups: Vec<Group>,
    pub projects: Vec<Project>,
}
impl Default for Catalog {
    fn default() -> Self {
        Self {
            preferences: Default::default(),
            version: 1,
            next_id: 2,
            groups: vec![Group {
                id: 1,
                name: "默认分组".into(),
            }],
            projects: vec![],
        }
    }
}
impl Catalog {
    pub fn id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}
pub fn enabled_default() -> bool {
    true
}
impl Script {
    pub fn display_label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.name)
    }
    /// Presentation metadata never participates in executable identity or argv.
    pub fn validate_display_name(value: Option<String>) -> Result<Option<String>, String> {
        use unicode_segmentation::UnicodeSegmentation;
        let Some(value) = value else { return Ok(None) };
        let value = value.trim();
        if value.is_empty()
            || value.chars().any(char::is_control)
            || value.graphemes(true).count() > 50
        {
            return Err("显示名需要 1–50 个字符，且不能包含换行或控制字符".into());
        }
        Ok(Some(value.to_owned()))
    }

    pub fn launch_block_reason(&self) -> Option<&str> {
        if !self.enabled {
            return Some("启动项未启用");
        }
        match &self.entry {
            Entry::Java { enabled: false, .. } => Some("仅导入此配置，尚未启用执行"),
            _ => None,
        }
    }
    pub fn source_identity(&self) -> String {
        match &self.entry {
            Entry::File { .. } => self.file.display().to_string(),
            Entry::Package { name, .. } => format!("{}:{name}", self.file.display()),
            Entry::Command { program } => format!("{}:{program}", self.file.display()),
            Entry::Java { configuration, .. } => {
                format!("{}:java:{configuration}", self.file.display())
            }
        }
    }
    pub fn identity(&self) -> String {
        // All fields are strings or string collections; serialization cannot contain non-finite values.
        serde_json::to_string(&(
            &self.file,
            &self.entry,
            &self.cwd,
            &self.args,
            &self.environment,
        ))
        .expect("serializable launch identity")
    }
    pub fn invocation(&self) -> String {
        let base = match &self.entry {
            Entry::File { interpreter } => {
                format!("{interpreter} {}", quote(&self.file.display().to_string()))
            }
            Entry::Package { manager, name } => format!("{} run {}", quote(manager), quote(name)),
            Entry::Command { program } => quote(program),
            Entry::Java {
                program,
                main_class,
                vm_args,
                class_paths,
                ..
            } => format!(
                "{}{} -cp {} {}",
                quote(program),
                format_args(vm_args),
                quote(
                    &std::env::join_paths(class_paths)
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_default()
                ),
                quote(main_class)
            ),
        };
        let separator = if matches!(self.entry, Entry::Package { .. }) && !self.args.is_empty() {
            " --"
        } else {
            ""
        };
        format!("{base}{separator}{}", format_args(&self.args))
    }
    pub fn tab_command(&self) -> String {
        if let Some(command) = &self.display_command {
            return command.clone();
        }
        match &self.entry {
            Entry::File { .. } => format!(
                "{}{}",
                self.file
                    .strip_prefix(&self.cwd)
                    .unwrap_or(&self.file)
                    .display(),
                format_args(&self.args)
            ),
            Entry::Java {
                program,
                main_class,
                ..
            } => format!(
                "{} {}{}",
                std::path::Path::new(program)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy(),
                main_class,
                format_args(&self.args)
            ),
            _ => self.invocation(),
        }
    }
}
fn format_args(args: &[String]) -> String {
    args.iter().map(|arg| format!(" {}", quote(arg))).collect()
}
fn quote(value: &str) -> String {
    if !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_alphanumeric() || "_./:-=".contains(c))
    {
        value.into()
    } else {
        format!("'{}'", value.replace('\'', "'\"'\"'"))
    }
}

/// Monotonic process-local identifiers remain unique even at coarse clock resolution.
pub fn unique_id() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64;
    let mut previous = LAST.load(Ordering::Relaxed);
    loop {
        let next = now.max(previous.saturating_add(1));
        match LAST.compare_exchange_weak(previous, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(observed) => previous = observed,
        }
    }
}
