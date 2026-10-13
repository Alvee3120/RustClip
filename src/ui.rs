//! The history popup.
//!
//! Design choices worth calling out:
//!
//! * The window is built lazily on the first `show` and then *hidden*, not
//!   destroyed. Rebuilding a GtkWindow means reconnecting signal handlers,
//!   which is error-prone; hiding keeps a few KB of widget tree alive. If you
//!   want to chase the last megabyte of idle RAM, destroy it in `hide` and
//!   rebuild in `ensure_window` (documented in the README).
//! * Positioned with `WindowPosition::Mouse` so it appears under the pointer,
//!   matching Win+V muscle memory. Cinnamon's WM keeps it centred otherwise.
//! * It is a decoration-less `Toplevel` with a `PopupMenu` type hint: it looks
//!   like a menu but still receives keyboard focus (a real `WindowType::Popup`
//!   would not, and Up/Down/Enter would not reach us).
//! * Image rows show their stored thumbnail directly; the thumbnail was scaled
//!   to `thumbnail_px` at capture time, so no resizing happens while drawing.

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::pango;

use crate::log;
use crate::paths;
use crate::storage::{Item, Kind};
use crate::App;

/// Build the window on first use.
fn ensure_window(app: &Rc<RefCell<App>>) {
    if app.borrow().window.is_some() {
        return;
    }
    let (width, height) = {
        let c = &app.borrow().config;
        (c.popup_width, c.popup_height)
    };

    let window = gtk::Window::new(gtk::WindowType::Toplevel);
    window.set_title("Clipboard History");
    window.set_decorated(false);
    window.set_resizable(false);
    window.set_skip_taskbar_hint(true);
    window.set_skip_pager_hint(true);
    window.set_keep_above(true);
    window.set_accept_focus(true);
    window.set_can_focus(true);
    window.set_type_hint(gtk::gdk::WindowTypeHint::PopupMenu);
    window.set_position(gtk::WindowPosition::Mouse);
    window.set_default_size(width, -1);

    // Vertical scrolling only; the popup never grows horizontally.
    let scroller = gtk::ScrolledWindow::new(None::<&gtk::Adjustment>, None::<&gtk::Adjustment>);
    scroller.set_policy(gtk::PolicyType::Never, gtk::PolicyType::Automatic);
    scroller.set_max_content_height(height);
    scroller.set_propagate_natural_height(true);

    let listbox = gtk::ListBox::new();
    listbox.set_selection_mode(gtk::SelectionMode::Single);
    scroller.add(&listbox);

    // Footer with the key hints, so pin/delete/clear are discoverable without a
    // manual — the same role Win+V's per-row buttons play.
    let footer = gtk::Label::new(Some(
        "Enter copy · P pin · Del remove · Ctrl+Del clear all · Esc close",
    ));
    footer.set_sensitive(false);
    footer.set_margin_top(4);
    footer.set_margin_bottom(4);
    footer.set_margin_start(8);
    footer.set_margin_end(8);
    footer.set_ellipsize(pango::EllipsizeMode::End);

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 0);
    vbox.pack_start(&scroller, true, true, 0);
    vbox.pack_start(&footer, false, false, 0);
    window.add(&vbox);

    // Close on focus loss (click outside, alt-tab, another window appears).
    {
        let app_focus = app.clone();
        window.connect_focus_out_event(move |_w, _ev| {
            hide(&app_focus);
            glib::Propagation::Proceed
        });
    }

    // Keyboard navigation.
    {
        let app_key = app.clone();
        window.connect_key_press_event(move |_w, ev| {
            handle_key(&app_key, ev);
            glib::Propagation::Stop
        });
    }

    // Click or Enter on a row copies it back.
    {
        let app_row = app.clone();
        listbox.connect_row_activated(move |_, row| {
            activate_index(&app_row, row.index());
        });
    }

    app.borrow_mut().window = Some(window);
    app.borrow_mut().listbox = Some(listbox);
}

/// Show (and refresh) the popup.
pub fn show(app: &Rc<RefCell<App>>) {
    ensure_window(app);
    rebuild(app);

    let window = match app.borrow().window.clone() {
        Some(w) => w,
        None => return,
    };

    // Remember what had focus, so auto-paste can send Ctrl+V back to it.
    record_previous_window(app);

    window.show_all();
    window.present();

    // Preselect the newest item so Enter is a one-key paste of the last copy.
    if let Some(listbox) = app.borrow().listbox.clone() {
        if let Some(row) = listbox.row_at_index(0) {
            listbox.select_row(Some(&row));
        }
    }
    log::debug("popup shown");
}

