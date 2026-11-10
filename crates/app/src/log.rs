//! A small append-only log beside the settings file.
//!
//! There is no console in a GUI build, and the interesting failures (a click that
//! could not be replayed, a pick that could not be raised) are not visible to the
//! user either. A capped log makes those reportable.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::Mutex;

static LOG: Mutex<Option<std::fs::File>> = Mutex::new(None);

/// The file this module writes. The settings page opens the same path, so the
/// name lives here once.
pub fn path() -> std::path::PathBuf {
    crate::settings::data_dir().join("shaizero.log")
}

/// Opens the log. Called once, early, before anything else can fail.
pub fn init() {
    let path = path();
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    rotate(&path);
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .ok();
    if let Ok(mut guard) = LOG.lock() {
        *guard = file;
    }
    write(format_args!(
        "--- ShaiZero {} (pid {}) ---",
        env!("CARGO_PKG_VERSION"),
        std::process::id()
    ));
}

fn rotate(path: &Path) {
    const LIMIT: u64 = 256 * 1024;
    let too_big = std::fs::metadata(path)
        .map(|m| m.len() > LIMIT)
        .unwrap_or(false);
    if too_big {
        let _ = std::fs::rename(path, path.with_extension("old.log"));
    }
}

pub fn write(args: std::fmt::Arguments) {
    let Ok(mut guard) = LOG.lock() else { return };
    let Some(file) = guard.as_mut() else { return };
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let _ = writeln!(file, "{stamp} {args}");
    let _ = file.flush();
}

/// `log!("wheel opened at {}, {}", x, y)`.
#[macro_export]
macro_rules! log {
    ($($arg:tt)*) => {
        $crate::log::write(format_args!($($arg)*))
    };
}
