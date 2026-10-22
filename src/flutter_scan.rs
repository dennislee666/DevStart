//! Read-only Flutter source discovery; no SDK commands or dependency installation.
use super::*;
/// Conventional application marker; Dart packages without an app entry are not launch targets.
pub fn is_platform_directory(root: &Path, name: &str) -> bool {
    is_app(root) && ["android", "ios", "ohos", "macos", "windows", "linux"].contains(&name)
}
pub fn is_app(root: &Path) -> bool {
    root.join("pubspec.yaml").is_file() && root.join("lib/main.dart").is_file()
}
pub fn detect(root: &Path) -> Result<Vec<crate::model::Script>, String> {
    if !is_app(root) {
        return Ok(vec![]);
    }
    let file = root.join("pubspec.yaml");
    let source = crate::project_scan::read_source(&file)?;
    if !source.lines().any(|s| {
        s.trim().split_once(':').is_some_and(|(key, value)| {
            key == "sdk" && value.trim().trim_matches(['\'', '"']) == "flutter"
        })
    }) && !root.join(".metadata").is_file()
    {
        return Ok(vec![]);
    }
    let settings = root.join(".vscode/settings.json");
    let configured = if settings.is_file() {
        crate::project_scan::parse_jsonc(&crate::project_scan::read_source(&settings)?)?["dart.flutterSdkPath"].as_str().map(str::to_owned)
    } else {
        None
    };
    let program = if let Some(path) = configured {
        let path = path.replace("${workspaceFolder}", &root.display().to_string());
        if path.contains("${") {
            return Err("dart.flutterSdkPath 包含无法解析的变量".into());
        }
        let path = if let Some(suffix) = path.strip_prefix("~/") {
            PathBuf::from(std::env::var_os("HOME").ok_or("HOME 未配置")?).join(suffix)
        } else {
            PathBuf::from(path)
        };
        let path = if path.is_absolute() {
            path
        } else {
            root.join(path)
        };
        path.join("bin/flutter").display().to_string()
    } else if root.join(".fvm/flutter_sdk/bin/flutter").is_file()
        || root.join(".fvmrc").is_file()
        || root.join(".fvm/fvm_config.json").is_file()
    {
        root.join(".fvm/flutter_sdk/bin/flutter")
            .display()
            .to_string()
    } else {
        "flutter".into()
    };
    let mut result = vec![];
    for (dir, platform) in [
        ("ios", Platform::Ios),
        ("android", Platform::Android),
        ("ohos", Platform::Ohos),
        ("web", Platform::Web),
        ("macos", Platform::Macos),
        ("windows", Platform::Windows),
        ("linux", Platform::Linux),
    ] {
        if !root.join(dir).is_dir() {
            continue;
        }
        let targets = if matches!(platform, Platform::Ios | Platform::Android | Platform::Ohos) {
            vec![Target::Simulator, Target::Device]
        } else {
            vec![Target::Desktop]
        };
        for target in targets {
            let mut launch = Launch {
                program: program.clone(),
                platform: platform.clone(),
                target,
                device: None,
                entrypoint: None,
                emulator: None,
            };
            launch.device = match platform {
                Platform::Web => Some("chrome".into()),
                Platform::Macos => Some("macos".into()),
                Platform::Windows => Some("windows".into()),
                Platform::Linux => Some("linux".into()),
                _ => None,
            };
            result.push(crate::model::Script { id:0,display_name:None,description_override:None,enabled:true,display_command:None,name:format!("Flutter / {}",launch.label()),file:file.clone(),cwd:root.into(),entry:crate::model::Entry::Flutter { launch },args:vec![],description:"启动时检测设备；模拟器未运行时按需启动。iOS 需要 macOS/Xcode，鸿蒙需要适配版 Flutter SDK 与 DevEco 设备。".into(),favorite:false,environment:Default::default(),expected_port:None,custom:true,resource_key:None });
        }
    }
    Ok(result)
}
