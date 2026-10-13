//! Clipboard monitoring.
//!
//! We watch the `CLIPBOARD` selection only — `PRIMARY` (the "select to copy"
//! buffer) is deliberately left alone so that merely highlighting text never
//! pollutes the history.
//!
//! Monitoring is fully event-driven: GTK's `owner-change` signal fires whenever
//! the selection owner changes, so there is zero polling and therefore zero CPU
//! while idle. Text, URLs and images are all captured; images are encoded to PNG
//! sidecars (plus a thumbnail) by the storage layer.
//!
//! On X11 this is exactly what we want. On Wayland GTK3 cannot observe other
//! clients' clipboard changes at all — see README for the consequences.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;

use crate::config::Config;
use crate::image;
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
/// Note on blocking: `wait_for_targets`/`wait_for_image`/`wait_for_text`
/// synchronously ask the selection owner for data. For a local X11 clipboard
/// this completes in well under a millisecond; if the owning application is
/// wedged the call can block briefly, which is the accepted tradeoff for the
/// simplicity of not threading asynchronous clipboard requests through the main
/// loop.
fn on_owner_change(app: &Rc<RefCell<App>>, clipboard: &gtk::Clipboard) {
    let cfg = app.borrow().config.clone();

    // One round-trip for the target list; reused for both the password-manager
    // check and the image-vs-text decision below.
    let targets = clipboard.wait_for_targets().unwrap_or_default();

    if cfg.ignore_password_hint && targets.iter().any(is_password_hint) {
        log::debug("skipping clipboard change: password-manager hint present");
        return;
    }

    // Prefer an image when the payload offers one: copying a picture often also
    // exports its URL or alt text as a text target, and the picture is what the
    // user meant to keep.
    if targets.iter().any(is_image_target) {
        if let Some(pixbuf) = clipboard.wait_for_image() {
            handle_image(app, &pixbuf, &cfg);
            return;
        }
    }

    let text = match clipboard.wait_for_text() {
        Some(t) => t.to_string(),
        None => return,
    };

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

/// Capture an image payload: encode it to PNG, build a thumbnail, and hand both
/// to the store. Runs entirely on the main thread, like the text path.
fn handle_image(app: &Rc<RefCell<App>>, pixbuf: &gtk::gdk_pixbuf::Pixbuf, cfg: &Config) {
    let (width, height) = (pixbuf.width(), pixbuf.height());
    if width <= 0 || height <= 0 {
        return;
    }

    let png = match image::encode_png(pixbuf) {
        Ok(bytes) => bytes,
        Err(e) => {
            log::warn(&format!("could not encode clipboard image: {}", e));
            return;
        }
    };
    if png.len() as u64 > cfg.image_size_limit_bytes {
        log::debug(&format!(
            "skipping {}-byte image (limit {} bytes)",
            png.len(),
            cfg.image_size_limit_bytes
        ));
        return;
    }
    let hash = image::hash_bytes(&png);

    // Ignore the echo of our own image paste-back, mirroring the text path.
    {
        let mut a = app.borrow_mut();
        if a.last_own_write_hash == Some(hash) {
            a.last_own_write_hash = None;
            log::debug("ignoring clipboard echo of our own image write");
            return;
        }
    }

    // A missing thumbnail is not fatal: the row falls back to a generic icon.
    let thumb = match image::thumbnail(pixbuf, cfg.thumbnail_px)
        .map(|t| image::encode_png(&t))
        .transpose()
    {
        Ok(t) => t,
        Err(e) => {
            log::warn(&format!("thumbnail encode failed: {}", e));
            None
        }
    };

    let changed = {
        let mut a = app.borrow_mut();
        match a.store.push_image(
            &png,
            thumb.as_deref(),
            hash,
            width as u32,
            height as u32,
            cfg,
        ) {
            Ok(changed) => {
                if changed {
                    if let Err(e) = a.store.save() {
                        log::error(&format!("history save failed: {}", e));
                    }
                }
                changed
            }
            Err(e) => {
                log::error(&format!("could not store image: {}", e));
                false
            }
        }
    };

    if changed {
        crate::ui::refresh_if_visible(app);
    }
}

/// True when the clipboard advertises the KDE "this is a secret" hint target,
/// which KeePassXC and other password managers set. We check only for the
/// target's *presence*: reading its value requires an extra round-trip and the
/// convention is that presence alone means "do not record".
fn is_password_hint(atom: &gtk::gdk::Atom) -> bool {
    let name = atom.name();
    name.as_str() == "x-kde-passwordManagerHint"
        || name.as_str() == "application/x-kde-passwordManagerHint"
}

/// True for any `image/*` target, which is how we decide a change carries an
/// image rather than (or in addition to) text.
fn is_image_target(atom: &gtk::gdk::Atom) -> bool {
    atom.name().as_str().starts_with("image/")
}

/// Classify text as a URL only when it looks unambiguous: a known scheme with no
/// internal whitespace, or a bare `www.`-prefixed host. Everything else is plain
/// text. Deliberately conservative so we do not badger ordinary prose.
fn detect_kind(text: &str) -> Kind {
    let t = text.trim();
    if t.contains(char::is_whitespace) {
        return Kind::Text;
    }
    const SCHEMES: [&str; 5] = ["http://", "https://", "ftp://", "ftps://", "www."];
    if SCHEMES.iter().any(|s| t.starts_with(s)) {
        Kind::Url
    } else {
        Kind::Text
    }
}
