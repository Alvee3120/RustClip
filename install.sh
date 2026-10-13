#!/usr/bin/env bash
#
# cliprust installer for Linux Mint / Cinnamon (X11).
#
# What it does:
#   1. installs build dependencies (libgtk-3-dev, build-essential, pkg-config)
#      via apt, using sudo only for that step;
#   2. installs the Rust toolchain via rustup if `cargo` is missing;
#   3. builds the release binary;
#   4. copies it to ~/.local/bin/cliprust;
#   5. writes an autostart .desktop entry;
#   6. registers Super+V as a Cinnamon custom shortcut running `cliprust --toggle`;
#   7. optionally starts the daemon now.
#
# Everything after step 1 is user-local (no root) and fully reversible with
# `./install.sh --uninstall`.
#
# Usage:
#   ./install.sh              install (or upgrade)
#   ./install.sh --no-deps    skip the apt dependency step
#   ./install.sh --uninstall  remove binary, autostart entry and shortcut
#   ./install.sh --help

set -euo pipefail

BIN_NAME="cliprust"
INSTALL_DIR="${HOME}/.local/bin"
BIN_PATH="${INSTALL_DIR}/${BIN_NAME}"
AUTOSTART_DIR="${HOME}/.config/autostart"
AUTOSTART_FILE="${AUTOSTART_DIR}/${BIN_NAME}.desktop"
HOTKEY="<Super>v"

# Colours only when attached to a terminal.
if [ -t 1 ]; then
    C_OK=$'\033[32m'; C_WARN=$'\033[33m'; C_ERR=$'\033[31m'; C_OFF=$'\033[0m'
else
    C_OK=""; C_WARN=""; C_ERR=""; C_OFF=""
fi
info() { printf '%s==>%s %s\n' "${C_OK}" "${C_OFF}" "$*"; }
warn() { printf '%s==>%s %s\n' "${C_WARN}" "${C_OFF}" "$*" >&2; }
die()  { printf '%s==>%s %s\n' "${C_ERR}" "${C_OFF}" "$*" >&2; exit 1; }

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"

# ---------------------------------------------------------------------------
# arg parsing
# ---------------------------------------------------------------------------
MODE="install"
SKIP_DEPS=0
for arg in "$@"; do
    case "$arg" in
        --uninstall) MODE="uninstall" ;;
        --no-deps)   SKIP_DEPS=1 ;;
        -h|--help)   sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'; exit 0 ;;
        *)           die "unknown argument: $arg" ;;
    esac
done

# ---------------------------------------------------------------------------
# Cinnamon shortcut management (shared by install/uninstall)
# ---------------------------------------------------------------------------
# We drive gsettings from a small Python helper because the custom shortcut
# list is a GVariant array that must be *merged*, not overwritten, or we would
# wipe the user's other custom shortcuts.
run_hotkey_helper() {
    python3 - "$@" <<'PYEOF'
import ast
import subprocess
import sys

BASE = "org.cinnamon.desktop.keybindings"
RELO = BASE + ".custom-keybinding:/org/cinnamon/desktop/keybindings/custom-keybindings/{}/"
NAME = "ClipRust Clipboard History"
BINDING = "['<Super>v']"


def gs_get(schema, key):
    return subprocess.check_output(["gsettings", "get", schema, key], text=True).strip()


def gs_set(schema, key, value):
    subprocess.check_call(["gsettings", "set", schema, key, value])


def read_entries():
    raw = gs_get(BASE, "custom-list")
    if raw in ("@as []", "[]"):
        return []
    try:
        return list(ast.literal_eval(raw))
    except Exception:
        return []


def write_entries(entries):
    value = "[" + ", ".join("'" + e + "'" for e in entries) + "]"
    gs_set(BASE, "custom-list", value)


def command_of(entry):
    try:
        out = gs_get(RELO.format(entry), "command")
    except subprocess.CalledProcessError:
        return ""
    return out.strip().strip("'")


def add(command):
    entries = read_entries()
    target = next((e for e in entries if "cliprust" in command_of(e)), None)
    if target is None:
        used = set(entries)
        i = 0
        while "custom{}".format(i) in used:
            i += 1
        target = "custom{}".format(i)
        entries.append(target)
    write_entries(entries)
    gs_set(RELO.format(target), "name", "'" + NAME + "'")
    gs_set(RELO.format(target), "command", "'" + command + "'")
    gs_set(RELO.format(target), "binding", BINDING)
    print("bound {} -> {}".format(BINDING, command))


def remove():
    entries = read_entries()
    keep = [e for e in entries if "cliprust" not in command_of(e)]
    if keep != entries:
        write_entries(keep)
        print("removed cliprust shortcut")
    else:
        print("no cliprust shortcut found")


if len(sys.argv) < 2:
    sys.exit("usage: helper add|remove [command]")
action = sys.argv[1]
if action == "add":
    if len(sys.argv) < 3:
        sys.exit("helper add needs the command path")
    add(sys.argv[2])
elif action == "remove":
    remove()
else:
    sys.exit("unknown action: " + action)
PYEOF
}

