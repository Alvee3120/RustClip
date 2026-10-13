# cliprust

A lightweight, Win+V style clipboard history manager for **Cinnamon on X11**
(Linux Mint). One background daemon watches the clipboard; **Super+V** pops up a
list of recent items near the pointer; click an item (or press `Enter`, an arrow
key, or a digit) to copy it back.

Built in Rust with GTK3 so it looks and behaves like the rest of your desktop.

## Status

Development proceeds in stages, with a stop for review between each:

| Stage | Scope | State |
|-------|-------|-------|
| 1 | Text-only MVP: daemon, capture, popup, copy-back, persistence, IPC, installer | **done, built & verified** |
| 2 | Images + links (thumbnails, PNG sidecars, URL detection/icon) | planned |
| 3 | Pinning, per-item delete, "Clear all", autostart polish | planned |
| 4 | Auto-paste (Ctrl+V), `.deb` packaging, size/RAM tuning | planned |

Stage 1 ships a working end-to-end text clipboard history. Image rows are stored
and listed but cannot yet be copied back or previewed.

## Requirements

- Linux Mint / Cinnamon, **X11 session** (see [Wayland](#wayland) below).
- Build: `libgtk-3-dev`, `build-essential`, `pkg-config`, and a Rust toolchain
  (Rust 1.74+). `install.sh` installs all of these for you.
- Runtime: GTK3 runtime libraries (already present on any Cinnamon system).

## Build

```
cargo build --release
```

The binary lands at `target/release/cliprust`.

## Install

One command — installs dependencies, builds, copies the binary to
`~/.local/bin`, sets up autostart, and binds Super+V:

```
./install.sh
```

Skip the apt step if the build deps are already installed:

```
./install.sh --no-deps
```

After it finishes, press **Super+V**. The daemon also starts automatically at
your next login via `~/.config/autostart/cliprust.desktop`.

## Usage

| Command | Effect |
|---------|--------|
| `cliprust` | Run the daemon (normally done by autostart) |
| `cliprust --toggle` | Show/hide the popup — this is what Super+V runs |
| `cliprust --show` / `--hide` | Force the popup visible / hidden |
| `cliprust --ping` | `PONG` if the daemon is running |
| `cliprust --quit` | Ask the daemon to exit |
| `cliprust --help` / `--version` | Help / version |

**Inside the popup:** `↑`/`↓` move, `Enter` copies the highlighted item, `1`–`9`
copy item by position, `Esc` closes. Clicking outside (focus loss) also closes.

Set verbosity with `CLIPRUST_LOG`:

```
CLIPRUST_LOG=debug cliprust
```

## Configuration

Edit `~/.config/cliprust/config.json` (created on first run; restart the daemon
after editing):

| Key | Default | Meaning |
|-----|---------|---------|
| `max_items` | `20` | Retained unpinned entries (pinned items are never evicted) |
| `auto_paste` | `false` | Also synthesise Ctrl+V on selection (see below) |
| `size_limit_bytes` | `5242880` | Skip clipboard payloads larger than this (5 MB) |
| `ignore_password_hint` | `true` | Honour `x-kde-passwordManagerHint` and skip those items |
| `popup_width` / `popup_height` | `520` / `480` | Popup size in pixels |
| `preview_chars` | `80` | Characters shown per row |
| `paste_delay_ms` | `120` | Delay before auto-paste (focus hand-back time) |

**State locations:** history in `~/.local/share/cliprust/history.json`, images
(stage 2) in `~/.local/share/cliprust/images/`, config in
`~/.config/cliprust/`, IPC socket in `$XDG_RUNTIME_DIR/cliprust/ipc.sock`.

## Uninstall

```
./install.sh --uninstall
```

This stops the daemon and removes the binary, the autostart entry, and the
Super+V shortcut. Your history and config are **kept**; the script prints the
exact paths so you can delete them too if you want.

### Reversing the hotkey by hand

If you ever want to bind Super+V yourself (e.g. to another manager), the
installer's entry lives under
`org.cinnamon.desktop.keybindings.custom-list`. Remove the `cliprust` entry from
that list (Cinnamon *System Settings → Keyboard → Shortcuts → Custom Shortcuts*
does the same thing), or just run `./install.sh --uninstall`.

## Auto-paste

Off by default. When enabled (`auto_paste: true`), selecting an item first
copies it, then after `paste_delay_ms` re-activates the previously focused
window and synthesises Ctrl+V via the X11 XTEST extension.

Why it is opt-in: to receive `↑`/`↓`/`Enter` the popup must take keyboard focus,
which means the window you were typing in stops being focused. Auto-paste has to
remember that window, hand focus back, and *then* send the key — a race that is
usually reliable but not guaranteed. If you see the key land in the wrong place,
raise `paste_delay_ms`.

> Not yet implemented: the key-synthesis step lands in **stage 4**. Until then
> the flag is accepted and logged but does nothing.

## Wayland

**This app targets X11 only**, and Cinnamon runs on X11, so that is fine for
Linux Mint. It will *not* work correctly under a Wayland session, for two
independent reasons:

1. **Clipboard monitoring.** Wayland gives clients no way to observe the global
   clipboard when they are not focused — there is no equivalent of X11's
   selection `owner-change` notification. A GTK3 app simply never learns that
   another app copied something.
2. **Global hotkeys.** There is no compositor-independent way to register a
   global shortcut; you must go through the compositor (KDE's
   `org.kde.kglobalaccel`, GNOME's `org.gnome.Shell.Extensions`, Hyprland's
   `bind`, etc.).

Supporting Wayland would require: reading the clipboard via the
`wlr-data-control` protocol (e.g. `wl-paste --watch`), drawing the popup as a
`wlr-layer-shell` surface instead of a normal GTK window (a plain window will be
positioned by the compositor, not at the pointer), and registering the hotkey
through each compositor's own API. That is a different program, not a flag.

## Packaging (.deb)

`cargo-deb` metadata is already in `Cargo.toml`. Build a package with:

```
cargo install cargo-deb
cargo deb
```

The `.deb` installs `/usr/bin/cliprust`. Note that the package installs the
binary only — the autostart entry and Super+V binding are per-user and are still
best set by `install.sh`.

## Size and idle RAM

Measured on **Linux Mint 22.2 "Zara" / Cinnamon 6.4.14 / X11**, GTK 3.24.41,
Rust 1.99.0, 8 cores, with the release profile (`opt-level="z"`, `lto="fat"`,
`codegen-units=1`, `panic="abort"`, `strip=true`):

| Metric | Measured |
|--------|----------|
| Binary on disk | **400 KB** (408,008 bytes), stripped, dynamically linked |
| Idle RSS (daemon up, popup never shown) | **28.6 MB** (28,648 kB) |
| Idle CPU | ~0% (event-driven; no polling) |
| Threads | 4 |
| Popup window | 520 px wide (per `popup_width`) |

Reproduce:

```
ls -lh target/release/cliprust
pid=$(pgrep -x cliprust); grep VmRSS /proc/$pid/status
```

The binary is small because GTK is linked **dynamically** — it is not baked into
the executable.

**On the spec's `<20 MB` idle-RAM goal:** 28.6 MB is the honest number and it
does not meet the goal. That floor is GTK's: glib/pango/gdk-pixbuf/Cairo/fontconfig
are mapped as soon as GTK initialises, *before* any window exists, so unlike
binary size it cannot be tuned away with compiler flags. Getting under 20 MB
requires not using GTK for the popup at all (a raw X11 window lands around
3–6 MB, at the cost of theming and a lot more code). This tradeoff was raised
during planning and GTK3 was chosen deliberately.

Further reductions, in order of impact:

1. **Destroy** the popup window in `hide()` instead of hiding it (rebuild in
   `ensure_window`): saves the widget tree after first use. The code is written
   to make this a small change.
2. Benchmark `opt-level = "3"` vs `"z"`. `"z"` minimises size but can be *slower*
   and occasionally larger after LTO; pick whichever wins for you.
3. Trim with `cargo-bloat` / `twiggy` to see what dominates the binary.
4. If RAM matters far more than theming, the non-GTK X11 popup is the only way
   to get under ~10 MB.

## Design notes / dependencies

Only four crates, each justified in `Cargo.toml`:

- `gtk` — clipboard `owner-change`, the popup, and Cinnamon-matching widgets.
- `glib` — main-loop integration for the IPC socket (`unix_fd_add_local`).
- `serde` + `serde_json` — the JSON history/config.

Deliberately avoided: `rusqlite` (C build for a ≤20-row dataset), `image`/`png`
(GdkPixbuf already decodes and thumbnails PNG), `log`/`env_logger` (a ~40-line
stderr logger in `src/log.rs` suffices), and `xdotool`/`x11rb` (only needed once
auto-paste is implemented).

The IPC socket path is `/proc`-free, built on `std::os::unix::net`, and
single-instance safe: the daemon tries to *connect* before binding, so a second
launch refuses to start rather than racing over the history file.

## Troubleshooting

- **Super+V does nothing.** Check the daemon is alive with `cliprust --ping`.
  If the shortcut is missing, re-run `./install.sh --no-deps`. Verify the
  binding in *System Settings → Keyboard → Shortcuts → Custom Shortcuts*.
- **Popup opens in the wrong place.** It uses `WindowPosition::Mouse`; some WMs
  override it. This is cosmetic.
- **Image copies are ignored.** Expected in stage 1; image support is stage 2.
- **Nothing is recorded from my password manager.** That is the
  `x-kde-passwordManagerHint` exclusion working as intended.
- **Startup fails with `path must be shorter than SUN_LEN`.** `XDG_RUNTIME_DIR`
  is too long for a Unix socket path (the whole path must fit in ~108 bytes).
  This does not happen with the normal `/run/user/1000`; if you are launching
  under a wrapper that sets a long runtime dir, unset `XDG_RUNTIME_DIR` and the
  socket falls back to `~/.cache/cliprust`.
