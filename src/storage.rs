//! Clipboard history persistence.
//!
//! Stage 1 stores text/URL entries in a single JSON file. The on-disk shape is
//! a small header (schema version + id counter) plus a newest-first list of
//! items:
//!
//!     { "version": 1, "next_id": 12, "items": [ ... ] }
//!
//! Image entries (stage 2) live as PNG files next to the JSON; the item keeps
//! the file name and a thumbnail name, and removing an item also removes its
//! files. Writes are atomic (temp file + rename) so a crash mid-save cannot
//! leave a truncated history behind.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

use crate::config::Config;
use crate::log;
use crate::paths;

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Eq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Text,
    Url,
    Image,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Item {
    /// Stable identifier; used by the UI to map a row back to an entry.
    pub id: u64,
    pub kind: Kind,
    /// Text content for `Text`/`Url`. Empty for images.
    pub text: String,
    /// File name (inside the images dir) for `Image` entries.
    #[serde(default)]
    pub file: Option<String>,
    /// Thumbnail file name for `Image` entries.
    #[serde(default)]
    pub thumb: Option<String>,
    /// Pinned entries survive both eviction and "clear all".
    #[serde(default)]
    pub pinned: bool,
    /// Unix seconds when the entry was last seen (promoted on duplicate).
    pub created: u64,
    /// Content hash for `Image` entries; used to promote a re-copied image
    /// instead of storing a duplicate.
    #[serde(default)]
    pub hash: Option<u64>,
    /// Pixel dimensions for `Image` entries (shown in the row preview).
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
}

impl Item {
    /// Playable text for the clipboard, or `None` for images (handled
    /// separately once image support lands).
    pub fn clipboard_text(&self) -> Option<&str> {
        match self.kind {
            Kind::Text | Kind::Url => Some(&self.text),
            Kind::Image => None,
        }
    }

    /// One-line preview collapsed to a single line and truncated to `chars`.
    pub fn preview(&self, chars: usize) -> String {
        match self.kind {
            Kind::Image => match (self.width, self.height) {
                (Some(w), Some(h)) => format!("Image {}×{}", w, h),
                _ => "(image)".to_string(),
            },
            _ => collapse_and_truncate(&self.text, chars),
        }
    }
}

/// Collapse all whitespace to single spaces and truncate on a char boundary,
/// appending an ellipsis when content was dropped. Multi-line clipboard text is
/// extremely common, so this is what keeps every row a single tidy line.
pub fn collapse_and_truncate(text: &str, chars: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.chars().count() <= chars {
        return collapsed;
    }
    let mut out: String = collapsed.chars().take(chars).collect();
    out.push('…');
    out
}

#[derive(Serialize, Deserialize)]
struct File {
    version: u32,
    next_id: u64,
    items: Vec<Item>,
}

pub struct Store {
    path: PathBuf,
    pub items: Vec<Item>,
    next_id: u64,
}

impl Store {
    /// Load history from disk, or start empty. Corrupt files are moved aside
    /// rather than silently discarded, so the user can inspect them.
    pub fn load() -> Store {
        let path = paths::history_file();
        match std::fs::read_to_string(&path) {
            Ok(raw) => match serde_json::from_str::<File>(&raw) {
                Ok(f) => {
                    log::info(&format!("loaded {} history items", f.items.len()));
                    Store {
                        path,
                        items: f.items,
                        next_id: f.next_id.max(1),
                    }
                }
                Err(e) => {
                    let bak = path.with_extension("json.corrupt");
                    log::warn(&format!(
                        "history file unreadable ({}); moving to {}",
                        e,
                        bak.display()
                    ));
                    let _ = std::fs::rename(&path, &bak);
                    Store {
                        path,
                        items: Vec::new(),
                        next_id: 1,
                    }
                }
            },
            Err(_) => Store {
                path,
                items: Vec::new(),
                next_id: 1,
            },
        }
    }

    fn next_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Insert a text/URL entry at the top. If an identical entry already
    /// exists it is *promoted* (moved to the top and re-timestamped) instead of
    /// duplicated — the Win+V behaviour. Pinned state is preserved across the
    /// promotion. Returns true when the history actually changed.
    pub fn push_text(&mut self, kind: Kind, text: String, cfg: &Config) -> bool {
        if text.is_empty() {
            return false;
        }
        if let Some(pos) = self
            .items
            .iter()
            .position(|it| it.kind == kind && it.text == text)
        {
            let mut existing = self.items.remove(pos);
            existing.created = now();
            // Already at the top? Nothing changed.
            if pos == 0 {
                self.items.insert(0, existing);
                return false;
            }
            self.items.insert(0, existing);
        } else {
            let id = self.next_id();
            self.items.insert(
                0,
                Item {
                    id,
                    kind,
                    text,
                    file: None,
                    thumb: None,
                    pinned: false,
                    created: now(),
                    hash: None,
                    width: None,
                    height: None,
                },
            );
        }
        self.enforce_limit(cfg);
        true
    }