/// Hide the popup and release its widget tree.
///
/// The window is *destroyed* rather than merely hidden, so the row widgets and
/// the thumbnails they hold are freed between uses; `ensure_window` rebuilds it
/// on the next show. Destruction is deferred to an idle callback so we never
/// tear down widgets from inside an event handler (focus-out, key press) that is
/// still dispatching to them.
pub fn hide(app: &Rc<RefCell<App>>) {
    let window = match app.borrow().window.clone() {
        Some(w) => w,
        None => return,
    };
    window.hide();

    let app = app.clone();
    glib::idle_add_local_once(move || {
        // A quick re-show may have intervened; only destroy if still hidden.
        let still_hidden = app
            .borrow()
            .window
            .as_ref()
            .map(|w| !w.is_visible())
            .unwrap_or(false);
        if !still_hidden {
            return;
        }
        if let Some(window) = app.borrow_mut().window.take() {
            // `close` is the safe equivalent of destroying the toplevel: with no
            // delete-event handler it tears the window (and its widget tree) down.
            window.close();
            log::debug("popup window destroyed");
        }
        app.borrow_mut().listbox = None;
    });
}

/// Toggle visibility — the action behind `Super+V`.
pub fn toggle(app: &Rc<RefCell<App>>) {
    let visible = app
        .borrow()
        .window
        .as_ref()
        .map(|w| w.is_visible())
        .unwrap_or(false);
    if visible {
        hide(app);
    } else {
        show(app);
    }
}

/// Refresh the rows if the popup is currently on screen (new clip captured).
pub fn refresh_if_visible(app: &Rc<RefCell<App>>) {
    let visible = app
        .borrow()
        .window
        .as_ref()
        .map(|w| w.is_visible())
        .unwrap_or(false);
    if visible {
        rebuild(app);
    }
}

/// Rebuild every row from the current history snapshot.
fn rebuild(app: &Rc<RefCell<App>>) {
    let listbox = match app.borrow().listbox.clone() {
        Some(l) => l,
        None => return,
    };

    // Snapshot under a short borrow, then build rows outside it: constructing
    // widgets must not happen while App is borrowed.
    let (items, preview_chars) = {
        let a = app.borrow();
        (a.store.items.clone(), a.config.preview_chars)
    };

    for child in listbox.children() {
        listbox.remove(&child);
    }

    let mut shown: Vec<u64> = Vec::with_capacity(items.len());
    if items.is_empty() {
        let label = gtk::Label::new(Some("Clipboard history is empty"));
        label.set_margin_top(12);
        label.set_margin_bottom(12);
        label.set_margin_start(12);
        label.set_margin_end(12);
        label.set_sensitive(false);
        listbox.add(&label);
    } else {
        for (i, item) in items.iter().enumerate() {
            let row = build_row(i, item, preview_chars);
            listbox.add(&row);
            shown.push(item.id);
        }
    }
    listbox.show_all();

    // Row index -> store id mapping used by activation.
    app.borrow_mut().shown = shown;
}

/// Build one `n  icon  preview  [pin]` row.
fn build_row(index: usize, item: &Item, preview_chars: usize) -> gtk::ListBoxRow {
    let row = gtk::ListBoxRow::new();
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    hbox.set_margin_top(6);
    hbox.set_margin_bottom(6);
    hbox.set_margin_start(8);
    hbox.set_margin_end(8);

    let number = gtk::Label::new(Some(&format!("{}", index + 1)));
    number.set_width_chars(2);
    number.set_xalign(1.0);
    number.set_sensitive(false);
    hbox.pack_start(&number, false, false, 0);

    // Leading icon: a link glyph for URLs, a generic text glyph for text, and
    // the stored thumbnail for images.
    let icon = match item.kind {
        Kind::Url => gtk::Image::from_icon_name(Some("insert-link"), gtk::IconSize::Menu)
            .upcast::<gtk::Widget>(),
        Kind::Text => gtk::Image::from_icon_name(Some("text-x-generic"), gtk::IconSize::Menu)
            .upcast::<gtk::Widget>(),
        Kind::Image => image_thumb_widget(item),
    };
    hbox.pack_start(&icon, false, false, 0);

    let preview = gtk::Label::new(None);
    preview.set_text(&item.preview(preview_chars));
    preview.set_ellipsize(pango::EllipsizeMode::End);
    preview.set_xalign(0.0);
    preview.set_hexpand(true);
    hbox.pack_start(&preview, true, true, 0);

    if item.pinned {
        let pin = gtk::Label::new(Some("pinned"));
        pin.set_sensitive(false);
        hbox.pack_end(&pin, false, false, 0);
    }

    row.add(&hbox);
    row
}

