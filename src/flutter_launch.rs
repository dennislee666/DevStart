//! Typed Flutter launch protocol. Scan reads source only; device discovery and boot happen on launch.
use serde::{Deserialize, Serialize};
use std::{
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub enum Platform {
    Ios,
    Android,
    Ohos,
    Web,
    Macos,
    Windows,
    Linux,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Debug)]
pub enum Target {
    Simulator,
    Device,
    Desktop,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct Launch {
    pub program: String,
    pub platform: Platform,
    pub target: Target,
    #[serde(default)]
    pub device: Option<String>,
    #[serde(default)]
    pub entrypoint: Option<String>,
    #[serde(default)]
    pub emulator: Option<String>,
}
impl Launch {
    pub fn label(&self) -> String {
        format!(
            "{} / {}",
            self.platform_name(),
            match self.target {
                Target::Simulator => "模拟器调试",
                Target::Device => "真机调试",
                Target::Desktop => "开发调试",
            }
        )
    }
    pub fn platform_name(&self) -> &'static str {
        match self.platform {
            Platform::Ios => "iOS",
            Platform::Android => "Android",
            Platform::Ohos => "鸿蒙",
            Platform::Web => "Web",
            Platform::Macos => "macOS",
            Platform::Windows => "Windows",
            Platform::Linux => "Linux",
        }
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.program.trim().is_empty() || self.device.as_ref().is_some_and(|s| s.is_empty()) {
            return Err("Flutter 启动配置不完整".into());
        }
        Ok(())
    }
}
#[path = "flutter_scan.rs"]
mod scan;
pub use scan::{detect, is_app, is_platform_directory};
#[derive(Deserialize, Clone)]
struct Device {
    id: String,
    name: String,
    #[serde(rename = "targetPlatform")]
    platform: String,
    #[serde(default)]
    emulator: bool,
    #[serde(default, rename = "isSupported")]
    supported: Option<bool>,
}
/// Bound both output and duration; machine output is parsed strictly, never inferred from prose.
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let mut command = Command::new(program);
    if std::env::var_os("PUB_HOSTED_URL").is_none() {
        command.env("PUB_HOSTED_URL", "https://pub.flutter-io.cn");
    }
    if std::env::var_os("FLUTTER_STORAGE_BASE_URL").is_none() {
        command.env("FLUTTER_STORAGE_BASE_URL", "https://storage.flutter-io.cn");
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{program}: {e}"))?;
    let read = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            stream
                .by_ref()
                .take(1_048_577)
                .read_to_end(&mut bytes)
                .map(|_| bytes)
        })
    };
    let out = read(Box::new(child.stdout.take().unwrap()));
    let err = read(Box::new(child.stderr.take().unwrap()));
    let end = Instant::now() + Duration::from_secs(20);
    let status = loop {
        if let Some(s) = child.try_wait().map_err(|e| e.to_string())? {
            break s;
        }
        if Instant::now() >= end {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("设备命令超时: {program} {}", args.join(" ")));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = out
        .join()
        .map_err(|_| "设备输出读取失败")?
        .map_err(|e| e.to_string())?;
    let err = err
        .join()
        .map_err(|_| "设备错误读取失败")?
        .map_err(|e| e.to_string())?;
    if out.len() > 1_048_576 || err.len() > 1_048_576 {
        return Err("设备命令输出超过上限".into());
    }
    if !status.success() {
        return Err(format!(
            "{program} {}: {} {}",
            args.join(" "),
            String::from_utf8_lossy(&out),
            String::from_utf8_lossy(&err)
        ));
    }
    String::from_utf8(out).map_err(|e| e.to_string())
}
fn devices(launch: &Launch) -> Result<Vec<Device>, String> {
    serde_json::from_str(&output(&launch.program, &["devices", "--machine"])?)
        .map_err(|e| format!("Flutter 设备返回格式无效: {e}"))
}
fn matches(launch: &Launch, d: &Device) -> bool {
    let platform = match launch.platform {
        Platform::Ios => d.platform == "ios",
        Platform::Android => d.platform.starts_with("android"),
        Platform::Ohos => d.platform.starts_with("ohos"),
        Platform::Web => d.platform == "web-javascript",
        Platform::Macos => d.platform.starts_with("darwin"),
        Platform::Windows => d.platform.starts_with("windows"),
        Platform::Linux => d.platform.starts_with("linux"),
    };
    platform
        && d.supported != Some(false)
        && match launch.target {
            Target::Simulator => d.emulator,
            Target::Device => !d.emulator,
            Target::Desktop => true,
        }
        && launch.device.as_ref().is_none_or(|id| id == &d.id)
}
fn choose(launch: &Launch, all: Vec<Device>) -> Result<Option<Device>, String> {
    let mut selected = all.into_iter().filter(|d| matches(launch, d));
    let first = selected.next();
    if let Some(second) = selected.next() {
        return Err(format!(
            "存在多个可用设备（{}、{}），请在启动配置的 device 字段指定设备 ID",
            first.as_ref().unwrap().name,
            second.name
        ));
    }
    Ok(first)
}
fn android_emulator() -> Result<PathBuf, String> {
    for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
        if let Some(root) = std::env::var_os(key) {
            let p = PathBuf::from(root).join("emulator/emulator");
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    if let Some(home) = std::env::var_os("HOME") {
        for dir in [
            "Library/Android/sdk/emulator/emulator",
            "Android/Sdk/emulator/emulator",
        ] {
            let p = PathBuf::from(&home).join(dir);
            if p.is_file() {
                return Ok(p);
            }
        }
    }
    Err("未找到 Android emulator，请配置 ANDROID_HOME 或 ANDROID_SDK_ROOT".into())
}
fn ohos_emulator() -> PathBuf {
    if let Some(path) = std::env::var_os("DEVECO_EMULATOR") {
        return path.into();
    }
    if let Some(root) = std::env::var_os("DEVECO_STUDIO_HOME") {
        let path = PathBuf::from(root).join("tools/emulator/Emulator");
        if path.is_file() {
            return path;
        }
    }
    let path = PathBuf::from("/Applications/DevEco-Studio.app/Contents/tools/emulator/Emulator");
    if path.is_file() {
        path
    } else {
        PathBuf::from("Emulator")
    }
}
struct Boot {
    completed: std::sync::Arc<std::sync::Mutex<Option<Result<(), String>>>>,
}
impl Boot {
    fn check(&self) -> Result<(), String> {
        self.completed
            .lock()
            .map_err(|e| e.to_string())?
            .clone()
            .unwrap_or(Ok(()))
    }
}
#[derive(Serialize, Deserialize)]
struct EmulatorService {
    program: PathBuf,
    args: Vec<String>,
    log: PathBuf,
}
/// Own the emulator's pipes independently of the short-lived debug launcher.
pub fn emulator_service(config: &str) -> Result<(), String> {
    let config: EmulatorService = serde_json::from_str(config).map_err(|e| e.to_string())?;
    let mut child = Command::new(&config.program)
        .args(&config.args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| e.to_string())?;
    let mut readers = vec![];
    for mut stream in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()) as Box<dyn Read + Send>,
    ] {
        let path = config.log.clone();
        readers.push(std::thread::spawn(move || -> Result<(), String> {
            let mut buffer = [0; 4096];
            loop {
                let n = stream.read(&mut buffer).map_err(|e| e.to_string())?;
                if n == 0 {
                    break;
                }
                crate::log_store::append(&path, &buffer[..n]).map_err(|e| e.to_string())?;
            }
            Ok(())
        }));
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    for reader in readers {
        reader.join().map_err(|_| "模拟器日志线程中断")??;
    }
    if !status.success() {
        let error = format!("模拟器退出: {status}");
        let _ = crate::log_store::append(&config.log, error.as_bytes());
        return Err(error);
    }
    Ok(())
}
/// SDK service and bounded log readers survive debugger replacement and shutdown.
fn start_detached(program: &Path, args: &[&str]) -> Result<Boot, String> {
    use std::os::unix::process::CommandExt;
    let directory = crate::app_identity::workspace()
        .map_err(|e| e.to_string())?
        .join("emulator-logs");
    std::fs::create_dir_all(&directory)
        .map_err(|e| format!("创建模拟器日志目录 {}: {e}", directory.display()))?;
    crate::log_store::maintain(&directory, true).map_err(|e| e.to_string())?;
    let log = directory.join(format!("run-{}.log", crate::model::unique_id()));
    crate::log_store::append(
        &log,
        format!("[dev start flutter] 启动模拟器服务 {}\n", program.display()).as_bytes(),
    )
    .map_err(|e| e.to_string())?;
    let config = EmulatorService {
        program: program.into(),
        args: args.iter().map(|s| s.to_string()).collect(),
        log: log.clone(),
    };
    let mut child = Command::new(std::env::current_exe().map_err(|e| e.to_string())?)
        .arg("--flutter-emulator-service")
        .arg(serde_json::to_string(&config).map_err(|e| e.to_string())?)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()
        .map_err(|e| e.to_string())?;
    let completed = std::sync::Arc::new(std::sync::Mutex::new(None));
    let result = completed.clone();
    std::thread::spawn(move || {
        let status = child.wait().map_err(|e| e.to_string()).and_then(|s| {
            if s.success() {
                Ok(())
            } else {
                Err(format!("模拟器启动失败 ({s})，请查看 {}", log.display()))
            }
        });
        if let Ok(mut value) = result.lock() {
            *value = Some(status);
        }
    });
    Ok(Boot { completed })
}
fn boot(launch: &Launch) -> Result<Option<Boot>, String> {
    match launch.platform {
        Platform::Ios => {
            if !cfg!(target_os = "macos") {
                return Err("iOS 模拟器调试需要 macOS 和 Xcode".into());
            }
            let data: serde_json::Value = serde_json::from_str(&output(
                "xcrun",
                &["simctl", "list", "devices", "available", "--json"],
            )?)
            .map_err(|e| e.to_string())?;
            let mut sims: Vec<_> = data["devices"]
                .as_object()
                .ok_or("Xcode 模拟器列表无效")?
                .iter()
                .filter(|(runtime, _)| runtime.contains("iOS"))
                .flat_map(|(_, v)| v.as_array().into_iter().flatten())
                .filter_map(|d| d["udid"].as_str())
                .filter(|id| launch.device.as_ref().is_none_or(|v| v == id))
                .collect();
            sims.sort();
            let id = sims
                .first()
                .ok_or("未找到可用 iOS 模拟器，请在 Xcode 安装模拟器运行时")?;
            output("xcrun", &["simctl", "boot", id])?;
            output("open", &["-a", "Simulator"])?;
        }
        Platform::Android => {
            let program = android_emulator()?;
            let list = output(&program.display().to_string(), &["-list-avds"])?;
            let mut names: Vec<_> = list
                .lines()
                .map(str::trim)
                .filter(|s| {
                    !s.is_empty()
                        && s.chars()
                            .all(|c| c.is_ascii_alphanumeric() || "_.-".contains(c))
                })
                .filter(|name| launch.emulator.as_ref().is_none_or(|wanted| wanted == name))
                .collect();
            names.sort();
            let name = names
                .first()
                .ok_or("没有可用 Android AVD，请在 Android Studio 创建模拟器")?;
            return start_detached(&program, &["-avd", name]).map(Some);
        }
        Platform::Ohos => {
            let program = ohos_emulator();
            let list = output(&program.display().to_string(), &["-list", "-details"])?;
            let instances: serde_json::Value = serde_json::from_str(&list)
                .map_err(|e| format!("DevEco 模拟器列表格式无效: {e}"))?;
            let mut names: Vec<_> = instances
                .as_array()
                .ok_or("DevEco 模拟器列表不是数组")?
                .iter()
                .filter_map(|item| item["name"].as_str())
                .filter(|name| launch.emulator.as_ref().is_none_or(|wanted| wanted == name))
                .collect();
            names.sort();
            let name = names
                .first()
                .ok_or("没有可用鸿蒙模拟器，请在 DevEco Studio 创建模拟器并完成许可确认")?;
            return start_detached(&program, &["-start", name]).map(Some);
        }
        _ => return Err("该平台没有移动模拟器".into()),
    }
    Ok(None)
}
/// Internal CLI runner inherits the login-shell environment and tracked process group.
pub fn run(mut launch: Launch, extra: &[String]) -> Result<(), String> {
    launch.validate()?;
    if launch.platform == Platform::Ohos
        && let Some(root) = std::env::var_os("FLUTTER_OHOS_ROOT")
    {
        launch.program = PathBuf::from(root)
            .join("bin/flutter")
            .display()
            .to_string();
        eprintln!("[dev start flutter] 使用 FLUTTER_OHOS_ROOT 中的鸿蒙 Flutter SDK");
    }
    if launch.platform == Platform::Ios && !cfg!(target_os = "macos") {
        return Err("iOS 开发调试需要 macOS 和 Xcode".into());
    }
    if launch.platform == Platform::Ohos
        && !output(&launch.program, &["build", "--help"])?
            .split_whitespace()
            .any(|word| word == "hap")
    {
        return Err("当前 Flutter SDK 不支持鸿蒙 hap，请配置 FLUTTER_OHOS_ROOT 或在启动配置中指定鸿蒙适配版 SDK".into());
    }
    let mut selected = choose(&launch, devices(&launch)?)?;
    if selected.is_none() && launch.target == Target::Simulator {
        eprintln!(
            "[dev start flutter] 正在启动 {} 模拟器…",
            launch.platform_name()
        );
        let boot = boot(&launch)?;
        let end = Instant::now() + Duration::from_secs(120);
        while Instant::now() < end {
            if let Some(boot) = &boot {
                boot.check()?;
            }
            selected = choose(&launch, devices(&launch)?)?;
            if selected.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }
    let selected = selected.ok_or_else(|| {
        format!(
            "未发现可用的 {} 设备；请连接并授权真机或启动模拟器。鸿蒙需要适配版 Flutter SDK。",
            launch.platform_name()
        )
    })?;
    eprintln!(
        "[dev start flutter] 使用设备 {} ({})",
        selected.name, selected.id
    );
    let mut cmd = Command::new(&launch.program);
    if std::env::var_os("PUB_HOSTED_URL").is_none() {
        cmd.env("PUB_HOSTED_URL", "https://pub.flutter-io.cn");
    }
    if std::env::var_os("FLUTTER_STORAGE_BASE_URL").is_none() {
        cmd.env("FLUTTER_STORAGE_BASE_URL", "https://storage.flutter-io.cn");
    }
    cmd.args(["run", "--debug", "-d", &selected.id]);
    if let Some(entry) = launch.entrypoint {
        cmd.args(["-t", &entry]);
    }
    cmd.args(extra);
    use std::os::unix::process::CommandExt;
    Err(format!("Flutter 启动失败: {}", cmd.exec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn launch(target: Target) -> Launch {
        Launch {
            program: "flutter".into(),
            platform: Platform::Android,
            target,
            device: None,
            entrypoint: None,
            emulator: None,
        }
    }
    fn device(id: &str, emulator: bool) -> Device {
        Device {
            id: id.into(),
            name: id.into(),
            platform: "android-arm64".into(),
            emulator,
            supported: Some(true),
        }
    }
    #[test]
    fn device_roles_and_ambiguity_are_explicit() {
        let all = vec![device("phone", false), device("emulator-5554", true)];
        assert_eq!(
            choose(&launch(Target::Device), all.clone())
                .unwrap()
                .unwrap()
                .id,
            "phone"
        );
        assert_eq!(
            choose(&launch(Target::Simulator), all).unwrap().unwrap().id,
            "emulator-5554"
        );
        assert!(
            choose(
                &launch(Target::Device),
                vec![device("one", false), device("two", false)]
            )
            .is_err()
        );
        let mut explicit = launch(Target::Device);
        explicit.device = Some("two".into());
        assert_eq!(
            choose(&explicit, vec![device("one", false), device("two", false)])
                .unwrap()
                .unwrap()
                .id,
            "two"
        );
    }
}
