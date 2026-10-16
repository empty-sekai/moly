//! Typed, source-authored environment selection data. No closed weather-ID table.
//! Optional per-site assets are independent lookups, not an all-or-nothing package.
use std::collections::{BTreeMap, BTreeSet};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
pub struct PhenomenonIndex {
    pub version: u32,
    pub phenomena: BTreeMap<String, PhenomenonDefinition>,
    #[serde(default, rename = "iconSources")]
    pub icon_sources: BTreeMap<String, crate::weather_icons::WeatherIconReceipt>,
    #[serde(default, rename = "environmentController")]
    pub environment_controller: Option<EnvironmentController>,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentController {
    pub schema_version: u32,
    pub source: crate::weather_timeline::SourceObjectIdentity,
    pub director: EnvironmentDirector,
}
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentDirector {
    pub source: crate::weather_timeline::SourceObjectIdentity,
    pub enabled: bool,
    pub initial_state: i32,
    pub wrap_mode: i32,
    pub update_mode: i32,
    pub initial_time: f64,
    pub scene_bindings: Vec<serde_json::Value>,
    pub exposed_references: Vec<serde_json::Value>,
}
impl EnvironmentController {
    pub fn validate(&self) -> Result<(), String> {
        self.source.validate()?;
        self.director.source.validate()?;
        let d = &self.director;
        if self.schema_version != 1 || !d.enabled || d.initial_state != 1
            || d.wrap_mode != 1 || d.update_mode != 1 || d.initial_time != 0.0
            || !d.scene_bindings.is_empty() || !d.exposed_references.is_empty() {
            return Err("unsupported source environment director clock/binding settings".into());
        }
        Ok(())
    }
}
#[derive(Debug, Deserialize)]
pub struct PhenomenonDefinition {
    pub id: i32,
    #[serde(default)]
    pub icon: Option<String>,
    #[serde(default)]
    pub master: Option<PhenomenonMetadata>,
    pub config: String,
    pub postprocess: String,
    pub ramp: SkyRamp,
    #[serde(default)]
    pub overrides: BTreeMap<String, SiteOverride>,
    pub fx: EffectDocument,
    #[serde(default)]
    pub timeline: Option<TimelineDocument>,
}
/// Display fields come from the master row, not the package's editor name.
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PhenomenonMetadata {
    pub id: i32,
    pub name: String,
    #[serde(default)]
    pub english_name: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub time_period_type: Option<String>,
    #[serde(default)]
    pub brightness_type: Option<String>,
    #[serde(default)]
    pub background_color_id: Option<i32>,
    #[serde(default)]
    pub icon_assetbundle_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TimelineDocument {
    pub file: String,
    pub duration: f64,
    pub tracks: u32,
    pub clips: u32,
}
#[derive(Debug, Deserialize)]
pub struct EffectDocument { pub file: String }
#[derive(Debug, Deserialize)]
pub struct SiteOverride {
    pub config: Option<String>,
    pub postprocess: Option<String>,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkyRamp {
    pub file: String,
    pub width: u32,
    pub height: u32,
    pub sky_bottom_color: [f32; 4],
}
impl PhenomenonIndex {
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let index: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        if index.version != 1 || index.phenomena.is_empty() {
            return Err("unsupported or empty phenomenon index".into());
        }
        for (key, receipt) in &index.icon_sources { receipt.validate(key)?; }
        let mut ids = BTreeSet::new();
        for (name, row) in &index.phenomena {
            if !ids.insert(row.id) { return Err(format!("duplicate phenomenon id {} ({name})", row.id)); }
            if let Some(icon) = &row.icon {
                validate_path(icon)?;
                let key = row.master.as_ref().and_then(|master| master.icon_assetbundle_name.as_deref())
                    .ok_or_else(|| format!("icon without authored master route for {name}"))?;
                let receipt = index.icon_sources.get(key)
                    .ok_or_else(|| format!("source icon receipt missing for {name}; re-extract source icons"))?;
                if receipt.artifact.file != *icon { return Err(format!("source icon route mismatch for {name}")); }
            } else if row.master.as_ref().and_then(|master| master.icon_assetbundle_name.as_ref()).is_some() {
                return Err(format!("authored source icon missing for {name}"));
            }
            if let Some(metadata) = &row.master {
                if metadata.id != row.id || metadata.name.is_empty() {
                    return Err(format!("invalid source display metadata for {name}"));
                }
            }
            for file in [&row.config, &row.postprocess, &row.ramp.file, &row.fx.file] {
                validate_path(file)?;
            }
            if let Some(timeline) = &row.timeline {
                validate_path(&timeline.file)?;
                if !timeline.duration.is_finite() || timeline.duration <= 0.0
                    || timeline.tracks == 0 || timeline.clips == 0
                {
                    return Err(format!("invalid source timeline summary for {name}"));
                }
            }
            for site in row.overrides.values() {
                for file in [&site.config, &site.postprocess].into_iter().flatten() { validate_path(file)?; }
            }
            if row.ramp.width == 0 || row.ramp.height == 0 || !row.ramp.sky_bottom_color.iter().all(|v| v.is_finite()) {
                return Err(format!("invalid source sky ramp for {name}"));
            }
        }
        Ok(index)
    }
}
fn validate_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.contains(['\\', ':', '?', '#']) || path.chars().any(char::is_control)
        || path.split('/').any(|part| part.is_empty() || part == "." || part == "..") {
        return Err(format!("invalid relative weather asset path: {path}"));
    }
    Ok(())
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentSelection {
    pub renderer_type: i32,
    pub emission_type: i32,
    pub light: EnvironmentLight,
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EnvironmentLight {
    #[serde(rename = "angleXZ")]
    pub angle_xz: f32,
    #[serde(rename = "angleY")]
    pub angle_y: f32,
    pub home_site_light_angle: HomeSiteLightAngle,
    pub character_directional_light_color: [f32; 4],
    pub character_shade_skin_color: [f32; 4],
    pub character_body_shade_color: [f32; 4],
    pub phenomena_directional_light_color: [f32; 4],
    pub phenomena_shade_color: [f32; 4],
    pub drop_shadow_color1: [f32; 4],
}
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomeSiteLightAngle {
    pub active: bool,
    #[serde(rename = "angleXZ")]
    pub angle_xz: f32,
    #[serde(rename = "angleY")]
    pub angle_y: f32,
}
impl EnvironmentLight {
    /// GetLightDirection's site gate is independent of unique config lookup.
    pub fn angles(&self, is_home_site: bool) -> [f32; 2] {
        if is_home_site && self.home_site_light_angle.active {
            [self.home_site_light_angle.angle_xz, self.home_site_light_angle.angle_y]
        } else { [self.angle_xz, self.angle_y] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn source_sky_bottom_color_is_required_and_preserved() {
        let ramp = json!({"file":"new/ramp.png","width":32,"height":1,"skyBottomColor":[0.551886796951294,0.8096057772636414,1.0,1.0]});
        let row: SkyRamp = serde_json::from_value(ramp.clone()).unwrap();
        assert_eq!(row.sky_bottom_color, [0.5518868,0.8096058,1.0,1.0]);
        let mut missing=ramp; missing.as_object_mut().unwrap().remove("skyBottomColor");
        assert!(serde_json::from_value::<SkyRamp>(missing).is_err());
    }
    #[test]
    fn home_angle_is_not_a_generic_config_override() {
        let mut light: EnvironmentLight = serde_json::from_value(json!({
            "angleXZ":80.0,"angleY":57.900001525878906,
            "homeSiteLightAngle":{"active":true,"angleXZ":240.0,"angleY":70.0},
            "characterDirectionalLightColor":[1,1,1,1],"characterShadeSkinColor":[1,1,1,1],"characterBodyShadeColor":[1,1,1,1],
            "phenomenaDirectionalLightColor":[1,1,1,1],"phenomenaShadeColor":[1,1,1,1],"dropShadowColor1":[0,0,0,0]
        })).unwrap();
        assert_eq!(light.angles(false), [80.0,57.9]);
        assert_eq!(light.angles(true), [240.0,70.0]);
        light.home_site_light_angle.active=false;
        assert_eq!(light.angles(true), [80.0,57.9]);
    }
}
