//! Release identity keeps the public edition independent from existing personal runtime data.
use std::{
    io,
    path::{Path, PathBuf},
};
pub fn edition() -> &'static str {
    if cfg!(feature = "personal") {
        "personal"
    } else {
        "open-source"
    }
}
pub fn default_data_root(home: &Path, personal: bool, macos: bool) -> PathBuf {
    let name = if personal { "dev-start" } else { "devstart" };
    home.join(if macos {
        "Library/Application Support"
    } else {
        ".local/share"
    })
    .join(name)
}
pub fn workspace() -> io::Result<PathBuf> {
    if let Some(path) = std::env::var_os("DEV_START_DATA_DIR") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").ok_or_else(|| io::Error::other("HOME is unavailable"))?;
    Ok(default_data_root(
        Path::new(&home),
        cfg!(feature = "personal"),
        cfg!(target_os = "macos"),
    ))
}
