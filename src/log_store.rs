//! Bounded raw logs. All writers and API readers share a lock so compaction never tears a page.
use serde::Serialize;
use std::{
    collections::{HashMap, VecDeque},
    fs::{self, File, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    sync::{Arc, Mutex, OnceLock, Weak},
    time::{Duration, Instant, SystemTime},
};
pub const MAX_LINES: usize = 10_000;
pub const MAX_BYTES: u64 = 10 * 1024 * 1024;
pub const MAX_TOTAL_BYTES: u64 = 200 * 1024 * 1024;
pub const MAX_FILES: usize = 50;
pub const MAX_AGE: Duration = Duration::from_secs(7 * 24 * 3600);
const HEADER: &str = "[dev start log v1] retained byte offset: ";
const HEADER_LEN: usize = HEADER.len() + 21;
const INTERVAL: Duration = Duration::from_secs(30);
#[derive(Default)]
struct Stream {
    base: u64,
    bytes: u64,
    lines: VecDeque<usize>,
    ended: bool,
    valid: bool,
}
#[derive(Default)]
struct Budget {
    files: HashMap<PathBuf, (u64, SystemTime)>,
    checked: Option<Instant>,
    total: u64,
}
#[derive(Default)]
struct Hub {
    streams: HashMap<PathBuf, Weak<Mutex<Stream>>>,
    directories: HashMap<PathBuf, Budget>,
}
type WorkspaceHub = Arc<Mutex<Hub>>;
static HUBS: OnceLock<Mutex<HashMap<PathBuf, WorkspaceHub>>> = OnceLock::new();
fn hub(directory: &Path) -> io::Result<WorkspaceHub> {
    let mut hubs = HUBS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| io::Error::other("日志工作空间注册锁已损坏"))?;
    if hubs.len() >= 32 && !hubs.contains_key(directory) {
        hubs.retain(|_, value| Arc::strong_count(value) > 1);
    }
    Ok(hubs.entry(directory.to_path_buf()).or_default().clone())
}

