//! Inter-process communication over a Unix domain socket.
//!
//! A second invocation (`cliprust --toggle`) is not a second daemon: it connects
//! to the running daemon and sends a one-line command, so the hotkey and the
//! daemon share a single source of truth for history and popup state.
//!
//! We use a Unix socket rather than D-Bus because it needs no session-bus
//! coupling and no extra crates (`std::os::unix::net` is in the standard
//! library). The listening socket is handed to the GLib main loop with
//! `glib::source::unix_fd_add_local`, so accepted commands are dispatched on the
//! same thread as the UI — no channels, no locking.
//!
//! Protocol: one command per connection, one reply line back.
//!   PING -> PONG            TOGGLE -> OK      SHOW -> OK
//!   HIDE -> OK              QUIT   -> OK      anything else -> "ERR ..."

use std::io::{BufRead, BufReader, Write};
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::cell::RefCell;
use std::rc::Rc;

use glib::ControlFlow;

use crate::{log, paths, ui, App};

/// Bind the socket and start dispatching commands on the GLib main loop.
///
/// Single-instance handling matters here: before binding we try to *connect*.
/// If that succeeds another daemon is alive and we refuse to start, which
/// prevents two daemons racing over the same history file. Only when the
/// connect fails do we treat the leftover socket file as stale and remove it.
pub fn serve(app: &Rc<RefCell<App>>) -> std::io::Result<()> {
    let path = paths::socket_path();
    if let Some(dir) = path.parent() {
        if let Err(e) = paths::ensure_dir(dir) {
            log::warn(&format!("could not create runtime dir: {}", e));
        }
    }

    if UnixStream::connect(&path).is_ok() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "another cliprust instance is already running",
        ));
    }
    // Stale socket from a crashed daemon; safe to remove now.
    let _ = std::fs::remove_file(&path);

    let listener = UnixListener::bind(&path)?;
    listener.set_nonblocking(true)?;
    let fd = listener.as_raw_fd();

    let app_for_ipc = app.clone();
    // The listener is moved into the closure so the fd stays open for the
    // lifetime of the source; the returned SourceId is kept in App so the
    // source is not dropped (which would tear the watch down).
    let source = glib::source::unix_fd_add_local(fd, glib::IOCondition::IN, move |_fd, _cond| {
        drain(&listener, &app_for_ipc);
        ControlFlow::Continue
    });
    app.borrow_mut().ipc_source = Some(source);
    log::info(&format!("ipc listening on {}", path.display()));
    Ok(())
}

/// Accept every pending connection without blocking the main loop.
fn drain(listener: &UnixListener, app: &Rc<RefCell<App>>) {
    loop {
        match listener.accept() {
            Ok((stream, _addr)) => handle(app, stream),
            Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) => {
                log::warn(&format!("ipc accept error: {}", e));
                break;
            }
        }
    }
}

fn handle(app: &Rc<RefCell<App>>, stream: UnixStream) {
    // Clone for writing; keep the original for the buffered reader.
    let mut writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            log::warn(&format!("ipc clone failed: {}", e));
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    if let Err(e) = reader.read_line(&mut line) {
        log::warn(&format!("ipc read failed: {}", e));
        return;
    }
    let cmd = line.trim().to_ascii_uppercase();
    let reply = dispatch(app, &cmd);
    let _ = writer.write_all(reply.as_bytes());
    let _ = writer.write_all(b"\n");
    let _ = writer.flush();
}

fn dispatch(app: &Rc<RefCell<App>>, cmd: &str) -> String {
    match cmd {
        "PING" => "PONG".to_string(),
        "TOGGLE" => {
            ui::toggle(app);
            "OK".to_string()
        }
        "SHOW" => {
            ui::show(app);
            "OK".to_string()
        }
        "HIDE" => {
            ui::hide(app);
            "OK".to_string()
        }
        "QUIT" => {
            ui::hide(app);
            gtk::main_quit();
            "OK".to_string()
        }
        other => format!("ERR unknown command: {}", other),
    }
}

/// Client side: connect, send one command, read the reply. Used by the
/// `--toggle`/`--show`/... command-line flags.
pub fn send(cmd: &str) -> Result<String, String> {
    let path = paths::socket_path();
    let stream = UnixStream::connect(&path).map_err(|e| {
        format!(
            "cannot reach the daemon at {} ({}). Is cliprust running?",
            path.display(),
            e
        )
    })?;
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    writer
        .write_all(format!("{}\n", cmd).as_bytes())
        .map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())?;

    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    reader.read_line(&mut line).map_err(|e| e.to_string())?;
    Ok(line.trim().to_string())
}
