//! Read-only discovery and identity-checked control of externally launched services.
use crate::model::{Entry, Project};
use std::{
    collections::{BTreeMap, HashSet},
    path::PathBuf,
    process::Command,
};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: u32,
    pub parent: u32,
    pub started: String,
    pub command: String,
    pub cwd: PathBuf,
}
#[derive(Clone)]
pub struct Candidate {
    pub project: String,
    pub scripts: Vec<u64>,
    pub process: Process,
    pub port: u16,
}
fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "{program} {}: {}",
            out.status,
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
pub fn process(pid: u32) -> Result<Option<Process>, String> {
    let id = pid.to_string();
    let out = Command::new("/bin/ps")
        .env("LC_ALL", "C")
        .args(["-ww", "-p", &id, "-o", "ppid=,lstart=,stat=,command="])
        .output()
        .map_err(|e| e.to_string())?;
    if out.stdout.is_empty() {
        return Ok(None);
    }
    if !out.status.success() {
        return Err(format!(
            "读取 PID {pid}: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.len() < 8 {
        return Err(format!("PID {pid} 的 ps 记录不完整"));
    }
    if parts[6].starts_with('Z') {
        return Ok(None);
    }
    let parent = parts[0].parse().map_err(|e| format!("PID {pid}: {e}"))?;
    let cwd = match output("/usr/sbin/lsof", &["-a", "-p", &id, "-d", "cwd", "-Fn"]) {
        Ok(cwd) => cwd,
        Err(error) => {
            let status = Command::new("/bin/ps")
                .args(["-p", &id, "-o", "stat="])
                .output()
                .map_err(|e| e.to_string())?;
            let state = String::from_utf8_lossy(&status.stdout);
            if state.trim().is_empty() || state.trim().starts_with('Z') {
                return Ok(None);
            }
            return Err(error);
        }
    };
    let cwd = cwd
        .lines()
        .find_map(|l| l.strip_prefix('n'))
        .ok_or_else(|| format!("PID {pid} cwd 不可读取"))?;
    Ok(Some(Process {
        pid,
        parent,
        started: parts[1..6].join(" "),
        command: parts[7..].join(" "),
        cwd: std::fs::canonicalize(cwd).map_err(|e| format!("PID {pid} cwd {cwd}: {e}"))?,
    }))
}
pub fn same_process(expected: &Process) -> Result<bool, String> {
    Ok(process(expected.pid)?.is_some_and(|current| {
        current.pid == expected.pid
            && current.started == expected.started
            && current.command == expected.command
            && current.cwd == expected.cwd
    }))
}
pub(crate) fn listeners() -> Result<BTreeMap<u16, Vec<u32>>, String> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-nP", "-iTCP", "-sTCP:LISTEN", "-Fpn"])
        .output()
        .map_err(|e| e.to_string())?;
    // lsof uses exit 1 for no matching open files.
    if !(out.status.success() || out.status.code() == Some(1) && out.stderr.is_empty()) {
        return Err(format!(
            "监听端口扫描失败: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let mut map: BTreeMap<u16, Vec<u32>> = BTreeMap::new();
    let mut pid = None;
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        if let Some(id) = line.strip_prefix('p') {
            pid = id.parse::<u32>().ok();
        }
        if let Some(name) = line.strip_prefix('n')
            && let Some(port) = name.rsplit(':').next().and_then(|p| p.parse::<u16>().ok())
            && let Some(pid) = pid
        {
            let ids = map.entry(port).or_default();
            if !ids.contains(&pid) {
                ids.push(pid);
            }
        }
    }
    Ok(map)
}
fn evidence(process: &Process, project: &Project) -> bool {
    // Resolve command paths as well as cwd: /tmp and symlinked workspaces may
    // refer to the same project using different spellings.
    let root = project.path.canonicalize().ok();
    if process
        .command
        .split_whitespace()
        .flat_map(|word| word.split(':'))
        .any(|word| {
            let path = std::path::Path::new(word);
            path.is_absolute()
                && path
                    .canonicalize()
                    .ok()
                    .is_some_and(|p| root.as_ref().is_some_and(|root| p.starts_with(root)))
        })
    {
        return true;
    }
    if process
        .command
        .contains(&format!("{}/", project.path.display()))
    {
        return true;
    }
    project.scripts.iter().any(|s| match &s.entry {
        Entry::Java { main_class, .. } => process
            .command
            .split_whitespace()
            .any(|word| word == main_class),
        Entry::Package { name, .. } => process.command.split_whitespace().any(|w| w == name),
        Entry::File { .. } => process
            .command
            .contains(&s.file.to_string_lossy().to_string()),
        Entry::Command { program } => {
            process
                .command
                .split_whitespace()
                .next()
                .and_then(|p| std::path::Path::new(p).file_name())
                == std::path::Path::new(program).file_name()
        }
    })
}
pub fn discover(projects: &[Project], excluded: &HashSet<u32>) -> Result<Vec<Candidate>, String> {
    let listeners = listeners()?;
    let mut result = vec![];
    let mut seen = HashSet::new();
    for project in projects {
        if !project.scripts.iter().any(|s| s.expected_port.is_some()) {
            continue;
        }
        let root = project
            .path
            .canonicalize()
            .map_err(|e| format!("项目 {}: {e}", project.path.display()))?;
        // Two catalog entries for one directory cannot establish unique ownership.
        if projects
            .iter()
            .filter(|p| p.path.canonicalize().ok().as_ref() == Some(&root))
            .count()
            != 1
        {
            continue;
        }
        for script in &project.scripts {
            let Some(port) = script.expected_port else {
                continue;
            };
            for &pid in listeners.get(&port).into_iter().flatten() {
                if excluded.contains(&pid)
                    || excluded.contains(&(unsafe { libc::getpgid(pid as i32) } as u32))
                    || seen.contains(&pid)
                {
                    continue;
                }
                let Some(listener) = process(pid)? else {
                    continue;
                };
                if listener.cwd != root {
                    continue;
                }
                let mut ancestor = listener.clone();
                let mut matched = evidence(&ancestor, project);
                let mut owned = false;
                for _ in 0..16 {
                    if excluded.contains(&ancestor.pid) {
                        owned = true;
                        break;
                    }
                    if ancestor.parent <= 1 {
                        break;
                    }
                    let Some(parent) = process(ancestor.parent)? else {
                        break;
                    };
                    if parent.cwd != root {
                        break;
                    }
                    matched |= evidence(&parent, project);
                    ancestor = parent;
                }
                if matched && !owned {
                    seen.insert(pid);
                    result.push(Candidate {
                        project: project.name.clone(),
                        scripts: project
                            .scripts
                            .iter()
                            .filter(|s| s.expected_port == Some(port))
                            .map(|s| s.id)
                            .collect(),
                        process: listener,
                        port,
                    });
                }
            }
        }
    }
    Ok(result)
}
/// Snapshot only descendants; never signal the shared terminal's process group.
pub fn tree(root: &Process) -> Result<Vec<Process>, String> {
    if !same_process(root)? {
        return Err("外部进程身份已变化，请等待状态刷新".into());
    }
    let text = output("/bin/ps", &["-axo", "pid=,ppid="])?;
    let pairs: Vec<(u32, u32)> = text
        .lines()
        .filter_map(|l| {
            let mut p = l.split_whitespace();
            Some((p.next()?.parse().ok()?, p.next()?.parse().ok()?))
        })
        .collect();
    let mut ids = vec![root.pid];
    let mut found = vec![root.clone()];
    let mut i = 0;
    while i < ids.len() {
        for &(pid, parent) in &pairs {
            if parent == ids[i]
                && !ids.contains(&pid)
                && let Some(p) = process(pid)?
            {
                ids.push(pid);
                found.push(p);
            }
        }
        i += 1;
    }
    Ok(found)
}
pub fn signal_checked(process: &Process, signal: i32) -> Result<(), String> {
    if !same_process(process)? {
        return Ok(());
    }
    if unsafe { libc::kill(process.pid as i32, signal) } != 0 {
        let e = std::io::Error::last_os_error();
        if e.raw_os_error() != Some(libc::ESRCH) {
            return Err(format!("停止外部 PID {}: {e}", process.pid));
        }
    }
    Ok(())
}

/// Read argv only; never decode or retain the environment section returned by macOS.
#[cfg(target_os = "macos")]
fn arguments(pid: u32) -> Result<Vec<String>, String> {
    let mut max: i32 = 0;
    let mut size = std::mem::size_of::<i32>();
    let mut limit = [libc::CTL_KERN, libc::KERN_ARGMAX];
    if unsafe {
        libc::sysctl(
            limit.as_mut_ptr(),
            2,
            (&mut max as *mut i32).cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(format!("读取argv容量: {}", std::io::Error::last_os_error()));
    }
    if !(4..=8_388_608).contains(&max) {
        return Err("argv容量无效".into());
    }
    let mut buffer = vec![0u8; max as usize];
    size = buffer.len();
    let mut query = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as i32];
    if unsafe {
        libc::sysctl(
            query.as_mut_ptr(),
            3,
            buffer.as_mut_ptr().cast(),
            &mut size,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return Err(format!(
            "读取PID {pid} argv: {}",
            std::io::Error::last_os_error()
        ));
    }
    buffer.truncate(size);
    if buffer.len() < 4 {
        return Err("argv头不完整".into());
    }
    let count = i32::from_ne_bytes(buffer[..4].try_into().expect("four bytes"));
    if !(1..=65536).contains(&count) {
        return Err("argv数量无效".into());
    }
    let mut pos = 4;
    while pos < buffer.len() && buffer[pos] != 0 {
        pos += 1;
    }
    while pos < buffer.len() && buffer[pos] == 0 {
        pos += 1;
    }
    let mut args = vec![];
    for _ in 0..count {
        if pos >= buffer.len() {
            return Err("argv记录不完整".into());
        }
        let end = buffer[pos..]
            .iter()
            .position(|b| *b == 0)
            .map(|n| pos + n)
            .ok_or("argv终止符缺失")?;
        args.push(String::from_utf8_lossy(&buffer[pos..end]).into_owned());
        pos = end + 1;
    }
    Ok(args)
}
#[cfg(not(target_os = "macos"))]
fn arguments(pid: u32) -> Result<Vec<String>, String> {
    Ok(std::fs::read(format!("/proc/{pid}/cmdline"))
        .map_err(|e| e.to_string())?
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .map(|s| String::from_utf8_lossy(s).into_owned())
        .collect())
}
fn path_matches(value: &str, expected: &std::path::Path, cwd: &std::path::Path) -> bool {
    let path = std::path::Path::new(value);
    let path = if path.is_absolute() {
        path.to_path_buf()
    } else {
        cwd.join(path)
    };
    std::fs::canonicalize(path)
        .ok()
        .zip(expected.canonicalize().ok())
        .is_some_and(|(a, b)| a == b)
}
fn executable_matches(value: &str, program: &str, script: &crate::model::Script) -> bool {
    // macOS framework Python replaces the small `python3` launcher with this binary.
    let expected = std::path::Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    let python = expected
        .strip_prefix("python")
        .is_some_and(|version| version.chars().all(|c| c.is_ascii_digit() || c == '.'));
    let actual = std::path::Path::new(value);
    if python
        && actual.file_name().is_some_and(|name| name == "Python")
        && actual.components().any(|part| {
            part.as_os_str() == "Python.framework" || part.as_os_str() == "Python3.framework"
        })
    {
        return true;
    }
    if std::path::Path::new(value).file_name() == std::path::Path::new(program).file_name() {
        return true;
    }
    let locate = |name: &str| {
        let path = std::path::Path::new(name);
        if path.is_absolute() {
            return path.canonicalize().ok();
        }
        let search = script
            .environment
            .get("PATH")
            .cloned()
            .or_else(|| std::env::var("PATH").ok())
            .unwrap_or_default();
        std::env::split_paths(&search).find_map(|dir| {
            let path = dir.join(name);
            if path.is_file() {
                path.canonicalize().ok()
            } else {
                None
            }
        })
    };
    locate(value)
        .zip(locate(program))
        .is_some_and(|(a, b)| a == b)
}
fn command_matches(args: &[String], script: &crate::model::Script) -> bool {
    let suffix = |prefix: usize| {
        args.get(prefix..)
            .is_some_and(|rest| rest == script.args.as_slice())
    };
    match &script.entry {
        Entry::File { interpreter } => {
            let mut parts = interpreter.split_whitespace();
            let mut expected = parts.next().unwrap_or("");
            if std::path::Path::new(expected)
                .file_name()
                .is_some_and(|n| n == "env")
            {
                expected = parts.next().unwrap_or("");
            }
            let executable = args.first().is_some_and(|arg| {
                executable_matches(arg, expected, script)
                    || path_matches(arg, &script.file, &script.cwd)
            });
            executable
                && args
                    .iter()
                    .enumerate()
                    .any(|(i, s)| path_matches(s, &script.file, &script.cwd) && suffix(i + 1))
        }
        Entry::Java {
            main_class,
            program,
            ..
        } => {
            let direct = args.first().is_some_and(|arg| {
                std::path::Path::new(arg).file_name() == std::path::Path::new(program).file_name()
            });
            let wrapper = args
                .first()
                .and_then(|a| std::path::Path::new(a).file_name())
                .is_some_and(|p| p == "bash" || p == "sh" || p == "zsh")
                && args.get(1).is_some_and(|arg| {
                    path_matches(arg, std::path::Path::new(program), &script.cwd)
                });
            (direct || wrapper)
                && args
                    .iter()
                    .enumerate()
                    .any(|(i, arg)| arg == main_class && suffix(i + 1))
        }
        Entry::Package { manager, name } => {
            let manager_name = std::path::Path::new(manager).file_name();
            let argv = if args.first().is_some_and(|s| {
                s.starts_with(&format!(
                    "{} run ",
                    std::path::Path::new(manager)
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                ))
            }) && !script.args.iter().any(|s| s.contains(char::is_whitespace))
            {
                args[0]
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect::<Vec<_>>()
            } else {
                args.to_vec()
            };
            let manager_present = argv
                .iter()
                .take(2)
                .any(|arg| std::path::Path::new(arg).file_name() == manager_name);
            manager_present
                && argv.windows(2).enumerate().any(|(i, w)| {
                    w[0] == "run" && w[1] == *name && {
                        let rest = &argv[i + 2..];
                        rest == script.args.as_slice()
                            || rest.first().is_some_and(|s| s == "--") && rest[1..] == script.args
                    }
                })
        }
        Entry::Command { program } => {
            args.first().is_some_and(|arg| {
                std::path::Path::new(arg).file_name() == std::path::Path::new(program).file_name()
            }) && suffix(1)
        }
    }
}
fn candidate_cwd_matches(pid: u32, roots: &[PathBuf]) -> Result<bool, String> {
    let out = Command::new("/usr/sbin/lsof")
        .args(["-a", "-p", &pid.to_string(), "-d", "cwd", "-Fn"])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        if out.status.code() == Some(1) && out.stderr.is_empty() {
            return Ok(false);
        }
        return Err(format!(
            "读取候选PID {pid}目录: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let Some(path) = text.lines().find_map(|s| s.strip_prefix('n')) else {
        return Ok(false);
    };
    match std::fs::canonicalize(path) {
        Ok(path) => Ok(roots.contains(&path)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(e) => Err(format!("核实候选PID {pid}目录: {e}")),
    }
}
/// Immediate checks for one requested entry. An occupied unverified port aborts before signals.
pub fn find_existing(
    project: &Project,
    script: &crate::model::Script,
    excluded: &HashSet<u32>,
) -> Result<Vec<Process>, String> {
    let root = project.path.canonicalize().map_err(|e| e.to_string())?;
    let cwd = script.cwd.canonicalize().map_err(|e| e.to_string())?;
    let related: Vec<_> = project
        .scripts
        .iter()
        .filter(|s| {
            s.id == script.id
                || script
                    .resource_key
                    .as_ref()
                    .is_some_and(|key| s.resource_key.as_ref() == Some(key))
                || script.expected_port.is_some() && s.expected_port == script.expected_port
        })
        .collect();
    let mut related = related;
    if !related.iter().any(|s| s.id == script.id) {
        related.push(script);
    }
    let text = output("/bin/ps", &["-axo", "pid=,ppid=,command="])?;
    let mut parents = std::collections::HashMap::new();
    let mut candidates = HashSet::new();
    for line in text.lines() {
        let mut parts = line.split_whitespace();
        let Some(pid) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        let Some(parent) = parts.next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        parents.insert(pid, parent);
        let command = parts.collect::<Vec<_>>().join(" ");
        if related.iter().any(|script| {
            let needle = match &script.entry {
                Entry::Java { main_class, .. } => main_class.as_str(),
                Entry::Package { name, .. } => name.as_str(),
                Entry::File { .. } => script
                    .file
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(""),
                Entry::Command { program } => std::path::Path::new(program)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or(""),
            };
            !needle.is_empty() && command.contains(needle)
        }) {
            candidates.insert(pid);
        }
    }
    let is_excluded = |mut pid: u32| {
        if excluded.contains(&(unsafe { libc::getpgid(pid as i32) } as u32)) {
            return true;
        }
        for _ in 0..64 {
            if excluded.contains(&pid) {
                return true;
            }
            let Some(parent) = parents.get(&pid) else {
                break;
            };
            if *parent <= 1 || *parent == pid {
                break;
            }
            pid = *parent;
        }
        false
    };
    let mut confirmed = std::collections::BTreeMap::new();
    if let Some(port) = script.expected_port {
        for &pid in listeners()?.get(&port).into_iter().flatten() {
            if is_excluded(pid) {
                continue;
            }
            let Some(p) = process(pid)? else { continue };
            if !(p.cwd == cwd || p.cwd.starts_with(&root)) || !evidence(&p, project) {
                return Err(format!(
                    "端口 {port} 由无法确认归属的 PID {pid} 占用；未关闭该进程，取消启动"
                ));
            }
            confirmed.insert(pid, p);
        }
    }
    let allowed_cwds: Vec<_> = related
        .iter()
        .filter_map(|s| s.cwd.canonicalize().ok())
        .collect();
    for pid in candidates {
        if is_excluded(pid) || confirmed.contains_key(&pid) {
            continue;
        }
        // Filter unrelated/vanished working directories before reading full identity and argv.
        if !candidate_cwd_matches(pid, &allowed_cwds)? {
            continue;
        }
        let Some(p) = process(pid)? else { continue };
        let matching_cwd: Vec<_> = related
            .iter()
            .filter(|s| s.cwd.canonicalize().ok().as_ref() == Some(&p.cwd))
            .collect();
        if matching_cwd.is_empty() {
            continue;
        }
        let argv = arguments(pid)?;
        if matching_cwd.iter().any(|s| command_matches(&argv, s)) {
            confirmed.insert(pid, p);
        }
    }
    let ids: HashSet<_> = confirmed.keys().copied().collect();
    Ok(confirmed
        .into_values()
        .filter(|p| {
            let mut parent = p.parent;
            for _ in 0..64 {
                if ids.contains(&parent) {
                    return false;
                }
                let Some(next) = parents.get(&parent) else {
                    break;
                };
                if *next <= 1 || *next == parent {
                    break;
                }
                parent = *next;
            }
            true
        })
        .collect())
}

#[cfg(test)]
pub fn inspect_arguments(pid: u32) -> Result<Vec<String>, String> {
    arguments(pid)
}
