//! The phenomenon master as the site map, the forecast dialog and the learn
//! flow read it: `MasterDataManager.GetMysekaiPhenomena(id)` over the
//! upstream table `mysekaiPhenomenas`, and
//! `GetMysekaiPhenomenaBackgroundColor(id)` over
//! `mysekaiPhenomenaBackgroundColors`. Both tables are read from the region's
//! master hosts ([`moly_assets::json::master`]); the client has no copy of its
//! own.
//!
//! A table its consumer cannot get is named once and leaves its lookups
//! empty: each lookup that misses logs as the source's `LogError` does and
//! takes the source's branch for a missing row.

use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};

/// One `MasterMysekaiPhenomena` row, the fields the readers use.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct PhenomenaRow {
    pub(crate) id: i32,
    /// `assetbundleName` (`001_sunny`), the weather chain's phenomenon key.
    pub(crate) asset: String,
    /// `iconAssetbundleName` (`env_sunny`): the site-map sprite is
    /// `"icon_" + icon`, the forecast thumbnail bundle
    /// `mysekai/thumbnail/phenomena/` + icon.
    pub(crate) icon: String,
    /// `englishName`: the site-map panel's second line.
    pub(crate) en: String,
    /// `name`: the panel's first line and the forecast cell's label.
    pub(crate) name: String,
    /// `mysekaiPhenomenaBackgroundColorId`.
    pub(crate) background_color_id: i32,
}

/// One `MasterMysekaiPhenomenaBackgroundColor` row, each colour code already
/// through `ColorUtility.ColorCodeToColor`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BackgroundColorRow {
    pub(crate) id: i32,
    pub(crate) base: [f32; 4],
    pub(crate) ground: [f32; 4],
    pub(crate) gradation: [f32; 4],
    pub(crate) corner: [f32; 4],
    pub(crate) ground_highlight: [f32; 4],
}

const PHENOMENAS: MasterTable<Vec<PhenomenaRow>> = MasterTable {
    table: "mysekaiPhenomenas",
    name: "mysekaiPhenomenas (the site-map weather panel and the forecast)",
    parse: parse_phenomenas,
};

const BACKGROUND_COLORS: MasterTable<Vec<BackgroundColorRow>> = MasterTable {
    table: "mysekaiPhenomenaBackgroundColors",
    name: "mysekaiPhenomenaBackgroundColors (the site-map background)",
    parse: parse_background_colors,
};

/// The two tables once both have resolved; `Err` carries the master layer's
/// reason for a table that did not arrive.
#[derive(Resource, Debug)]
pub(crate) struct PhenomenaMaster {
    pub(crate) rows: Result<Vec<PhenomenaRow>, String>,
    pub(crate) colors: Result<Vec<BackgroundColorRow>, String>,
}

impl PhenomenaMaster {
    /// `GetMysekaiPhenomena(id)`: the row, or `None` (the source returns
    /// null).
    pub(crate) fn phenomena(&self, id: i32) -> Option<&PhenomenaRow> {
        self.rows.as_ref().ok()?.iter().find(|row| row.id == id)
    }

    /// `GetMysekaiPhenomenaBackgroundColor(id)`.
    pub(crate) fn background_color(&self, id: i32) -> Option<&BackgroundColorRow> {
        self.colors.as_ref().ok()?.iter().find(|row| row.id == id)
    }

    /// The rows in master order (empty when the table did not arrive).
    pub(crate) fn rows(&self) -> &[PhenomenaRow] {
        self.rows.as_deref().unwrap_or(&[])
    }
}

/// Startup: request both tables.
pub(crate) fn load(mut masters: ResMut<MasterData>) {
    masters.request(&PHENOMENAS);
    masters.request(&BACKGROUND_COLORS);
}

type Part<T> = Option<Result<Vec<T>, String>>;

