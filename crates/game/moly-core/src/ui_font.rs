//! The UI font: the subset of the open font, read by every text path.
//!
//! Native builds embed it. Browser builds receive the same bytes with their
//! release: the page passes them to `install_ui_font`, which checks them
//! against the digest below, and a browser start without them is refused.

use std::ops::Deref;

/// SHA-256 of `assets/font/ResourceHanRoundedSC-Medium.subset.ttf`.
pub const UI_FONT_SHA256: &str = "c1dd2f1714e57799b3c535bb747c17a3370cd0b9615a76b3841f7d4aba8b7aa0";
/// Length in bytes of the same file.
pub const UI_FONT_BYTES: usize = 9_449_520;

/// The refusal of a browser start whose UI font was not installed.
pub const UI_FONT_MISSING: &str =
    "UI font not installed: pass the release's UI font to install_ui_font before starting";

/// The font bytes, read through `Deref` (`&UI_FONT` is a `&[u8]`).
pub struct UiFont;
pub static UI_FONT: UiFont = UiFont;

impl Deref for UiFont {
    type Target = [u8];
    fn deref(&self) -> &[u8] {
        bytes()
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn bytes() -> &'static [u8] {
    include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf")
}

#[cfg(target_arch = "wasm32")]
static INSTALLED: std::sync::OnceLock<Box<[u8]>> = std::sync::OnceLock::new();

#[cfg(target_arch = "wasm32")]
fn bytes() -> &'static [u8] {
    // Every browser start calls `require` first, so this is reached only by
    // code that runs without a start.
    INSTALLED
        .get()
        .map(Box::as_ref)
        .unwrap_or_else(|| panic!("{UI_FONT_MISSING}"))
}

/// The UI font as one font asset, shared by every bevy_ui text.
#[derive(bevy::prelude::Resource, Clone)]
pub struct UiFontAsset(pub bevy::prelude::Handle<bevy::prelude::Font>);

/// PreStartup: adds the UI font to the font assets, once.
pub fn add_asset(
    mut commands: bevy::prelude::Commands,
    mut fonts: bevy::prelude::ResMut<bevy::prelude::Assets<bevy::prelude::Font>>,
) {
    let font = bevy::prelude::Font::from_bytes(UI_FONT.to_vec());
    commands.insert_resource(UiFontAsset(fonts.add(font)));
}

/// Checks font bytes against the font this build was made with.
pub fn verify(bytes: &[u8]) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    if bytes.len() != UI_FONT_BYTES {
        return Err(format!(
            "UI font refused: {} bytes, this engine needs {UI_FONT_BYTES}",
            bytes.len()
        ));
    }
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != UI_FONT_SHA256 {
        return Err(format!(
            "UI font refused: SHA-256 {digest}, this engine needs {UI_FONT_SHA256}"
        ));
    }
    Ok(())
}

/// Installs the release's UI font. Bytes that differ from the font this
/// engine was built with are refused, including on a repeated install.
#[cfg(target_arch = "wasm32")]
pub fn install(bytes: Vec<u8>) -> Result<(), String> {
    verify(&bytes)?;
    let _ = INSTALLED.set(bytes.into_boxed_slice());
    Ok(())
}

