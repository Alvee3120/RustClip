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

use std::cell::RefCell;
use std::rc::Rc;

use gtk::prelude::*;
use gtk::pango;

use crate::log;
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
    window.add(&scroller);

    // Close on focus loss (click outside, alt-tab, another window appears).
    window.connect_focus_out_event(|w, _ev| {
        w.hide();
        glib::Propagation::Proceed
    });

    // Keyboard navigation.
    {
        let app_key = app.clone();
        window.connect_key_press_event(move |w, ev| {
            handle_key(&app_key, w, ev);
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

/// Hide the popup if it exists. Idempotent.
pub fn hide(app: &Rc<RefCell<App>>) {
    if let Some(window) = app.borrow().window.clone() {
        window.hide();
    }
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

/// Build one `index  kind  preview  [pin]` row.
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

    let badge = match item.kind {
        Kind::Url => "link",
        Kind::Image => "image",
        Kind::Text => "text",
    };
    let kind_label = gtk::Label::new(Some(badge));
    kind_label.set_width_chars(5);
    kind_label.set_xalign(0.0);
    kind_label.set_sensitive(false);
    hbox.pack_start(&kind_label, false, false, 0);

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

/// Translate a key press into navigation/activation actions.
fn handle_key(app: &Rc<RefCell<App>>, window: &gtk::Window, ev: &gtk::gdk::EventKey) {
    use gtk::gdk::keys::constants as key;

    match ev.keyval() {
        key::Escape => window.hide(),
        key::Return => activate_selected(app),
        key::Up => move_selection(app, -1),
        key::Down => move_selection(app, 1),
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

/// Copy the row at `index` back to the clipboard and close the popup.
fn activate_index(app: &Rc<RefCell<App>>, index: i32) {
    if index < 0 {
        return;
    }
    let id = match app.borrow().shown.get(index as usize).copied() {
        Some(id) => id,
        None => return,
    };

    // Pull the text out; images are not selectable yet (stage 2).
    let (text, auto_paste) = {
        let a = app.borrow();
        let item = match a.store.items.iter().find(|it| it.id == id) {
            Some(i) => i,
            None => return,
        };
        (
            item.clipboard_text().map(|s| s.to_string()),
            a.config.auto_paste,
        )
    };
    let text = match text {
        Some(t) => t,
        None => {
            log::warn("selected an image entry, which stage 1 cannot copy back");
            return;
        }
    };

    log::debug(&format!("copying entry {} back to the clipboard", id));

    // Mark this write so the clipboard monitor does not promote it to the top
    // (it is already there).
    app.borrow_mut().last_own_write = Some(text.clone());

    let clipboard = gtk::Clipboard::get(&gtk::gdk::SELECTION_CLIPBOARD);
    clipboard.set_text(&text);
    // store() hands the selection to the clipboard manager so it survives the
    // process losing ownership (and, if a manager is configured, a reboot).
    clipboard.store();

    hide(app);

    if auto_paste {
        // Stage 4: re-activate the previously focused window, wait
        // config.paste_delay_ms, then synthesise Ctrl+V via XTest.
        log::info("auto_paste is enabled but Ctrl+V synthesis lands in stage 4");
    }
}
