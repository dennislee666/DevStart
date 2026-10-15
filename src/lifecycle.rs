//! Lifecycle events share the visible Console and bounded raw log.
use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
pub const GRACE: Duration = Duration::from_secs(5);
pub const FORCE_WAIT: Duration = Duration::from_secs(2);
#[derive(Clone)]
pub struct Journal {
    path: PathBuf,
    emit: Arc<dyn Fn(bool, String) + Send + Sync>,
}
impl Journal {
    pub fn new(path: PathBuf, emit: impl Fn(bool, String) + Send + Sync + 'static) -> Self {
        Self {
            path,
            emit: Arc::new(emit),
        }
    }
    pub fn workspace_root(&self) -> &std::path::Path {
        self.path
            .parent()
            .and_then(|p| p.parent())
            .expect("run log workspace")
    }
    pub fn note(&self, message: impl AsRef<str>) {
        self.write(false, message.as_ref());
    }
    pub fn error(&self, message: impl AsRef<str>) {
        self.write(true, message.as_ref());
    }
    fn write(&self, error: bool, message: &str) {
        let line = format!("[dev start] {message}\n");
        if let Err(e) = crate::log_store::append(&self.path, line.as_bytes()) {
            (self.emit)(true, format!("[dev start] 生命周期日志写入失败: {e}\n"));
        }
        (self.emit)(error, line);
    }
}
#[derive(Clone, Default)]
pub struct Control {
    pub stop: Arc<AtomicBool>,
    done: Arc<Mutex<Option<Result<(), String>>>>,
    observer: Arc<Mutex<Option<Journal>>>,
}
impl Control {
    pub fn request(&self) {
        self.stop.store(true, Ordering::Release);
    }
    pub fn requested(&self) -> bool {
        self.stop.load(Ordering::Acquire)
    }
    pub fn completion(&self) -> Option<Result<(), String>> {
        self.done.lock().expect("control lock").clone()
    }
    pub fn observe(&self, journal: Journal) {
        let mut observer = self.observer.lock().expect("control lock");
        if self.completion().is_none() {
            *observer = Some(journal);
        }
    }
    pub fn note(&self, own: &Journal, message: impl AsRef<str>) {
        own.note(message.as_ref());
        if let Some(observer) = self.observer.lock().expect("control lock").as_ref() {
            observer.note(message.as_ref());
        }
    }
    pub fn error(&self, own: &Journal, message: &str) {
        own.error(message);
        if let Some(observer) = self.observer.lock().expect("control lock").as_ref() {
            observer.error(message);
        }
    }
    pub fn finish(&self, result: Result<(), String>) {
        *self.done.lock().expect("control lock") = Some(result);
        self.observer.lock().expect("control lock").take();
    }
}
pub fn terminate_external(
    process: &crate::external::Process,
    note: &impl Fn(&str),
) -> Result<(), String> {
    if !crate::external::same_process(process)? {
        note(&format!("原 PID {} 已退出，无需关闭", process.pid));
        return Ok(());
    }
    let mut tree = crate::external::tree(process)?;
    note(&format!(
        "已确认外部实例 PID {}，请求优雅关闭 SIGTERM（宽限 {} 秒）",
        process.pid,
        GRACE.as_secs()
    ));
    for p in &tree {
        crate::external::signal_checked(p, libc::SIGTERM)?;
    }
    if wait_external(&tree, GRACE)? {
        note(&format!("外部实例 PID {} 已优雅退出", process.pid));
        return Ok(());
    }
    // A supervisor can fork while handling TERM. Refresh proven descendants before KILL.
    let known = tree.clone();
    for parent in known {
        if crate::external::same_process(&parent)? {
            for child in crate::external::tree(&parent)? {
                if !tree
                    .iter()
                    .any(|p| p.pid == child.pid && p.started == child.started)
                {
                    tree.push(child);
                }
            }
        }
    }
    note(&format!(
        "优雅关闭超时，仍存活的子树将强制关闭 SIGKILL（根 PID {}）",
        process.pid
    ));
    for p in std::iter::once(process).chain(tree.iter().rev().filter(|p| p.pid != process.pid)) {
        if crate::external::same_process(p)? {
            note(&format!("强制关闭 PID {}", p.pid));
            crate::external::signal_checked(p, libc::SIGKILL)?;
        }
    }
    if !wait_external(&tree, FORCE_WAIT)? {
        return Err(format!(
            "外部 PID {} 强制关闭后仍未退出，取消启动",
            process.pid
        ));
    }
    note(&format!(
        "外部实例 PID {} 已强制结束并确认退出",
        process.pid
    ));
    Ok(())
}
fn wait_external(tree: &[crate::external::Process], timeout: Duration) -> Result<bool, String> {
    let deadline = Instant::now() + timeout;
    loop {
        let mut alive = false;
        for p in tree {
            alive |= crate::external::same_process(p)?;
        }
        if !alive {
            return Ok(true);
        }
        if Instant::now() >= deadline {
            return Ok(false);
        }
        std::thread::sleep(Duration::from_millis(80));
    }
}
