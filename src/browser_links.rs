//! One serialized browser operation across UI and API callers: reuse before creating a tab.
use std::{path::PathBuf, sync::mpsc};

pub fn canonical_url(target: &str) -> Result<String, String> {
    if target.len() > crate::console_buffer::MAX_LINE_BYTES || target.chars().any(char::is_control)
    {
        return Err("网址过长或包含控制字符".into());
    }
    let url = url::Url::parse(target).map_err(|e| format!("无效网址: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err("仅支持 HTTP(S) 网址".into());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err("网址不能包含账号或密码".into());
    }
    Ok(url.to_string())
}

#[derive(Default)]
struct Targets {
    loaded: bool,
    tabs: std::collections::BTreeMap<String, serde_json::Value>,
}
pub fn open(target: &str) -> Result<(), String> {
    let url = canonical_url(target)?;
    use sha2::{Digest, Sha256};
    let key = format!("{:x}", Sha256::digest(url.as_bytes()));
    static SERIAL: std::sync::OnceLock<std::sync::Mutex<Targets>> = std::sync::OnceLock::new();
    let mut targets = SERIAL
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "浏览器打开任务异常中断")?;
    let path = crate::app_identity::workspace()
        .map_err(|e| e.to_string())?
        .join("browser-targets.json");
    if !targets.loaded {
        targets.loaded = true;
        match std::fs::read(&path) {
            Ok(bytes) => {
                targets.tabs = serde_json::from_slice(&bytes)
                    .map_err(|e| format!("读取浏览器标签映射失败: {e}"))?
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(format!("读取浏览器标签映射失败: {e}")),
        }
    }
    // Persist a URL fingerprint, never query strings, tokens or fragments in clear text.
    targets
        .tabs
        .retain(|key, _| key.len() == 64 && key.bytes().all(|b| b.is_ascii_hexdigit()));
    let hint = targets
        .tabs
        .get(&key)
        .cloned()
        .unwrap_or(serde_json::json!({}));
    let opened = platform_open(&url, &hint)?;
    if targets.tabs.get(&key) != Some(&opened) {
        targets.tabs.insert(key, opened);
        while targets.tabs.len() > 512 {
            targets.tabs.pop_first();
        }
        crate::store::atomic_write(
            &path,
            &serde_json::to_vec(&targets.tabs).map_err(|e| e.to_string())?,
        )
        .map_err(|e| format!("网址已打开，但标签映射保存失败: {e}"))?;
    }
    Ok(())
}
#[cfg(target_os = "macos")]
fn platform_open(url: &str, hint: &serde_json::Value) -> Result<serde_json::Value, String> {
    use std::{
        io::Read,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };
    // The URL is an argv value, never executable script text. Requests remain serialized
    // through permission checks and tab creation, including clicks from different views.
    let mut child = Command::new("/usr/bin/osascript")
        .args([
            "-l",
            "JavaScript",
            "-e",
            include_str!("browser_links_macos.js"),
            "--",
            url,
            &hint.to_string(),
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("无法调用浏览器标签接口: {e}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|e| format!("浏览器标签接口异常: {e}"))?
        {
            if status.success() {
                let mut output = String::new();
                if let Some(stdout) = child.stdout.take() {
                    stdout
                        .take(16384)
                        .read_to_string(&mut output)
                        .map_err(|e| e.to_string())?;
                }
                return serde_json::from_str(output.trim())
                    .map_err(|e| format!("浏览器标签接口返回无效结果: {e}"));
            }
            let mut detail = String::new();
            if let Some(stderr) = child.stderr.take() {
                let _ = stderr.take(16384).read_to_string(&mut detail);
            }
            eprintln!("[dev start browser] {}", detail.trim());
            let permission = if detail.contains("-1743") || detail.contains("not authorized") {
                "。请在 macOS“隐私与安全性 → 自动化”中允许 DevStart 控制默认浏览器"
            } else {
                ""
            };
            return Err(format!("打开浏览器标签失败：{}{permission}", detail.trim()));
        }
        if started.elapsed() >= Duration::from_secs(45) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("浏览器未在 45 秒内响应，请处理系统授权提示后重试；未重复创建标签".into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
#[cfg(not(target_os = "macos"))]
fn platform_open(_url: &str, _hint: &serde_json::Value) -> Result<serde_json::Value, String> {
    Err("当前系统尚未提供默认浏览器标签复用接口；未新建重复标签".into())
}

/// UI interactions enqueue work; native browser automation never blocks rendering or service polling.
#[derive(Default)]
pub struct Pending {
    tasks: Vec<mpsc::Receiver<Result<(), String>>>,
}
impl Pending {
    pub fn request(&mut self, target: String, cwd: PathBuf) {
        let (tx, rx) = mpsc::channel();
        self.tasks.push(rx);
        std::thread::spawn(move || {
            let _ = tx.send(crate::console_text::open(&target, &cwd));
        });
    }
    pub fn errors(&mut self) -> Vec<String> {
        let mut errors = Vec::new();
        self.tasks.retain(|rx| match rx.try_recv() {
            Ok(Ok(())) => false,
            Ok(Err(e)) => {
                errors.push(e);
                false
            }
            Err(mpsc::TryRecvError::Empty) => true,
            Err(mpsc::TryRecvError::Disconnected) => {
                errors.push("浏览器打开任务意外中断".into());
                false
            }
        });
        errors
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_identity_preserves_routes_and_normalizes_authority() {
        assert_eq!(
            canonical_url("HTTP://LOCALHOST:80").unwrap(),
            "http://localhost/"
        );
        assert_eq!(
            canonical_url("https://EXAMPLE.com:443/?a=1#route").unwrap(),
            "https://example.com/?a=1#route"
        );
        assert_eq!(
            canonical_url("http://[::1]:8000/path").unwrap(),
            "http://[::1]:8000/path"
        );
        assert_ne!(
            canonical_url("http://localhost:8000/#/a").unwrap(),
            canonical_url("http://localhost:8000/#/b").unwrap()
        );
    }
    #[test]
    fn browser_requests_reject_non_web_schemes_credentials_and_controls() {
        for target in [
            "javascript:alert(1)",
            "file:///etc/passwd",
            "http://a:b@localhost",
            "http://localhost/\n",
            "data:text/html,hello",
        ] {
            assert!(canonical_url(target).is_err());
        }
    }
}
