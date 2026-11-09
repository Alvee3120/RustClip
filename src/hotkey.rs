//! Cinnamon global-shortcut registration.
//!
//! Cinnamon keeps global shortcuts in GSettings rather than exposing a
//! per-application hotkey API, so the shortcut lives in
//! `org.cinnamon.desktop.keybindings`. The custom shortcut list is an array of
//! names (`custom0`, `custom1`, ...) that each point at a relocatable schema
//! instance holding a name, a command and a binding.
//!
//! A package hook runs as root and cannot write the logged-in user's dconf, so
//! the daemon — which runs as that user — ensures the Super+V binding exists
//! when it starts.

use std::process::Command;

use crate::log;

const BASE_SCHEMA: &str = "org.cinnamon.desktop.keybindings";
const ENTRY_SCHEMA: &str = "org.cinnamon.desktop.keybindings.custom-keybinding";
const ENTRY_PATH_PREFIX: &str = "/org/cinnamon/desktop/keybindings/custom-keybindings/";
const SHORTCUT_NAME: &str = "ClipRust Clipboard History";
const DEFAULT_BINDING: &str = "<Super>v";
/// Substring that marks a command as ours, so re-runs update one entry instead
/// of adding a duplicate.
const MARKER: &str = "cliprust";

/// Called by the daemon at startup. Non-fatal: on a non-Cinnamon desktop, or
/// when GSettings is unavailable, we log and carry on without a global hotkey.
pub fn ensure_registered() {
    if !std::env::var("XDG_CURRENT_DESKTOP")
        .map(|v| v.to_ascii_lowercase().contains("cinnamon"))
        .unwrap_or(false)
    {
        log::debug("not a Cinnamon session; skipping Super+V registration");
        return;
    }
    match register() {
        Ok(msg) => log::info(&format!("hotkey: {}", msg)),
        Err(e) => log::warn(&format!("could not register Super+V: {}", e)),
    }
}

/// Add a `cliprust --toggle` shortcut bound to Super+V, or refresh the command
/// of an existing one. A binding the user changed is left alone.
pub fn register() -> Result<String, String> {
    let entries = parse_strv(&gsettings_get(BASE_SCHEMA, "custom-list")?);
    let exe = std::env::current_exe().map_err(|e| format!("cannot determine own path: {}", e))?;
    let command = format!("{} --toggle", shell_quote(&exe.to_string_lossy()));

    if let Some(entry) = entries.iter().find(|e| command_of(e).contains(MARKER)) {
        if command_of(entry).trim_matches('\'') != command {
            gsettings_set(&entry_schema(entry), "command", &quote(&command))?;
            return Ok(format!("updated {} -> {}", entry, command));
        }
        return Ok(format!("already registered as {}", entry));
    }

    let entry = next_entry_name(&entries);
    let mut updated = entries;
    updated.push(entry.clone());

    gsettings_set(BASE_SCHEMA, "custom-list", &format_strv(&updated))?;
    let schema = entry_schema(&entry);
    gsettings_set(&schema, "name", &quote(SHORTCUT_NAME))?;
    gsettings_set(&schema, "command", &quote(&command))?;
    gsettings_set(
        &schema,
        "binding",
        &format_strv(&[DEFAULT_BINDING.to_string()]),
    )?;
    Ok(format!("bound {} -> {}", DEFAULT_BINDING, command))
}

/// Drop every shortcut whose command is ours.
pub fn unregister() -> Result<String, String> {
    unregister_matching(MARKER)
}

/// Drop shortcuts whose command contains `needle`. The package's prerm passes
/// `/usr/bin/cliprust`, so removing the `.deb` does not delete a shortcut that
/// an `install.sh` setup points at `~/.local/bin`.
pub fn unregister_matching(needle: &str) -> Result<String, String> {
    let entries = parse_strv(&gsettings_get(BASE_SCHEMA, "custom-list")?);
    let keep: Vec<String> = entries
        .iter()
        .filter(|e| !command_of(e).contains(needle))
        .cloned()
        .collect();
    if keep.len() == entries.len() {
        return Ok("no matching shortcut found".to_string());
    }
    gsettings_set(BASE_SCHEMA, "custom-list", &format_strv(&keep))?;
    Ok("removed cliprust shortcut".to_string())
}

