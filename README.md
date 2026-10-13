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
| 2 | Images + links (thumbnails, PNG sidecars, URL detection/icon) | **done, built & verified** |
| 3 | Pinning, per-item delete, "Clear all", autostart polish | **done, built & verified** |
| 4 | Auto-paste (Ctrl+V), `.deb` packaging, size/RAM tuning | **done, built & verified** |

Stages 1–4 ship a complete history for text, URLs and images: captures are
written to disk (images as PNG sidecars with a small thumbnail), listed in the
popup, and copied back on selection. Items can be pinned, deleted individually,
or cleared in bulk. With `auto_paste` enabled, selecting an item also sends Ctrl+V
to the window you were in. A `.deb` can be built with `cargo deb`.

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
your next login via `~/.config/autostart/cliprust.desktop` (a couple of seconds
after login — `X-GNOME-Autostart-Delay=2` — so the session is ready first). Pass
`--no-autostart` if you would rather launch the daemon yourself and not have an
autostart entry created.

## Usage

| Command | Effect |
|---------|--------|
| `cliprust` | Run the daemon (normally done by autostart) |
| `cliprust --toggle` | Show/hide the popup — this is what Super+V runs |
| `cliprust --show` / `--hide` | Force the popup visible / hidden |
| `cliprust --ping` | `PONG` if the daemon is running |
| `cliprust --clear` | Remove all unpinned entries (pinned are kept) |
| `cliprust --quit` | Ask the daemon to exit |
| `cliprust --help` / `--version` | Help / version |

**Inside the popup:** `↑`/`↓` move, `Enter` copies the highlighted item, `1`–`9`
copy item by position, `Esc` closes. Clicking outside (focus loss) also closes.
Text and URL rows copy their text back; **image rows put the original PNG back
on the clipboard**. The footer line lists the editing keys:

| Key | Action |
|-----|--------|
| `P` | Pin/unpin the highlighted item (pinned items survive eviction and clears) |
| `Delete` | Remove the highlighted item |
| `Ctrl+Delete` | Remove every unpinned item (pinned are kept) |

Pinned items are shown with a "pinned" marker and keep their position in the
list; they are simply exempt from eviction and from "clear". Interactively,
Ctrl+Delete and `cliprust --clear` do the same thing.

Set verbosity with `CLIPRUST_LOG`:

```
CLIPRUST_LOG=debug cliprust
```

## Configuration

Edit `~/.config/cliprust/config.json` (created on first run; restart the daemon
after editing). Missing keys fall back to their defaults, so a config written by
an older version keeps working:

| Key | Default | Meaning |
|-----|---------|---------|
| `max_items` | `20` | Retained unpinned entries (pinned items are never evicted) |
| `auto_paste` | `false` | Also synthesise Ctrl+V on selection (see below) |
| `size_limit_bytes` | `5242880` | Skip clipboard payloads larger than this (5 MB) |
| `ignore_password_hint` | `true` | Honour `x-kde-passwordManagerHint` and skip those items |
| `popup_width` / `popup_height` | `520` / `480` | Popup size in pixels |
| `preview_chars` | `80` | Characters shown per row |
| `paste_delay_ms` | `120` | Delay before auto-paste (focus hand-back time) |
| `thumbnail_px` | `40` | Thumbnail height for image rows (width follows the aspect ratio) |
| `image_size_limit_bytes` | `20971520` | Skip clipboard images whose encoded PNG exceeds this (20 MB) |

**State locations:** history in `~/.local/share/cliprust/history.json`, images
(full PNG plus a thumbnail sidecar) in `~/.local/share/cliprust/images/`, config
in `~/.config/cliprust/`, IPC socket in `$XDG_RUNTIME_DIR/cliprust/ipc.sock`.

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

Off by default. When enabled (`auto_paste: true`), selecting an item copies it,
hides the popup, re-activates the window that was focused before the popup
opened, waits `paste_delay_ms`, and finally synthesises Ctrl+V via the X11 XTEST
extension — so the item lands where you were typing.