/// Refuses a browser start whose UI font is not installed. Native builds
/// embed the font and always pass.
pub fn require() -> Result<(), String> {
    #[cfg(target_arch = "wasm32")]
    if INSTALLED.get().is_none() {
        return Err(UI_FONT_MISSING.to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_font_matches_the_published_digest() {
        assert_eq!(verify(&UI_FONT), Ok(()));
    }

    #[test]
    fn changed_font_bytes_are_refused() {
        let mut changed = UI_FONT.to_vec();
        changed[UI_FONT_BYTES / 2] ^= 1;
        assert!(verify(&changed).unwrap_err().contains("SHA-256"));
        assert!(verify(&changed[1..]).unwrap_err().contains("bytes"));
    }

    #[test]
    fn shared_font_has_latin_japanese_and_chinese_glyphs() {
        let font = swash::FontRef::from_index(&UI_FONT, 0).expect("valid shared UI font");
        for character in ['A', '0', 'あ', 'が', '家', '界', '设', '置'] {
            assert_ne!(
                font.charmap().map(character as u32),
                0,
                "missing {character}"
            );
        }
    }
}

/// Fonts selected by the current text session, shared by rasterization and
/// measurement. Readers retain Arc snapshots; replacing the selected set does
/// not invalidate an in-flight reader or the texture handles of an older atlas.
static TEXT_FONTS: std::sync::OnceLock<std::sync::Mutex<Vec<std::sync::Arc<[u8]>>>> =
    std::sync::OnceLock::new();
static TEXT_FONT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static TEXT_FONT_UPDATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
const MAX_TEXT_FONTS: usize = 8;
const MAX_TEXT_FONT_BYTES: usize = 32 * 1024 * 1024;

fn verify_text_font(bytes: &[u8], expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};
    if bytes.is_empty() || bytes.len() > MAX_TEXT_FONT_BYTES {
        return Err("translation font exceeds size limit".into());
    }
    let digest: String = Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    if digest != expected || swash::FontRef::from_index(bytes, 0).is_none() {
        return Err("translation font integrity or format mismatch".into());
    }
    Ok(())
}

/// Compatibility entry point for callers that install one font before a text
/// session. Repeated playback uses `with_text_font_session` to replace the set.
pub fn install_text_font(bytes: Vec<u8>, expected: &str) -> Result<(), String> {
    verify_text_font(&bytes, expected)?;
    let _update = TEXT_FONT_UPDATE.lock().map_err(|_| "translation font update unavailable")?;
    let mut fonts = TEXT_FONTS
        .get_or_init(|| std::sync::Mutex::new(Vec::new()))
        .lock()
        .map_err(|_| "translation font state unavailable")?;
    if fonts.iter().any(|font| font.as_ref() == bytes.as_slice()) {
        return Ok(());
    }
    if fonts.len() >= MAX_TEXT_FONTS {
        return Err("translation font resource limit reached".into());
    }
    fonts.push(std::sync::Arc::from(bytes));
    TEXT_FONT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
    Ok(())
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct TextFontReference {
    sha256: String,
    bytes: usize,
}

/// Atomically selects a bounded font set while installing its display session.
/// The browser caller must invoke this synchronously at an idle playback boundary.
/// Bytes contain the referenced fonts concatenated in manifest order. All font
/// hashes and formats are checked before the selected set changes. If text
/// configuration fails, the previous Arc set is restored before this returns.
/// No render tick or async operation may run inside `configure`.
pub fn with_text_font_session<T>(
    bytes: Vec<u8>,
    references: &str,
    configure: impl FnOnce() -> Result<T, String>,
) -> Result<T, String> {
    if references.len() > 4096 || bytes.len() > MAX_TEXT_FONTS * MAX_TEXT_FONT_BYTES {
        return Err("translation font manifest exceeds size limit".into());
    }
    let references: Vec<TextFontReference> = serde_json::from_str(references)
        .map_err(|_| "invalid translation font manifest")?;
    if references.len() > MAX_TEXT_FONTS {
        return Err("translation font resource limit reached".into());
    }
    let _update = TEXT_FONT_UPDATE.lock().map_err(|_| "translation font update unavailable")?;
    let installed = text_fonts();
    let mut selected = Vec::with_capacity(references.len());
    let mut offset = 0usize;
    let mut identities = std::collections::HashSet::new();
    for reference in references {
        let end = offset.checked_add(reference.bytes)
            .filter(|end| *end <= bytes.len())
            .ok_or("translation font byte boundary mismatch")?;
        let font = &bytes[offset..end];
        verify_text_font(font, &reference.sha256)?;
        if !identities.insert(reference.sha256) {
            return Err("duplicate translation font identity".into());
        }
        selected.push(installed.iter().find(|existing| existing.as_ref() == font)
            .cloned().unwrap_or_else(|| std::sync::Arc::from(font)));
        offset = end;
    }
    if offset != bytes.len() {
        return Err("unreferenced translation font bytes".into());
    }
    drop(bytes);
    let changed = installed.len() != selected.len()
        || installed.iter().zip(&selected).any(|(old, new)| !std::sync::Arc::ptr_eq(old, new));
    if !changed {
        return configure();
    }
    let replace = |fonts| -> Result<(), String> {
        *TEXT_FONTS.get_or_init(|| std::sync::Mutex::new(Vec::new()))
            .lock().map_err(|_| "translation font state unavailable")? = fonts;
        TEXT_FONT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
        Ok(())
    };
    replace(selected)?;
    let result = configure();
    if result.is_err() {
        replace(installed)?;
    }
    result
}

pub fn text_fonts() -> Vec<std::sync::Arc<[u8]>> {
    TEXT_FONTS
        .get()
        .and_then(|fonts| fonts.lock().ok().map(|fonts| fonts.clone()))
        .unwrap_or_default()
}

pub fn text_font_revision() -> u64 {
    TEXT_FONT_REVISION.load(std::sync::atomic::Ordering::Acquire)
}

/// Prefer the release font, then the installed publication fonts. The borrow
/// cannot outlive the retained bytes and no raw FontRef is cached globally.
pub fn with_text_font<T>(ch: char, use_font: impl FnOnce(swash::FontRef<'_>) -> T) -> Option<T> {
    let primary = swash::FontRef::from_index(&UI_FONT, 0)?;
    if primary.charmap().map(ch as u32) != 0 {
        return Some(use_font(primary));
    }
    for bytes in text_fonts() {
        let Some(font) = swash::FontRef::from_index(&bytes, 0) else {
            continue;
        };
        if font.charmap().map(ch as u32) != 0 {
            return Some(use_font(font));
        }
    }
    None
}