install_autostart() {
    mkdir -p "${AUTOSTART_DIR}"
    cat > "${AUTOSTART_FILE}" <<EOF
[Desktop Entry]
Type=Application
Name=ClipRust Clipboard History
Comment=Win+V style clipboard history for Cinnamon
Exec=${BIN_PATH}
Icon=edit-paste
Terminal=false
StartupNotify=false
Categories=Utility;
X-GNOME-Autostart-enabled=true
EOF
    info "autostart entry written to ${AUTOSTART_FILE}"
}

# ---------------------------------------------------------------------------
# uninstall
# ---------------------------------------------------------------------------
if [ "$MODE" = "uninstall" ]; then
    info "stopping daemon (if running)"
    "${BIN_PATH}" --quit >/dev/null 2>&1 || true

    if [ -f "${AUTOSTART_FILE}" ]; then
        rm -f "${AUTOSTART_FILE}"
        info "removed ${AUTOSTART_FILE}"
    fi

    run_hotkey_helper remove || warn "could not update Cinnamon shortcuts"

    if [ -f "${BIN_PATH}" ]; then
        rm -f "${BIN_PATH}"
        info "removed ${BIN_PATH}"
    fi

    # History/config are intentionally kept. Tell the user how to remove them.
    info "history and config were left in place. To delete them too:"
    printf '    rm -rf "%s/.local/share/cliprust" "%s/.config/cliprust"\n' "${HOME}" "${HOME}"
    exit 0
fi

# ---------------------------------------------------------------------------
# install
# ---------------------------------------------------------------------------
info "installing ${BIN_NAME} from ${SCRIPT_DIR}"

# 1. build dependencies ------------------------------------------------------
if [ "$SKIP_DEPS" -eq 0 ]; then
    if pkg-config --exists gtk+-3.0; then
        info "GTK3 development headers already present"
    else
        info "installing build dependencies (sudo required)"
        sudo apt-get update
        sudo apt-get install -y libgtk-3-dev build-essential pkg-config python3
    fi
else
    info "--no-deps: skipping the apt step"
fi

pkg-config --exists gtk+-3.0 || die "gtk+-3.0 dev package is missing; rerun without --no-deps"

# 2. Rust toolchain ----------------------------------------------------------
if command -v cargo >/dev/null 2>&1; then
    info "using $(cargo --version)"
elif [ -x "${HOME}/.cargo/bin/cargo" ]; then
    info "using existing rustup toolchain"
    export PATH="${HOME}/.cargo/bin:${PATH}"
else
    info "installing the Rust toolchain via rustup"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --no-modify-path
    export PATH="${HOME}/.cargo/bin:${PATH}"
fi
command -v cargo >/dev/null 2>&1 || die "cargo still not found after rustup"

# 3. build -------------------------------------------------------------------
info "building release binary (this can take a few minutes the first time)"
( cd "${SCRIPT_DIR}" && cargo build --release )

# 4. install binary ----------------------------------------------------------
mkdir -p "${INSTALL_DIR}"
install -m 755 "${SCRIPT_DIR}/target/release/${BIN_NAME}" "${BIN_PATH}"
info "installed binary to ${BIN_PATH}"

case ":${PATH}:" in
    *":${INSTALL_DIR}:"*) ;;
    *) warn "${INSTALL_DIR} is not on your PATH; add it to use the \`${BIN_NAME}\` command directly" ;;
esac

# 5. autostart ---------------------------------------------------------------
install_autostart

# 6. hotkey ------------------------------------------------------------------
info "registering ${HOTKEY} as a Cinnamon custom shortcut"
run_hotkey_helper add "${BIN_PATH} --toggle" || warn "could not register the shortcut"

# 7. start now ---------------------------------------------------------------
if "${BIN_PATH}" --ping >/dev/null 2>&1; then
    info "daemon already running; restarting it to pick up the new binary"
    "${BIN_PATH}" --quit >/dev/null 2>&1 || true
    sleep 1
fi
info "starting daemon"
setsid "${BIN_PATH}" >/dev/null 2>&1 &
sleep 1
if "${BIN_PATH}" --ping >/dev/null 2>&1; then
    info "daemon is up (${BIN_NAME} --ping -> PONG)"
else
    warn "daemon did not answer --ping; run \`${BIN_NAME}\` in a terminal to see errors"
fi

printf '\n'
info "done. Press Super+V to open the clipboard history."
info "The daemon also starts automatically on your next login."
info "Logs: run \`CLIPRUST_LOG=debug ${BIN_NAME}\` in a terminal."