/// Leading widget for an image row: the saved thumbnail when it loads, else a
/// generic image icon. A missing sidecar is not an error — the entry may simply
/// have been created by an older build.
fn image_thumb_widget(item: &Item) -> gtk::Widget {
    if let Some(name) = &item.thumb {
        if let Ok(pixbuf) = gtk::gdk_pixbuf::Pixbuf::from_file(paths::image_dir().join(name)) {
            return gtk::Image::from_pixbuf(Some(&pixbuf)).upcast();
        }
        log::debug(&format!("thumbnail {} could not be loaded", name));
    }
    gtk::Image::from_icon_name(Some("image-x-generic"), gtk::IconSize::Menu).upcast()
}

/// Translate a key press into navigation/activation actions.
fn handle_key(app: &Rc<RefCell<App>>, ev: &gtk::gdk::EventKey) {
    use gtk::gdk::keys::constants as key;

    let ctrl = ev.state().contains(gtk::gdk::ModifierType::CONTROL_MASK);
    // Keep the event timestamp so _NET_ACTIVE_WINDOW (auto-paste) is accepted.
    app.borrow_mut().last_event_time = ev.time();

    match ev.keyval() {
        key::Escape => hide(app),
        key::Return => activate_selected(app),
        key::Up => move_selection(app, -1),
        key::Down => move_selection(app, 1),
        // Delete removes the highlighted entry; Ctrl+Delete clears everything
        // that is not pinned.
        key::Delete => {
            if ctrl {
                clear_history(app);
            } else {
                delete_selected(app);
            }
        }
        // P pins/unpins the highlighted entry so it survives eviction and clears.
        key::p | key::P => toggle_pin_selected(app),
        // Quick-select: the number keys 1..9 map to rows 0..8.
        key::_1 => activate_index(app, 0),
        key::_2 => activate_index(app, 1),
        key::_3 => activate_index(app, 2),
        key::_4 => activate_index(app, 3),
        key::_5 => activate_index(app, 4),
        key::_6 => activate_index(app, 5),
        key::_7 => activate_index(app, 6),
        key::_8 => activate_index(app, 7),
        key::_9 => activate_index(app, 8),
        _ => {}
    }
}

/// Index of the currently highlighted row, if any.
fn selected_index(app: &Rc<RefCell<App>>) -> Option<i32> {
    let listbox = app.borrow().listbox.clone()?;
    listbox.selected_row().map(|r| r.index())
}

/// Highlight the row at `index` if the list is long enough.
fn select_row(app: &Rc<RefCell<App>>, index: i32) {
    if index < 0 {
        return;
    }
    if let Some(listbox) = app.borrow().listbox.clone() {
        if let Some(row) = listbox.row_at_index(index) {
            listbox.select_row(Some(&row));
        }
    }
}

/// Persist the history, logging (not propagating) failures: the in-memory state
/// is already correct, and a failed write is not worth aborting the UI over.
fn persist(app: &Rc<RefCell<App>>) {
    if let Err(e) = app.borrow().store.save() {
        log::error(&format!("history save failed: {}", e));
    }
}

/// Store id behind the highlighted row, if any.
fn selected_id(app: &Rc<RefCell<App>>) -> Option<u64> {
    let index = selected_index(app)?;
    app.borrow().shown.get(index as usize).copied()
}

/// Toggle the pin flag on the highlighted entry, then rebuild so the row's
/// "pinned" marker updates. Pinned entries survive eviction and "clear".
fn toggle_pin_selected(app: &Rc<RefCell<App>>) {
    let (index, id) = match selected_index(app).zip(selected_id(app)) {
        Some(pair) => pair,
        None => return,
    };
    if !app.borrow_mut().store.toggle_pin(id) {
        return;
    }
    persist(app);
    rebuild(app);
    select_row(app, index);
}

/// Remove the highlighted entry (and its image sidecars, if any), keeping the
/// highlight at the same position when possible.
fn delete_selected(app: &Rc<RefCell<App>>) {
    let index = match selected_index(app) {
        Some(i) => i,
        None => return,
    };
    let id = match selected_id(app) {
        Some(id) => id,
        None => return,
    };
    if !app.borrow_mut().store.remove(id) {
        return;
    }
    persist(app);
    rebuild(app);
    let count = app
        .borrow()
        .listbox
        .as_ref()
        .map(|l| l.children().len() as i32)
        .unwrap_or(0);
    select_row(app, index.min(count - 1));
}