fn entry_schema(entry: &str) -> String {
    format!("{}:{}{}/", ENTRY_SCHEMA, ENTRY_PATH_PREFIX, entry)
}

/// Raw (still GVariant-quoted) command of an entry. Missing or unreadable
/// entries yield an empty string so a malformed list cannot abort the walk.
fn command_of(entry: &str) -> String {
    gsettings_get(&entry_schema(entry), "command").unwrap_or_default()
}

fn gsettings_get(schema: &str, key: &str) -> Result<String, String> {
    let out = Command::new("gsettings")
        .args(["get", schema, key])
        .output()
        .map_err(|e| format!("cannot run gsettings: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "gsettings get {} {} failed: {}",
            schema,
            key,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn gsettings_set(schema: &str, key: &str, value: &str) -> Result<(), String> {
    let out = Command::new("gsettings")
        .args(["set", schema, key, value])
        .output()
        .map_err(|e| format!("cannot run gsettings: {}", e))?;
    if !out.status.success() {
        return Err(format!(
            "gsettings set {} {} failed: {}",
            schema,
            key,
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(())
}

/// Parse the subset of GVariant we read back: a list of single-quoted strings,
/// e.g. `['custom0', 'custom2']` or the empty forms `[]` / `@as []`.
fn parse_strv(raw: &str) -> Vec<String> {
    let mut items = Vec::new();
    let mut current = String::new();
    let mut in_string = false;
    let mut escaped = false;
    for ch in raw.chars() {
        if !in_string {
            if ch == '\'' {
                in_string = true;
            }
            continue;
        }
        if escaped {
            current.push(ch);
            escaped = false;
        } else {
            match ch {
                '\\' => escaped = true,
                '\'' => {
                    items.push(std::mem::take(&mut current));
                    in_string = false;
                }
                _ => current.push(ch),
            }
        }
    }
    items
}

fn format_strv(items: &[String]) -> String {
    if items.is_empty() {
        return "@as []".to_string();
    }
    let quoted: Vec<String> = items.iter().map(|item| quote(item)).collect();
    format!("[{}]", quoted.join(", "))
}

/// Quote a string as a GVariant string value.
fn quote(value: &str) -> String {
    format!("'{}'", value.replace('\\', "\\\\").replace('\'', "\\'"))
}

/// Quote a path for the command string Cinnamon will shell-parse. Only paths
/// that would otherwise split into several words need it.
fn shell_quote(path: &str) -> String {
    let special = "'\"\\$`*?[]{}()!&;<>|";
    if path
        .chars()
        .any(|c| c.is_whitespace() || special.contains(c))
    {
        format!("'{}'", path.replace('\'', "'\\''"))
    } else {
        path.to_string()
    }
}

fn next_entry_name(entries: &[String]) -> String {
    let mut i = 0;
    loop {
        let name = format!("custom{}", i);
        if !entries.iter().any(|e| e == &name) {
            return name;
        }
        i += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_empty_lists() {
        assert!(parse_strv("@as []").is_empty());
        assert!(parse_strv("[]").is_empty());
    }

    #[test]
    fn parses_multiple_entries() {
        assert_eq!(
            parse_strv("['custom0', 'custom2']"),
            vec!["custom0".to_string(), "custom2".to_string()]
        );
    }

    #[test]
    fn formats_and_parses_roundtrip() {
        let items = vec!["custom0".to_string(), "custom1".to_string()];
        assert_eq!(format_strv(&items), "['custom0', 'custom1']");
        assert_eq!(parse_strv(&format_strv(&items)), items);
        assert_eq!(format_strv(&[]), "@as []");
    }

    #[test]
    fn quoting_escapes_variant_strings() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "'it\\'s'");
    }

    #[test]
    fn picks_first_free_entry_name() {
        let used = vec!["custom0".to_string(), "custom1".to_string()];
        assert_eq!(next_entry_name(&used), "custom2");
        assert_eq!(next_entry_name(&[]), "custom0");
    }

    #[test]
    fn paths_with_spaces_are_shell_quoted() {
        assert_eq!(shell_quote("/usr/bin/cliprust"), "/usr/bin/cliprust");
        assert_eq!(
            shell_quote("/opt/my apps/cliprust"),
            "'/opt/my apps/cliprust'"
        );
    }
}
