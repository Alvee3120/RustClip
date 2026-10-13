//! User configuration, persisted as JSON at `~/.config/cliprust/config.json`.
//!
//! JSON (not TOML) is deliberate: we already depend on serde_json for the
//! history file, so the config costs zero additional crates. On first run the
//! defaults are written out so the file is discoverable and editable.

use serde::{Deserialize, Serialize};

use crate::{log, paths};

/// Image defaults live in consts so the `Default` impl and the serde
/// `default = ...` hooks (used when upgrading an older config file that predates
/// these keys) cannot drift apart.
const DEFAULT_THUMBNAIL_PX: i32 = 40;
const DEFAULT_IMAGE_SIZE_LIMIT_BYTES: u64 = 20 * 1024 * 1024;

fn default_thumbnail_px() -> i32 {
    DEFAULT_THUMBNAIL_PX
}

fn default_image_size_limit_bytes() -> u64 {
    DEFAULT_IMAGE_SIZE_LIMIT_BYTES
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Config {
    /// Maximum number of *unpinned* history entries to retain. Pinned items are
    /// never evicted and are not counted against this limit.
    pub max_items: usize,
    /// When true, selecting an entry also synthesises Ctrl+V into the
    /// previously focused window. Off by default: focus stealing makes this
    /// racy (see README).
    pub auto_paste: bool,
    /// Drop any clipboard payload whose serialised text exceeds this many bytes.
    pub size_limit_bytes: u64,
    /// Honour the `x-kde-passwordManagerHint` clipboard target and skip such
    /// items (KeePassXC and friends set it).
    pub ignore_password_hint: bool,
    /// Preferred width of the popup in pixels.
    pub popup_width: i32,
    /// Maximum height of the popup before it scrolls.
    pub popup_height: i32,
    /// Characters of text shown in a row's preview.
    pub preview_chars: usize,
    /// Milliseconds to wait between hiding the popup and synthesising Ctrl+V,
    /// so the target window has time to regain focus.
    pub paste_delay_ms: u64,
    /// Height in pixels of the thumbnail stored for image entries. Width follows
    /// the aspect ratio (capped so ultra-wide images stay reasonable), and
    /// images smaller than the box are never upscaled.
    #[serde(default = "default_thumbnail_px")]
    pub thumbnail_px: i32,
    /// Skip image clipboard payloads whose encoded PNG exceeds this many bytes.
    /// Kept separate from `size_limit_bytes` because images compress very
    /// differently from text.
    #[serde(default = "default_image_size_limit_bytes")]
    pub image_size_limit_bytes: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            max_items: 20,
            auto_paste: false,
            size_limit_bytes: 5 * 1024 * 1024,
            ignore_password_hint: true,
            popup_width: 520,
            popup_height: 480,
            preview_chars: 80,
            paste_delay_ms: 120,
            thumbnail_px: DEFAULT_THUMBNAIL_PX,
            image_size_limit_bytes: DEFAULT_IMAGE_SIZE_LIMIT_BYTES,
        }
    }
}

impl Config {
    /// Load the config, writing defaults to disk if the file is absent. A
    /// malformed config is reported but does not abort the daemon: we fall back
    /// to defaults so the user is never locked out by a typo.
    pub fn load() -> Config {
        let path = paths::config_file();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<Config>(&raw) {
                Ok(cfg) => {
                    log::debug(&format!("loaded config from {}", path.display()));
                    cfg
                }
                Err(e) => {
                    log::warn(&format!(
                        "config {} is invalid ({}); using defaults",
                        path.display(),
                        e
                    ));
                    Config::default()
                }
            },
            Err(_) => {
                let cfg = Config::default();
                if let Err(e) = cfg.save() {
                    log::warn(&format!("could not write default config: {}", e));
                }
                cfg
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        paths::ensure_dir(&paths::config_dir())?;
        let path = paths::config_file();
        let pretty = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        std::fs::write(&path, pretty).map_err(|e| format!("{}: {}", path.display(), e))
    }
}
