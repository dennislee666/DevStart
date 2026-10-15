//! Compile only an explicitly editor-saved Java source; never execute project build scripts.
use crate::{
    editor,
    lifecycle::{Control, Journal},
    model::{Entry, Script},
};
use std::{
    fs,
    io::Read,
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
pub struct Compilation {
    staging: PathBuf,
    destination: PathBuf,
    source: PathBuf,
    revision: String,
}
impl Drop for Compilation {
    fn drop(&mut self) {
        if let Err(e) = fs::remove_dir_all(&self.staging) {
            eprintln!("[dev start] 清理 Java 编辑编译目录失败: {e}");
        }
    }
}
pub fn prepare(
    script: &Script,
    root: &Path,
    cancel: &Control,
    journal: &Journal,
) -> Result<Option<Compilation>, String> {
    let Entry::Java {
        program,
        class_paths,
        main_class,
        ..
    } = &script.entry
    else {
        return Ok(None);
    };
    if script.file.extension().is_none_or(|s| s != "java") {
        return Ok(None);
    }
    let source = script.file.canonicalize().map_err(|e| e.to_string())?;
    let marker = root
        .join("source-versions")
        .join(script.id.to_string())
        .join("edited-source");
    let managed: PathBuf = match fs::read(&marker) {
        Ok(bytes) => {
            serde_json::from_slice(&bytes).map_err(|e| format!("Java 编辑来源记录损坏: {e}"))?
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("读取 Java 编辑来源记录失败: {e}")),
    };
    if managed != source {
        return Ok(None);
    }
    let revision = editor::revision(&editor::read_source(&source)?);
    let class = main_class.replace('.', "/") + ".class";
    let destination = class_paths
        .iter()
        .find(|p| p.is_dir() && p.join(&class).is_file())
        .ok_or("无法定位 Java 主类输出目录，取消启动")?
        .clone();
    let staging = root
        .join("java-edit-build")
        .join(crate::model::unique_id().to_string());
    fs::create_dir_all(&staging).map_err(|e| e.to_string())?;
    let result = Compilation {
        staging,
        destination,
        source,
        revision,
    };
    let javac = Path::new(program)
        .parent()
        .unwrap_or(Path::new(""))
        .join("javac");
    journal.note(
        "启动文件已在内置编辑器修改：使用同一 JDK 的 javac 编译该源文件，不执行项目构建或部署脚本",
    );
    let mut child = Command::new(javac)
        .args(["-proc:none", "-implicit:none", "-encoding", "UTF-8", "-cp"])
        .arg(std::env::join_paths(class_paths).map_err(|e| e.to_string())?)
        .arg("-d")
        .arg(&result.staging)
        .arg(&result.source)
        .current_dir(&script.cwd)
        .envs(&script.environment)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("无法启动 Java 源文件编译器: {e}"))?;
    let mut readers = vec![];
    for mut stream in [
        Box::new(child.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.stderr.take().unwrap()),
    ] {
        let journal = journal.clone();
        readers.push(std::thread::spawn(move || {
            let mut buf = [0; 4096];
            loop {
                match stream.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        journal.note(format!("[javac] {}", String::from_utf8_lossy(&buf[..n])))
                    }
                    Err(e) => {
                        journal.error(format!("读取 javac 输出失败: {e}"));
                        break;
                    }
                }
            }
        }));
    }
    let deadline = Instant::now() + Duration::from_secs(60);
    let status = loop {
        if cancel.requested() || Instant::now() >= deadline {
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.wait();
            for reader in readers {
                let _ = reader.join();
            }
            return Err("Java 编译已取消或超过 60 秒；原服务未停止".into());
        }
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            break status;
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    for reader in readers {
        reader.join().map_err(|_| "Java 编译输出线程异常")?;
    }
    if !status.success() {
        return Err(format!(
            "Java 编译失败（{status}），原服务未停止；请检查 Console 中的 javac 错误"
        ));
    }
    if !result.staging.join(class).is_file() {
        return Err("javac 没有生成预期主类，取消启动".into());
    }
    journal.note("Java 源文件编译通过，原服务仍保持运行，准备正常切换");
    Ok(Some(result))
}
impl Compilation {
    pub fn install(self, journal: &Journal) -> Result<(), String> {
        if editor::revision(&editor::read_source(&self.source)?) != self.revision {
            return Err("编译期间源文件发生变化，取消启动，请重试".into());
        }
        fn copy_tree(from: &Path, to: &Path) -> Result<(), String> {
            fs::create_dir_all(to).map_err(|e| e.to_string())?;
            for entry in fs::read_dir(from).map_err(|e| e.to_string())? {
                let entry = entry.map_err(|e| e.to_string())?;
                let target = to.join(entry.file_name());
                if entry.file_type().map_err(|e| e.to_string())?.is_dir() {
                    copy_tree(&entry.path(), &target)?;
                } else {
                    crate::store::atomic_write(
                        &target,
                        &fs::read(entry.path()).map_err(|e| e.to_string())?,
                    )?;
                }
            }
            Ok(())
        }
        copy_tree(&self.staging, &self.destination)?;
        journal.note("已应用此次 Java 源文件编译结果，下一实例使用当前保存版本");
        Ok(())
    }
}
