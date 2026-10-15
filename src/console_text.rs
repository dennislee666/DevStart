//! Bounded SGR metadata and explicit, inert console link targets. No shell evaluation.
use std::{
    ops::Range,
    path::{Path, PathBuf},
};
#[derive(Clone, Copy, Default, Debug, PartialEq, Eq, Hash)]
pub struct Style {
    pub color: Option<[u8; 3]>,
    pub bold: bool,
    pub italic: bool,
    pub underline: bool,
}
#[derive(Clone, Debug)]
pub struct Span {
    pub range: Range<usize>,
    pub style: Style,
}
#[derive(Default)]
pub struct Ansi {
    style: Style,
}
impl Ansi {
    pub fn parse(&mut self, text: &str) -> Vec<(Style, String)> {
        let mut out = Vec::new();
        let mut rest = text;
        while let Some(i) = rest.find("\x1b[") {
            if i > 0 {
                out.push((self.style, rest[..i].to_owned()));
            }
            rest = &rest[i + 2..];
            let Some(end) = rest.find('m') else { break };
            if end <= 128 {
                self.sgr(&rest[..end]);
            }
            rest = &rest[end + 1..];
        }
        if !rest.is_empty() {
            out.push((self.style, rest.to_owned()));
        }
        out
    }
    fn sgr(&mut self, codes: &str) {
        let values: Vec<u16> = codes.split(';').map(|s| s.parse().unwrap_or(0)).collect();
        let mut i = 0;
        while i < values.len() {
            let n = values[i];
            match n {
                0 => self.style = Style::default(),
                1 => self.style.bold = true,
                3 => self.style.italic = true,
                4 => self.style.underline = true,
                22 => self.style.bold = false,
                23 => self.style.italic = false,
                24 => self.style.underline = false,
                30..=37 => self.style.color = Some(palette((n - 30) as u8)),
                90..=97 => self.style.color = Some(palette((n - 90 + 8) as u8)),
                39 => self.style.color = None,
                38 if values.get(i + 1) == Some(&2) && values.len() > i + 4 => {
                    self.style.color = Some([
                        values[i + 2].min(255) as u8,
                        values[i + 3].min(255) as u8,
                        values[i + 4].min(255) as u8,
                    ]);
                    i += 4;
                }
                38 if values.get(i + 1) == Some(&5) && values.len() > i + 2 => {
                    self.style.color = Some(palette(values[i + 2].min(255) as u8));
                    i += 2;
                }
                _ => {}
            }
            i += 1;
        }
    }
}
fn palette(n: u8) -> [u8; 3] {
    const BASIC: [[u8; 3]; 16] = [
        [80, 85, 90],
        [220, 100, 100],
        [100, 195, 140],
        [210, 180, 100],
        [110, 155, 225],
        [190, 135, 215],
        [95, 190, 200],
        [205, 210, 215],
        [125, 130, 140],
        [245, 135, 130],
        [145, 225, 165],
        [240, 210, 135],
        [150, 185, 250],
        [220, 170, 240],
        [140, 220, 230],
        [245, 245, 245],
    ];
    match n {
        0..=15 => BASIC[n as usize],
        16..=231 => {
            let v = n - 16;
            let c = |x: u8| if x == 0 { 0 } else { 55 + x * 40 };
            [c(v / 36), c(v / 6 % 6), c(v % 6)]
        }
        _ => [8 + (n - 232) * 10; 3],
    }
}
#[derive(Clone, Debug)]
pub struct Link {
    pub range: Range<usize>,
    pub target: String,
}
/// Recognize URL and path tokens; existence is checked only when the user opens a path.
/// Work per retained line is bounded and never scans the filesystem during rendering.
pub fn links(text: &str) -> Vec<Link> {
    let mut result = Vec::new();
    let mut tokens = Vec::new();
    let mut cursor = 0;
    while cursor < text.len() && tokens.len() < 1024 {
        let c = text[cursor..].chars().next().expect("character boundary");
        if matches!(c, '\"' | '\'' | '`') {
            let start = cursor + c.len_utf8();
            if let Some(end) = text[start..].find(c) {
                tokens.push(start..start + end);
                cursor = start + end + c.len_utf8();
                continue;
            }
        }
        if c.is_whitespace() || matches!(c, '<' | '>' | '|' | '(' | ')' | '[' | ']' | '{' | '}') {
            cursor += c.len_utf8();
            continue;
        }
        let start = cursor;
        cursor += c.len_utf8();
        while cursor < text.len() {
            let c = text[cursor..].chars().next().expect("character boundary");
            if c.is_whitespace()
                || matches!(
                    c,
                    '\"' | '\'' | '`' | '<' | '>' | '|' | '(' | ')' | '[' | ']' | '{' | '}'
                )
            {
                break;
            }
            cursor += c.len_utf8();
        }
        tokens.push(start..cursor);
    }
    for range in tokens {
        let word = text[range.clone()].trim_end_matches([',', ';', '.']);
        let offset = if word.starts_with("http://")
            || word.starts_with("https://")
            || word.starts_with("file://")
        {
            0
        } else {
            word.find('=').map_or(0, |p| p + 1)
        };
        let token = &word[offset..];
        if token.starts_with("https://")
            || token.starts_with("http://")
            || token.starts_with("file://")
            || token.starts_with('/')
            || token.starts_with("~/")
            || (token.contains('/') && !token.contains("://") && !token.starts_with('-'))
        {
            result.push(Link {
                range: range.start + offset..range.start + word.len(),
                target: token.to_owned(),
            });
            if result.len() == 64 {
                break;
            }
        }
    }
    result
}