/// Remove every unpinned entry; pinned entries are kept. Shared by Ctrl+Delete
/// in the popup and `cliprust --clear`.
pub fn clear_history(app: &Rc<RefCell<App>>) {
    let changed = {
        let mut a = app.borrow_mut();
        let before = a.store.items.len();
        a.store.clear_unpinned();
        a.store.items.len() != before
    };
    if !changed {
        return;
    }
    persist(app);
    rebuild(app);
    select_row(app, 0);
}

/// Move the highlight by `delta`, wrapping around the ends.
fn move_selection(app: &Rc<RefCell<App>>, delta: i32) {
    let listbox = match app.borrow().listbox.clone() {
        Some(l) => l,
        None => return,
    };
    let count = listbox.children().len() as i32;
    if count == 0 {
        return;
    }
    let current = listbox.selected_row().map(|r| r.index()).unwrap_or(0);
    let next = (current + delta).rem_euclid(count);
    if let Some(row) = listbox.row_at_index(next) {
        listbox.select_row(Some(&row));
    }
}

/// Activate whatever row is currently selected.
fn activate_selected(app: &Rc<RefCell<App>>) {
    let listbox = match app.borrow().listbox.clone() {
        Some(l) => l,
        None => return,
    };
    let index = listbox.selected_row().map(|r| r.index()).unwrap_or(0);
    activate_index(app, index);
}

/// Copy the row at `index` back to the clipboard and close the popup. Text,
/// URLs and images are all supported.
fn activate_index(app: &Rc<RefCell<App>>, index: i32) {
    if index < 0 {
        return;
    }
    let id = match app.borrow().shown.get(index as usize).copied() {
        Some(id) => id,
        None => return,
    };

    // Snapshot everything needed, then release the borrow before touching GTK.
    let snapshot = {
        let a = app.borrow();
        let item = match a.store.items.iter().find(|it| it.id == id) {
            Some(i) => i,
            None => return,
        };
        (
            item.kind,
            item.clipboard_text().map(|s| s.to_string()),
            item.file.clone(),
            item.hash,
            a.config.auto_paste,
        )
    };
    let (kind, text, file, hash, auto_paste) = snapshot;

    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);

    match kind {
        Kind::Image => {
            let name = match file {
                Some(f) => f,
                None => {
                    log::warn("image entry has no sidecar file");
                    return;
                }
            };
            let pixbuf = match gtk::gdk_pixbuf::Pixbuf::from_file(paths::image_dir().join(&name)) {
                Ok(p) => p,
                Err(e) => {
                    log::warn(&format!("could not load image {}: {}", name, e));
                    return;
                }
            };
            log::debug(&format!("copying image entry {} back to the clipboard", id));
            // Mark our own write so the monitor ignores the echo.
            app.borrow_mut().last_own_write_hash = hash;
            clipboard.set_image(&pixbuf);
        }
        Kind::Text | Kind::Url => {
            let text = match text {
                Some(t) => t,
                None => return,
            };
            log::debug(&format!("copying entry {} back to the clipboard", id));
            app.borrow_mut().last_own_write = Some(text.clone());
            clipboard.set_text(&text);
        }
    }

    // store() hands the selection to the clipboard manager so it survives the
    // process losing ownership (and, if a manager is configured, a reboot).
    clipboard.store();

    hide(app);

    if auto_paste {
        perform_auto_paste(app);
    }
}

/// Remember the window that had focus before the popup opened, for auto-paste.
/// Only computed when auto-paste is enabled (it needs the X11 connection).
fn record_previous_window(app: &Rc<RefCell<App>>) {
    let prev = {
        let a = app.borrow();
        if a.config.auto_paste {
            a.x11.as_ref().and_then(|x| x.active_window())
        } else {
            None
        }
    };
    log::debug(&format!("auto-paste: previous window = {:?}", prev));
    app.borrow_mut().prev_active = prev;
}

/// Hand focus back to the pre-popup window, wait `paste_delay_ms`, then
/// synthesise Ctrl+V. Best-effort: failures are logged, never fatal.
fn perform_auto_paste(app: &Rc<RefCell<App>>) {
    let (prev, delay, timestamp) = {
        let a = app.borrow();
        (
            a.prev_active,
            a.config.paste_delay_ms.min(5000),
            a.last_event_time,
        )
    };
    let prev = match prev {
        Some(w) => w,
        None => {
            log::warn("auto-paste: no previous window was recorded");
            return;
        }
    };

    let result = {
        let a = app.borrow();
        match a.x11.as_ref() {
            Some(x11) => crate::autopaste::deliver(x11, prev, delay, timestamp),
            None => Err("no X11 connection available".to_string()),
        }
    };
    if let Err(e) = result {
        log::warn(&format!("auto-paste: {}", e));
    }
}
