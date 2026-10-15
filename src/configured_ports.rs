//! Read-only, conservative startup-port inference. Never execute configuration or assume defaults.
use crate::model::{Entry, Script};
use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
};
#[derive(Clone, Debug)]
pub struct Hint {
    pub port: u16,
    pub source: String,
}
pub type Cache = HashMap<u64, Result<Option<Hint>, String>>;
pub fn collect(projects: &[crate::model::Project]) -> Cache {
    projects
        .iter()
        .flat_map(|p| &p.scripts)
        .map(|s| {
            let hint = resolve(s);
            if let Err(e) = &hint {
                eprintln!("[dev start] 读取启动端口配置失败 {}: {e}", s.name);
            }
            (s.id, hint)
        })
        .collect()
}
fn read(path: &Path) -> Result<String, String> {
    let mut value = String::new();
    std::fs::File::open(path)
        .map_err(|e| format!("{}: {e}", path.display()))?
        .take(2_000_001)
        .read_to_string(&mut value)
        .map_err(|e| e.to_string())?;
    if value.len() > 2_000_000 {
        return Err("端口配置文件超过2MiB".into());
    }
    Ok(value)
}
fn number(value: &str) -> Option<u16> {
    value.parse::<u16>().ok().filter(|p| *p > 0)
}
enum Declared {
    Absent,
    Known(u16),
    Unknown,
}
fn flag_port(args: &[String], short: bool) -> Declared {
    let mut value = None;
    for (i, arg) in args.iter().enumerate() {
        if ["--port", "--server.port"].contains(&arg.as_str()) || short && arg == "-p" {
            value = Some(args.get(i + 1).map(String::as_str).unwrap_or(""));
        }
        for key in [
            "--port=",
            "--server.port=",
            "-Dserver.port=",
            "-Djboot.server.port=",
        ] {
            if let Some(v) = arg.strip_prefix(key) {
                value = Some(v);
            }
        }
    }
    match value {
        None => Declared::Absent,
        Some(v) => number(v).map(Declared::Known).unwrap_or(Declared::Unknown),
    }
}
/// Infer a Python server port only from a literal module default and an explicit argv fallback.
/// This never imports or executes the Python source.
fn python_default_port(
    script: &Script,
    interpreter: &str,
    source: &str,
) -> Option<(u16, &'static str)> {
    let executable = interpreter.split_whitespace().last()?;
    let name = Path::new(executable).file_name()?.to_str()?;
    let version = name.strip_prefix("python")?;
    if !version.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return None;
    }

    let compact: String = source.chars().filter(|c| !c.is_whitespace()).collect();
    // Require an HTTP/TCP server constructor to bind the value called `port`.
    if !compact.contains("Server((host,port),")
        && !compact.contains("Server((host,port)")
        && !compact.contains(".bind((host,port))")
    {
        return None;
    }

    let mut constants: HashMap<String, Option<u16>> = HashMap::new();
    for line in source.lines() {
        if line.starts_with(char::is_whitespace) || line.trim_start().starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        if key.is_empty()
            || !key.chars().enumerate().all(|(i, c)| {
                c == '_' || c.is_ascii_alphanumeric() && (i > 0 || !c.is_ascii_digit())
            })
        {
            continue;
        }
        let value = value.split('#').next()?.trim().replace('_', "");
        if let Some(port) = number(&value) {
            constants.insert(key.into(), Some(port));
        } else if constants.contains_key(key) {
            constants.insert(key.into(), None);
        }
    }

    let mut fallbacks = Vec::new();
    for line in source
        .lines()
        .map(str::trim)
        .filter(|line| !line.starts_with('#'))
    {
        let Some((lhs, rhs)) = line.split_once('=') else {
            continue;
        };
        if lhs.trim() != "port" || !rhs.contains("sys.argv[1]") {
            continue;
        }
        let (_, fallback) = rhs.rsplit_once("else")?;
        let fallback = fallback.trim().trim_end_matches(')').trim();
        let value = if let Some(port) = number(&fallback.replace('_', "")) {
            Some(port)
        } else {
            constants.get(fallback).copied().flatten()
        }?;
        fallbacks.push(value);
    }
    if !script.args.is_empty() {
        if script.args.len() != 1 {
            return None;
        }
        if let Some(argument) = number(&script.args[0]) {
            return Some((argument, "Python 启动参数"));
        }
        return None;
    }
    let ports: std::collections::HashSet<_> = fallbacks.into_iter().collect();
    (ports.len() == 1).then(|| (*ports.iter().next().unwrap(), "Python 静态默认端口"))
}
fn option(args: &[String], key: &str) -> Option<String> {
    let mut value = None;
    for (i, arg) in args.iter().enumerate() {
        if arg == key {
            value = args.get(i + 1).cloned();
        } else if let Some(v) = arg.strip_prefix(&format!("{key}=")) {
            value = Some(v.into());
        }
    }
    value
}
/// Tokenize a single shell command only. Reject pipelines/subshells rather than guessing which process binds.
fn words(command: &str) -> Option<Vec<String>> {
    let mut words = vec![];
    let mut word = String::new();
    let mut quote = None;
    let mut escape = false;
    for c in command.chars() {
        if escape {
            word.push(c);
            escape = false;
            continue;
        }
        if c == '\\' && quote != Some('\'') {
            escape = true;
            continue;
        }
        if let Some(q) = quote {
            if c == q {
                quote = None;
            } else {
                word.push(c);
            }
            continue;
        }
        if c == '\'' || c == '"' {
            quote = Some(c);
        } else if ";|&`()\n".contains(c) {
            return None;
        } else if c.is_whitespace() {
            if !word.is_empty() {
                words.push(std::mem::take(&mut word));
            }
        } else {
            word.push(c);
        }
    }
    if quote.is_some() || escape {
        return None;
    }
    if !word.is_empty() {
        words.push(word);
    }
    Some(words)
}
fn program(args: &[String]) -> Option<&str> {
    let mut index = 0;
    if args.first().is_some_and(|s| s == "cross-env") {
        index += 1;
    }
    while args
        .get(index)
        .is_some_and(|s| s.contains('=') && !s.starts_with('-'))
    {
        index += 1;
    }
    if args.get(index).is_some_and(|s| s == "npx") {
        index += 1;
    }
    args.get(index).map(|s| s.rsplit('/').next().unwrap_or(s))
}
pub fn resolve(script: &Script) -> Result<Option<Hint>, String> {
    if let Some(port) = script.expected_port {
        return Ok(Some(Hint {
            port,
            source: "启动清单 expected_port".into(),
        }));
    }
    let mut args = if let Entry::Java { vm_args, .. } = &script.entry {
        vm_args.clone()
    } else {
        vec![]
    };
    args.extend(script.args.clone());
    let package = if let Entry::Package { name, .. } = &script.entry {
        let data: serde_json::Value =
            serde_json::from_str(&read(&script.file)?).map_err(|e| e.to_string())?;
        let command = data["scripts"][name]
            .as_str()
            .ok_or("package.json启动命令不存在")?;
        let Some(mut tokens) = words(command) else {
            return Ok(None);
        };
        tokens.extend(args);
        args = tokens;
        true
    } else {
        false
    };
    let executable = program(&args);
    match flag_port(&args, package && executable == Some("next")) {
        Declared::Known(port) => {
            return Ok(Some(Hint {
                port,
                source: if package {
                    "package.json 启动命令/参数"
                } else {
                    "启动参数/JVM参数"
                }
                .into(),
            }));
        }
        Declared::Unknown => return Ok(None),
        Declared::Absent => {}
    }
    if !package {
        if let Entry::File { interpreter } = &script.entry {
            let executable = interpreter.split_whitespace().last().unwrap_or("");
            let python = Path::new(executable)
                .file_name()
                .and_then(|s| s.to_str())
                .and_then(|n| n.strip_prefix("python"))
                .is_some_and(|suffix| suffix.chars().all(|c| c.is_ascii_digit() || c == '.'));
            if python {
                let source = read(&script.file)?;
                if let Some((port, origin)) = python_default_port(script, interpreter, &source) {
                    return Ok(Some(Hint {
                        port,
                        source: format!("{} · {origin}", script.file.display()),
                    }));
                }
            }
        }
        if let Entry::File { interpreter } = &script.entry
            && ["sh", "bash", "zsh"]
                .contains(&interpreter.rsplit('/').next().unwrap_or(interpreter))
        {
            let source = read(&script.file)?;
            let commands: Vec<_> = source
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty() && !l.starts_with('#') && !l.starts_with("set -"))
                .collect();
            if commands.len() == 1
                && let Some(mut command) = words(commands[0])
            {
                if command.first().is_some_and(|s| s == "exec") {
                    command.remove(0);
                }
                if program(&command).is_some_and(|p| {
                    [
                        "vite",
                        "next",
                        "uvicorn",
                        "gunicorn",
                        "http-server",
                        "webpack",
                        "vue-cli-service",
                        "uni",
                        "node",
                        "java",
                        "python3",
                        "python",
                    ]
                    .contains(&p)
                }) && let Declared::Known(port) =
                    flag_port(&command, program(&command) == Some("next"))
                {
                    return Ok(Some(Hint {
                        port,
                        source: format!("{} · 静态启动命令", script.file.display()),
                    }));
                }
            }
        }
        return Ok(None);
    }
    let (prefix, section) = match executable {
        Some("vite") if !args.iter().any(|s| s == "build") => (
            "vite.config",
            if args.iter().any(|s| s == "preview") {
                "preview"
            } else {
                "server"
            },
        ),
        Some("uni")
            if !args.iter().any(|s| s == "build")
                && option(&args, "-p").is_none_or(|p| p == "h5") =>
        {
            ("vite.config", "server")
        }
        Some("vue-cli-service") if args.iter().any(|s| s == "serve") => ("vue.config", "devServer"),
        _ => return Ok(None),
    };
    let files: Vec<PathBuf> = if let Some(path) = option(&args, "--config") {
        vec![script.cwd.join(path)]
    } else {
        ["ts", "js", "mjs", "cjs"]
            .iter()
            .map(|ext| script.cwd.join(format!("{prefix}.{ext}")))
            .filter(|p| p.is_file())
            .collect()
    };
    if files.len() != 1 {
        return Ok(None);
    }
    let source = read(&files[0])?;
    Ok(literal_port(&source, section).map(|port| Hint {
        port,
        source: format!("{} · {section}.port", files[0].display()),
    }))
}
#[derive(Debug, PartialEq)]
enum Token {
    Name(String),
    Number(String),
    Symbol(char),
    Opaque,
}
fn tokens(source: &str) -> Vec<Token> {
    let chars: Vec<_> = source.chars().collect();
    let mut out = vec![];
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            i += 1;
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'/') {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
            continue;
        }
        if c == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i = (i + 2).min(chars.len());
            continue;
        }
        if ['\'', '"', '`', '/'].contains(&c) {
            let start = c;
            i += 1;
            let mut value = String::new();
            let mut escape = false;
            while i < chars.len() {
                let ch = chars[i];
                i += 1;
                if escape {
                    value.push(ch);
                    escape = false;
                } else if ch == '\\' {
                    escape = true;
                } else if ch == start {
                    break;
                } else {
                    value.push(ch);
                }
            }
            out.push(if start == '`' || start == '/' {
                Token::Opaque
            } else {
                Token::Name(value)
            });
            continue;
        }
        if c.is_ascii_digit() {
            let start = i;
            i += 1;
            while i < chars.len() && (chars[i].is_ascii_digit() || chars[i] == '_') {
                i += 1;
            }
            out.push(Token::Number(
                chars[start..i].iter().filter(|c| **c != '_').collect(),
            ));
            continue;
        }
        if c.is_alphabetic() || c == '_' || c == '$' {
            let start = i;
            i += 1;
            while i < chars.len()
                && (chars[i].is_alphanumeric() || chars[i] == '_' || chars[i] == '$')
            {
                i += 1;
            }
            out.push(Token::Name(chars[start..i].iter().collect()));
            continue;
        }
        out.push(Token::Symbol(c));
        i += 1;
    }
    out
}
/// Static object literals only; reject dynamic values, spreads, duplicate or ambiguous sections.
fn literal_port(source: &str, section: &str) -> Option<u16> {
    let tokens = tokens(source);
    if tokens
        .windows(3)
        .any(|w| w == [Token::Symbol('.'), Token::Symbol('.'), Token::Symbol('.')])
    {
        return None;
    }
    let starts: Vec<_> = tokens
        .windows(3)
        .enumerate()
        .filter(|(_, w)| {
            w[0] == Token::Name(section.into())
                && w[1] == Token::Symbol(':')
                && w[2] == Token::Symbol('{')
        })
        .map(|(i, _)| i + 3)
        .collect();
    if starts.len() != 1 {
        return None;
    }
    let mut depth = 1;
    let mut ports = vec![];
    let mut i = starts[0];
    while i < tokens.len() && depth > 0 {
        if depth == 1
            && tokens[i] == Token::Name("port".into())
            && tokens.get(i + 1) == Some(&Token::Symbol(':'))
        {
            let Some(Token::Number(value)) = tokens.get(i + 2) else {
                return None;
            };
            if !matches!(tokens.get(i + 3), Some(Token::Symbol(',' | '}'))) {
                return None;
            }
            ports.push(number(value)?);
        }
        match tokens[i] {
            Token::Symbol('{') => depth += 1,
            Token::Symbol('}') => depth -= 1,
            _ => {}
        }
        i += 1;
    }
    if depth == 0 && ports.len() == 1 {
        Some(ports[0])
    } else {
        None
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn extracts_literals_ignoring_comments_strings_and_proxy_addresses() {
        assert_eq!(
            literal_port(
                "// server:{port:1}\nexport default {server:{port:8_000,proxy:{target:'http://localhost:8082'}}}",
                "server"
            ),
            Some(8000)
        );
        for source in [
            "{server:{port: Number(env.PORT)}}",
            "{server:{port:3000+1}}",
            "{server:{port:3000,...other}}",
            "if(mode){return {server:{port:3000}}}else{return {server:{port:4000}}}",
        ] {
            assert_eq!(literal_port(source, "server"), None);
        }
        assert_eq!(
            literal_port("{preview:{port:4173},server:{port:3000}}", "preview"),
            Some(4173)
        );
        assert!(words("vite --port 8000 && upload").is_none());
        assert_eq!(
            words("vite --port='8000'").unwrap(),
            vec!["vite", "--port=8000"]
        );
    }
}