pub fn local_path(target: &str, cwd: &Path) -> Result<PathBuf, String> {
    let mut raw = if let Some(uri) = target.strip_prefix("file://") {
        let uri = uri.strip_prefix("localhost").unwrap_or(uri);
        if !uri.starts_with('/') {
            return Err("仅支持本地 file:// 路径".into());
        }
        let mut bytes = Vec::new();
        let mut chars = uri.bytes();
        while let Some(b) = chars.next() {
            if b == b'%' {
                let digits = [
                    chars.next().ok_or("文件URL转义不完整")?,
                    chars.next().ok_or("文件URL转义不完整")?,
                ];
                let hex = std::str::from_utf8(&digits).map_err(|_| "文件URL转义无效")?;
                bytes.push(u8::from_str_radix(hex, 16).map_err(|_| "文件URL转义无效")?);
            } else {
                bytes.push(b);
            }
        }
        String::from_utf8(bytes).map_err(|_| "文件URL不是UTF-8")?
    } else {
        target.to_owned()
    };
    // Strip conventional compiler :line[:column] suffixes, preserving the actual path.
    for _ in 0..2 {
        if let Some((path, n)) = raw.rsplit_once(':')
            && !n.is_empty()
            && n.chars().all(|c| c.is_ascii_digit())
        {
            raw = path.to_owned();
        }
    }
    if raw.contains('\0') {
        return Err("无效路径".into());
    }
    let path = if let Some(relative) = raw.strip_prefix("~/") {
        PathBuf::from(std::env::var_os("HOME").ok_or("无法确定用户目录")?).join(relative)
    } else {
        let p = PathBuf::from(raw);
        if p.is_absolute() { p } else { cwd.join(p) }
    };
    path.canonicalize()
        .map_err(|e| format!("无法打开路径 {}: {e}", path.display()))
}
pub fn open(target: &str, cwd: &Path) -> Result<(), String> {
    if target.len() > crate::console_buffer::MAX_LINE_BYTES {
        return Err("打开目标超过长度限制".into());
    }
    if target.chars().any(char::is_control) {
        return Err("链接包含控制字符".into());
    }
    if target.starts_with("http://") || target.starts_with("https://") {
        #[cfg(target_os = "macos")]
        let mut cmd = std::process::Command::new("open");
        #[cfg(not(target_os = "macos"))]
        let mut cmd = std::process::Command::new("xdg-open");
        let status = cmd.arg(target).status().map_err(|e| e.to_string())?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("打开链接失败: {status}"))
        }
    } else if target.starts_with("file://")
        || target.starts_with('/')
        || target.starts_with("~/")
        || target.starts_with("./")
        || target.starts_with("../")
        || (target.contains('/') && !target.contains("://") && !target.starts_with('-'))
    {
        crate::service::open_file(&local_path(target, cwd)?)
    } else {
        Err("仅支持 HTTP(S) 链接和文件路径".into())
    }
}
