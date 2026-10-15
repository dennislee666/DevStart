use crate::{
    lifecycle::{Control, Journal},
    model::{Entry, Project, Script},
};
use std::os::unix::process::CommandExt;
use std::{
    collections::HashSet,
    io::{Read, Write},
    path::PathBuf,
    process::{Command, Stdio},
    sync::{
        Arc, Mutex,
        mpsc::{self, Receiver, Sender, SyncSender},
    },
    thread,
    time::{Duration, Instant, SystemTime},
};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum State {
    Starting,
    Running,
    Stopping,
    Succeeded,
    Failed,
    Stopped,
}
impl State {
    pub fn label(self) -> &'static str {
        match self {
            Self::Starting => "启动中",
            Self::Running => "运行中",
            Self::Stopping => "停止中",
            Self::Succeeded => "已完成",
            Self::Failed => "失败",
            Self::Stopped => "已停止",
        }
    }
    pub fn active(self) -> bool {
        matches!(self, Self::Starting | Self::Running | Self::Stopping)
    }
}
pub struct Run {
    pub id: u64,
    /// Stable presentation identity; process events always use the unique id above.
    pub console_id: u64,
    pub superseded: bool,
    pub tab_open: bool,
    pub close_requested: bool,
    pub script_id: u64,
    pub title: String,
    pub command: String,
    pub display_command: String,
    pub launch_name: String,
    pub resource_key: Option<String>,
    pub pid: u32,
    pub cwd: PathBuf,
    pub expected_port: Option<u16>,
    pub listening_ports: Vec<u16>,
    pub external: Option<crate::external::Process>,
    pub associated_scripts: Vec<u64>,
    pub state: State,
    pub started: Instant,
    pub started_at: Option<SystemTime>,
    pub elapsed_offset: Duration,
    pub elapsed: Duration,
    pub exit: Option<i32>,
    pub log_path: PathBuf,
    pub lines: crate::console_buffer::ConsoleBuffer,
    control: Control,
}
enum Event {
    Output(u64, bool, String),
    Error(u64, String),
    Finished(u64, State, Option<i32>),
    Prepared(Box<Prepared>),
}
struct Prepared {
    id: u64,
    project: String,
    script: Script,
    logs: PathBuf,
    result: Result<(), String>,
}
pub struct Runtime {
    pub runs: Vec<Run>,
    log_rx: Receiver<Event>,
    log_tx: SyncSender<Event>,
    state_rx: Receiver<Event>,
    state_tx: Sender<Event>,
    scan: Option<Receiver<Result<Vec<crate::external::Candidate>, String>>>,
    scanned: Option<Instant>,
    ports: crate::runtime_ports::Monitor,
    pub error: Option<String>,
}
impl Default for Runtime {
    fn default() -> Self {
        let (log_tx, log_rx) = mpsc::sync_channel(512);
        let (state_tx, state_rx) = mpsc::channel();
        Self {
            runs: vec![],
            log_rx,
            log_tx,
            state_rx,
            state_tx,
            scan: None,
            scanned: None,
            ports: Default::default(),
            error: None,
        }
    }
}
impl Runtime {
    pub fn launch_checked(
        &mut self,
        project: &Project,
        script: &Script,
        logs: PathBuf,
    ) -> Result<u64, String> {
        let conflicts = |r: &Run| {
            r.associated_scripts.contains(&script.id)
                || script
                    .resource_key
                    .as_ref()
                    .is_some_and(|key| r.resource_key.as_ref() == Some(key))
                || script.expected_port.is_some()
                    && r.expected_port == script.expected_port
                    && r.cwd == script.cwd
        };
        if let Some(pending) = self
            .runs
            .iter()
            .find(|r| r.state == State::Starting && conflicts(r))
        {
            if pending.script_id == script.id {
                self.journal(pending.id, &pending.log_path)
                    .note("已有启动检查正在执行，合并重复请求");
                return Ok(pending.id);
            }
            self.journal(pending.id, &pending.log_path).error(format!(
                "{} 请求启动，但同一端口或共享资源正在检查中；拒绝并行切换",
                script.name
            ));
            return Err("同一端口或共享资源正在启动检查中，请等待完成".into());
        }
        let tracked: Vec<_> = self
            .runs
            .iter()
            .filter(|r| r.state.active() && conflicts(r))
            .map(|r| crate::launch::Tracked {
                pid: r.pid,
                control: r.control.clone(),
            })
            .collect();
        let id = crate::model::unique_id();
        let path = logs.join(format!("run-{id}.log"));
        crate::log_store::append(
            &path,
            format!(
                "[dev start] {} / {}\n[command] {}\n[cwd] {}\n",
                project.name,
                script.name,
                script.invocation(),
                script.cwd.display()
            )
            .as_bytes(),
        )
        .map_err(|e| format!("创建启动Console日志: {e}"))?;
        let control = Control::default();
        let journal = self.journal(id, &path);
        self.runs.push(Run {
            console_id: id,
            superseded: false,
            tab_open: true,
            close_requested: false,
            id,
            script_id: script.id,
            title: format!("{} / {}", project.name, script.display_label()),
            command: script.invocation(),
            display_command: script.tab_command(),
            launch_name: script.display_label().to_owned(),
            resource_key: script.resource_key.clone(),
            pid: 0,
            cwd: script.cwd.clone(),
            expected_port: script.expected_port,
            listening_ports: vec![],
            external: None,
            associated_scripts: vec![script.id],
            state: State::Starting,
            started: Instant::now(),
            started_at: None,
            elapsed_offset: Duration::ZERO,
            elapsed: Duration::ZERO,
            exit: None,
            log_path: path,
            lines: Default::default(),
            control: control.clone(),
        });
        journal.note("校验启动项配置…");
        let project = project.clone();
        let script = script.clone();
        let tx = self.state_tx.clone();
        thread::spawn(move || {
            let result = (|| {
                crate::profile::validate(&script)?;
                crate::project_manifest::validate_launch(&script)?;
                crate::launch::prepare(&project, &script, &tracked, &control, &journal)
            })();
            let _ = tx.send(Event::Prepared(Box::new(Prepared {
                id,
                project: project.name,
                script,
                logs,
                result,
            })));
        });
        Ok(id)
    }
    /// A new process generation occupies the same logical console and visual position.
    /// Keeping unique event ids prevents delayed old-process events from stopping the replacement.
    pub fn restart_checked(
        &mut self,
        target: u64,
        project: &Project,
        script: &Script,
        logs: PathBuf,
    ) -> Result<u64, String> {
        let index = self
            .runs
            .iter()
            .position(|r| r.id == target)
            .ok_or("运行记录不存在")?;
        let console_id = self.runs[index].console_id;
        let id = self.launch_checked(project, script, logs)?;
        if id == target {
            return Ok(id);
        }
        let new_index = self
            .runs
            .iter()
            .position(|r| r.id == id)
            .ok_or("重启记录不存在")?;
        for run in &mut self.runs {
            if run.id != id
                && (run.console_id == console_id || run.associated_scripts.contains(&script.id))
            {
                run.tab_open = false;
                run.superseded = true;
                run.console_id = console_id;
            }
        }
        let mut replacement = self.runs.remove(new_index);
        replacement.console_id = console_id;
        self.runs.insert(index.min(self.runs.len()), replacement);
        self.note(id, "在当前 Console 中重启；确认原实例退出后启动新实例");
        Ok(id)
    }
    pub fn note(&self, id: u64, message: impl AsRef<str>) {
        if let Some(run) = self.runs.iter().find(|r| r.id == id) {
            self.journal(id, &run.log_path).note(message);
        }
    }
    fn journal(&self, id: u64, path: &std::path::Path) -> Journal {
        let tx = self.state_tx.clone();
        Journal::new(path.to_path_buf(), move |err, text| {
            let _ = tx.send(Event::Output(id, err, text));
        })
    }
    #[cfg(test)]
    pub fn start(
        &mut self,
        project: &str,
        script: &Script,
        log_dir: PathBuf,
    ) -> Result<u64, String> {
        self.spawn(project, script, log_dir, None)
    }
    fn spawn(
        &mut self,
        project: &str,
        script: &Script,
        log_dir: PathBuf,
        slot: Option<u64>,
    ) -> Result<u64, String> {
        if self.runs.iter().any(|r| {
            Some(r.id) != slot
                && r.associated_scripts.contains(&script.id)
                && r.state.active()
                && !matches!(r.control.completion(), Some(Ok(())))
        }) {
            return Err("该脚本已经运行".into());
        }
        if !script.cwd.is_dir() || !script.file.is_file() {
            return Err(format!("项目目录或脚本不存在: {}", script.file.display()));
        }
        if let Some(key) = &script.resource_key
            && let Some(existing) = self.runs.iter().find(|r| {
                Some(r.id) != slot
                    && r.state.active()
                    && !matches!(r.control.completion(), Some(Ok(())))
                    && r.resource_key.as_ref() == Some(key)
            })
        {
            return Err(format!(
                "共享调试资源已有运行项：{}；请先停止该项再选择另一模式",
                existing.title
            ));
        }
        crate::profile::validate(script)?;
        crate::project_manifest::validate_launch(script)?;
        if let Some(port) = script.expected_port {
            let addresses = [
                std::net::SocketAddr::from(([127, 0, 0, 1], port)),
                std::net::SocketAddr::from(([0, 0, 0, 0, 0, 0, 0, 1], port)),
            ];
            for address in addresses {
                match std::net::TcpStream::connect_timeout(&address, Duration::from_millis(300)) {
                    Ok(_) => {
                        return Err(format!(
                            "本地端口 {port} 已有服务；未重复启动 {}，也未停止现有进程",
                            script.name
                        ));
                    }
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionRefused
                                | std::io::ErrorKind::AddrNotAvailable
                        ) => {}
                    Err(e) => return Err(format!("无法确认本地端口 {port} 是否可用: {e}")),
                }
            }
        }
        let id = slot.unwrap_or_else(crate::model::unique_id);
        let log_path = log_dir.join(format!("run-{id}.log"));
        let control = slot
            .and_then(|id| self.runs.iter().find(|r| r.id == id))
            .map(|r| r.control.clone())
            .unwrap_or_default();
        if control.requested() {
            return Err("已取消启动".into());
        }
        let mut file =
            crate::log_store::Writer::open(&log_path).map_err(|e| format!("创建日志: {e}"))?;
        if slot.is_none() {
            writeln!(
                file,
                "[dev start] {}\n[command] {}\n[cwd] {}",
                project,
                script.invocation(),
                script.cwd.display()
            )
            .map_err(|e| e.to_string())?;
        }
        let file = Arc::new(Mutex::new(file));
        let journal = self.journal(id, &log_path);
        journal.note(format!("校验通过，执行启动命令：{}", script.tab_command()));
        // A login shell restores the development PATH when opened from Finder.
        // Arguments are positional, so project paths and package names are never shell code.
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let mut command = Command::new(shell);
        command.args(["-lc", "exec \"$@\"", "dev-start"]);
        if !script.environment.is_empty() {
            command.arg("/usr/bin/env");
            for (key, value) in &script.environment {
                command.arg(format!("{key}={value}"));
            }
        }
        match &script.entry {
            Entry::File { interpreter } => {
                let mut parts = interpreter.split_whitespace();
                command
                    .arg(parts.next().ok_or("解释器为空")?)
                    .args(parts)
                    .arg(&script.file);
            }
            Entry::Package { manager, name } => {
                command.arg(manager).arg("run").arg(name);
                if !script.args.is_empty() {
                    command.arg("--");
                }
            }
            Entry::Command { program } => {
                command.arg(program);
            }
            Entry::Java {
                program,
                main_class,
                vm_args,
                class_paths,
                ..
            } => {
                command
                    .arg(program)
                    .args(vm_args)
                    .arg("-cp")
                    .arg(std::env::join_paths(class_paths).map_err(|e| e.to_string())?)
                    .arg(main_class);
            }
        }
        command.args(&script.args);
        // A dedicated process group owns the shell and its descendants, not unrelated processes.
        command
            .current_dir(&script.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0);
        let mut child = command
            .spawn()
            .map_err(|e| format!("启动 {} 失败: {e}", script.invocation()))?;
        let pid = child.id();
        if let Some(id) = slot {
            let run = self
                .runs
                .iter_mut()
                .find(|r| r.id == id)
                .ok_or("启动Console不存在")?;
            run.pid = pid;
            run.started = Instant::now();
            run.started_at = Some(SystemTime::now());
            run.elapsed_offset = Duration::ZERO;
            run.state = State::Running;
        } else {
            self.runs.push(Run {
                console_id: id,
                superseded: false,
                tab_open: true,
                close_requested: false,
                id,
                script_id: script.id,
                title: format!("{project} / {}", script.display_label()),
                command: script.invocation(),
                display_command: script.tab_command(),
                launch_name: script.display_label().to_owned(),
                resource_key: script.resource_key.clone(),
                pid,
                cwd: script.cwd.clone(),
                expected_port: script.expected_port,
                listening_ports: vec![],
                external: None,
                associated_scripts: vec![script.id],
                state: State::Running,
                started: Instant::now(),
                started_at: Some(SystemTime::now()),
                elapsed_offset: Duration::ZERO,
                elapsed: Duration::ZERO,
                exit: None,
                log_path,
                lines: Default::default(),
                control: control.clone(),
            });
        }
        journal.note(format!("新实例已启动，PID {pid}，开始接收实时输出"));
        let stdout = child.stdout.take().ok_or("无法捕获 stdout")?;
        let stderr = child.stderr.take().ok_or("无法捕获 stderr")?;
        let stdout_done = pump(
            stdout,
            false,
            id,
            self.log_tx.clone(),
            self.state_tx.clone(),
            file.clone(),
        );
        let stderr_done = pump(
            stderr,
            true,
            id,
            self.log_tx.clone(),
            self.state_tx.clone(),
            file.clone(),
        );
        let tx = self.state_tx.clone();
        thread::spawn(move || {
            let mut status = None;
            let mut termination = None;
            let mut forced = None;
            loop {
                if control.requested() && termination.is_none() {
                    control.note(
                        &journal,
                        format!(
                            "请求优雅关闭进程组 {pid}：SIGTERM，宽限 {} 秒",
                            crate::lifecycle::GRACE.as_secs()
                        ),
                    );
                    if let Err(e) = signal(pid, libc::SIGTERM) {
                        control.error(&journal, &e);
                    }
                    termination = Some(Instant::now());
                }
                if forced.is_none()
                    && termination.is_some_and(|t| t.elapsed() >= crate::lifecycle::GRACE)
                    && group_alive(pid)
                {
                    control.note(
                        &journal,
                        format!("优雅关闭超时，强制关闭进程组 {pid}：SIGKILL"),
                    );
                    if let Err(e) = signal(pid, libc::SIGKILL) {
                        control.error(&journal, &e);
                    }
                    forced = Some(Instant::now());
                }
                if forced.is_some_and(|t| t.elapsed() > crate::lifecycle::FORCE_WAIT)
                    && group_alive(pid)
                {
                    let message = format!("进程组 {pid} 强制关闭后仍未退出，取消重新启动");
                    journal.error(&message);
                    let _ = tx.send(Event::Finished(id, State::Failed, None));
                    control.finish(Err(message));
                    return;
                }
                if status.is_none() {
                    match child.try_wait() {
                        Ok(s) => status = s,
                        Err(e) => {
                            let _ = tx.send(Event::Error(id, format!("wait: {e}")));
                            let _ = signal(pid, libc::SIGKILL);
                            let _ = child.wait();
                            let _ = tx.send(Event::Finished(id, State::Failed, None));
                            control.finish(Err(format!("等待进程退出失败: {e}")));
                            return;
                        }
                    }
                }
                // A launcher can exit before its background services. Keep tracking the group.
                if let Some(s) = status
                    && !group_alive(pid)
                {
                    let state = if termination.is_some() {
                        State::Stopped
                    } else if s.success() {
                        State::Succeeded
                    } else {
                        State::Failed
                    };
                    for done in [&stdout_done, &stderr_done] {
                        if done.recv_timeout(Duration::from_millis(300)).is_err() {
                            journal.note(
                                "进程已退出，输出管道尚未关闭；后续输出仍写入有保留上限的原日志",
                            );
                        }
                    }
                    let message = format!(
                        "\n[dev start] {} · {}\n",
                        state.label(),
                        exit_label(s.code())
                    );
                    if let Ok(mut f) = file.lock()
                        && let Err(e) = f.write_all(message.as_bytes())
                    {
                        let _ = tx.send(Event::Error(id, format!("写日志: {e}")));
                    }
                    control.note(
                        &journal,
                        format!(
                            "进程组 {pid} 已{}退出",
                            if forced.is_some() {
                                "强制结束并确认"
                            } else if termination.is_some() {
                                "优雅"
                            } else {
                                "自行"
                            }
                        ),
                    );
                    let _ = tx.send(Event::Finished(id, state, s.code()));
                    control.finish(Ok(()));
                    break;
                }
                thread::sleep(Duration::from_millis(60));
            }
        });
        Ok(id)
    }
    pub fn stop(&mut self, id: u64) {
        if let Some(run) = self
            .runs
            .iter_mut()
            .find(|r| r.id == id && r.state.active())
        {
            run.control.request();
            run.state = State::Stopping;
        }
    }
    /// Stop every still-active generation owned by one current logical tab.
    pub fn stop_console(&mut self, id: u64) -> Result<(), String> {
        let target = self
            .runs
            .iter()
            .find(|r| r.id == id && !r.superseded)
            .ok_or("当前标签不存在")?;
        let console_id = target.console_id;
        let journal = self.journal(id, &target.log_path);
        let ids: Vec<_> = self
            .runs
            .iter()
            .filter(|r| r.console_id == console_id && r.state.active())
            .map(|r| r.id)
            .collect();
        for other in self
            .runs
            .iter()
            .filter(|r| ids.contains(&r.id) && r.id != id)
        {
            other.control.observe(journal.clone());
        }
        journal.note("停止当前标签及其尚未退出的进程实例");
        for id in ids {
            self.stop(id);
        }
        Ok(())
    }
    /// Closing an active tab requires explicit consent and completes only after exit.
    pub fn close_tab(&mut self, id: u64, stop: bool) -> Result<(), String> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("运行记录不存在")?;
        if run.state.active() {
            if !stop {
                return Err("启动项仍在运行，需要确认停止并关闭".into());
            }
            run.close_requested = true;
            self.stop(id);
        } else {
            run.tab_open = false;
        }
        Ok(())
    }
    pub fn open_tab(&mut self, id: u64) -> Result<(), String> {
        let run = self
            .runs
            .iter_mut()
            .find(|r| r.id == id)
            .ok_or("运行记录不存在")?;
        if run.superseded {
            return Err("该记录已被重启替换，请打开当前 Console".into());
        }
        if run.close_requested {
            return Err("正在停止并关闭，请等待完成".into());
        }
        run.tab_open = true;
        Ok(())
    }
    fn trim_history(&mut self) {
        // Explicit tabs and active generations survive; only closed history is capped at eight.
        let mut completed: Vec<_> = self
            .runs
            .iter()
            .filter(|r| !r.state.active() && (!r.tab_open || r.superseded))
            .map(|r| r.id)
            .collect();
        completed.sort_unstable_by(|a, b| b.cmp(a));
        let keep: HashSet<_> = completed
            .into_iter()
            .take(8)
            .chain(
                self.runs
                    .iter()
                    .filter(|r| r.state.active() || (r.tab_open && !r.superseded))
                    .map(|r| r.id),
            )
            .collect();
        self.runs.retain(|r| keep.contains(&r.id));
    }
    pub fn stop_all(&mut self) {
        for r in &mut self.runs {
            if r.state.active() {
                r.control.request();
                r.state = State::Stopping;
            }
        }
    }
    pub fn poll(&mut self) {
        for run in &mut self.runs {
            if run.state == State::Running && run.control.requested() {
                run.state = State::Stopping;
            }
        }
        let events: Vec<_> = self
            .log_rx
            .try_iter()
            .chain(self.state_rx.try_iter())
            .collect();
        for event in events {
            match event {
                Event::Output(id, err, text) => {
                    if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
                        r.lines.append(err, &text);
                    }
                }
                Event::Error(id, text) => {
                    if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
                        r.lines.append(true, &text);
                    }
                }
                Event::Prepared(prepared) => {
                    let Prepared {
                        id,
                        project,
                        script,
                        logs,
                        result,
                    } = *prepared;
                    let Some(run) = self.runs.iter().find(|r| r.id == id) else {
                        continue;
                    };
                    let cancelled = run.control.requested();
                    let journal = self.journal(id, &run.log_path);
                    let result = if cancelled {
                        Err("已取消启动".to_string())
                    } else {
                        result
                            .and_then(|_| self.spawn(&project, &script, logs, Some(id)).map(|_| ()))
                    };
                    if let Err(e) = result {
                        eprintln!("[dev start] 启动检查失败: {e}");
                        journal.error(format!("启动未执行：{e}"));
                        if let Some(run) = self.runs.iter().find(|r| r.id == id) {
                            run.control.finish(Err(e.clone()));
                        }
                        let console_id =
                            self.runs.iter().find(|r| r.id == id).map(|r| r.console_id);
                        if let Some(old_index) = self.runs.iter().position(|r| {
                            Some(r.console_id) == console_id
                                && r.id != id
                                && r.state.active()
                                && !r.control.requested()
                        }) {
                            let old = &self.runs[old_index];
                            self.journal(old.id, &old.log_path)
                                .error("本次重启准备失败，原实例继续运行；失败详情见重启日志");
                            self.journal(old.id, &old.log_path).error(&e);
                            self.runs[old_index].tab_open = true;
                            self.runs[old_index].superseded = false;
                            if let Some(new) = self.runs.iter_mut().find(|r| r.id == id) {
                                new.tab_open = false;
                                new.superseded = true;
                            }
                        }
                        let _ = self.state_tx.send(Event::Finished(
                            id,
                            if cancelled {
                                State::Stopped
                            } else {
                                State::Failed
                            },
                            None,
                        ));
                    }
                }
                Event::Finished(id, state, exit) => {
                    if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
                        r.state = state;
                        if r.close_requested {
                            if state == State::Failed {
                                self.error = Some(
                                    "停止未成功，标签已保留；请检查 Console 中的具体原因".into(),
                                );
                            } else {
                                r.tab_open = false;
                            }
                            r.close_requested = false;
                        }
                        r.exit = exit;
                        r.elapsed = r.started.elapsed();
                        if r.external.is_none() && r.pid > 0 {
                            r.lines.append(
                                state == State::Failed,
                                &format!(
                                    "\n[dev start] {} · {}\n",
                                    state.label(),
                                    exit_label(exit)
                                ),
                            );
                        }
                    }
                }
            }
        }
        self.trim_history();
        self.trim_console_budget();
        self.ports.poll(&mut self.runs);
    }
    pub fn refresh_external(&mut self, projects: &[crate::model::Project], logs: PathBuf) {
        if let Some(rx) = &self.scan {
            match rx.try_recv() {
                Ok(result) => {
                    self.scan = None;
                    match result {
                        Ok(candidates) => {
                            for candidate in candidates {
                                if !self.runs.iter().any(|r| {
                                    r.state.active()
                                        && (r.pid == candidate.process.pid
                                            || r.associated_scripts
                                                .iter()
                                                .any(|id| candidate.scripts.contains(id)))
                                }) && let Err(e) = self.adopt(candidate, &logs)
                                {
                                    self.error = Some(e);
                                }
                            }
                        }
                        Err(e) => {
                            eprintln!("[dev start] 外部运行扫描: {e}");
                            self.error = Some(format!("外部运行扫描失败: {e}"));
                        }
                    }
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.scan = None;
                    self.error = Some("外部运行扫描线程异常退出".into());
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if self.scan.is_none()
            && self
                .scanned
                .is_none_or(|t| t.elapsed() > Duration::from_secs(4))
        {
            self.scanned = Some(Instant::now());
            let projects = projects.to_vec();
            let excluded: HashSet<_> = self
                .runs
                .iter()
                .filter(|r| r.state.active())
                .map(|r| r.pid)
                .collect();
            let (tx, rx) = mpsc::channel();
            self.scan = Some(rx);
            thread::spawn(move || {
                let _ = tx.send(crate::external::discover(&projects, &excluded));
            });
        }
    }
    pub fn adopt(
        &mut self,
        candidate: crate::external::Candidate,
        logs: &std::path::Path,
    ) -> Result<u64, String> {
        if !crate::external::same_process(&candidate.process)? {
            return Err("识别的外部进程已变化，取消登记".into());
        }
        let id = crate::model::unique_id();
        let log_path = logs.join(format!("run-{id}.log"));
        let process = candidate.process;
        let message = format!(
            "[dev start] 识别外部运行 · PID {} · 端口 {}\n[cwd] {}\n参数未识别；原进程输出由原终端管理。重启后可在此查看实时输出。\n",
            process.pid,
            candidate.port,
            process.cwd.display()
        );
        crate::log_store::append(&log_path, message.as_bytes())
            .map_err(|e| format!("外部运行日志: {e}"))?;
        let control = Control::default();
        let journal = self.journal(id, &log_path);
        let started_at = match crate::time_display::process_start(&process.started) {
            Ok(time) => Some(time),
            Err(error) => {
                self.journal(id, &log_path).error(error);
                None
            }
        };
        let elapsed_offset = started_at
            .and_then(|t| SystemTime::now().duration_since(t).ok())
            .unwrap_or_default();
        self.runs.push(Run {
            console_id: id,
            superseded: false,
            tab_open: true,
            close_requested: false,
            id,
            script_id: *candidate.scripts.first().ok_or("外部运行缺少启动项")?,
            associated_scripts: candidate.scripts,
            title: format!("{} / 外部运行（参数未识别）", candidate.project),
            command: process.command.clone(),
            display_command: process
                .command
                .split_whitespace()
                .next()
                .and_then(|p| std::path::Path::new(p).file_name())
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "外部进程".into()),
            launch_name: format!("{} · 外部运行", candidate.project),
            resource_key: None,
            pid: process.pid,
            cwd: process.cwd.clone(),
            expected_port: Some(candidate.port),
            listening_ports: vec![candidate.port],
            external: Some(process.clone()),
            state: State::Running,
            started: Instant::now(),
            started_at,
            elapsed_offset,
            elapsed: Duration::ZERO,
            exit: None,
            log_path: log_path.clone(),
            lines: {
                let mut buffer = crate::console_buffer::ConsoleBuffer::default();
                buffer.append(false, &message);
                buffer
            },
            control: control.clone(),
        });
        let tx = self.state_tx.clone();
        thread::spawn(move || {
            let result = monitor_external(&process, &control, &journal);
            let completion = result.as_ref().map(|_| ()).map_err(Clone::clone);
            let (state, text) = match result {
                Ok(stopped) => (
                    if stopped {
                        State::Stopped
                    } else {
                        State::Succeeded
                    },
                    if stopped {
                        "外部进程已停止".to_string()
                    } else {
                        "外部进程已退出".to_string()
                    },
                ),
                Err(e) => (State::Failed, e),
            };
            if let Err(error) =
                crate::log_store::append(&log_path, format!("[dev start] {text}\n").as_bytes())
            {
                let _ = tx.send(Event::Error(id, format!("日志写入失败: {error}")));
            }
            let _ = tx.send(Event::Output(
                id,
                state == State::Failed,
                format!("\n[dev start] {text}\n"),
            ));
            let _ = tx.send(Event::Finished(id, state, None));
            control.finish(completion);
        });
        Ok(id)
    }
    pub fn trim_console_budget(&mut self) {
        use crate::console_buffer::{MAX_TOTAL_BYTES, MAX_TOTAL_LINES};
        let mut bytes: usize = self.runs.iter().map(|r| r.lines.bytes()).sum();
        let mut lines: usize = self.runs.iter().map(|r| r.lines.len()).sum();
        for active in [false, true] {
            for run in self.runs.iter_mut().filter(|r| r.state.active() == active) {
                while bytes > MAX_TOTAL_BYTES || lines > MAX_TOTAL_LINES {
                    let before = run.lines.bytes();
                    if !run.lines.evict_oldest() {
                        break;
                    }
                    bytes -= before - run.lines.bytes();
                    lines -= 1;
                }
            }
        }
    }
    pub fn clear(&mut self, id: u64) {
        if let Some(r) = self.runs.iter_mut().find(|r| r.id == id) {
            r.lines.clear();
        }
    }
}
impl Drop for Runtime {
    fn drop(&mut self) {
        for run in &mut self.runs {
            if run.external.is_none() && run.state.active() {
                run.control.request();
                run.state = State::Stopping;
            }
        }
        let deadline = Instant::now()
            + crate::lifecycle::GRACE
            + crate::lifecycle::FORCE_WAIT
            + Duration::from_millis(300);
        while Instant::now() < deadline
            && self
                .runs
                .iter()
                .any(|r| r.external.is_none() && r.state.active())
        {
            self.poll();
            thread::sleep(Duration::from_millis(30));
        }
        for r in &self.runs {
            if r.external.is_none() && r.state.active() && r.pid > 0 {
                let _ = signal(r.pid, libc::SIGKILL);
            }
        }
    }
}
fn pump<R: Read + Send + 'static>(
    mut reader: R,
    err: bool,
    id: u64,
    tx: SyncSender<Event>,
    control: Sender<Event>,
    file: Arc<Mutex<crate::log_store::Writer>>,
) -> Receiver<()> {
    let (done_tx, done_rx) = mpsc::channel();
    thread::spawn(move || {
        let mut buffer = [0u8; 8192];
        let mut pending = String::new();
        let mut skipped = 0;
        let mut decoder = crate::output::Decoder::default();
        loop {
            match reader.read(&mut buffer) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut f) = file.lock()
                        && let Err(e) = f.write_all(&buffer[..n])
                    {
                        let _ = control.send(Event::Error(id, format!("日志写入失败: {e}")));
                        break;
                    }
                    let text = decoder.push(&buffer[..n]);
                    if err {
                        let _ = std::io::stderr().write_all(text.as_bytes());
                    } else {
                        let _ = std::io::stdout().write_all(text.as_bytes());
                    }
                    pending.push_str(&text);
                    match tx.try_send(Event::Output(id, err, std::mem::take(&mut pending))) {
                        Ok(()) => {}
                        Err(mpsc::TrySendError::Full(Event::Output(_, _, text))) => {
                            skipped += n;
                            let mut from = text.len().saturating_sub(8192);
                            while !text.is_char_boundary(from) {
                                from += 1;
                            }
                            pending = text[from..].to_string();
                        }
                        Err(_) => {}
                    }
                }
                Err(e) => {
                    let _ = control.send(Event::Error(id, format!("读取输出: {e}")));
                    break;
                }
            }
        }
        let tail = decoder.finish();
        pending.push_str(&tail);
        if !pending.is_empty() {
            let _ = control.send(Event::Output(id, err, pending));
        }
        if skipped > 0 {
            let _ = control.send(Event::Error(
                id,
                format!("Console 队列繁忙，{skipped} 字节未实时显示，已保留末尾片段；原始输出已写入有保留上限的日志文件。"),
            ));
        }
        let _ = done_tx.send(());
    });
    done_rx
}
fn group_alive(pid: u32) -> bool {
    unsafe { libc::kill(-(pid as i32), 0) == 0 }
}
fn signal(pid: u32, sig: i32) -> Result<(), String> {
    if unsafe { libc::kill(-(pid as i32), sig) } == 0 {
        Ok(())
    } else {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() == Some(libc::ESRCH) {
            Ok(())
        } else {
            Err(format!("停止进程组 {pid}: {e}"))
        }
    }
}

