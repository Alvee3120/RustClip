//! Image encoding helpers for image clipboard entries.
//!
//! GdkPixbuf (already pulled in by GTK) does all the work: it encodes the
//! captured pixbuf to PNG for the on-disk sidecar and produces a small
//! thumbnail PNG for the popup. No extra image crate is needed.
//!
//! Memory: callers own the source pixbuf and drop it when the capture is done.
//! We only ever hold one full-size and one thumbnail pixbuf at a time, and the
//! encoded bytes are handed straight to `Store::push_image` and then dropped —
//! nothing image-sized is retained by the daemon between captures.

use gtk::gdk_pixbuf::{InterpType, Pixbuf};

use crate::log;

/// Encode a pixbuf as PNG bytes.
pub fn encode_png(pixbuf: &Pixbuf) -> Result<Vec<u8>, String> {
    pixbuf
        .save_to_bufferv("png", &[])
        .map_err(|e| format!("png encode failed: {}", e))
}

/// Build a thumbnail that fits inside a `height`-tall by `4*height`-wide box,
/// preserving the aspect ratio and never upscaling. Returns `None` only when the
/// pixbuf is degenerate or the scale fails.
pub fn thumbnail(pixbuf: &Pixbuf, height: i32) -> Option<Pixbuf> {
    let box_h = height.max(1);
    let (width, src_h) = (pixbuf.width(), pixbuf.height());
    if width <= 0 || src_h <= 0 {
        return None;
    }
    let ratio = (box_h as f64 * 4.0 / width as f64)
        .min(box_h as f64 / src_h as f64)
        .min(1.0);
    let tw = ((width as f64 * ratio).round() as i32).max(1);
    let th = ((src_h as f64 * ratio).round() as i32).max(1);
    match pixbuf.scale_simple(tw, th, InterpType::Bilinear) {
        Some(thumb) => Some(thumb),
        None => {
            log::warn("thumbnail scaling failed");
            None
        }
    }
}

/// Cheap FNV-1a hash of the encoded image, used to recognise a re-copy of an
/// image we already hold so the history promotes the existing entry instead of
/// piling up duplicates. Hashing the PNG (rather than the raw pixels) keeps this
/// proportional to the compressed size.
pub fn hash_bytes(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}