fn regular(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(m) if !m.is_file() => Err(io::Error::other("日志路径不是普通文件")),
        Ok(_) => Ok(()),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e),
    }
}
fn managed(path: &Path) -> bool {
    path.file_name()
        .and_then(|s| s.to_str())
        .and_then(|s| s.strip_prefix("run-"))
        .and_then(|s| s.strip_suffix(".log"))
        .is_some_and(|s| s.parse::<u64>().is_ok())
}
fn open_raw(path: &Path) -> io::Result<File> {
    regular(path)?;
    OpenOptions::new()
        .create(true)
        .append(true)
        .mode(0o600)
        .open(path)
}
fn base(file: &mut File) -> io::Result<(u64, u64)> {
    let mut header = [0u8; HEADER_LEN];
    let read = file.read(&mut header)?;
    if read == HEADER_LEN
        && header.starts_with(HEADER.as_bytes())
        && header[HEADER_LEN - 1] == b'\n'
    {
        let value = std::str::from_utf8(&header[HEADER.len()..HEADER_LEN - 1])
            .ok()
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| io::Error::other("日志游标头损坏"))?;
        Ok((value, HEADER_LEN as u64))
    } else {
        Ok((0, 0))
    }
}
fn lengths(data: &[u8]) -> VecDeque<usize> {
    data.split_inclusive(|b| *b == b'\n')
        .map(|s| s.len())
        .collect()
}
/// Trim with headroom to avoid rewriting a large file for every small output chunk.
fn compact(path: &Path, state: &mut Stream, data: &[u8]) -> io::Result<()> {
    let target_bytes = MAX_BYTES as usize * 4 / 5;
    let target_lines = MAX_LINES * 4 / 5;
    let mut cut = data.len().saturating_sub(target_bytes);
    let mut count = usize::from(data.last().is_some_and(|b| *b != b'\n'));
    for i in (cut..data.len()).rev() {
        if data[i] == b'\n' {
            count += 1;
            if count > target_lines {
                cut = i + 1;
                break;
            }
        }
    }
    // Avoid starting in the middle of a line where a later complete line is available.
    if cut > 0
        && data[cut - 1] != b'\n'
        && let Some(next) = data[cut..].iter().position(|b| *b == b'\n')
        && cut + next + 1 < data.len()
    {
        cut += next + 1;
    }
    let next_base = state
        .base
        .checked_add(cut as u64)
        .ok_or_else(|| io::Error::other("日志游标溢出"))?;
    let temporary = path.with_extension("log.next");
    let result = (|| {
        regular(&temporary)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        writeln!(file, "{HEADER}{next_base:020}")?;
        file.write_all(&data[cut..])?;
        file.sync_data()?;
        fs::rename(&temporary, path)?;
        Ok::<_, io::Error>(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result?;
    state.base = next_base;
    state.bytes = (data.len() - cut) as u64;
    state.lines = lengths(&data[cut..]);
    state.ended = data.last() == Some(&b'\n');
    state.valid = true;
    Ok(())
}
fn load(path: &Path) -> io::Result<Stream> {
    regular(path)?;
    let mut file = match File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return Ok(Stream {
                valid: true,
                ..Default::default()
            });
        }
        Err(e) => return Err(e),
    };
    let size = file.metadata()?.len();
    let (origin, header) = base(&mut file)?;
    let skip = size.saturating_sub(header).saturating_sub(MAX_BYTES);
    file.seek(SeekFrom::Start(header + skip))?;
    let mut data = Vec::new();
    file.take(MAX_BYTES).read_to_end(&mut data)?;
    let count = data.iter().filter(|b| **b == b'\n').count()
        + usize::from(data.last().is_some_and(|b| *b != b'\n'));
    let mut state = Stream {
        base: origin
            .checked_add(skip)
            .ok_or_else(|| io::Error::other("日志游标溢出"))?,
        bytes: data.len() as u64,
        ended: data.last() == Some(&b'\n'),
        valid: true,
        ..Default::default()
    };
    if skip > 0 || size > MAX_BYTES || count > MAX_LINES - usize::from(header > 0) {
        compact(path, &mut state, &data)?;
    } else {
        state.lines = lengths(&data);
    }
    Ok(state)
}
impl Hub {
    fn scan(&mut self, directory: &Path) -> io::Result<()> {
        // The app owns one workspace; keep test/short-lived workspace cache growth bounded as well.
        if self.directories.len() >= 32 && !self.directories.contains_key(directory) {
            self.directories.clear();
        }
        let mut budget = Budget::default();
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            // A crash may leave one atomic-compaction temporary file. Never retain orphan copies.
            if entry.path().extension().is_some_and(|e| e == "next")
                && managed(&entry.path().with_extension(""))
                && entry.file_type()?.is_file()
            {
                fs::remove_file(entry.path())?;
                continue;
            }
            if !managed(&entry.path()) || !entry.file_type()?.is_file() {
                continue;
            }
            let m = entry.metadata()?;
            budget.total += m.len();
            budget.files.insert(entry.path(), (m.len(), m.modified()?));
        }
        budget.checked = Some(Instant::now());
        self.directories.insert(directory.to_path_buf(), budget);
        self.streams.retain(|_, v| v.strong_count() > 0);
        Ok(())
    }
    fn prune(&mut self, directory: &Path, current: Option<&Path>) -> io::Result<()> {
        let budget = self.directories.get_mut(directory).expect("budget scanned");
        let mut files: Vec<_> = budget
            .files
            .iter()
            .map(|(p, (s, t))| (p.clone(), *s, *t))
            .collect();
        files.sort_by_key(|(p, _, t)| (*t, p.clone()));
        for (path, size, modified) in files {
            if Some(path.as_path()) == current {
                continue;
            }
            let expired = SystemTime::now()
                .duration_since(modified)
                .is_ok_and(|age| age > MAX_AGE);
            if !expired && budget.total <= MAX_TOTAL_BYTES && budget.files.len() <= MAX_FILES {
                continue;
            }
            match fs::remove_file(&path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::NotFound => {}
                Err(e) => return Err(e),
            }
            budget.files.remove(&path);
            budget.total = budget.total.saturating_sub(size);
        }
        Ok(())
    }
}
#[derive(Clone)]
pub struct Writer {
    path: PathBuf,
    state: Arc<Mutex<Stream>>,
    hub: WorkspaceHub,
}
impl Writer {
    pub fn open(path: &Path) -> io::Result<Self> {
        let shared = hub(path
            .parent()
            .ok_or_else(|| io::Error::other("日志目录不存在"))?)?;
        let mut hub = shared
            .lock()
            .map_err(|_| io::Error::other("日志锁已损坏"))?;
        let state = if let Some(state) = hub.streams.get(path).and_then(Weak::upgrade) {
            state
        } else {
            let state = Arc::new(Mutex::new(load(path)?));
            hub.streams
                .insert(path.to_path_buf(), Arc::downgrade(&state));
            state
        };
        Ok(Self {
            path: path.to_path_buf(),
            state,
            hub: shared.clone(),
        })
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let mut hub = self
            .hub
            .lock()
            .map_err(|_| io::Error::other("日志锁已损坏"))?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| io::Error::other("日志流锁已损坏"))?;
        if !state.valid {
            *state = load(&self.path)?;
        }
        if !self.path.exists() && state.bytes > 0 {
            state.base = state
                .base
                .checked_add(state.bytes)
                .ok_or_else(|| io::Error::other("日志游标溢出"))?;
            state.bytes = 0;
            state.lines.clear();
            state.ended = true;
        }
        let result = (|| {
            regular(&self.path)?;
            let new_lines = bytes.iter().filter(|b| **b == b'\n').count()
                + usize::from(bytes.last().is_some_and(|b| *b != b'\n'));
            let joined = usize::from(!state.ended && state.bytes > 0 && !bytes.is_empty());
            let header = if state.base > 0 { HEADER_LEN as u64 } else { 0 };
            // Reserve quota before writing: an unlink/permission failure must not permit unlimited append.
            if managed(&self.path) {
                let directory = self.path.parent().expect("log directory");
                if hub
                    .directories
                    .get(directory)
                    .is_none_or(|b| b.checked.is_none_or(|t| t.elapsed() >= INTERVAL))
                {
                    hub.scan(directory)?;
                }
                let projected = (state.bytes + bytes.len() as u64 + header).min(MAX_BYTES);
                let budget = hub
                    .directories
                    .get_mut(directory)
                    .expect("budget initialized");
                let old = budget
                    .files
                    .insert(self.path.clone(), (projected, SystemTime::now()))
                    .map_or(0, |v| v.0);
                budget.total = budget.total.saturating_sub(old) + projected;
                hub.prune(directory, Some(&self.path))?;
            }
            if state.bytes + bytes.len() as u64 + header > MAX_BYTES
                || state.lines.len() + new_lines - joined > MAX_LINES - usize::from(header > 0)
            {
                let mut data = Vec::with_capacity(state.bytes as usize + bytes.len());
                if self.path.exists() {
                    let mut file = File::open(&self.path)?;
                    let (_, prefix) = base(&mut file)?;
                    file.seek(SeekFrom::Start(prefix))?;
                    file.read_to_end(&mut data)?;
                }
                data.extend_from_slice(bytes);
                compact(&self.path, &mut state, &data)?;
            } else if state.bytes == 0 && state.base > 0 {
                // A quota-evicted active log can resume without reusing earlier byte cursors.
                compact(&self.path, &mut state, bytes)?;
            } else {
                open_raw(&self.path)?.write_all(bytes)?;
                for part in bytes.split_inclusive(|b| *b == b'\n') {
                    if state.ended || state.lines.is_empty() {
                        state.lines.push_back(0);
                    }
                    *state.lines.back_mut().expect("line created") += part.len();
                    state.ended = part.last() == Some(&b'\n');
                }
                state.bytes += bytes.len() as u64;
            }
            if managed(&self.path) {
                let directory = self.path.parent().expect("log directory");
                if hub
                    .directories
                    .get(directory)
                    .is_none_or(|b| b.checked.is_none_or(|t| t.elapsed() >= INTERVAL))
                {
                    hub.scan(directory)?;
                }
                let metadata = fs::metadata(&self.path)?;
                let budget = hub
                    .directories
                    .get_mut(directory)
                    .expect("budget initialized");
                let old = budget
                    .files
                    .insert(self.path.clone(), (metadata.len(), metadata.modified()?))
                    .map_or(0, |v| v.0);
                budget.total = budget.total.saturating_sub(old) + metadata.len();
                hub.prune(directory, Some(&self.path))?;
            }
            Ok(bytes.len())
        })();
        if result.is_err() {
            state.valid = !self.path.exists();
            if let Some(budget) = self.path.parent().and_then(|p| hub.directories.get_mut(p)) {
                budget.checked = None;
            }
        }
        result
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
pub fn append(path: &Path, bytes: &[u8]) -> io::Result<()> {
    Writer::open(path)?.write_all(bytes)
}
/// Clean old files at startup and periodically even while no project produces output.
pub fn maintain(directory: &Path, force: bool) -> io::Result<()> {
    let shared = hub(directory)?;
    let mut hub = shared
        .lock()
        .map_err(|_| io::Error::other("日志锁已损坏"))?;
    if !force
        && hub
            .directories
            .get(directory)
            .is_some_and(|b| b.checked.is_some_and(|t| t.elapsed() < INTERVAL))
    {
        return Ok(());
    }
    hub.scan(directory)?;
    hub.prune(directory, None)?;
    if force {
        let paths: Vec<_> = hub.directories[directory].files.keys().cloned().collect();
        for path in paths {
            if let Some(stream) = hub.streams.get(&path).and_then(Weak::upgrade) {
                *stream
                    .lock()
                    .map_err(|_| io::Error::other("日志流锁已损坏"))? = load(&path)?;
            } else {
                load(&path)?;
            }
        }
    }

    hub.scan(directory)?;
    hub.prune(directory, None)
}
#[derive(Serialize)]
pub struct Page {
    offset: u64,
    next_offset: u64,
    size: u64,
    pub base_offset: u64,
    retained_bytes: u64,
    reset: bool,
    eof: bool,
    text: String,
    bytes: Vec<u8>,
}
pub fn read(
    path: &Path,
    requested: Option<u64>,
    tail: Option<u64>,
    limit: u64,
) -> io::Result<Page> {
    let shared = hub(path
        .parent()
        .ok_or_else(|| io::Error::other("日志目录不存在"))?)?;
    let _guard = shared
        .lock()
        .map_err(|_| io::Error::other("日志锁已损坏"))?;
    regular(path)?;
    let mut file = File::open(path)?;
    let (origin, header) = base(&mut file)?;
    let retained = file.metadata()?.len().saturating_sub(header);
    let size = origin
        .checked_add(retained)
        .ok_or_else(|| io::Error::other("日志游标溢出"))?;
    let wanted = tail.map_or_else(
        || requested.unwrap_or(0),
        |n| size.saturating_sub(n.min(65536)).max(origin),
    );
    let offset = if wanted < origin || wanted > size {
        origin
    } else {
        wanted
    };
    file.seek(SeekFrom::Start(header + offset - origin))?;
    let mut bytes = Vec::new();
    file.take(limit.min(65536)).read_to_end(&mut bytes)?;
    let next_offset = offset + bytes.len() as u64;
    Ok(Page {
        offset,
        next_offset,
        size,
        base_offset: origin,
        retained_bytes: retained,
        reset: offset != wanted,
        eof: next_offset >= size,
        text: String::from_utf8_lossy(&bytes).into_owned(),
        bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            let p = std::env::temp_dir()
                .join(format!("ds-log-retention-{}", crate::model::unique_id()));
            fs::create_dir(&p).unwrap();
            Self(p)
        }
        fn path(&self, id: u64) -> PathBuf {
            self.0.join(format!("run-{id}.log"))
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, fs::Permissions::from_mode(0o700));
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    #[test]
    fn line_retention_keeps_recent_output_and_absolute_paging_cursors() {
        let w = Workspace::new();
        let p = w.path(1);
        let mut writer = Writer::open(&p).unwrap();
        for n in 0..14_000 {
            writeln!(writer, "line-{n}").unwrap();
        }
        let raw = fs::read(&p).unwrap();
        assert!(lengths(&raw).len() <= MAX_LINES);
        let first = read(&p, Some(0), None, 123).unwrap();
        assert!(first.reset && first.base_offset > 0);
        assert_eq!(first.offset, first.base_offset);
        let next = read(&p, Some(first.next_offset), None, 123).unwrap();
        assert!(!next.reset);
        let last = read(&p, None, Some(128), 128).unwrap();
        assert!(last.text.contains("line-13999"));
        assert!(last.eof && fs::metadata(p).unwrap().len() <= MAX_BYTES);
    }
    #[test]
    fn huge_unterminated_lines_are_byte_bounded_and_retain_the_tail() {
        let w = Workspace::new();
        let p = w.path(1);
        let mut writer = Writer::open(&p).unwrap();
        for _ in 0..1400 {
            writer.write_all(&[b'x'; 8192]).unwrap();
        }
        writer.write_all(b"LATEST_END").unwrap();
        assert!(fs::metadata(&p).unwrap().len() <= MAX_BYTES);
        assert!(
            read(&p, None, Some(64), 64)
                .unwrap()
                .text
                .ends_with("LATEST_END")
        );
    }
    #[test]
    fn global_count_eviction_allows_a_live_writer_to_resume_without_cursor_reuse() {
        let w = Workspace::new();
        let p = w.path(1);
        let mut first = Writer::open(&p).unwrap();
        first.write_all(b"before\n").unwrap();
        for id in 2..=55 {
            append(&w.path(id), b"other\n").unwrap();
        }
        assert!(!p.exists());
        first.write_all(b"after\n").unwrap();
        let page = read(&p, Some(7), None, 100).unwrap();
        assert_eq!(page.text, "after\n");
        assert_eq!(page.base_offset, 7);
        assert!(!page.reset);
        assert!(read(&p, Some(0), None, 100).unwrap().reset);
        assert!(
            fs::read_dir(&w.0)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|e| managed(&e.path()))
                .count()
                <= MAX_FILES
        );
    }
    #[test]
    fn startup_migrates_large_files_enforces_global_budget_and_expiry_without_touching_other_files()
    {
        let w = Workspace::new();
        for id in 1..=4 {
            File::create(w.path(id))
                .unwrap()
                .set_len(MAX_TOTAL_BYTES / 3 + 1)
                .unwrap();
        }
        let old = w.path(99);
        fs::write(&old, b"expired\n").unwrap();
        File::options()
            .write(true)
            .open(&old)
            .unwrap()
            .set_times(
                fs::FileTimes::new()
                    .set_modified(SystemTime::now() - MAX_AGE - Duration::from_secs(60)),
            )
            .unwrap();
        fs::write(w.0.join("business.log"), b"not owned\n").unwrap();
        fs::write(w.0.join("run-100.log.next"), b"orphaned compaction").unwrap();
        maintain(&w.0, true).unwrap();
        assert!(!old.exists());
        assert!(!w.0.join("run-100.log.next").exists());
        let mut total = 0;
        for e in fs::read_dir(&w.0)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|e| managed(&e.path()))
        {
            let size = e.metadata().unwrap().len();
            assert!(size <= MAX_BYTES);
            total += size;
        }
        assert!(total <= MAX_TOTAL_BYTES);
        assert_eq!(fs::read(w.0.join("business.log")).unwrap(), b"not owned\n");
    }
    #[test]
    fn concurrent_streams_and_journal_share_atomic_retention_and_reject_symlinks() {
        let w = Workspace::new();
        let p = w.path(1);
        let writer = Writer::open(&p).unwrap();
        let threads: Vec<_> = (0..4)
            .map(|id| {
                let mut writer = writer.clone();
                std::thread::spawn(move || {
                    for i in 0..3000 {
                        writeln!(writer, "worker-{id}-{i}").unwrap();
                    }
                })
            })
            .collect();
        for thread in threads {
            thread.join().unwrap();
        }
        let journal = crate::lifecycle::Journal::new(p.clone(), |_, _| {});
        journal.note("FINAL_JOURNAL");
        assert!(
            read(&p, None, Some(128), 128)
                .unwrap()
                .text
                .contains("FINAL_JOURNAL")
        );
        assert!(lengths(&fs::read(&p).unwrap()).len() <= MAX_LINES);
        let link = w.path(100);
        std::os::unix::fs::symlink(&p, &link).unwrap();
        assert!(Writer::open(&link).is_err());
        assert!(read(&link, None, None, 64).is_err());
    }
}

#[cfg(test)]
#[test]
fn failed_quota_cleanup_never_appends_more_disk_bytes() {
    use std::os::unix::fs::PermissionsExt;
    if unsafe { libc::geteuid() } == 0 {
        return;
    }
    let directory =
        std::env::temp_dir().join(format!("ds-log-quota-error-{}", crate::model::unique_id()));
    fs::create_dir(&directory).unwrap();
    let path = directory.join("run-1.log");
    let mut writer = Writer::open(&path).unwrap();
    writer.write_all(b"before\n").unwrap();
    for id in [2, 3] {
        File::create(directory.join(format!("run-{id}.log")))
            .unwrap()
            .set_len(MAX_TOTAL_BYTES / 2)
            .unwrap();
    }
    writer.hub.lock().unwrap().directories.clear();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o500)).unwrap();
    let result = writer.write_all(b"MUST_NOT_APPEND\n");
    let retained = fs::read(&path).unwrap();
    fs::set_permissions(&directory, fs::Permissions::from_mode(0o700)).unwrap();
    fs::remove_dir_all(directory).unwrap();
    assert!(result.is_err());
    assert_eq!(retained, b"before\n");
}