Why it is opt-in: to receive `↑`/`↓`/`Enter` the popup must take keyboard focus,
so the window you were typing in stops being focused. Auto-paste has to remember
that window, hand focus back, and *then* send the key — a race that is usually
reliable but not guaranteed. If you see the key land in the wrong place, raise
`paste_delay_ms`.

Implementation notes:

- The previously focused window is captured from `_NET_ACTIVE_WINDOW` (falling
  back to the input focus) just before the popup is shown.
- Focus hand-back uses the EWMH `_NET_ACTIVE_WINDOW` client message carrying the
  real event timestamp, because window managers reject a zero timestamp.
- Key synthesis uses XTEST via `x11rb`. If the display has no XTEST (or no
  `v`/Control keycodes) the daemon logs a warning once at startup and auto-paste
  does nothing rather than misbehaving. `CLIPRUST_LOG=debug` shows each step.

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

This produces `target/debian/cliprust_<version>_amd64.deb` (~220 KB). It installs
`/usr/bin/cliprust`, the README and the licence under `/usr/share/doc/cliprust/`,
and declares the runtime libraries it links against (GTK3, GdkPixbuf, glib, libc)
via `depends = "$auto"`.

The package installs the **binary only** — the autostart entry and Super+V
binding are per-user and are still best set by `install.sh`.

## Size and idle RAM

Measured on **Linux Mint 22.2 "Zara" / Cinnamon 6.4.14 / X11**, GTK 3.24.41,
Rust 1.99.0, 8 cores, with the release profile (`opt-level="z"`, `lto="fat"`,
`codegen-units=1`, `panic="abort"`, `strip=true`):

| Metric | Measured |
|--------|----------|
| Binary on disk | **512 KB** (524,080 bytes), stripped, dynamically linked |
| Idle RSS (daemon up, popup never shown) | **28.5 MB** (29,176 kB) |
| Idle CPU | ~0% (event-driven; no polling) |
| Threads | 4 |
| Popup window | 520 px wide (per `popup_width`) |

Reproduce:

```
ls -lh target/release/cliprust
pid=$(pgrep -x cliprust); grep VmRSS /proc/$pid/status
```

The binary is small because GTK is linked **dynamically** — it is not baked into
the executable. Images add no crate: GdkPixbuf (already part of GTK) encodes and
decodes the PNG sidecars. Auto-paste adds one crate (`x11rb`): it costs ~60 kB of
RSS and no extra idle thread.

**Popup cost.** Idle is ~28.5 MB; opening the popup for the first time raises RSS
to ~37 MB and it stays there. That ~8 MB is one-time GTK window/graphics
initialisation, not per-row memory. `hide()` now *destroys* the popup window
(rebuilding it on the next show) so the row widgets and thumbnails are freed —
verified by watching the X window disappear — but the freed pages are not handed
back to the OS, so RSS does not fall.

**On the spec's `<20 MB` idle-RAM goal:** 28.5 MB is the honest number and it
does not meet the goal. That floor is GTK's: glib/pango/gdk-pixbuf/Cairo/fontconfig
are mapped as soon as GTK initialises, *before* any window exists, so unlike
binary size it cannot be tuned away with compiler flags. Getting under 20 MB
requires not using GTK for the popup at all (a raw X11 window lands around
3–6 MB, at the cost of theming and a lot more code). This tradeoff was raised
during planning and GTK3 was chosen deliberately.

Measured results, in order of impact:

1. **Destroy the popup on hide** — done. The window really is destroyed between
   uses (verified), but as noted above it does not move RSS: the ~8 MB belongs to
   GTK's one-time window/graphics setup, not to the widget tree.
2. **`opt-level`:** benchmarked `"z"` against `3`. `"z"` wins on size — **512 KB**
   versus **581 KB** (595,368 bytes) for `3` — so the release profile keeps `"z"`.
   Both use `lto="fat"`, `codegen-units=1`, `panic="abort"`, `strip=true`.
3. Trim with `cargo-bloat` / `twiggy` to see what dominates the binary.
4. If RAM matters far more than theming, the non-GTK X11 popup is the only way to
   get under ~10 MB — and it is also where the ~8 MB popup cost would disappear.