/// Update: take both tables once they have resolved.
pub(crate) fn resolve(
    mut commands: Commands,
    mut masters: ResMut<MasterData>,
    mut parts: Local<(Part<PhenomenaRow>, Part<BackgroundColorRow>)>,
    installed: Option<Res<PhenomenaMaster>>,
) {
    if installed.is_some() {
        return;
    }
    if parts.0.is_none() {
        parts.0 = masters
            .take(&PHENOMENAS)
            .map(|result| result.map_err(|error| error.to_string()));
    }
    if parts.1.is_none() {
        parts.1 = masters
            .take(&BACKGROUND_COLORS)
            .map(|result| result.map_err(|error| error.to_string()));
    }
    if parts.0.is_none() || parts.1.is_none() {
        return;
    }
    let (Some(rows), Some(colors)) = (parts.0.take(), parts.1.take()) else {
        return;
    };
    match &rows {
        Ok(rows) => info!(
            "[phenomena-master] mysekaiPhenomenas: {} rows, ids {:?}",
            rows.len(),
            rows.iter().map(|row| row.id).collect::<Vec<_>>()
        ),
        Err(reason) => error!(
            "[phenomena-master] mysekaiPhenomenas did not arrive ({reason}): every phenomenon lookup misses"
        ),
    }
    match &colors {
        Ok(colors) => info!(
            "[phenomena-master] mysekaiPhenomenaBackgroundColors: {} rows, ids {:?}",
            colors.len(),
            colors.iter().map(|row| row.id).collect::<Vec<_>>()
        ),
        Err(reason) => error!(
            "[phenomena-master] mysekaiPhenomenaBackgroundColors did not arrive ({reason}): every background colour lookup misses"
        ),
    }
    commands.insert_resource(PhenomenaMaster { rows, colors });
}

fn parse_phenomenas(text: &str) -> Result<Vec<PhenomenaRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            Ok(PhenomenaRow {
                id: master::int32(row, "id")?,
                asset: master::text(row, "assetbundleName")?.to_owned(),
                icon: master::text(row, "iconAssetbundleName")?.to_owned(),
                en: master::text(row, "englishName")?.to_owned(),
                name: master::text(row, "name")?.to_owned(),
                background_color_id: master::int32(row, "mysekaiPhenomenaBackgroundColorId")?,
            })
        })
        .collect()
}

fn parse_background_colors(text: &str) -> Result<Vec<BackgroundColorRow>, String> {
    master::rows(text)?
        .iter()
        .map(|row| {
            let color = |field: &str| -> Result<[f32; 4], String> {
                let code = master::text(row, field)?;
                color_code_to_color(code).ok_or_else(|| {
                    format!("{field} {code:?} is not a #RGB, #RGBA, #RRGGBB or #RRGGBBAA code")
                })
            };
            Ok(BackgroundColorRow {
                id: master::int32(row, "id")?,
                base: color("baseColor")?,
                ground: color("groundColor")?,
                gradation: color("gradationColor")?,
                corner: color("cornerColor")?,
                ground_highlight: color("groundHighlightColor")?,
            })
        })
        .collect()
}

/// `ColorUtility.ColorCodeToColor` on a hex code: `TryParseHtmlString`
/// reads `#RGB`, `#RGBA`, `#RRGGBB` or `#RRGGBBAA` into a `Color32` (a short
/// form doubles each digit; alpha 255 when absent), and the `Color` is each
/// byte over 255. `None` for any other text: the source falls back to white
/// there, and the master parser refuses such a table by name instead.
pub(crate) fn color_code_to_color(code: &str) -> Option<[f32; 4]> {
    let digits = code.strip_prefix('#')?;
    if !digits.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let mut out = [255u8; 4];
    match digits.len() {
        3 | 4 => {
            for (k, slot) in out.iter_mut().enumerate().take(digits.len()) {
                *slot = u8::from_str_radix(&digits[k..=k], 16).ok()? * 17;
            }
        }
        6 | 8 => {
            for (k, slot) in out.iter_mut().enumerate().take(digits.len() / 2) {
                *slot = u8::from_str_radix(&digits[k * 2..k * 2 + 2], 16).ok()?;
            }
        }
        _ => return None,
    }
    Some(out.map(|b| f32::from(b) / 255.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_codes_read_as_html_hex() {
        assert_eq!(
            color_code_to_color("#a0eeff"),
            Some([160.0 / 255.0, 238.0 / 255.0, 1.0, 1.0])
        );
        assert_eq!(color_code_to_color("#fff"), Some([1.0, 1.0, 1.0, 1.0]));
        assert_eq!(
            color_code_to_color("#00000080"),
            Some([0.0, 0.0, 0.0, 128.0 / 255.0])
        );
        assert_eq!(color_code_to_color("a0eeff"), None);
        assert_eq!(color_code_to_color("#a0eef"), None);
        assert_eq!(color_code_to_color("#gg0000"), None);
    }
}
