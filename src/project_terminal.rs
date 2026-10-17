//! Project-scoped, bounded PTY sessions. No shell command is constructed from user text.
use serde::Serialize;
use std::{
    collections::HashMap,
    fs::File,
    io::{Read, Write},
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::process::CommandExt,
    },
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
};
#[derive(Default)]
struct Screen {
    bytes: Vec<u8>,
    colors: crate::terminal_style::Colors,
    csi: Vec<u8>,
    escape: u8,
    osc: Vec<u8>,
    ready: u64,
    exit: i32,
    rewrite: bool,
}
impl Screen {
    fn feed(&mut self, input: &[u8]) {
        for &b in input {
            match self.escape {
                1 => {
                    self.escape = match b {
                        b'[' => 2,
                        b']' => 3,
                        _ => 0,
                    };
                }
                2 => {
                    if (0x40..=0x7e).contains(&b) {
                        if b == b'm' {
                            self.colors.sgr(&self.csi);
                        }
                        self.csi.clear();
                        self.escape = 0;
                    } else if self.csi.len() < 128 {
                        self.csi.push(b);
                    }
                }
                3 => {
                    if b == 7 {
                        self.finish_osc();
                        self.escape = 0;
                    } else if b == 27 {
                        self.escape = 4;
                    } else if self.osc.len() < 1024 {
                        self.osc.push(b);
                    }
                }
                4 => {
                    if b == b'\\' {
                        self.finish_osc();
                        self.escape = 0;
                    } else {
                        self.escape = 3;
                    }
                }
                _ => match b {
                    27 => {
                        self.escape = 1;
                        self.osc.clear();
                        self.csi.clear();
                    }
                    b'\r' => self.rewrite = true,
                    b'\n' => {
                        self.colors.push(self.bytes.len());
                        self.bytes.push(b);
                        self.rewrite = false;
                    }
                    8 | 127 => {
                        self.bytes.pop();
                        self.colors.truncate(self.bytes.len());
                    }
                    b'\t' | 32..=126 | 128..=255 => {
                        if self.rewrite {
                            while self.bytes.last().is_some_and(|v| *v != b'\n') {
                                self.bytes.pop();
                            }
                            self.rewrite = false;
                            self.colors.truncate(self.bytes.len());
                        }
                        self.colors.push(self.bytes.len());
                        self.bytes.push(b);
                    }
                    _ => {}
                },
            }
        }
        self.colors.fed(&self.bytes);
        if self.bytes.len() > 262144 {
            let mut start = self.bytes.len() - 262144;
            while start < self.bytes.len() && self.bytes[start] & 0xc0 == 0x80 {
                start += 1;
            }
            self.colors.trim(start);
            self.bytes.drain(..start);
        }
        let lines = self.bytes.iter().filter(|b| **b == b'\n').count();
        if lines > 3000 {
            let mut remaining = lines - 3000;
            let mut end = 0;
            for (i, b) in self.bytes.iter().enumerate() {
                if *b == b'\n' {
                    remaining -= 1;
                    if remaining == 0 {
                        end = i + 1;
                        break;
                    }
                }
            }
            self.colors.trim(end);
            self.bytes.drain(..end);
        }
    }
    fn finish_osc(&mut self) {
        if let Some(code) = self.osc.strip_prefix(b"777;devstart;")
            && let Ok(s) = std::str::from_utf8(code)
            && let Ok(code) = s.parse()
        {
            self.ready += 1;
            self.colors.start_prompt(self.bytes.len());
            self.exit = code;
        }
        self.osc.clear();
    }
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub id: u64,
    pub project_id: u64,
    pub directory: PathBuf,
    pub text: String,
    pub segments: Vec<crate::terminal_style::Segment>,
    pub prompt: String,
    pub active: bool,
    pub busy: bool,
    pub echo: bool,
    pub generation: u64,
    pub exit_code: i32,
    pub history: Vec<String>,
}
struct Session {
    id: u64,
    project: u64,
    directory: PathBuf,
    child: Option<Child>,
    master: File,
    input: std::sync::mpsc::SyncSender<Vec<u8>>,
    screen: Arc<Mutex<Screen>>,
    history: Vec<String>,
    waiting: Option<u64>,
}
impl Drop for Session {
    fn drop(&mut self) {
        let Some(mut child) = self.child.take() else {
            return;
        };
        if child.try_wait().ok().flatten().is_some() {
            return;
        }
        let pid = child.id() as i32;
        // HUP closes interactive shells; TERM also reaches foreground children. Reap before return.
        unsafe {
            libc::kill(-pid, libc::SIGHUP);
            libc::kill(-pid, libc::SIGTERM);
        }
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        loop {
            let _ = child.try_wait();
            if unsafe { libc::kill(-pid, 0) } != 0 {
                break;
            }
            if std::time::Instant::now() >= deadline {
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let _ = child.wait();
    }
}
#[derive(Default)]
pub struct Manager {
    sessions: HashMap<u64, Session>,
}
impl Manager {
    pub fn retain_projects(&mut self, ids: &[u64]) {
        self.sessions.retain(|_, s| ids.contains(&s.project));
    }
    pub fn open(&mut self, project: u64, path: &Path) -> Result<u64, String> {
        for s in self.sessions.values_mut() {
            if s.project == project
                && s.child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .map_err(|e| e.to_string())?
                    .is_none()
            {
                return Ok(s.id);
            }
        }
        let closed: Vec<_> = self
            .sessions
            .iter_mut()
            .filter_map(|(id, s)| {
                s.child
                    .as_mut()
                    .unwrap()
                    .try_wait()
                    .ok()
                    .flatten()
                    .map(|_| *id)
            })
            .collect();
        for id in closed {
            self.sessions.remove(&id);
        }
        if self.sessions.len() >= 8 {
            return Err("At most eight project terminal sessions may be open".into());
        }
        let path = path.canonicalize().map_err(|e| e.to_string())?;
        let (mut master, mut slave) = (-1, -1);
        let mut size = libc::winsize {
            ws_row: 30,
            ws_col: 100,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // The descriptors are immediately owned by File; CLOEXEC prevents unrelated launches inheriting them.
        if unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut size,
            )
        } != 0
        {
            return Err(std::io::Error::last_os_error().to_string());
        }
        let (master, slave) = unsafe { (File::from_raw_fd(master), File::from_raw_fd(slave)) };
        unsafe {
            libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
            libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        }
        let shell = std::env::var_os("SHELL").unwrap_or_else(|| "/bin/sh".into());
        let mut command = Command::new(shell);
        // Login initialization supplies development PATH; the positional directory cannot become shell code.
        command
            .args([
                "-lc",
                "cd -- \"$1\" && exec /bin/bash --noprofile --norc --noediting -i +m",
                "devstart-terminal",
            ])
            .arg(&path)
            .current_dir(&path)
            .env("BASH_SILENCE_DEPRECATION_WARNING","1")
            .env("HISTFILE","/dev/null").env("HISTSIZE","0").env("HISTFILESIZE","0")
            .env("TERM", "xterm-256color").env("COLORTERM","truecolor").env("CLICOLOR","1")
            .env("PS1", "\\w $ ")
            .env(
                "PROMPT_COMMAND",
                r#"__devstart_status=$?; PS1='\w $ '; printf '\033[0m\033]777;devstart;%s\007' "$__devstart_status""#,
            )
            .env("GIT_PAGER", "cat")
            .env("PAGER", "cat")
            .env("GIT_EDITOR", "false")
            .stdin(Stdio::from(slave.try_clone().map_err(|e| e.to_string())?))
            .stdout(Stdio::from(slave.try_clone().map_err(|e| e.to_string())?))
            .stderr(Stdio::from(slave));
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                if libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let mut reader = master.try_clone().map_err(|e| e.to_string())?;
        let mut writer = master.try_clone().map_err(|e| e.to_string())?;
        let child = command.spawn().map_err(|e| e.to_string())?;
        let screen = Arc::new(Mutex::new(Screen::default()));
        let output = screen.clone();
        let (input, queue) = std::sync::mpsc::sync_channel::<Vec<u8>>(4);
        std::thread::spawn(move || {
            while let Ok(bytes) = queue.recv() {
                if let Err(e) = writer.write_all(&bytes) {
                    eprintln!("[dev start terminal input] {e}");
                    break;
                }
            }
        });
        std::thread::spawn(move || {
            let mut bytes = [0u8; 4096];
            loop {
                match reader.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(n) => {
                        if let Ok(mut out) = output.lock() {
                            out.feed(&bytes[..n]);
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(e) => {
                        if e.raw_os_error() != Some(libc::EIO) {
                            eprintln!("[dev start terminal] {e}");
                        }
                        break;
                    }
                }
            }
        });
        let id = crate::model::unique_id();
        self.sessions.insert(
            id,
            Session {
                id,
                project,
                directory: path,
                child: Some(child),
                master,
                input,
                screen,
                history: vec![],
                waiting: None,
            },
        );
        Ok(id)
    }
    pub fn read(&mut self, id: u64) -> Result<Snapshot, String> {
        let s = self
            .sessions
            .get_mut(&id)
            .ok_or("Terminal session not found")?;
        let active = s
            .child
            .as_mut()
            .unwrap()
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_none();
        let mut screen = s.screen.lock().map_err(|e| e.to_string())?;
        let mut term: libc::termios = unsafe { std::mem::zeroed() };
        let echo = unsafe { libc::tcgetattr(s.master.as_raw_fd(), &mut term) } == 0
            && term.c_lflag & libc::ECHO != 0;
        let busy = active && (screen.ready == 0 || s.waiting.is_some_and(|g| screen.ready <= g));
        let mut text = String::from_utf8_lossy(&screen.bytes).into_owned();
        let mut prompt = String::new();
        if !busy && screen.ready > 0 {
            let start = text.rfind(char::from(10)).map_or(0, |i| i + 1);
            if text[start..].ends_with(" $ ") {
                prompt = text[start..].to_owned();
                text.truncate(start);
            }
        }
        let end = text.len().min(screen.bytes.len());
        let bytes = std::mem::take(&mut screen.bytes);
        let segments = screen.colors.segments(&bytes, end);
        screen.bytes = bytes;
        Ok(Snapshot {
            id,
            project_id: s.project,
            directory: s.directory.clone(),
            text,
            segments,
            prompt,
            active,
            busy,
            echo,
            generation: screen.ready,
            exit_code: screen.exit,
            history: s.history.clone(),
        })
    }
    pub fn line(&mut self, id: u64, text: &str) -> Result<(), String> {
        if text.len() > 8192 || text.contains(['\n', '\r']) {
            return Err("Terminal input must be a single line up to 8192 bytes".into());
        }
        let snapshot = self.read(id)?;
        if snapshot.generation == 0 {
            return Err("Terminal is initializing; wait for the first prompt".into());
        }
        if !snapshot.active {
            return Err("Terminal has exited".into());
        }
        let s = self.sessions.get_mut(&id).unwrap();
        if snapshot.echo && !text.trim().is_empty() {
            s.history.push(text.into());
            if s.history.len() > 100 {
                s.history.remove(0);
            }
        }
        if snapshot.echo {
            let mut screen = s.screen.lock().map_err(|e| e.to_string())?;
            let end = screen.bytes.len();
            screen
                .colors
                .expect_input(end, text, !snapshot.prompt.is_empty());
        }
        let mut bytes = text.as_bytes().to_vec();
        bytes.push(10);
        s.input
            .try_send(bytes)
            .map_err(|e| format!("Terminal input queue: {e}"))?;
        s.waiting = Some(snapshot.generation);
        Ok(())
    }
    pub fn interrupt(&mut self, id: u64) -> Result<(), String> {
        self.sessions
            .get_mut(&id)
            .ok_or("Terminal session not found")?
            .master
            .write_all(&[3])
            .map_err(|e| e.to_string())
    }
    pub fn clear(&mut self, id: u64) -> Result<(), String> {
        let prompt = self.read(id)?.prompt;
        let mut screen = self
            .sessions
            .get_mut(&id)
            .ok_or("Terminal session not found")?
            .screen
            .lock()
            .map_err(|e| e.to_string())?;
        screen.bytes.clear();
        screen.colors.clear();
        if !prompt.is_empty() {
            screen.bytes.extend_from_slice(prompt.as_bytes());
            screen.colors.start_prompt(0);
        }
        Ok(())
    }
    pub fn close(&mut self, id: u64) -> Result<(), String> {
        self.sessions
            .remove(&id)
            .ok_or("Terminal session not found")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn wait(manager: &mut Manager, id: u64, predicate: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            let s = manager.read(id).unwrap();
            if predicate(&s) {
                return s;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "terminal timed out: {}",
                s.text
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
    }
    #[test]
    fn persistent_pty_uses_project_directory_streams_and_interrupts() {
        let path = std::env::temp_dir().join(format!("devstart-pty-{}", crate::model::unique_id()));
        std::fs::create_dir_all(path.join("nested")).unwrap();
        let mut manager = Manager::default();
        let id = manager.open(1, &path).unwrap();
        wait(&mut manager, id, |s| {
            s.generation > 0 && !s.prompt.is_empty()
        });
        manager.line(id, "pwd; printf 'TERMINAL_READY\\n'").unwrap();
        let s = wait(&mut manager, id, |s| s.generation >= 2);
        assert!(s.text.contains(path.file_name().unwrap().to_str().unwrap()));
        assert!(s.text.contains("TERMINAL_READY"));
        manager.line(id, "cd nested").unwrap();
        wait(&mut manager, id, |s| s.generation >= 3);
        manager.line(id, "pwd").unwrap();
        let s = wait(&mut manager, id, |s| s.generation >= 4);
        assert!(s.text.contains("nested"));
        manager.line(id, "printf 'STREAMING\\n'; sleep 20").unwrap();
        wait(&mut manager, id, |s| s.busy && s.text.contains("STREAMING"));
        manager.interrupt(id).unwrap();
        let s = wait(&mut manager, id, |s| s.generation >= 5);
        assert!(s.active);
        assert!(s.history.iter().any(|c| c == "pwd"));
        assert_eq!(manager.open(1, &path).unwrap(), id);
        manager
            .line(id, "printf '\\033[32mCOLOR_PROBE\\033[0m\\n'")
            .unwrap();
        let colored = wait(&mut manager, id, |s| s.generation >= 6);
        assert!(
            colored
                .segments
                .iter()
                .any(|s| s.text.contains("COLOR_PROBE") && s.style.color == Some([100, 195, 140]))
        );
        assert!(
            colored
                .segments
                .iter()
                .any(|s| s.role == crate::terminal_style::Role::Input)
        );
        manager.close(id).unwrap();
        assert!(manager.read(id).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
    #[test]
    fn terminal_transcript_is_bounded_and_escape_sequences_are_removed() {
        let mut screen = Screen::default();
        screen.feed(b"\x1b[32mhello\x1b[0m\n\x1b]777;devstart;0\x07");
        assert_eq!(screen.bytes, b"hello\n");
        assert_eq!(screen.ready, 1);
        screen.feed(&vec![b'x'; 300000]);
        assert!(screen.bytes.len() <= 262144);
        for _ in 0..4000 {
            screen.feed(b"line\n");
        }
        assert!(screen.bytes.iter().filter(|b| **b == b'\n').count() <= 3000);
    }
    #[test]
    fn ansi_segments_survive_chunking_resets_and_line_rewrites() {
        let mut screen = Screen::default();
        screen.feed(b"\x1b[3");
        screen.feed(b"1mRED\x1b[0m plain\n");
        let segments = screen.colors.segments(&screen.bytes, screen.bytes.len());
        assert_eq!(
            segments.iter().map(|s| s.text.as_str()).collect::<String>(),
            "RED plain\n"
        );
        assert_eq!(segments[0].style.color, Some([220, 100, 100]));
        assert_eq!(segments.last().unwrap().style.color, None);
        let mut screen = Screen::default();
        screen.feed(b"\x1b[38;5;196mold\rnew\x1b[0m\n\x1b[1;3;4;38;2;1;2;3mRGB\x1b[0m");
        let segments = screen.colors.segments(&screen.bytes, screen.bytes.len());
        assert_eq!(
            segments.iter().map(|s| s.text.as_str()).collect::<String>(),
            "new\nRGB"
        );
        assert_eq!(segments[0].style.color, Some([255, 0, 0]));
        let last = segments.last().unwrap();
        assert_eq!(last.style.color, Some([1, 2, 3]));
        assert!(last.style.bold && last.style.italic && last.style.underline);
    }
    #[test]
    fn terminal_semantic_roles_distinguish_own_input_and_explicit_output_prefixes() {
        use crate::terminal_style::Role;
        let mut screen = Screen::default();
        screen.feed(b"\x1b]777;devstart;0\x07/project $ ");
        screen
            .colors
            .expect_input(screen.bytes.len(), "echo hello", true);
        screen.feed(b"echo hello\r\nhello\nwarning: caution\nerror: failed\nAlready up to date.\n");
        let segments = screen.colors.segments(&screen.bytes, screen.bytes.len());
        assert!(
            segments
                .iter()
                .any(|s| s.role == Role::Prompt && s.text.contains("/project"))
        );
        assert!(
            segments
                .iter()
                .any(|s| s.role == Role::Input && s.text.contains("echo hello"))
        );
        assert!(segments.iter().any(|s| s.role == Role::Warning));
        assert!(segments.iter().any(|s| s.role == Role::Error));
        assert!(segments.iter().any(|s| s.role == Role::Success));
    }
}
