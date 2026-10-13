//! Minimal dependency-free logger.
//!
//! Everything goes to stderr so it shows up in the Cinnamon session log / when
//! the daemon is launched from a terminal. Verbosity is controlled by the
//! `CLIPRUST_LOG` environment variable (`error`, `warn`, `info`, `debug`),
//! defaulting to `info`. We intentionally avoid the `log`/`env_logger` crates
//! to keep the dependency graph empty.

use std::sync::atomic::{AtomicU8, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const LEVEL_ERROR: u8 = 1;
pub const LEVEL_WARN: u8 = 2;
pub const LEVEL_INFO: u8 = 3;
pub const LEVEL_DEBUG: u8 = 4;

static LEVEL: AtomicU8 = AtomicU8::new(LEVEL_INFO);

/// Read `CLIPRUST_LOG` once at startup and cache the resolved level.
pub fn init_from_env() {
    let level = match std::env::var("CLIPRUST_LOG").ok().as_deref() {
        Some("error") => LEVEL_ERROR,
        Some("warn") => LEVEL_WARN,
        Some("debug") | Some("trace") => LEVEL_DEBUG,
        _ => LEVEL_INFO,
    };
    LEVEL.store(level, Ordering::Relaxed);
}

fn enabled(level: u8) -> bool {
    level <= LEVEL.load(Ordering::Relaxed)
}

/// Seconds since the epoch. We format the raw number rather than pulling in a
/// date crate just to print a timestamp nobody parses.
fn stamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn emit(tag: &str, msg: &str) {
    eprintln!("[{}] cliprust {}: {}", stamp(), tag, msg);
}

pub fn error(msg: &str) {
    if enabled(LEVEL_ERROR) {
        emit("ERROR", msg);
    }
}

pub fn warn(msg: &str) {
    if enabled(LEVEL_WARN) {
        emit("WARN ", msg);
    }
}

pub fn info(msg: &str) {
    if enabled(LEVEL_INFO) {
        emit("INFO ", msg);
    }
}

pub fn debug(msg: &str) {
    if enabled(LEVEL_DEBUG) {
        emit("DEBUG", msg);
    }
}
