//! cliprust — a Win+V style clipboard history manager for Cinnamon/X11.
//!
//! One binary plays two roles:
//!   * with no arguments it runs the long-lived daemon (started by autostart);
//!   * with `--toggle`/`--show`/... it is a thin client that talks to that
//!     daemon over a Unix socket. This is what the Super+V hotkey runs.
//!
//! All clipboard access has to happen on the process that owns the GTK main
//! loop, so the daemon is the single source of truth for history and popup
//! state; the client only sends a command and prints the reply.

mod autopaste;
mod clipboard;
mod config;
mod image;
mod ipc;
mod log;
mod paths;
mod storage;
mod ui;

use std::cell::RefCell;
use std::process::ExitCode;
use std::rc::Rc;

use config::Config;
use storage::Store;

/// Shared application state. Everything lives on the GTK main thread, so a
/// plain `Rc<RefCell<..>>` is sufficient — no locks, no `Send` requirements.
pub struct App {
    pub config: Config,
    pub store: Store,
    /// Last text we wrote to the clipboard ourselves, used to ignore the
    /// resulting owner-change echo (see `clipboard::on_owner_change`).
    pub last_own_write: Option<String>,
    /// Last image content hash we wrote to the clipboard ourselves (set on image
    /// paste-back) so the owner-change echo is ignored instead of being
    /// re-encoded and re-hashed.
    pub last_own_write_hash: Option<u64>,
    /// X11 connection for auto-paste (focus hand-back + XTEST key synthesis).
    /// `None` when the feature is off, or no display/XTEST is available.
    pub x11: Option<autopaste::X11>,
    /// Window that held focus before the popup opened, restored on auto-paste.
    pub prev_active: Option<u32>,
    /// Timestamp of the most recent key event; passed to `_NET_ACTIVE_WINDOW` so
    /// the WM accepts the focus request instead of rejecting a zero timestamp.
    pub last_event_time: u32,
    pub window: Option<gtk::Window>,
    pub listbox: Option<gtk::ListBox>,
    /// Kept alive so the IPC fd watch is not dropped/removed.
    pub ipc_source: Option<glib::SourceId>,
    /// Row index -> store id for the currently displayed list.
    pub shown: Vec<u64>,
}

fn main() -> ExitCode {
    log::init_from_env();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str).unwrap_or("");

    match command {
        "" | "--daemon" | "run" => run_daemon(),
        "--toggle" => client("TOGGLE"),
        "--show" => client("SHOW"),
        "--hide" => client("HIDE"),
        "--quit" => client("QUIT"),
        "--ping" => client("PING"),
        "--clear" => client("CLEAR"),
        "--help" | "-h" => {
            print_help();
            ExitCode::SUCCESS
        }
        "--version" | "-V" => {
            println!("cliprust {}", env!("CARGO_PKG_VERSION"));
            ExitCode::SUCCESS
        }
        other => {
            eprintln!("cliprust: unknown argument `{}`\n", other);
            print_help();
            ExitCode::FAILURE
        }
    }
}

/// Send a single command to a running daemon and print the reply.
fn client(command: &str) -> ExitCode {
    match ipc::send(command) {
        Ok(reply) => {
            println!("{}", reply);
            if reply.starts_with("ERR") {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(e) => {
            eprintln!("cliprust: {}", e);
            ExitCode::FAILURE
        }
    }
}

/// Run the daemon: initialise GTK, load state, attach the clipboard monitor and
/// the IPC socket, then hand control to the GTK main loop.
fn run_daemon() -> ExitCode {
    if let Err(e) = gtk::init() {
        log::error(&format!("failed to initialise GTK: {}", e));
        return ExitCode::FAILURE;
    }

    let config = Config::load();
    let store = Store::load();
    // Only open the extra X connection when auto-paste is actually wanted.
    let x11 = if config.auto_paste {
        autopaste::X11::connect()
    } else {
        None
    };
    let app = Rc::new(RefCell::new(App {
        config,
        store,
        last_own_write: None,
        last_own_write_hash: None,
        x11,
        prev_active: None,
        last_event_time: 0,
        window: None,
        listbox: None,
        ipc_source: None,
        shown: Vec::new(),
    }));

    // Binding first means a second `cliprust` invocation fails fast instead of
    // racing over the history file.
    if let Err(e) = ipc::serve(&app) {
        log::error(&format!("cannot start IPC: {}", e));
        return ExitCode::FAILURE;
    }

    clipboard::start(&app);
    log::info("cliprust daemon started");

    gtk::main();

    // Best-effort cleanup of the socket on a clean exit.
    let _ = std::fs::remove_file(paths::socket_path());
    log::info("cliprust daemon stopped");
    ExitCode::SUCCESS
}

fn print_help() {
    println!(
        "cliprust {version} — clipboard history for Cinnamon/X11

USAGE:
    cliprust [COMMAND]

COMMANDS:
    (none)      Run the background daemon (normally via autostart)
    --toggle    Show the popup if hidden, hide it if shown (bind this to Super+V)
    --show      Show the popup
    --hide      Hide the popup
    --ping      Check whether the daemon is running
    --clear     Remove all unpinned history entries (pinned are kept)
    --quit      Ask the daemon to exit
    -h, --help  Print this help
    -V, --version
                Print the version

ENVIRONMENT:
    CLIPRUST_LOG=error|warn|info|debug   Log verbosity (default: info)",
        version = env!("CARGO_PKG_VERSION")
    );
}
