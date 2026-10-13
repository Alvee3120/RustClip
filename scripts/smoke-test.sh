#!/usr/bin/env bash
#
# End-to-end smoke test for cliprust.
#
# Drives the real daemon through the clipboard with `xclip` and asserts on the
# JSON history and the image sidecars. It needs a display but NOT a window
# manager, because it never opens the popup. Everything runs against a private
# temp tree, so your own clipboard, history and config are never touched.
#
# Requirements: xclip, and a display ($DISPLAY) or `xvfb-run` (used
# automatically when there is no display).
#
# Usage: scripts/smoke-test.sh
#
# Exit status: 0 all checks passed, 1 a check failed, 2 a prerequisite is missing.
set -euo pipefail

SELF="$(cd -- "$(dirname -- "$0")" && pwd)/$(basename -- "$0")"

# Re-exec under a virtual display when we do not have one.
if [ "${1:-}" != "--inner" ]; then
    if [ -n "${DISPLAY:-}" ]; then
        exec "$SELF" --inner
    elif command -v xvfb-run >/dev/null 2>&1; then
        exec xvfb-run -a -s "-screen 0 1024x768x24" "$SELF" --inner
    else
        echo "no DISPLAY and xvfb-run is unavailable; cannot run the smoke test" >&2
        exit 2
    fi
fi

ROOT="$(cd -- "$(dirname -- "$SELF")/.." && pwd)"
BIN="${CLIPRUST_BIN:-$ROOT/target/release/cliprust}"

command -v xclip >/dev/null 2>&1 || { echo "xclip is required" >&2; exit 2; }
if [ ! -x "$BIN" ]; then
    echo "building $BIN ..."
    (cd "$ROOT" && cargo build --release)
fi

WORK="$(mktemp -d)"
export XDG_DATA_HOME="$WORK/data"
export XDG_CONFIG_HOME="$WORK/config"
export XDG_RUNTIME_DIR="$WORK/run"
mkdir -p "$XDG_RUNTIME_DIR" "$XDG_CONFIG_HOME/cliprust"
chmod 700 "$XDG_RUNTIME_DIR"
export CLIPRUST_LOG=debug

IMAGES="$XDG_DATA_HOME/cliprust/images"
HIST="$XDG_DATA_HOME/cliprust/history.json"
DAEMON=""
pass=0
fail=0
ok() { printf '  \033[32mPASS\033[0m %s\n' "$1"; pass=$((pass + 1)); }
bad() { printf '  \033[31mFAIL\033[0m %s\n' "$1"; fail=$((fail + 1)); }

cleanup() {
    "$BIN" --quit >/dev/null 2>&1 || true
    [ -n "$DAEMON" ] && kill "$DAEMON" 2>/dev/null || true
    rm -rf "$WORK"
}
trap cleanup EXIT

wait_for() { # <grep pattern> [tries]
    local pattern="$1" tries="${2:-80}" i
    for ((i = 0; i < tries; i++)); do
        grep -q -- "$pattern" "$HIST" 2>/dev/null && return 0
        sleep 0.1
    done
    return 1
}

# Count entries of a given kind. `|| true` swallows grep's "no match" exit so
# `set -o pipefail` does not abort the script when the history is empty.
kind_count() { { grep -o -- "\"kind\":\"$1\"" "$HIST" 2>/dev/null || true; } | wc -l | tr -d ' '; }

echo "cliprust smoke test"
echo "  binary: $BIN"
echo "  state:  $WORK"

"$BIN" >"$WORK/daemon.log" 2>&1 &
DAEMON=$!
up=no
for ((i = 0; i < 80; i++)); do
    "$BIN" --ping >/dev/null 2>&1 && { up=yes; break; }
    sleep 0.1
done
if [ "$up" = yes ]; then
    ok "daemon answers --ping"
else
    bad "daemon did not start"
    cat "$WORK/daemon.log" >&2
    exit 1
fi

# --- text ------------------------------------------------------------------
printf 'smoke test text value' | xclip -selection clipboard
wait_for '"kind":"text"' && ok "text capture" || bad "text not captured"

# --- url -------------------------------------------------------------------
printf 'https://example.com/smoke' | xclip -selection clipboard
wait_for '"kind":"url"' && ok "url capture" || bad "url not captured"

# --- image -----------------------------------------------------------------
base64 -d >"$WORK/px.png" <<'PNG'
iVBORw0KGgoAAAANSUhEUgAAAMgAAABkCAYAAADDhn8LAAAABHNCSVQICAgIfAhkiAAAAR9JREFUeJzt0zERgDAAwMBSQbWGI6xSA72sMPwryJJr3c87gKP5dQD8mUEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGASCQSAYBIJBIBgEgkEgGATCBkbGAwJpxdu4AAAAAElFTkSuQmCC
PNG
xclip -selection clipboard -t image/png -i "$WORK/px.png"
wait_for '"kind":"image"' && ok "image capture" || bad "image not captured"

sidecar="$(sed -n 's/.*"file":"\([^"]*\)".*/\1/p' "$HIST" | head -1)"
thumb="$(sed -n 's/.*"thumb":"\([^"]*\)".*/\1/p' "$HIST" | head -1)"
if [ -n "$sidecar" ] && [ -f "$IMAGES/$sidecar" ]; then
    ok "image sidecar written ($sidecar)"
else
    bad "image sidecar missing"
fi
if [ -n "$thumb" ] && [ -f "$IMAGES/$thumb" ]; then
    ok "thumbnail written ($thumb)"
else
    bad "thumbnail missing"
fi
if [ -f "$IMAGES/$sidecar" ] && [ -f "$IMAGES/$thumb" ]; then
    full_bytes=$(wc -c <"$IMAGES/$sidecar")
    thumb_bytes=$(wc -c <"$IMAGES/$thumb")
    if [ "$thumb_bytes" -lt "$full_bytes" ]; then
        ok "thumbnail smaller than full image ($thumb_bytes < $full_bytes bytes)"
    else
        bad "thumbnail not smaller than full image ($thumb_bytes vs $full_bytes)"
    fi
fi

# --- de-duplication --------------------------------------------------------
xclip -selection clipboard -t image/png -i "$WORK/px.png"
sleep 0.6
if [ "$(kind_count image)" = 1 ]; then
    ok "re-copied image de-duplicated"
else
    bad "image was duplicated ($(kind_count image) entries)"
fi

# --- clear (IPC) -----------------------------------------------------------
"$BIN" --clear >/dev/null
sleep 0.4
remaining="$(kind_count text)$(kind_count url)$(kind_count image)"
if [ "$remaining" = "000" ]; then
    ok "--clear emptied the (unpinned) history"
else
    bad "--clear left entries behind"
fi

# --- shutdown --------------------------------------------------------------
"$BIN" --quit >/dev/null
sleep 0.4
if kill -0 "$DAEMON" 2>/dev/null; then
    bad "daemon still running after --quit"
else
    ok "--quit stops the daemon"
fi

echo
echo "smoke test: $pass passed, $fail failed"
[ "$fail" -eq 0 ]
