//! Local-only transport. Socket workers never touch configuration or runtime directly.
use crate::{api, service::Service};
use serde_json::Value;
use std::{
    io::{BufRead, BufReader, Read, Write},
    os::unix::{
        fs::{FileTypeExt, PermissionsExt},
        net::{UnixListener, UnixStream},
    },
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::{Duration, Instant},
};
pub const REQUEST_LIMIT: u64 = 2 * 1024 * 1024;
pub const RESPONSE_LIMIT: u64 = 8 * 1024 * 1024;
struct Job {
    request: Value,
    reply: mpsc::Sender<Value>,
    deadline: Instant,
}
pub struct Server {
    incoming: Receiver<Job>,
    stop: Arc<AtomicBool>,
    path: PathBuf,
}
pub fn workspace() -> Result<PathBuf, String> {
    crate::app_identity::workspace().map_err(|e| e.to_string())
}

pub fn socket_path(root: &Path) -> PathBuf {
    root.join("api/control.sock")
}
impl Server {
    pub fn bind(root: &Path) -> Result<Self, String> {
        let path = socket_path(root);
        let dir = path.parent().ok_or("API路径无效")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        if std::fs::symlink_metadata(dir)
            .map_err(|e| e.to_string())?
            .file_type()
            .is_symlink()
        {
            return Err("API目录不可为符号链接".into());
        }
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))
            .map_err(|e| e.to_string())?;
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if !metadata.file_type().is_socket() {
                return Err("API路径已存在且不是socket".into());
            }
            if UnixStream::connect(&path).is_ok() {
                return Err("API socket已有服务".into());
            }
            std::fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        let listener = UnixListener::bind(&path)
            .map_err(|e| format!("本地API监听失败 {}: {e}", path.display()))?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, incoming) = mpsc::sync_channel(32);
        let (connections, sockets) = mpsc::sync_channel::<UnixStream>(16);
        let sockets = Arc::new(Mutex::new(sockets));
        for _ in 0..4 {
            let sockets = sockets.clone();
            let tx = tx.clone();
            thread::spawn(move || {
                loop {
                    let stream = { sockets.lock().expect("socket queue").recv() };
                    let Ok(stream) = stream else { break };
                    serve(stream, &tx);
                }
            });
        }
        let ending = stop.clone();
        thread::spawn(move || {
            while !ending.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let _ = connections.try_send(stream);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        thread::sleep(Duration::from_millis(20))
                    }
                    Err(e) => {
                        eprintln!("[dev start api] accept: {e}");
                        break;
                    }
                }
            }
        });
        eprintln!("[dev start api] listening {}", path.display());
        Ok(Self {
            incoming,
            stop,
            path,
        })
    }
    pub fn poll(&self, s: &mut Service) {
        for _ in 0..8 {
            let Ok(job) = self.incoming.try_recv() else {
                break;
            };
            if Instant::now() > job.deadline {
                continue;
            }
            let _ = job.reply.send(api::response(s, job.request));
        }
    }
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = std::fs::remove_file(&self.path);
    }
}
fn serve(mut stream: UnixStream, sender: &SyncSender<Job>) {
    // macOS can inherit O_NONBLOCK from the listener. Workers need blocking reads
    // with a timeout so accepting before the client writes does not close a valid connection.
    if let Err(e) = stream.set_nonblocking(false) {
        eprintln!("[dev start api] stream mode: {e}");
        return;
    }
    let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
    let result = (|| -> Result<Value, String> {
        let mut line = String::new();
        BufReader::new((&stream).take(REQUEST_LIMIT + 1))
            .read_line(&mut line)
            .map_err(|e| e.to_string())?;
        if line.len() as u64 > REQUEST_LIMIT || !line.ends_with('\n') {
            return Err("请求超限或缺少换行".into());
        }
        let request: Value =
            serde_json::from_str(&line).map_err(|e| format!("JSON解析失败: {e}"))?;
        let (reply, rx) = mpsc::channel();
        sender
            .try_send(Job {
                request,
                reply,
                deadline: Instant::now() + Duration::from_secs(12),
            })
            .map_err(|_| "API队列繁忙".to_string())?;
        rx.recv_timeout(Duration::from_secs(15))
            .map_err(|_| "服务超时；执行结果未知，请查询状态后再决定是否重试".into())
    })();
    let value = result.unwrap_or_else(|e| api::error(Value::Null, -32001, &e));
    let mut bytes = serde_json::to_vec(&value).unwrap_or_default();
    if bytes.len() as u64 > RESPONSE_LIMIT {
        bytes = serde_json::to_vec(&api::error(
            value["id"].clone(),
            -32002,
            "响应超限，请缩小查询范围",
        ))
        .unwrap_or_default();
    }
    bytes.push(b'\n');
    if let Err(e) = stream.write_all(&bytes) {
        eprintln!("[dev start api] response write: {e}");
    }
}
pub fn call(root: &Path, method: &str, params: Value) -> Result<Value, String> {
    let path = socket_path(root);
    let mut stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "连接API {}: {e}；请先打开Dev Start或运行 --serve",
            path.display()
        )
    })?;
    stream
        .set_read_timeout(Some(Duration::from_secs(20)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(3)))
        .map_err(|e| e.to_string())?;
    let mut bytes = serde_json::to_vec(
        &serde_json::json!({"jsonrpc":"2.0","id":1,"method":method,"params":params}),
    )
    .map_err(|e| e.to_string())?;
    if bytes.len() as u64 >= REQUEST_LIMIT {
        return Err("请求超过2MiB".into());
    }
    bytes.push(b'\n');
    stream.write_all(&bytes).map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream.take(RESPONSE_LIMIT + 1))
        .read_line(&mut line)
        .map_err(|e| e.to_string())?;
    if line.len() as u64 > RESPONSE_LIMIT {
        return Err("响应超限".into());
    }
    serde_json::from_str(&line).map_err(|e| e.to_string())
}
