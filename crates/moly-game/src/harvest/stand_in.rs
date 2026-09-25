//! The SD stand-in table for the source swing clips (a product adaptation).
//!
//! The player's body is an SD character, and the u000 swing clips are never
//! played on it. Each source clip family maps to one SD clip of the shared
//! motion library, chosen by reading the clips as numbers: for every
//! candidate, the per-bone peak rotation profile of the arms and spine
//! (degrees from the rest pose, over the clip), the head drop, and the
//! length ratio were compared with the source clip's, and the lowest
//! distance (angle profile / 90 + 3 x head-drop difference + 0.5 x |ln length
//! ratio|, on the level-1 clips) won. Each row below is that choice, named as
//! an adaptation; the CW and CM families are the player's own motion family.
//! The level-5 Start clip has no fitting stand-in: the SD idle plays.
//!
//! The stand-in plays at `sd length / source length` times the harvest
//! animation speed, so the visible motion spans the source clip's time. It
//! never drives the hit clock.

use super::law::ToolState;

/// What to play for one source clip.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum StandIn {
    Clip { name: String },
    /// No SD clip fits: the player's SD idle (named).
    Idle,
}

/// Rows: source clip family -> (CW, CM) SD clip.
fn family_row(family: &str) -> Option<(&'static str, &'static str)> {
    Some(match family {
        "ax_l" => ("mov_cw_normal_stumble001_S", "mov_cm_happy_sad001_S"),
        "ax_e" => ("mov_cw_adult_fun001_E", "mov_cm_happy_clap001_S"),
        "pickax_l" => ("mov_cw_normal_dance001_L", "mov_cm_normal_angry002_S"),
        "pickax_e" => ("mov_cw_adult_fun001_E", "mov_cm_happy_talk001_E"),
        "pick001_o" => ("mov_cw_happy_02joy001_S", "mov_cm_happy_sad001_S"),
        "pick04_o" => ("mov_cw_happy_02joy001_S", "mov_cm_cool_02joy001_S"),
        "barrel01_o" => ("mov_cw_happy_02joy001_S", "mov_cm_normal_stumble001_S"),
        "toolbox01_o" => ("mov_cw_adult_surprise001_S", "mov_cm_happy_angry001_S"),
        "birthday_plant01_o" => ("mov_cw_happy_02joy001_S", "mov_cm_cool_02joy001_S"),
        "treasure01_o" => ("mov_cw_happy_rotate001_S", "mov_cm_cool_02joy001_S"),
        "listen01_o" => ("mov_cw_cool_sky001_S", "mov_cm_happy_doya001l_S"),
        _ => return None,
    })
}

/// The family key of a lower-case source clip name
/// (`mov_u000_site_ax01_l` -> `ax_l`, `mov_u000_site_pick001_o` ->
/// `pick001_o`).
fn family_of(source: &str, state: ToolState) -> Option<String> {
    let rest = source.strip_prefix("mov_u000_site_")?;
    for tool in ["pickax", "ax"] {
        if let Some(level_and_suffix) = rest.strip_prefix(tool) {
            if level_and_suffix.len() == 4 && level_and_suffix[..2].chars().all(|c| c.is_ascii_digit()) {
                let suffix = &level_and_suffix[3..];
                return Some(format!("{tool}_{suffix}"));
            }
        }
    }
    let _ = state;
    Some(rest.to_owned())
}

/// The stand-in for a source clip, for the player's SD family ("cw" / "cm").
pub(crate) fn stand_in(source: &str, state: ToolState, sd_family: &str) -> Result<StandIn, String> {
    if state == ToolState::Start {
        return Ok(StandIn::Idle);
    }
    let family = family_of(source, state).ok_or_else(|| format!("{source} is not a site clip"))?;
    let (cw, cm) = family_row(&family).ok_or_else(|| format!("no SD stand-in row for the family {family} of {source}"))?;
    match sd_family {
        "cw" => Ok(StandIn::Clip { name: cw.to_owned() }),
        "cm" => Ok(StandIn::Clip { name: cm.to_owned() }),
        other => Err(format!("SD motion family {other} has no stand-in column")),
    }
}

#[cfg(test)]
mod value_checks {
    use super::*;

    #[test]
    fn families_resolve_for_every_cn_tool_level() {
        for level in ["01", "02", "04", "05"] {
            let loop_clip = format!("mov_u000_site_ax{level}_l");
            assert_eq!(
                stand_in(&loop_clip, ToolState::Loop, "cw").unwrap(),
                StandIn::Clip { name: "mov_cw_normal_stumble001_S".into() }
            );
            let end = format!("mov_u000_site_pickax{level}_e");
            assert_eq!(
                stand_in(&end, ToolState::End, "cm").unwrap(),
                StandIn::Clip { name: "mov_cm_happy_talk001_E".into() }
            );
        }
        assert_eq!(stand_in("mov_u000_site_ax05_s", ToolState::Start, "cw").unwrap(), StandIn::Idle);
        assert_eq!(
            stand_in("mov_u000_site_pick04_o", ToolState::None, "cw").unwrap(),
            StandIn::Clip { name: "mov_cw_happy_02joy001_S".into() }
        );
        assert!(stand_in("mov_u000_site_unknown_o", ToolState::None, "cw").is_err());
    }
}
