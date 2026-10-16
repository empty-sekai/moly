//! The TMP font assets a region UI root carries, and the character lookup
//! TMP performs over them.
//!
//! Each asset keeps its FaceInfo, its character lookup table (the first
//! entry per code point of the serialized character table, each bound to the
//! first glyph of its index), its glyph pair adjustment records, its
//! population mode and the fallback list the text sees at run time.
//!
//! Fallback lists: the game's font manager clears each base asset's
//! serialized fallback list when it initializes, sets it to the built-in
//! list, and after the start-up download clears it again and sets the
//! on-demand list. The field runs after that download, so a base asset's
//! list is the document's on-demand list; every other asset keeps its
//! serialized list.
//!
//! A static asset also answers the characters TMP synthesizes when it reads
//! the asset (a zero-metric glyph of index 0 for each missing one of 0x03,
//! 0x09, 0x0A, 0x0B, 0x0D, 0x061C, 0x200B, 0x200E, 0x200F, 0x2028, 0x2029,
//! 0x2060). A dynamic asset adds a character it lacks at run time from its
//! source font file, with metrics the engine's font engine computes then;
//! none of that is stored, so such a character resolves to
//! [`Element::Dynamic`] and carries no source advance.

use serde_json::Value;
use std::collections::{HashMap, HashSet};

/// Code points TMP synthesizes into a static asset that lacks them.
const SYNTHESIZED: [u32; 12] = [
    0x03, 0x09, 0x0A, 0x0B, 0x0D, 0x061C, 0x200B, 0x200E, 0x200F, 0x2028, 0x2029, 0x2060,
];
/// TMP's replacement for a character no asset answers when the TMP
/// settings name none (U+25A1 WHITE SQUARE).
const DEFAULT_MISSING_GLYPH: u32 = 9633;
/// `FontFeatureLookupFlags.IgnoreSpacingAdjustments`.
const IGNORE_SPACING_ADJUSTMENTS: i64 = 0x100;

/// An asset's FaceInfo fields the layout reads.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Face {
    pub(super) point_size: f32,
    pub(super) scale: f32,
    pub(super) line_height: f32,
    pub(super) ascent_line: f32,
    pub(super) descent_line: f32,
    pub(super) baseline: f32,
}

/// A glyph's horizontal advance and scale (asset units).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Glyph {
    pub(super) index: u32,
    pub(super) horizontal_advance: f32,
    pub(super) scale: f32,
}

#[derive(Debug, Clone, Copy)]
struct Character {
    scale: f32,
    glyph: Glyph,
}

/// One side of a glyph pair adjustment record.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(super) struct GlyphValue {
    pub(super) x_placement: f32,
    pub(super) y_placement: f32,
    pub(super) x_advance: f32,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct PairRecord {
    pub(super) first: GlyphValue,
    pub(super) second: GlyphValue,
    pub(super) ignore_spacing_adjustments: bool,
}

pub(super) struct FontAsset {
    pub(super) name: String,
    pub(super) face: Face,
    pub(super) normal_spacing_offset: f32,
    dynamic: bool,
    characters: HashMap<u32, Character>,
    fallbacks: Vec<usize>,
    /// Keyed as TMP keys its lookup: second glyph index << 16 | first.
    pairs: HashMap<u32, PairRecord>,
}

impl FontAsset {
    pub(super) fn has_pairs(&self) -> bool {
        !self.pairs.is_empty()
    }

    pub(super) fn pair(&self, first: u32, second: u32) -> Option<&PairRecord> {
        self.pairs.get(&(second << 16 | first))
    }
}