pub fn exit_label(exit: Option<i32>) -> String {
    exit.map(|code| format!("退出码 {code}"))
        .unwrap_or_else(|| "信号终止".into())
}

fn monitor_external(
    process: &crate::external::Process,
    control: &Control,
    journal: &Journal,
) -> Result<bool, String> {
    loop {
        if control.requested() {
            crate::lifecycle::terminate_external(process, &|s| control.note(journal, s))?;
            return Ok(true);
        }
        if !crate::external::same_process(process)? {
            return Ok(false);
        }
        thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
#[test]
fn console_pump_keeps_the_final_fragment_when_the_display_queue_is_full() {
    let path =
        std::env::temp_dir().join(format!("dev-start-pump-{}.log", crate::model::unique_id()));
    let file = Arc::new(Mutex::new(crate::log_store::Writer::open(&path).unwrap()));
    let (tx, _rx) = mpsc::sync_channel(1);
    let (state_tx, state_rx) = mpsc::channel();
    let payload = "中\n".repeat(9000) + "LAST_OUTPUT\n";
    let done = pump(
        std::io::Cursor::new(payload.as_bytes().to_vec()),
        false,
        1,
        tx,
        state_tx,
        file,
    );
    done.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        state_rx
            .try_iter()
            .any(|event| matches!(event,Event::Output(_,_,text) if text.contains("LAST_OUTPUT")))
    );
    assert_eq!(std::fs::read_to_string(&path).unwrap(), payload);
    std::fs::remove_file(path).unwrap();
}
