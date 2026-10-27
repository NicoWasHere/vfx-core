//! One-line notes about how the plugin is rendering: which path (CPU or GPU-native),
//! which bit depths, and any fallback. Each distinct note is written once per session,
//! so the file stays a few lines long.
//!
//! macOS: `~/Library/Logs/VfxAdobe.log`. Windows: `%TEMP%\VfxAdobe.log`.

use std::collections::HashSet;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Mutex;

static SEEN: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub fn note(message: &str) {
    let mut seen = SEEN.lock().unwrap_or_else(|e| e.into_inner());
    if !seen
        .get_or_insert_with(HashSet::new)
        .insert(message.to_owned())
    {
        return;
    }
    drop(seen);
    let Some(path) = log_path() else { return };
    // Logging must never disturb a render, so write errors are ignored.
    if let Ok(mut file) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
    {
        let _ = writeln!(file, "[pid {}] {message}", std::process::id());
    }
}

fn log_path() -> Option<PathBuf> {
    if cfg!(target_os = "macos") {
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Logs/VfxAdobe.log"))
    } else {
        std::env::var_os("TEMP").map(|t| PathBuf::from(t).join("VfxAdobe.log"))
    }
}