/// What a character resolves to.
#[derive(Debug, Clone, Copy)]
pub(super) enum Element {
    Stored { character_scale: f32, glyph: Glyph },
    /// Added at run time by a dynamic asset from its source font file.
    Dynamic,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct Resolved {
    pub(super) asset: usize,
    pub(super) element: Element,
    /// The code point TMP draws: the character itself, or the replacement
    /// its missing-glyph path chose.
    pub(super) unicode: u32,
}

pub(super) struct TmpFonts {
    assets: Vec<FontAsset>,
    by_source: HashMap<String, usize>,
    missing_glyph: u32,
    settings_fallbacks: Vec<usize>,
    default_font: Option<usize>,
}

fn num(value: &Value, what: &str) -> Result<f32, String> {
    let n = value.as_f64().ok_or_else(|| format!("TMP font document: {what} is not a number"))? as f32;
    if n.is_finite() { Ok(n) } else { Err(format!("TMP font document: {what} is not finite")) }
}

fn int(value: &Value, what: &str) -> Result<i64, String> {
    value.as_i64().ok_or_else(|| format!("TMP font document: {what} is not an integer"))
}

fn glyph_value(record: &Value, what: &str) -> Result<GlyphValue, String> {
    let v = &record["m_GlyphValueRecord"];
    Ok(GlyphValue {
        x_placement: num(&v["m_XPlacement"], what)?,
        y_placement: num(&v["m_YPlacement"], what)?,
        x_advance: num(&v["m_XAdvance"], what)?,
    })
}

impl TmpFonts {
    /// Parse the font document of a region root of `region` / `client_version`.
    pub(super) fn parse(doc: &Value, region: &str, client_version: &str) -> Result<Self, String> {
        if doc["version"].as_u64() != Some(1) {
            return Err("TMP font document version is not 1".into());
        }
        if doc["region"].as_str() != Some(region) || doc["clientVersion"].as_str() != Some(client_version) {
            return Err(format!("TMP font document is not {region} {client_version}"));
        }
        let records = doc["fontAssets"].as_object().ok_or("TMP font document has no fontAssets")?;
        let names: Vec<&String> = records.keys().collect();
        let index_of = |name: &str| names.iter().position(|n| n.as_str() == name);
        let runtime = doc["runtimeFallbacks"]["afterSetupOnDemandFontAsset"]
            .as_object()
            .ok_or("TMP font document has no on-demand fallback lists")?;
        let mut assets = Vec::with_capacity(records.len());
        for (name, record) in records {
            let what = |field: &str| format!("{name} {field}");
            if record["m_Name"].as_str() != Some(name.as_str()) {
                return Err(format!("TMP font document: asset {name} carries another m_Name"));
            }
            let fi = &record["m_FaceInfo"];
            let face = Face {
                point_size: num(&fi["m_PointSize"], &what("m_PointSize"))?,
                scale: num(&fi["m_Scale"], &what("m_Scale"))?,
                line_height: num(&fi["m_LineHeight"], &what("m_LineHeight"))?,
                ascent_line: num(&fi["m_AscentLine"], &what("m_AscentLine"))?,
                descent_line: num(&fi["m_DescentLine"], &what("m_DescentLine"))?,
                baseline: num(&fi["m_Baseline"], &what("m_Baseline"))?,
            };
            if face.point_size <= 0.0 || face.scale <= 0.0 {
                return Err(format!("TMP font document: {name} has a non-positive point size or scale"));
            }
            let dynamic = match int(&record["m_AtlasPopulationMode"], &what("m_AtlasPopulationMode"))? {
                0 => false,
                1 => true,
                mode => return Err(format!("TMP font document: {name} atlas population mode {mode}")),
            };
            let mut glyphs: HashMap<u32, Glyph> = HashMap::new();
            for g in record["m_GlyphTable"].as_array().ok_or_else(|| what("m_GlyphTable"))? {
                let index = int(&g["m_Index"], &what("glyph m_Index"))? as u32;
                let glyph = Glyph {
                    index,
                    horizontal_advance: num(&g["m_Metrics"]["m_HorizontalAdvance"], &what("glyph advance"))?,
                    scale: num(&g["m_Scale"], &what("glyph m_Scale"))?,
                };
                glyphs.entry(index).or_insert(glyph);
            }
            let mut characters: HashMap<u32, Character> = HashMap::new();
            for c in record["m_CharacterTable"].as_array().ok_or_else(|| what("m_CharacterTable"))? {
                if int(&c["m_ElementType"], &what("character m_ElementType"))? != 1 {
                    return Err(format!("TMP font document: {name} has a non-character text element"));
                }
                let unicode = int(&c["m_Unicode"], &what("character m_Unicode"))? as u32;
                let index = int(&c["m_GlyphIndex"], &what("character m_GlyphIndex"))? as u32;
                let glyph = *glyphs.get(&index).ok_or_else(|| {
                    format!("TMP font document: {name} U+{unicode:04X} names glyph {index}, absent from its glyph table")
                })?;
                let scale = num(&c["m_Scale"], &what("character m_Scale"))?;
                characters.entry(unicode).or_insert(Character { scale, glyph });
            }
            if !dynamic {
                for unicode in SYNTHESIZED {
                    characters.entry(unicode).or_insert(Character {
                        scale: 1.0,
                        glyph: Glyph { index: 0, horizontal_advance: 0.0, scale: 1.0 },
                    });
                }
            }
            let mut pairs = HashMap::new();
            let feature = &record["m_FontFeatureTable"]["m_GlyphPairAdjustmentRecords"];
            for p in feature.as_array().ok_or_else(|| what("pair adjustment records"))? {
                let first = &p["m_FirstAdjustmentRecord"];
                let second = &p["m_SecondAdjustmentRecord"];
                let key = (int(&second["m_GlyphIndex"], &what("pair second glyph"))? as u32) << 16
                    | int(&first["m_GlyphIndex"], &what("pair first glyph"))? as u32;
                let flags = int(&p["m_FeatureLookupFlags"], &what("pair m_FeatureLookupFlags"))?;
                pairs.entry(key).or_insert(PairRecord {
                    first: glyph_value(first, &what("pair first record"))?,
                    second: glyph_value(second, &what("pair second record"))?,
                    ignore_spacing_adjustments: flags & IGNORE_SPACING_ADJUSTMENTS == IGNORE_SPACING_ADJUSTMENTS,
                });
            }
            let fallbacks = match runtime.get(name) {
                Some(list) => list
                    .as_array()
                    .ok_or_else(|| what("on-demand fallback list"))?
                    .iter()
                    .map(|n| {
                        n.as_str().and_then(index_of).ok_or_else(|| {
                            format!("TMP font document: {name} on-demand fallback {n} is not one of its assets")
                        })
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => {
                    let serialized = record["m_FallbackFontAssetTable"].as_array().ok_or_else(|| what("m_FallbackFontAssetTable"))?;
                    if serialized.iter().any(|entry| !entry.is_null()) {
                        return Err(format!(
                            "TMP font document: {name} keeps a serialized fallback list the document does not resolve"
                        ));
                    }
                    Vec::new()
                }
            };
            assets.push(FontAsset {
                name: name.clone(),
                face,
                normal_spacing_offset: num(&record["normalSpacingOffset"], &what("normalSpacingOffset"))?,
                dynamic,
                characters,
                fallbacks,
                pairs,
            });
        }
        let mut by_source = HashMap::new();
        for (source, name) in doc["bySource"].as_object().ok_or("TMP font document has no bySource")? {
            let index = name.as_str().and_then(index_of)
                .ok_or_else(|| format!("TMP font document: bySource {source} names no asset"))?;
            by_source.insert(source.clone(), index);
        }
        let settings = &doc["tmpSettings"];
        let missing = int(&settings["m_missingGlyphCharacter"], "TMP settings m_missingGlyphCharacter")?;
        let settings_fallbacks = settings["m_fallbackFontAssets"]
            .as_array()
            .ok_or("TMP font document: TMP settings fallback list")?;
        if !settings_fallbacks.is_empty() {
            return Err("TMP font document: the TMP settings fallback list names assets it does not resolve".into());
        }
        if !settings["m_defaultFontAsset"].is_null() {
            return Err("TMP font document: the TMP settings default font asset is not resolved".into());
        }
        Ok(Self {
            assets,
            by_source,
            missing_glyph: if missing == 0 { DEFAULT_MISSING_GLYPH } else { missing as u32 },
            settings_fallbacks: Vec::new(),
            default_font: None,
        })
    }

    pub(super) fn asset(&self, index: usize) -> &FontAsset {
        &self.assets[index]
    }

    /// The asset a text component's `m_fontAsset` names, by the serialized
    /// file and path id the reference resolves to.
    pub(super) fn primary(&self, file: &str, path_id: i64) -> Result<usize, String> {
        self.by_source
            .get(&format!("{file}:{path_id}"))
            .copied()
            .ok_or_else(|| format!("TMP text font {file}:{path_id} is not in the font document"))
    }

    fn own(&self, unicode: u32, asset: usize) -> Option<Resolved> {
        let a = &self.assets[asset];
        if let Some(c) = a.characters.get(&unicode) {
            return Some(Resolved {
                asset,
                element: Element::Stored { character_scale: c.scale, glyph: c.glyph },
                unicode,
            });
        }
        a.dynamic.then_some(Resolved { asset, element: Element::Dynamic, unicode })
    }

    /// `TMP_FontAssetUtilities.GetCharacterFromFontAsset_Internal` for a
    /// regular-weight, non-italic request.
    fn internal(&self, unicode: u32, asset: usize, searched: Option<&mut HashSet<usize>>) -> Option<Resolved> {
        if let Some(hit) = self.own(unicode, asset) {
            return Some(hit);
        }
        let searched = searched?;
        for &fallback in &self.assets[asset].fallbacks {
            if !searched.insert(fallback) {
                continue;
            }
            if let Some(hit) = self.internal(unicode, fallback, Some(searched)) {
                return Some(hit);
            }
        }
        None
    }

    /// `TMP_FontAssetUtilities.GetCharacterFromFontAssets` with fallbacks.
    fn from_assets(&self, unicode: u32, list: &[usize]) -> Option<Resolved> {
        let mut searched = HashSet::new();
        list.iter().find_map(|&asset| self.internal(unicode, asset, Some(&mut searched)))
    }

    /// `TMP_FontAssetUtilities.GetCharacterFromFontAsset` with fallbacks.
    fn with_fallbacks(&self, unicode: u32, asset: usize) -> Option<Resolved> {
        self.internal(unicode, asset, Some(&mut HashSet::new()))
    }

    /// `TMP_Text.GetTextElement` when the current asset is the primary one
    /// (a fallback hit never persists to the next character), with no
    /// sprite asset on the text.
    fn text_element(&self, unicode: u32, primary: usize) -> Option<Resolved> {
        self.internal(unicode, primary, None)
            .or_else(|| self.from_assets(unicode, &self.assets[primary].fallbacks))
            .or_else(|| self.from_assets(unicode, &self.settings_fallbacks))
    }

    /// The element TMP lays one character out with, and whether the
    /// missing-glyph path replaced it.
    pub(super) fn resolve(&self, unicode: u32, primary: usize) -> Result<(Resolved, bool), String> {
        if let Some(hit) = self.text_element(unicode, primary) {
            return Ok((hit, false));
        }
        let replacement = self
            .with_fallbacks(self.missing_glyph, primary)
            .or_else(|| self.from_assets(self.missing_glyph, &self.settings_fallbacks))
            .or_else(|| self.default_font.and_then(|d| self.with_fallbacks(self.missing_glyph, d)))
            .or_else(|| self.with_fallbacks(0x20, primary))
            .or_else(|| self.with_fallbacks(0x03, primary))
            .ok_or_else(|| {
                format!(
                    "TMP U+{unicode:04X} and every replacement are in no asset of {}'s chain",
                    self.assets[primary].name
                )
            })?;
        Ok((replacement, true))
    }
}
