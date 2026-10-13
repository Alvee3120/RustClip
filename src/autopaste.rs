//! Optional X11 auto-paste (stage 4).
//!
//! Selecting an entry normally just puts it back on the clipboard. With
//! `auto_paste` enabled we additionally hand focus back to the window that was
//! active before the popup opened, wait `paste_delay_ms`, and synthesise Ctrl+V
//! so the item lands where the user was typing.
//!
//! This is the only place cliprust needs the XTEST extension, and the reason for
//! the single `x11rb` dependency (pure Rust, no libxcb). Everything here is
//! best-effort: on any protocol error we log and give up rather than disturbing
//! the UI, because focus hand-back is inherently racy (see README "Auto-paste").

use std::time::Duration;

use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    self, AtomEnum, ClientMessageData, ClientMessageEvent, EventMask, InputFocus,
};
use x11rb::protocol::xtest;
use x11rb::rust_connection::RustConnection;
use x11rb::x11_utils::Serialize;

use crate::log;

/// Keysym for the lowercase `v` (so we never need Shift).
const KEYSYM_V: u32 = 0x0076;
/// Keysym for the left Control key.
const KEYSYM_CONTROL_L: u32 = 0xffe3;

const KEY_PRESS: u8 = 2;
const KEY_RELEASE: u8 = 3;

/// A connection used purely for focus management and XTEST key synthesis. Kept
/// separate from GDK's own connection; the X server is happy with both.
pub struct X11 {
    conn: RustConnection,
    root: u32,
    min_keycode: u8,
    keysyms_per_keycode: u8,
    keysyms: Vec<u32>,
    net_active_window: u32,
    have_xtest: bool,
}

impl X11 {
    /// Open the connection and cache the keyboard mapping. Returns `None` (with
    /// a log line) when there is no usable X display — auto-paste then simply
    /// does nothing rather than aborting the daemon.
    pub fn connect() -> Option<X11> {
        let (conn, screen_num) = match x11rb::connect(None) {
            Ok(v) => v,
            Err(e) => {
                log::warn(&format!(
                    "auto-paste unavailable: cannot open X display: {}",
                    e
                ));
                return None;
            }
        };

        let setup = conn.setup();
        let root = setup.roots.get(screen_num)?.root;
        let min_keycode = setup.min_keycode;
        let max_keycode = setup.max_keycode;

        let (keysyms_per_keycode, keysyms) =
            match xproto::get_keyboard_mapping(&conn, min_keycode, max_keycode - min_keycode + 1)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
            {
                Some(reply) => (reply.keysyms_per_keycode, reply.keysyms),
                None => (0, Vec::new()),
            };

        let net_active_window = xproto::intern_atom(&conn, false, b"_NET_ACTIVE_WINDOW")
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.atom)
            .unwrap_or(x11rb::NONE);

        // Probe XTEST so we can warn once at startup instead of failing quietly
        // on the first paste.
        let have_xtest = xtest::get_version(&conn, 2, 2)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .is_some();
        if !have_xtest {
            log::warn("auto-paste: XTEST extension not available; Ctrl+V cannot be synthesised");
        } else {
            log::info("auto-paste: XTEST ready");
        }

