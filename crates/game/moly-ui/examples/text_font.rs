//! Measures caller-supplied display text with the actual translation font path.
//! No game data or font is included; run with an optional font file argument.
use bevy::math::Vec2;
use moly_assets::ui_layout::UiComponent;
use moly_ui::ui_layout::tmp_layout::{layout, translated_inputs, TextRules};
use serde_json::json;
use sha2::{Digest, Sha256};

fn main() -> Result<(), String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut added = None;
    if let Some(path) = args.first() {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let source = swash::FontRef::from_index(&moly_core::ui_font::UI_FONT, 0)
            .ok_or("release font unavailable")?;
        let extra = swash::FontRef::from_index(&bytes, 0).ok_or("caller font invalid")?;
        added = (0x4e00..=0x9fff).filter_map(char::from_u32).find(|ch| {
            source.charmap().map(*ch as u32) == 0 && extra.charmap().map(*ch as u32) != 0
        });
        let digest: String = Sha256::digest(&bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect();
        moly_core::ui_font::install_text_font(bytes, &digest)?;
    }
    let text = args
        .get(1)
        .cloned()
        .unwrap_or_else(|| format!("<b>Literal</b> 字形{}", added.unwrap_or('测')));
    let component: UiComponent = serde_json::from_value(json!({
        "pathId": 1, "class": "TextMeshProUGUI", "enabled": true,
        "fields": {
            "m_isRightToLeft": false, "m_fontStyle": 0, "m_fontWeight": 400,
            "m_HorizontalAlignment": 1, "m_VerticalAlignment": 256,
            "m_overflowMode": 0, "m_margin": [0,0,0,0], "m_fontColor": [1,1,1,1],
            "m_fontSize": 32, "m_fontSizeBase": 32, "m_fontSizeMin": 16, "m_fontSizeMax": 32,
            "m_enableAutoSizing": false, "m_enableWordWrapping": true,
            "m_enableKerning": true, "m_isRichText": true, "m_parseCtrlCharacters": true,
            "m_characterSpacing": 0, "m_wordSpacing": 0, "m_lineSpacing": 0,
            "m_paragraphSpacing": 0, "m_lineSpacingMax": 0, "m_charWidthMaxAdj": 0
        },
        "font": {"sourceFace": {"pointSize": 32, "scale": 1, "lineHeight": 40,
            "ascentLine": 30, "descentLine": -8}}, "sprite": null, "texture": null
    }))
    .map_err(|e| e.to_string())?;
    let rules = TextRules {
        leading: Default::default(),
        following: Default::default(),
        modern_hangul: true,
        fonts: None,
    };
    let (component, rules) = translated_inputs(&component, &rules);
    let measured = layout(
        &text,
        &component,
        Vec2::new(640., 200.),
        Vec2::ZERO,
        &rules,
        None,
        false,
    )?;
    let visible: String = measured.glyphs.iter().map(|glyph| glyph.ch).collect();
    if text.contains("<b>") && !visible.contains("<b>") {
        return Err("literal translation markup was interpreted".into());
    }
    if let Some(ch) = added {
        if !visible.contains(ch) && args.get(1).is_none() {
            return Err("fallback glyph was replaced".into());
        }
    }
    let mut context = swash::scale::ScaleContext::new();
    for glyph in &measured.glyphs {
        let drawn = moly_core::ui_font::with_text_font(glyph.ch, |font| {
            let gid = font.charmap().map(glyph.ch as u32);
            let mut scaler = context.builder(font).size(32.).build();
            swash::scale::Render::new(&[swash::scale::Source::Outline])
                .render(&mut scaler, gid)
                .is_some_and(|image| !image.data.is_empty())
        })
        .unwrap_or(false);
        if !drawn {
            return Err(format!("no raster for measured U+{:04X}", glyph.ch as u32));
        }
    }
    println!(
        "{} visible glyphs measured and rasterized; fallback={}",
        measured.glyphs.len(),
        added.map_or_else(|| "none".into(), |ch| format!("U+{:04X}", ch as u32))
    );
    Ok(())
}
