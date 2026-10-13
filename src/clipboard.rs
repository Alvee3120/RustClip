//! Clipboard monitoring.
//!
//! We watch the `CLIPBOARD` selection only — `PRIMARY` (the "select to copy"
//! buffer) is deliberately left alone so that merely highlighting text never
//! pollutes the history.
//!
//! Monitoring is fully event-driven: GTK's `owner-change` signal fires whenever
//! the selection owner changes, so there is zero polling and therefore zero CPU
//! while idle. Stage 1 handles text and URLs; images arrive in stage 2.
//!
//! On X11 this is exactly what we want. On Wayland GTK3 cannot observe other
//! clients' clipboard changes at all — see README for the consequences.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::log;
use crate::storage::Kind;
use crate::App;

/// Attach the owner-change handler. Call once, after GTK is initialised.
pub fn start(app: &Rc<RefCell<App>>) {
    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    let app_for_signal = app.clone();
    let clipboard_for_signal = clipboard.clone();

    // gtk-rs does *not* generate a typed `connect_owner_change`: the signal's
    // GdkEventOwnerChange argument is not a bindable type, so the generated
    // handler is omitted. We therefore connect to the signal by name with the
    // generic local handler. The event payload is unused — we simply re-read the
    // clipboard, which is all we need.
    let _ = clipboard.connect_local("owner-change", false, move |_values| {
        on_owner_change(&app_for_signal, &clipboard_for_signal);
        None
    });

    log::info("clipboard monitor attached (CLIPBOARD selection only)");
}

/// Handle a single owner-change event.
///
/// Note on blocking: `wait_for_targets`/`wait_for_text` synchronously ask the
/// selection owner for data. For a local X11 clipboard this completes in well
/// under a millisecond; if the owning application is wedged the call can block
/// briefly, which is the accepted tradeoff for the simplicity of not threading
/// asynchronous clipboard requests through the main loop.
fn on_owner_change(app: &Rc<RefCell<App>>, clipboard: &gtk::Clipboard) {
    let ignore_hint = app.borrow().config.ignore_password_hint;
    if ignore_hint && advertises_password_hint(clipboard) {
        log::debug("skipping clipboard change: password-manager hint present");
        return;
    }

    // Stage 1: text/URL only. `None` means the payload is non-text (e.g. an
    // image); stage 2 will handle that case via request_image.
    let text = match clipboard.wait_for_text() {
        Some(t) => t.to_string(),
        None => return,
    };

    let cfg = app.borrow().config.clone();
    if text.len() as u64 > cfg.size_limit_bytes {
        log::debug(&format!(
            "skipping {}-byte text (limit {} bytes)",
            text.len(),
            cfg.size_limit_bytes
        ));
        return;
    }

    // Ignore the echo of our own paste-back (set in ui::activate_index). We
    // clear the marker so the *next* genuine change is not accidentally eaten.
    {
        let mut a = app.borrow_mut();
        if a.last_own_write.as_deref() == Some(text.as_str()) {
            a.last_own_write = None;
            log::debug("ignoring clipboard echo of our own write");
            return;
        }
    }

    let kind = detect_kind(&text);
    let changed = {
        let mut a = app.borrow_mut();
        let changed = a.store.push_text(kind, text, &cfg);
        if changed {
            if let Err(e) = a.store.save() {
                log::error(&format!("history save failed: {}", e));
            }
        }
        changed
    };

    if changed {
        // Only touch the UI when the popup is actually on screen.
        crate::ui::refresh_if_visible(app);
    }
}

/// True when the clipboard advertises the KDE "this is a secret" hint target,
/// which KeePassXC and other password managers set. We check only for the
/// target's *presence*: reading its value requires an extra round-trip and the
/// convention is that presence alone means "do not record".
fn advertises_password_hint(clipboard: &gtk::Clipboard) -> bool {
    // `wait_for_targets` yields `gdk::Atom`s; `Atom::name` resolves them to the
    // target string. `Atom` is Copy, so dereferencing in the closure is fine.
    match clipboard.wait_for_targets() {
        Some(targets) => targets.iter().any(|t| {
            let name = t.name();
            name.as_str() == "x-kde-passwordManagerHint"
                || name.as_str() == "application/x-kde-passwordManagerHint"
        }),
        None => false,
    }
}

/// Classify text as a URL only when it looks unambiguous: an http(s) scheme and
/// no internal whitespace. Everything else is plain text.
fn detect_kind(text: &str) -> Kind {
    let t = text.trim();
    if (t.starts_with("http://") || t.starts_with("https://")) && !t.contains(char::is_whitespace) {
        Kind::Url
    } else {
        Kind::Text
    }
}