        Some(X11 {
            conn,
            root,
            min_keycode,
            keysyms_per_keycode,
            keysyms,
            net_active_window,
            have_xtest,
        })
    }

    /// True when key synthesis can actually work here.
    pub fn usable(&self) -> bool {
        self.have_xtest && self.keysyms_per_keycode > 0
    }

    /// The window the user was interacting with: the EWMH active window when the
    /// WM publishes it, otherwise whatever currently holds input focus.
    pub fn active_window(&self) -> Option<u32> {
        if self.net_active_window != x11rb::NONE {
            let reply = xproto::get_property(
                &self.conn,
                false,
                self.root,
                self.net_active_window,
                AtomEnum::WINDOW,
                0,
                1,
            )
            .ok()
            .and_then(|cookie| cookie.reply().ok());
            if let Some(reply) = reply {
                if let Some(win) = reply.value32().and_then(|mut it| it.next()) {
                    if win != x11rb::NONE {
                        return Some(win);
                    }
                }
            }
        }
        // Fallback for WMs without EWMH (or a bare server). PointerRoot (1) and
        // None (0) are not real windows, so treat them as "unknown".
        xproto::get_input_focus(&self.conn)
            .ok()
            .and_then(|cookie| cookie.reply().ok())
            .map(|reply| reply.focus)
            .filter(|&w| w != x11rb::NONE && w != 1)
    }

    /// Ask the WM to make `win` active again (raise + focus), falling back to a
    /// direct focus request when EWMH is not available.
    ///
    /// `timestamp` must be a real server timestamp (from the event that triggered
    /// the paste); WMs reject `_NET_ACTIVE_WINDOW` with a zero timestamp as a
    /// "buggy client" and silently do nothing.
    pub fn activate(&self, win: u32, timestamp: u32) -> Result<(), String> {
        if self.net_active_window != x11rb::NONE {
            let event = ClientMessageEvent {
                response_type: xproto::CLIENT_MESSAGE_EVENT,
                format: 32,
                sequence: 0,
                window: win,
                type_: self.net_active_window,
                // data.l[0] = source indication (1 = application), l[1] = time.
                data: ClientMessageData::from([1u32, timestamp, 0, 0, 0]),
            };
            let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
            let ok = xproto::send_event(&self.conn, false, self.root, mask, event.serialize())
                .ok()
                .and_then(|cookie| cookie.check().ok())
                .is_some();
            if ok {
                let _ = self.conn.flush();
                log::debug(&format!("auto-paste: asked the WM to activate {:#x}", win));
                return Ok(());
            }
        }
        log::debug("auto-paste: no EWMH; focusing the window directly");
        let cookie = xproto::set_input_focus(
            &self.conn,
            InputFocus::POINTER_ROOT,
            win,
            x11rb::CURRENT_TIME,
        )
        .map_err(|e| format!("set_input_focus failed: {}", e))?;
        cookie
            .check()
            .map_err(|e| format!("set_input_focus failed: {}", e))?;
        let _ = self.conn.flush();
        Ok(())
    }

    /// Press and release Ctrl+V.
    pub fn send_ctrl_v(&self) -> Result<(), String> {
        if !self.usable() {
            return Err("XTEST or keyboard mapping unavailable".to_string());
        }
        let ctrl = self
            .keycode(KEYSYM_CONTROL_L)
            .ok_or("no Control_L keycode")?;
        let v = self.keycode(KEYSYM_V).ok_or("no `v` keycode")?;
        log::debug(&format!("auto-paste: keycodes ctrl={} v={}", ctrl, v));

        self.fake(KEY_PRESS, ctrl)?;
        self.fake(KEY_PRESS, v)?;
        self.fake(KEY_RELEASE, v)?;
        self.fake(KEY_RELEASE, ctrl)?;
        self.conn
            .flush()
            .map_err(|e| format!("flush failed: {}", e))?;
        log::debug("auto-paste: sent Ctrl+V");
        Ok(())
    }

    fn fake(&self, kind: u8, keycode: u8) -> Result<(), String> {
        let cookie = xtest::fake_input(
            &self.conn,
            kind,
            keycode,
            x11rb::CURRENT_TIME,
            self.root,
            0,
            0,
            0,
        )
        .map_err(|e| format!("xtest fake_input failed: {}", e))?;
        cookie
            .check()
            .map_err(|e| format!("xtest fake_input failed: {}", e))
    }

    /// Keysym -> keycode using the unshifted (level 0) column of the mapping.
    fn keycode(&self, keysym: u32) -> Option<u8> {
        if self.keysyms_per_keycode == 0 {
            return None;
        }
        let stride = self.keysyms_per_keycode as usize;
        self.keysyms
            .chunks(stride)
            .position(|row| row.first().copied() == Some(keysym))
            .map(|i| self.min_keycode + i as u8)
    }
}

/// Hand focus back to `prev`, wait for the WM to settle, then synthesise Ctrl+V.
/// Split out from the UI so the sequencing (and the reason for the delay) reads
/// in one place.
pub fn deliver(x11: &X11, prev: u32, delay_ms: u64, timestamp: u32) -> Result<(), String> {
    log::debug(&format!(
        "auto-paste: re-activating {:#x} (ts {}) in {} ms",
        prev, timestamp, delay_ms
    ));
    x11.activate(prev, timestamp)?;
    std::thread::sleep(Duration::from_millis(delay_ms));
    x11.send_ctrl_v()
}
