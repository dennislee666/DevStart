//! Independent login-shell sessions; tab order and palette are application-owned state.
use egui_term::{BackendSettings, PtyEvent, TerminalBackend};
use serde::{Deserialize, Serialize};
use std::sync::mpsc::{self, Receiver, Sender};
#[derive(Default, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Palette {
    #[default]
    Mac,
    Linux,
}
pub struct Session {
    pub id: u64,
    pub name: String,
    pub backend: TerminalBackend,
    pub exited: bool,
    pub process: Option<crate::external::Process>,
    shell_name: String,
}
pub struct Manager {
    pub sessions: Vec<Session>,
    pub selected: Option<u64>,
    pub palette: Palette,
    pub font_size: f32,
    pub background: Option<String>,
    pub tiled: bool,
    pub pending_close: Option<u64>,
    pub pending_quit: bool,
    pub quit_ready: bool,
    quit_checking: bool,
    closing: std::collections::HashSet<u64>,
    close_tx: Sender<(u64, Result<bool, String>)>,
    close_rx: Receiver<(u64, Result<bool, String>)>,
    pub close_error: Option<String>,
    sequence: u64,
    pub dirty: bool,
    loaded: bool,
    tx: Sender<(u64, PtyEvent)>,
    rx: Receiver<(u64, PtyEvent)>,
}
impl Default for Manager {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let (close_tx, close_rx) = mpsc::channel();
        Self {
            sessions: vec![],
            selected: None,
            palette: Palette::Mac,
            font_size: 14.0,
            background: None,
            loaded: false,
            tiled: false,
            pending_close: None,
            pending_quit: false,
            quit_ready: false,
            quit_checking: false,
            closing: Default::default(),
            close_tx,
            close_rx,
            close_error: None,
            sequence: 0,
            dirty: false,
            tx,
            rx,
        }
    }
}
impl Manager {
    pub fn initialize(
        &mut self,
        ctx: &eframe::egui::Context,
        root: &std::path::Path,
    ) -> Result<(), String> {
        if self.loaded {
            return Ok(());
        }
        self.loaded = true;
        let path = root.join("desktop-terminal-state.json");
        match std::fs::read(&path) {
            Ok(data) => {
                let value: serde_json::Value =
                    serde_json::from_slice(&data).map_err(|e| format!("读取终端状态：{e}"))?;
                self.palette =
                    serde_json::from_value(value["palette"].clone()).map_err(|e| e.to_string())?;
                let count = value["tabs"].as_u64().unwrap_or(1).min(100);
                for _ in 0..count {
                    self.open(ctx)?;
                }
                let index = value["selected_index"].as_u64().unwrap_or(0) as usize;
                if let Some(names) = value["names"].as_array() {
                    for (session, name) in self.sessions.iter_mut().zip(names) {
                        if let Some(name) = name.as_str() {
                            session.name = name.to_owned();
                        }
                    }
                }
                self.font_size = value["font_size"]
                    .as_f64()
                    .map(|v| v as f32)
                    .filter(|v| v.is_finite())
                    .unwrap_or(14.0)
                    .clamp(10.0, 32.0);
                self.background = value["background"]
                    .as_str()
                    .filter(|v| valid_background(v))
                    .map(str::to_owned);
                self.sequence = value["sequence"].as_u64().unwrap_or(count);
                self.tiled = value["tiled"].as_bool().unwrap_or(false);
                self.selected = self
                    .sessions
                    .get(index)
                    .or_else(|| self.sessions.first())
                    .map(|s| s.id);
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => self.open(ctx)?,
            Err(e) => return Err(e.to_string()),
        }
        Ok(())
    }
    pub fn save(&mut self, root: &std::path::Path) -> Result<(), String> {
        let value = serde_json::json!({"palette":self.palette,"tabs":self.sessions.len(),"names":self.sessions.iter().map(|s|s.name.clone()).collect::<Vec<_>>(),"sequence":self.sequence,"tiled":self.tiled,"font_size":self.font_size,"background":self.background,"selected_index":self.sessions.iter().position(|s|Some(s.id)==self.selected).unwrap_or(0)});
        let target = root.join("desktop-terminal-state.json");
        let temp = root.join("desktop-terminal-state.json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .and_then(|_| std::fs::rename(temp, target))
        .map_err(|e| format!("保存终端状态：{e}"))?;
        self.dirty = false;
        Ok(())
    }
    pub fn open(&mut self, ctx: &eframe::egui::Context) -> Result<(), String> {
        let home = std::env::var_os("HOME").ok_or("无法确定当前用户主目录")?;
        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".into());
        let id = crate::model::unique_id();
        let backend = TerminalBackend::new(
            id,
            ctx.clone(),
            self.tx.clone(),
            BackendSettings {
                shell: shell.clone(),
                args: vec!["-l".into()],
                working_directory: Some(home.clone().into()),
            },
        )
        .map_err(|e| e.to_string())?;
        // Capture only stable identity here. Full cwd/process-tree inspection belongs to close workers.
        let output = std::process::Command::new("/bin/ps")
            .env("LC_ALL", "C")
            .args(["-p", &backend.shell_pid.to_string(), "-o", "lstart="])
            .output()
            .map_err(|e| e.to_string())?;
        let started = String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        if !output.status.success() || started.is_empty() {
            return Err("无法读取终端进程身份".into());
        }
        let process = Some(crate::external::Process {
            pid: backend.shell_pid,
            parent: 0,
            started,
            command: shell.clone(),
            cwd: home.into(),
        });
        self.sequence += 1;
        let mut random = [0u8; 1];
        use std::io::Read;
        std::fs::File::open("/dev/urandom")
            .and_then(|mut f| f.read_exact(&mut random))
            .map_err(|e| e.to_string())?;
        let symbols = ["❥", "★", "◇", "●", "↓", "→", "◆", "○"];
        self.loaded = true;
        self.sessions.insert(
            0,
            Session {
                id,
                name: format!(
                    "{}终端{:03}",
                    symbols[random[0] as usize % symbols.len()],
                    self.sequence
                ),
                backend,
                exited: false,
                process,
                shell_name: std::path::Path::new(&shell)
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
            },
        );
        self.selected = Some(id);
        Ok(())
    }
    pub fn poll(&mut self) {
        while let Ok((id, result)) = self.close_rx.try_recv() {
            if id == 0 {
                self.quit_checking = false;
                match result {
                    Ok(true) => self.pending_quit = true,
                    Ok(false) => self.quit_ready = true,
                    Err(e) => self.close_error = Some(e),
                }
                continue;
            }
            self.closing.remove(&id);
            match result {
                Ok(true) => self.pending_close = Some(id),
                Ok(false) => self.close(id),
                Err(e) => self.close_error = Some(e),
            }
        }
        while let Ok((id, event)) = self.rx.try_recv() {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                match event {
                    PtyEvent::Exit => session.exited = true,
                    PtyEvent::PtyWrite(text) => session
                        .backend
                        .process_command(egui_term::BackendCommand::Write(text.into_bytes())),
                    _ => {}
                }
            }
        }
    }
    pub fn request_quit(&mut self, confirmed: bool) {
        if self.quit_checking {
            return;
        }
        self.quit_checking = true;
        let sessions: Vec<_> = self
            .sessions
            .iter()
            .filter_map(|s| s.process.clone().map(|p| (p, s.shell_name.clone())))
            .collect();
        let sender = self.close_tx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<bool, String> {
                let mut processes = vec![];
                for (expected, shell_name) in sessions {
                    if let Some(process) = crate::external::process(expected.pid)?
                        .filter(|p| p.started == expected.started)
                    {
                        let shell = process
                            .command
                            .split_whitespace()
                            .next()
                            .and_then(|p| std::path::Path::new(p).file_name())
                            .is_some_and(|p| {
                                p.to_string_lossy().trim_start_matches('-') == shell_name
                            });
                        if !confirmed && (!shell || crate::external::tree(&process)?.len() > 1) {
                            return Ok(true);
                        }
                        processes.push(process);
                    }
                }
                for process in processes {
                    crate::lifecycle::terminate_external(&process, &|note| {
                        eprintln!("[dev start terminal quit] {note}")
                    })?;
                }
                Ok(false)
            })();
            let _ = sender.send((0, result));
        });
    }
    pub fn request_close(&mut self, id: u64, confirmed: bool) {
        if !self.closing.insert(id) {
            return;
        }
        let Some(session) = self.sessions.iter().find(|s| s.id == id) else {
            self.closing.remove(&id);
            return;
        };
        let process = session.process.clone();
        let shell_name = session.shell_name.clone();
        let sender = self.close_tx.clone();
        std::thread::spawn(move || {
            let result = (|| -> Result<bool, String> {
                if let Some(expected) = process {
                    if let Some(process) = crate::external::process(expected.pid)?
                        .filter(|p| p.started == expected.started)
                    {
                        if !confirmed
                            && (process
                                .command
                                .split_whitespace()
                                .next()
                                .and_then(|p| std::path::Path::new(p).file_name())
                                .is_none_or(|p| {
                                    p.to_string_lossy().trim_start_matches('-') != shell_name
                                })
                                || crate::external::tree(&process)?.len() > 1)
                        {
                            return Ok(true);
                        }
                        crate::lifecycle::terminate_external(&process, &|note| {
                            eprintln!("[dev start terminal close] {note}")
                        })?;
                    }
                }
                Ok(false)
            })();
            let _ = sender.send((id, result));
        });
    }
    pub fn close(&mut self, id: u64) {
        self.dirty = true;
        self.sessions.retain(|s| s.id != id);
        if self.selected == Some(id) {
            self.selected = self.sessions.first().map(|s| s.id);
        }
    }
    pub fn describe(&mut self) -> serde_json::Value {
        self.poll();
        serde_json::json!({"selected":self.selected,"palette":self.palette,"tiled":self.tiled,"font_size":self.font_size,"background":self.background,"pending_close":self.pending_close,"closing":self.closing,"close_error":self.close_error,"sessions":self.sessions.iter().map(|s|serde_json::json!({"id":s.id,"name":s.name,"active":!s.exited})).collect::<Vec<_>>()})
    }
    pub fn read_screen(&mut self, id: u64) -> Result<serde_json::Value, String> {
        self.poll();
        let session = self
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("终端不存在")?;
        let content = session.backend.sync();
        let mut lines = std::collections::BTreeMap::<i32, String>::new();
        for cell in content.grid.display_iter() {
            lines.entry(cell.point.line.0).or_default().push(cell.c);
        }
        let text = lines
            .values()
            .map(|s| s.trim_end())
            .collect::<Vec<_>>()
            .join("\n");
        Ok(
            serde_json::json!({"id":id,"text":text,"active":!session.exited,"scope":"visible_screen"}),
        )
    }
    pub fn write(&mut self, id: u64, text: &str) -> Result<(), String> {
        let session = self
            .sessions
            .iter_mut()
            .find(|s| s.id == id)
            .ok_or("终端不存在")?;
        if session.exited {
            return Err("终端已退出".into());
        }
        session
            .backend
            .process_command(egui_term::BackendCommand::Write(text.as_bytes().to_vec()));
        Ok(())
    }
    pub fn theme(&self) -> egui_term::TerminalTheme {
        let mut p = egui_term::ColorPalette::default();
        p.background = "#080a0c".into();
        if self.palette == Palette::Mac {
            p.foreground = "#39ff14".into();
            p.bright_foreground = Some("#75ff59".into());
            p.dim_foreground = "#249b16".into();
        } else {
            p.foreground = "#d8d8d8".into();
            p.green = "#55cc55".into();
            p.yellow = "#e5ad45".into();
            p.blue = "#5799e8".into();
            p.magenta = "#bb77d4".into();
            p.cyan = "#55c5d5".into();
        }
        if let Some(background) = &self.background {
            p.background = background.clone();
            if matches!(background.as_str(), "#f1eee6" | "#ffffff") {
                p.foreground = if self.palette == Palette::Mac {
                    "#205220".into()
                } else {
                    "#22252a".into()
                };
                p.bright_foreground = Some(p.foreground.clone());
            }
        }
        egui_term::TerminalTheme::new(Box::new(p))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn login_shell_tabs_are_independent_and_keep_ansi_colors() {
        let ctx = eframe::egui::Context::default();
        let mut m = Manager::default();
        m.open(&ctx).unwrap();
        let first = m.selected.unwrap();
        m.sessions[0]
            .backend
            .process_command(egui_term::BackendCommand::Write(
                b"export DEVSTART_TAB_TOKEN=FIRST; printf '\\033[31mCOLOR_TOKEN\\033[0m\\n'\n"
                    .to_vec(),
            ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let content: String = m.sessions[0]
                .backend
                .sync()
                .grid
                .display_iter()
                .map(|c| c.c)
                .collect();
            if content.contains("COLOR_TOKEN") {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        m.open(&ctx).unwrap();
        assert_ne!(m.selected, Some(first));
        assert_eq!(m.sessions[1].id, first);
        m.sessions[0]
            .backend
            .process_command(egui_term::BackendCommand::Write(
                b"printf 'NEW_TOKEN=%s' \"${DEVSTART_TAB_TOKEN:-EMPTY}\"\n".to_vec(),
            ));
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let content: String = m.sessions[0]
                .backend
                .sync()
                .grid
                .display_iter()
                .map(|c| c.c)
                .collect();
            if content.contains("NEW_TOKEN=EMPTY") {
                break;
            }
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        m.close(first);
        assert_eq!(m.sessions.len(), 1);
    }
}

#[cfg(test)]
mod close_tests {
    use super::*;
    #[test]
    fn busy_terminal_waits_for_confirmation_after_cd_and_stops_on_confirm() {
        let mut manager = Manager::default();
        manager.open(&eframe::egui::Context::default()).unwrap();
        let id = manager.selected.unwrap();
        let name = manager.sessions[0].name.clone();
        assert!(name.contains("终端001"));
        manager.write(id, "cd /tmp; sleep 60\n").unwrap();
        let shell = manager.sessions[0].process.clone().unwrap();
        let ready = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let current = crate::external::process(shell.pid).unwrap().unwrap();
            if crate::external::tree(&current)
                .unwrap()
                .iter()
                .any(|p| p.command.contains("sleep 60"))
            {
                break;
            }
            assert!(
                std::time::Instant::now() < ready,
                "foreground command did not start"
            );
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        manager.request_close(id, false);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while manager.pending_close != Some(id) {
            manager.poll();
            assert!(manager.close_error.is_none(), "{:?}", manager.close_error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
        assert_eq!(manager.sessions.len(), 1);
        manager.pending_close = None;
        manager.poll();
        assert_eq!(manager.sessions.len(), 1);
        manager.request_close(id, true);
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
        while !manager.sessions.is_empty() {
            manager.poll();
            assert!(manager.close_error.is_none(), "{:?}", manager.close_error);
            assert!(std::time::Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(30));
        }
    }
}

pub const BACKGROUNDS: &[(&str, &str)] = &[
    ("#080a0c", "黑色"),
    ("#182537", "深蓝"),
    ("#2b1e35", "深紫"),
    ("#193027", "深绿"),
    ("#30343b", "灰色"),
    ("#f1eee6", "米色"),
    ("#ffffff", "白色"),
];
pub fn valid_background(value: &str) -> bool {
    BACKGROUNDS.iter().any(|(color, _)| *color == value)
}