    /// Insert an image entry at the top.
    ///
    /// `png` is the encoded full-size sidecar and `thumb` the optional
    /// thumbnail; both are written into the images dir under names derived from
    /// the allocated id. A re-copy of an identical image (same `hash`) is
    /// *promoted* rather than duplicated, matching the text behaviour. Returns
    /// true when the history actually changed.
    pub fn push_image(
        &mut self,
        png: &[u8],
        thumb: Option<&[u8]>,
        hash: u64,
        width: u32,
        height: u32,
        cfg: &Config,
    ) -> Result<bool, String> {
        if let Some(pos) = self
            .items
            .iter()
            .position(|it| it.kind == Kind::Image && it.hash == Some(hash))
        {
            let mut existing = self.items.remove(pos);
            existing.created = now();
            // Already at the top? Nothing changed.
            if pos == 0 {
                self.items.insert(0, existing);
                return Ok(false);
            }
            self.items.insert(0, existing);
            return Ok(true);
        }

        let id = self.next_id();
        let dir = paths::image_dir();
        paths::ensure_dir(&dir)?;
        let file = format!("{}.png", id);
        let thumb_name = thumb.map(|_| format!("{}.thumb.png", id));

        std::fs::write(dir.join(&file), png)
            .map_err(|e| format!("{}: {}", dir.join(&file).display(), e))?;
        if let (Some(name), Some(bytes)) = (&thumb_name, thumb) {
            // A missing thumbnail is recoverable (the row falls back to a
            // generic icon), so keep the entry rather than failing the capture.
            if let Err(e) = std::fs::write(dir.join(name), bytes) {
                log::warn(&format!("could not write thumbnail {}: {}", name, e));
            }
        }

        self.items.insert(
            0,
            Item {
                id,
                kind: Kind::Image,
                text: String::new(),
                file: Some(file),
                thumb: thumb_name,
                pinned: false,
                created: now(),
                hash: Some(hash),
                width: Some(width),
                height: Some(height),
            },
        );
        self.enforce_limit(cfg);
        Ok(true)
    }

    /// Drop the oldest *unpinned* entries beyond `cfg.max_items`. Image files
    /// belonging to evicted entries are removed from disk.
    pub fn enforce_limit(&mut self, cfg: &Config) {
        // Walk newest-first, keep the first `max_items` unpinned survivors.
        let mut kept = 0usize;
        let mut remove_idx: Vec<usize> = Vec::new();
        for (i, it) in self.items.iter().enumerate() {
            if it.pinned {
                continue;
            }
            if kept < cfg.max_items {
                kept += 1;
            } else {
                remove_idx.push(i);
            }
        }
        for i in remove_idx.into_iter().rev() {
            let removed = self.items.remove(i);
            Self::remove_image_files(&removed);
        }
    }

    pub fn toggle_pin(&mut self, id: u64) -> bool {
        if let Some(it) = self.items.iter_mut().find(|it| it.id == id) {
            it.pinned = !it.pinned;
            return true;
        }
        false
    }

    pub fn remove(&mut self, id: u64) -> bool {
        if let Some(pos) = self.items.iter().position(|it| it.id == id) {
            let removed = self.items.remove(pos);
            Self::remove_image_files(&removed);
            return true;
        }
        false
    }

    /// Remove everything except pinned entries.
    pub fn clear_unpinned(&mut self) {
        let mut kept: Vec<Item> = Vec::with_capacity(self.items.len());
        for it in self.items.drain(..) {
            if it.pinned {
                kept.push(it);
            } else {
                Self::remove_image_files(&it);
            }
        }
        self.items = kept;
    }

    /// Best-effort deletion of an image entry's files. No-op for text entries.
    /// An associated function rather than a method: it needs nothing from
    /// `self`, and taking `&self` would conflict with the `&mut self` borrows in
    /// `enforce_limit`/`clear_unpinned` while they iterate over `self.items`.
    fn remove_image_files(item: &Item) {
        for name in [&item.file, &item.thumb].into_iter().flatten() {
            let p = paths::image_dir().join(name);
            if let Err(e) = std::fs::remove_file(&p) {
                if e.kind() != std::io::ErrorKind::NotFound {
                    log::warn(&format!("could not remove {}: {}", p.display(), e));
                }
            }
        }
    }

    /// Atomic save: write to a sibling temp file then rename over the target.
    pub fn save(&self) -> Result<(), String> {
        paths::ensure_dir(&paths::data_dir())?;
        let file = File {
            version: 1,
            next_id: self.next_id,
            items: self.items.clone(),
        };
        let json = serde_json::to_string(&file).map_err(|e| e.to_string())?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {}", tmp.display(), e))?;
        std::fs::rename(&tmp, &self.path)
            .map_err(|e| format!("{}: {}", self.path.display(), e))
    }
}

fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
