//! XDG path resolution.
//!
//! All state lives under the standard XDG locations so the app behaves like a
//! well-mannered desktop citizen:
//!
//!   * history + images  -> $XDG_DATA_HOME/cliprust   (~/.local/share/cliprust)
//!   * config            -> $XDG_CONFIG_HOME/cliprust (~/.config/cliprust)
//!   * IPC socket        -> $XDG_RUNTIME_DIR/cliprust/ipc.sock
//!
//! Using XDG_RUNTIME_DIR for the socket keeps it off disk and on a
//! user-private tmpfs, and keeps the path well under the ~108 byte sun_path
//! limit of Unix domain sockets.

use std::path::PathBuf;

fn env_path(var: &str, fallback: PathBuf) -> PathBuf {
    match std::env::var_os(var) {
        Some(v) if !v.is_empty() => PathBuf::from(v),
        _ => fallback,
    }
}

pub fn home() -> PathBuf {
    env_path("HOME", PathBuf::from("/tmp"))
}

pub fn data_dir() -> PathBuf {
    env_path("XDG_DATA_HOME", home().join(".local/share")).join("cliprust")
}

pub fn config_dir() -> PathBuf {
    env_path("XDG_CONFIG_HOME", home().join(".config")).join("cliprust")
}

/// Runtime directory for the IPC socket. Rarely unset on a real session, but
/// fall back to ~/.cache so we still work under e.g. a bare `su` shell.
pub fn runtime_dir() -> PathBuf {
    match std::env::var_os("XDG_RUNTIME_DIR") {
        Some(v) if !v.is_empty() => PathBuf::from(v).join("cliprust"),
        _ => home().join(".cache/cliprust"),
    }
}

pub fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn history_file() -> PathBuf {
    data_dir().join("history.json")
}

/// Directory holding full-size PNGs for image history entries (stage 2).
pub fn image_dir() -> PathBuf {
    data_dir().join("images")
}

pub fn socket_path() -> PathBuf {
    runtime_dir().join("ipc.sock")
}

/// Create a directory (and parents) if missing. Returns an error string that is
/// safe to log; callers decide whether to abort or continue.
pub fn ensure_dir(dir: &std::path::Path) -> Result<(), String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {}", dir.display(), e))
}
