//! Shared local timestamps and bounded-unit elapsed formatting for backend presentation DTOs.
use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub fn duration(seconds: u64) -> String {
    let mut text = String::new();
    for (amount, unit) in [
        (seconds / 86400, "天"),
        (seconds / 3600 % 24, "小时"),
        (seconds / 60 % 60, "分钟"),
    ] {
        if amount > 0 {
            text.push_str(&format!("{amount}{unit}"));
        }
    }
    text.push_str(&format!("{}秒", seconds % 60));
    text
}
pub fn local_timestamp(seconds: u64) -> String {
    let Ok(seconds) = libc::time_t::try_from(seconds) else {
        return "时间超出范围".into();
    };
    let mut local = std::mem::MaybeUninit::<libc::tm>::uninit();
    if unsafe { libc::localtime_r(&seconds, local.as_mut_ptr()) }.is_null() {
        return "时间未知".into();
    }
    let local = unsafe { local.assume_init() };
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        local.tm_year + 1900,
        local.tm_mon + 1,
        local.tm_mday,
        local.tm_hour,
        local.tm_min,
        local.tm_sec
    )
}
/// ps lstart is local time; numeric validation and mktime round-trip reject malformed evidence.
pub fn process_start(text: &str) -> Result<SystemTime, String> {
    let parts: Vec<_> = text.split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!("无法解析进程启动时间: {text}"));
    }
    let month = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ]
    .iter()
    .position(|m| *m == parts[1])
    .ok_or("进程启动月份无效")? as i32;
    let parse = |s: &str| {
        s.parse::<i32>()
            .map_err(|_| "进程启动时间数字无效".to_string())
    };
    let day = parse(parts[2])?;
    let year = parse(parts[4])?;
    let hms: Vec<_> = parts[3].split(':').collect();
    if hms.len() != 3 {
        return Err("进程启动时分秒无效".into());
    }
    let hour = parse(hms[0])?;
    let minute = parse(hms[1])?;
    let second = parse(hms[2])?;
    if !(1970..=9999).contains(&year)
        || !(1..=31).contains(&day)
        || !(0..=23).contains(&hour)
        || !(0..=59).contains(&minute)
        || !(0..=59).contains(&second)
    {
        return Err("进程启动时间超出范围".into());
    }
    let mut local: libc::tm = unsafe { std::mem::zeroed() };
    local.tm_year = year - 1900;
    local.tm_mon = month;
    local.tm_mday = day;
    local.tm_hour = hour;
    local.tm_min = minute;
    local.tm_sec = second;
    local.tm_isdst = -1;
    let seconds = unsafe { libc::mktime(&mut local) };
    if seconds < 0
        || (
            local.tm_year,
            local.tm_mon,
            local.tm_mday,
            local.tm_hour,
            local.tm_min,
            local.tm_sec,
        ) != (year - 1900, month, day, hour, minute, second)
    {
        return Err("进程启动日期无效".into());
    }
    UNIX_EPOCH
        .checked_add(Duration::from_secs(seconds as u64))
        .ok_or("进程启动时间溢出".into())
}
#[derive(serde::Serialize)]
pub struct RunTiming {
    pub started_at_unix_ms: Option<u64>,
    pub started_at: Option<String>,
    pub elapsed_seconds: u64,
    pub elapsed_display: String,
    pub display: String,
}
impl crate::runtime::Run {
    pub fn timing(&self) -> RunTiming {
        let elapsed = self.elapsed_offset.saturating_add(if self.state.active() {
            self.started.elapsed()
        } else {
            self.elapsed
        });
        let elapsed_seconds = elapsed.as_secs();
        let elapsed_display = duration(elapsed_seconds);
        let epoch = self
            .started_at
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok());
        let started_at = epoch.map(|t| local_timestamp(t.as_secs()));
        let started_at_unix_ms = epoch.and_then(|t| t.as_millis().try_into().ok());
        let display = if let Some(timestamp) = &started_at {
            format!("{timestamp} - {elapsed_display}")
        } else if self.external.is_some() {
            format!("启动时间未知 - 已监测 {elapsed_display}")
        } else {
            format!("未启动 - {elapsed_display}")
        };
        RunTiming {
            started_at_unix_ms,
            started_at,
            elapsed_seconds,
            elapsed_display,
            display,
        }
    }
}
