//! Minimal file logger + panic hook.
//!
//! The GUI runs without a console (windows_subsystem = "windows"), so without
//! this any panic or diagnostic disappears into the void. Everything goes to
//! `<app_data_dir>/gui.log`; panics are logged with location and backtrace.

use once_cell::sync::OnceCell;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static LOG: OnceCell<Mutex<std::fs::File>> = OnceCell::new();
static LOG_PATH: OnceCell<PathBuf> = OnceCell::new();

const MAX_LOG_BYTES: u64 = 1_000_000;

pub fn init(dir: &Path) {
    let _ = std::fs::create_dir_all(dir);
    let path = dir.join("gui.log");

    // Simple rotation: keep one previous generation.
    if let Ok(meta) = std::fs::metadata(&path) {
        if meta.len() > MAX_LOG_BYTES {
            let _ = std::fs::rename(&path, dir.join("gui.log.old"));
        }
    }

    if let Ok(file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        let _ = LOG.set(Mutex::new(file));
        let _ = LOG_PATH.set(path);
    }

    // Panics: log them before the process (or thread) dies. Chained so the
    // default hook still prints to stderr in dev runs.
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let location = info
            .location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "<unknown>".into());
        let payload = info
            .payload()
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "<non-string panic>".into());
        let backtrace = std::backtrace::Backtrace::force_capture();
        log(&format!(
            "PANIC at {}: {}\nbacktrace:\n{}",
            location, payload, backtrace
        ));
        default_hook(info);
    }));
}

pub fn log(msg: &str) {
    let line = format!(
        "[{}] {}",
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f"),
        msg
    );
    eprintln!("{}", line);
    if let Some(file) = LOG.get() {
        if let Ok(mut f) = file.lock() {
            let _ = writeln!(f, "{}", line);
            let _ = f.flush();
        }
    }
}

/// Redact a deep link for logging: scheme + first chars + length, never the
/// full payload (it contains credentials).
pub fn redact_link(uri: &str) -> String {
    let prefix: String = uri.chars().take(24).collect();
    format!("{}… ({} chars)", prefix, uri.chars().count())
}
