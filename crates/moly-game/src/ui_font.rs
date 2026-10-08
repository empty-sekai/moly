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
pub(crate) struct UiFont;
pub(crate) static UI_FONT: UiFont = UiFont;

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
            assert_ne!(font.charmap().map(character as u32), 0, "missing {character}");
        }
    }
}