## Design notes / dependencies

Five crates, each justified in `Cargo.toml`:

- `gtk` — clipboard `owner-change`, the popup, and Cinnamon-matching widgets.
  Also brings **GdkPixbuf**, which encodes/decodes the image PNG sidecars and
  scales thumbnails, so no separate image crate is needed.
- `glib` — main-loop integration for the IPC socket (`unix_fd_add_local`).
- `serde` + `serde_json` — the JSON history/config.
- `x11rb` — XTEST key synthesis and EWMH focus handling for auto-paste. Pure Rust
  (no libxcb, no C build step); only the `xtest` feature is enabled.

Deliberately avoided: `rusqlite` (C build for a ≤20-row dataset), `image`/`png`
(GdkPixbuf already decodes and thumbnails PNG), `log`/`env_logger` (a ~40-line
stderr logger in `src/log.rs` suffices), and `xdotool` (shelling out to an
external tool is worse than the few XTEST calls we make directly).

**Image memory:** a captured pixbuf is encoded to PNG and dropped immediately;
only the two small sidecar files remain on disk. Thumbnails are generated once,
at capture time, so the popup never scales an image while drawing. Nothing
image-sized is retained by the daemon between captures.

The IPC socket path is `/proc`-free, built on `std::os::unix::net`, and
single-instance safe: the daemon tries to *connect* before binding, so a second
launch refuses to start rather than racing over the history file.

## Development

```sh
cargo fmt --all
cargo clippy --all-targets -- -D warnings
cargo test                       # headless unit tests (no display needed)
./scripts/smoke-test.sh          # end-to-end, isolated from your own session
```

The unit tests live next to the code (`storage`, `config`, `image`, URL
detection) and run without an X server. The smoke test starts the real daemon
against a private temp tree, drives the clipboard with `xclip`, and checks the
JSON history and image sidecars; with no `$DISPLAY` it re-execs under `xvfb-run`.

CI (`.github/workflows/ci.yml`) runs `fmt` + `clippy -D warnings` + tests + the
release build, then the smoke test, on every push and pull request. Pushing a tag
like `v0.1.0` triggers `.github/workflows/release.yml`, which builds the `.deb`
with `cargo-deb` and attaches it to a GitHub Release.

## Contributing

Changes land through a pull request to `main`, and **every PR requires an
approving review from the maintainer (@Alvee3120)**. `CODEOWNERS` marks every
path as owned by them; `scripts/protect-branch.sh` applies the matching branch
protection (no direct pushes, code-owner review required, CI must pass). See
[CONTRIBUTING.md](CONTRIBUTING.md).

## Troubleshooting

- **Super+V does nothing.** Check the daemon is alive with `cliprust --ping`.
  If the shortcut is missing, re-run `./install.sh --no-deps`. Verify the
  binding in *System Settings → Keyboard → Shortcuts → Custom Shortcuts*.
- **Popup opens in the wrong place.** It uses `WindowPosition::Mouse`; some WMs
  override it. This is cosmetic.
- **Auto-paste does nothing.** It is off by default — set `auto_paste: true` and
  restart the daemon. Run with `CLIPRUST_LOG=debug`: if XTEST is unavailable the
  daemon logs a warning at startup. If the paste lands in the wrong window, raise
  `paste_delay_ms`.
- **An image row shows a generic icon instead of a thumbnail.** Its thumbnail
  sidecar is missing or unreadable; the entry still copies back. Delete the item
  to prune the stale entry.
- **Nothing is recorded from my password manager.** That is the
  `x-kde-passwordManagerHint` exclusion working as intended.
- **Startup fails with `path must be shorter than SUN_LEN`.** `XDG_RUNTIME_DIR`
  is too long for a Unix socket path (the whole path must fit in ~108 bytes).
  This does not happen with the normal `/run/user/1000`; if you are launching
  under a wrapper that sets a long runtime dir, unset `XDG_RUNTIME_DIR` and the
  socket falls back to `~/.cache/cliprust`.
