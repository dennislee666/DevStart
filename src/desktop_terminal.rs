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
}
pub struct Manager {
    pub sessions: Vec<Session>,
    pub selected: Option<u64>,
    pub palette: Palette,
    loaded: bool,
    tx: Sender<(u64, PtyEvent)>,
    rx: Receiver<(u64, PtyEvent)>,
}
impl Default for Manager {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            sessions: vec![],
            selected: None,
            palette: Palette::Mac,
            loaded: false,
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
    pub fn save(&self, root: &std::path::Path) -> Result<(), String> {
        let value = serde_json::json!({"palette":self.palette,"tabs":self.sessions.len(),"selected_index":self.sessions.iter().position(|s|Some(s.id)==self.selected).unwrap_or(0)});
        let target = root.join("desktop-terminal-state.json");
        let temp = root.join("desktop-terminal-state.json.tmp");
        std::fs::write(
            &temp,
            serde_json::to_vec_pretty(&value).map_err(|e| e.to_string())?,
        )
        .and_then(|_| std::fs::rename(temp, target))
        .map_err(|e| format!("保存终端状态：{e}"))
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
                shell,
                args: vec!["-l".into()],
                working_directory: Some(home.into()),
            },
        )
        .map_err(|e| e.to_string())?;
        self.loaded = true;
        self.sessions.insert(
            0,
            Session {
                id,
                name: format!("终端 {}", id % 100000),
                backend,
                exited: false,
            },
        );
        self.selected = Some(id);
        Ok(())
    }
    pub fn poll(&mut self) {
        while let Ok((id, event)) = self.rx.try_recv() {
            if let Some(session) = self.sessions.iter_mut().find(|s| s.id == id) {
                match event {
                    PtyEvent::Exit => session.exited = true,
                    PtyEvent::Title(title) => {
                        if !title.trim().is_empty() {
                            session.name = title;
                        }
                    }
                    PtyEvent::PtyWrite(text) => session
                        .backend
                        .process_command(egui_term::BackendCommand::Write(text.into_bytes())),
                    _ => {}
                }
            }
        }
    }
    pub fn close(&mut self, id: u64) {
        self.sessions.retain(|s| s.id != id);
        if self.selected == Some(id) {
            self.selected = self.sessions.first().map(|s| s.id);
        }
    }
    pub fn describe(&mut self) -> serde_json::Value {
        self.poll();
        serde_json::json!({"selected":self.selected,"palette":self.palette,"sessions":self.sessions.iter().map(|s|serde_json::json!({"id":s.id,"name":s.name,"active":!s.exited})).collect::<Vec<_>>()})
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
