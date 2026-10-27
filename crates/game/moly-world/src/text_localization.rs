//! Host-installed display translations. Source scripts remain immutable.
//!
//! Installation is atomic; a talk window clones the current configuration
//! when its owner is admitted. Existing playback keeps its old revision.
use moly_text::{Overlay, SourceCatalog, TextKind, TextSession};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

static CURRENT: OnceLock<Mutex<Option<TextSession>>> = OnceLock::new();
static REVISION: AtomicU64 = AtomicU64::new(0);
static GLYPHS_READY: AtomicU64 = AtomicU64::new(0);
pub use moly_core::ui_font::{install_text_font as install_font, text_fonts as fonts};

fn current() -> &'static Mutex<Option<TextSession>> {
    CURRENT.get_or_init(|| Mutex::new(None))
}

/// The embedding host has already verified publication hashes and source
/// resource identity. This parses the source and both optional overlays
/// before replacing anything, so a rejected update leaves the old one intact.
pub fn configure(
    source: &str,
    official: &str,
    community: &str,
    locale: &str,
) -> Result<(), String> {
    let source = Arc::new(SourceCatalog::parse(source)?);
    let parse = |value: &str| -> Result<Option<Arc<Overlay>>, String> {
        if value.is_empty() {
            Ok(None)
        } else {
            Overlay::parse(value).map(Arc::new).map(Some)
        }
    };
    let session = TextSession::new(source, locale, parse(official)?, parse(community)?)?;
    crate::ui_font::require()?;
    let font = swash::FontRef::from_index(&crate::ui_font::UI_FONT, 0)
        .ok_or("translation font is unavailable")?;
    let extra_bytes = fonts();
    let extra: Vec<_> = extra_bytes
        .iter()
        .filter_map(|bytes| swash::FontRef::from_index(bytes, 0))
        .collect();
    let missing: Vec<String> = session
        .characters()
        .into_iter()
        .filter(|ch| {
            !ch.is_control()
                && !crate::ui_inputs::atlas::tmp_zero_metric(*ch)
                && font.charmap().map(*ch as u32) == 0
                && !extra.iter().any(|font| font.charmap().map(*ch as u32) != 0)
        })
        .map(|ch| format!("U+{:04X}", ch as u32))
        .take(16)
        .collect();
    if !missing.is_empty() {
        return Err(format!(
            "translation needs unavailable glyphs: {}",
            missing.join(" ")
        ));
    }
    *current()
        .lock()
        .map_err(|_| "translation state unavailable")? = Some(session);
    REVISION.fetch_add(1, Ordering::Release);
    Ok(())
}

pub fn clear() -> Result<(), String> {
    *current()
        .lock()
        .map_err(|_| "translation state unavailable")? = None;
    REVISION.fetch_add(1, Ordering::Release);
    Ok(())
}

pub fn snapshot() -> Option<TextSession> {
    current().lock().ok().and_then(|value| value.clone())
}

pub fn characters() -> Vec<char> {
    snapshot().map_or_else(Vec::new, |session| session.characters())
}

pub fn revision() -> u64 {
    REVISION.load(Ordering::Acquire)
}
pub fn mark_glyphs_ready(revision: u64) {
    GLYPHS_READY.store(revision, Ordering::Release);
}
pub fn ready() -> bool {
    GLYPHS_READY.load(Ordering::Acquire) == revision()
}

/// Non-session text (catalogue lines and newly shown tweets) uses the current
/// version. A playback window must use its pinned session instead.
pub fn display(script: &str, step: Option<usize>, kind: TextKind, original: &str) -> String {
    snapshot().map_or_else(
        || original.to_owned(),
        |session| {
            session
                .resolve(script, step, kind, original)
                .text
                .to_owned()
        },
    )
}
