//! Typed source contract for MYSEKAI environment timelines.
//!
//! These documents are emitted by moly-root from Unity Timeline assets.  They
//! are not generic Timeline support: only the source-authored environment
//! color/value/noise tracks are admitted here, and unsupported clip semantics
//! fail closed rather than silently approximating them.

use serde::Deserialize;
use std::collections::HashSet;

/// Exact serialized identity, including JS-safe signed 64-bit path ID.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "camelCase")]
pub struct SourceObjectIdentity {
    pub bundle: String,
    pub archive: String,
    pub path_id: String,
}

impl SourceObjectIdentity {
    pub(crate) fn validate(&self) -> Result<(), String> {
        let id = self
            .path_id
            .parse::<i64>()
            .map_err(|_| "invalid source pathId".to_string())?;
        if id == 0
            || self.bundle.is_empty()
            || self.archive.is_empty()
            || id.to_string() != self.path_id
        {
            return Err("invalid source object identity".into());
        }
        Ok(())
    }
}

/// Original PPtr and its owning object; resolved identity alone is insufficient.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourceReference {
    pub owner: SourceObjectIdentity,
    pub pointer: SourcePointer,
    pub external: Option<String>,
    pub target: Option<SourceObjectIdentity>,
}
#[derive(Debug, Clone, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SourcePointer {
    pub file_id: i32,
    pub path_id: String,
}
impl SourceReference {
    pub(crate) fn validate(&self) -> Result<(), String> {
        self.owner.validate()?;
        let path = self
            .pointer
            .path_id
            .parse::<i64>()
            .map_err(|_| "invalid PPtr pathId")?;
        if path.to_string() != self.pointer.path_id || self.pointer.file_id < 0 {
            return Err("invalid serialized PPtr".into());
        }
        if path == 0 {
            return if self.target.is_none() {
                Ok(())
            } else {
                Err("null PPtr has a resolved target".into())
            };
        }
        let target = self.target.as_ref().ok_or("unresolved source PPtr")?;
        target.validate()?;
        if self.pointer.path_id != target.path_id {
            return Err("PPtr and target pathId disagree".into());
        }
        if self.pointer.file_id == 0 {
            if self.external.is_some()
                || self.owner.bundle != target.bundle
                || self.owner.archive != target.archive
            {
                return Err("local PPtr changed its owning serialized file".into());
            }
        } else if self
            .external
            .as_deref()
            .and_then(|name| name.rsplit(['/', '\\']).next())
            != Some(target.archive.as_str())
        {
            return Err("external PPtr and target archive disagree".into());
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherTimeline {
    pub schema_version: u32,
    pub source: SourceObjectIdentity,
    pub unsupported: Vec<serde_json::Value>,
    pub asset: String,
    pub duration: f64,
    pub duration_mode: i32,
    pub frame_rate: f64,
    pub name: String,
    pub root_references: Vec<SourceReference>,
    pub marker_reference: SourceReference,
    pub tracks: Vec<WeatherTimelineTrack>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherTimelineTrack {
    pub id: usize,
    pub parent_id: Option<usize>,
    pub source: SourceObjectIdentity,
    pub sibling_index: usize,
    pub reference: SourceReference,
    pub parent_reference: SourceReference,
    pub script_reference: SourceReference,
    pub child_references: Vec<SourceReference>,
    pub animation_references: std::collections::BTreeMap<String, SourceReference>,
    #[serde(default)]
    pub marker_track: bool,
    #[serde(default)]
    pub markers: Vec<serde_json::Value>,
    pub class: String,
    pub name: String,
    #[serde(default)]
    pub target: Option<String>,
    #[serde(default)]
    pub target_value: Option<i32>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub locked: bool,
    #[serde(default)]
    pub clips: Vec<WeatherTimelineClip>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherTimelineClip {
    pub source: SourceObjectIdentity,
    pub reference: SourceReference,
    pub script_reference: SourceReference,
    pub source_order: usize,
    pub class: String,
    pub label: String,
    pub start: f64,
    pub duration: f64,
    pub clip_in: f64,
    pub time_scale: f64,
    pub blend_in_duration: f64,
    pub blend_out_duration: f64,
    pub ease_in_duration: f64,
    pub ease_out_duration: f64,
    pub pre_extrapolation: i32,
    pub post_extrapolation: i32,
    pub asset: WeatherTimelineClipAsset,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WeatherTimelineClipAsset {
    #[serde(default)]
    pub gradient: Option<SourceGradient>,
    #[serde(default)]
    pub curve: Option<SourceAnimationCurve>,
    #[serde(default)]
    pub scale: Option<f32>,
    #[serde(default)]
    pub frequency: Option<f32>,
    #[serde(default)]
    pub intensity: Option<f32>,
    #[serde(default)]
    pub intensity_curve: Option<SourceAnimationCurve>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceGradient {
    pub mode: i32,
    pub color_space: i32,
    pub color_keys: Vec<SourceGradientColorKey>,
    pub alpha_keys: Vec<SourceGradientAlphaKey>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceGradientColorKey {
    pub color: [f32; 3],
    pub time: f32,
}

#[derive(Debug, Clone, Deserialize)]
pub struct SourceGradientAlphaKey {
    pub alpha: f32,
    pub time: f32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceAnimationCurve {
    pub keys: Vec<SourceCurveKey>,
    pub pre_infinity: i32,
    pub post_infinity: i32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceCurveKey {
    pub time: f32,
    pub value: f32,
    pub in_slope: f32,
    pub out_slope: f32,
    pub in_weight: f32,
    pub out_weight: f32,
    pub weighted_mode: u32,
}

impl WeatherTimeline {
    /// Compile once at the asset boundary; per-frame evaluation allocates nothing.
    pub fn compile(&self) -> Result<moly_law::weather::timeline::Timeline, String> {
        use moly_law::particle::value::{Gradient, GradientAlphaKey, GradientColorKey};
        use moly_law::weather::timeline::{Clip, ClipValue, Target, Timeline, Track};
        self.validate()?;
        let mut tracks = Vec::with_capacity(self.tracks.len());
        for source in &self.tracks {
            let target = match (source.class.as_str(), source.target_value) {
                ("SiteEnvironmentColorTrack", Some(1)) => Target::SkyColor,
                ("SiteEnvironmentColorTrack", Some(2)) => Target::LightColor,
                ("SiteEnvironmentValueTrack", Some(1)) => Target::SkyIntensity,
                ("SiteEnvironmentValueTrack", Some(2)) => Target::LightIntensity,
                ("ValueNoiseTrack" | "MarkerTrack", _) => Target::Noise,
                _ => return Err("unsupported source timeline numeric target".into()),
            };
            let mut clips = Vec::with_capacity(source.clips.len());
            for clip in &source.clips {
                let asset = &clip.asset;
                let value = match clip.class.as_str() {
                    "SiteEnvironmentColorClip" => {
                        let gradient = asset.gradient.as_ref().unwrap();
                        ClipValue::Color(Gradient {
                            color_keys: gradient
                                .color_keys
                                .iter()
                                .map(|k| GradientColorKey {
                                    time: k.time,
                                    color: k.color,
                                })
                                .collect(),
                            alpha_keys: gradient
                                .alpha_keys
                                .iter()
                                .map(|k| GradientAlphaKey {
                                    time: k.time,
                                    alpha: k.alpha,
                                })
                                .collect(),
                        })
                    }
                    "SiteEnvironmentValueClip" => ClipValue::Value {
                        curve: asset.curve.as_ref().unwrap().compile(),
                        scale: asset.scale.unwrap(),
                    },
                    "ValueNoiseClip" => ClipValue::Noise {
                        frequency: asset.frequency.unwrap(),
                        intensity: asset.intensity.unwrap(),
                        curve: asset.intensity_curve.as_ref().unwrap().compile(),
                    },
                    _ => return Err("unsupported source timeline clip class".into()),
                };
                clips.push(Clip {
                    start: clip.start,
                    duration: clip.duration,
                    value,
                });
            }
            tracks.push(Track {
                target,
                muted: source.muted,
                scale: source.scale.unwrap_or(1.0),
                noise_track: self
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, t)| t.parent_id == Some(source.id) && t.class == "ValueNoiseTrack")
                    .min_by_key(|(_, t)| t.sibling_index)
                    .map(|(index, _)| index),
                clips,
            });
        }
        Ok(Timeline {
            duration: self.duration,
            tracks,
        })
    }

    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        let value: Self = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != 3 {
            return Err("weather timeline requires structural schemaVersion 3".into());
        }
        self.source.validate()?;
        if !self.unsupported.is_empty() {
            return Err(format!(
                "weather timeline has {} unresolved source gaps",
                self.unsupported.len()
            ));
        }
        if self.asset.is_empty()
            || self.name.is_empty()
            || !self.duration.is_finite()
            || self.duration <= 0.0
            || !self.frame_rate.is_finite()
            || self.frame_rate <= 0.0
        {
            return Err("invalid weather timeline header".into());
        }
        // Current source asset is FixedLength.  A future source value must be
        // understood before the runtime may reinterpret its clock.
        if self.duration_mode != 1 {
            return Err(format!(
                "unsupported weather timeline durationMode {}",
                self.duration_mode
            ));
        }
        let mut identities = HashSet::new();
        for (index, track) in self.tracks.iter().enumerate() {
            track.source.validate()?;
            if track.id != index || !identities.insert(&track.source) {
                return Err("weather timeline track identity is duplicate or unordered".into());
            }
            if let Some(parent) = track.parent_id {
                if parent >= index {
                    return Err("weather timeline parent edge is cyclic or unresolved".into());
                }
                if track.class != "ValueNoiseTrack"
                    || self.tracks[parent].class != "SiteEnvironmentValueTrack"
                {
                    return Err("unsupported weather timeline parent relationship".into());
                }
            } else if track.class == "ValueNoiseTrack" {
                return Err("ValueNoiseTrack has no structural parent".into());
            }
            track.validate()?;
        }
        self.validate_graph()
    }

    fn validate_graph(&self) -> Result<(), String> {
        let mut edge_count = 0;
        for reference in self
            .root_references
            .iter()
            .chain(std::iter::once(&self.marker_reference))
        {
            reference.validate()?;
            if reference.owner != self.source {
                return Err("root PPtr owner is not the timeline".into());
            }
            edge_count += usize::from(reference.target.is_some());
        }
        for track in &self.tracks {
            for reference in &track.child_references {
                reference.validate()?;
                if reference.owner != track.source {
                    return Err("child PPtr owner is not its source track".into());
                }
                edge_count += usize::from(reference.target.is_some());
            }
            track.reference.validate()?;
            track.parent_reference.validate()?;
            track.script_reference.validate()?;
            if track.reference.target.as_ref() != Some(&track.source)
                || track.parent_reference.owner != track.source
                || track.script_reference.owner != track.source
                || track.script_reference.target.is_none()
            {
                return Err("track source/owner PPtrs disagree".into());
            }
            let (expected, parent) = if let Some(parent_id) = track.parent_id {
                if track.marker_track {
                    return Err("marker track has a structural parent".into());
                }
                let parent = &self.tracks[parent_id];
                (
                    parent.child_references.get(track.sibling_index),
                    &parent.source,
                )
            } else if track.marker_track {
                if track.sibling_index != 0 || track.class != "MarkerTrack" {
                    return Err("invalid marker track slot".into());
                }
                (Some(&self.marker_reference), &self.source)
            } else {
                (self.root_references.get(track.sibling_index), &self.source)
            };
            if expected != Some(&track.reference)
                || track.parent_reference.target.as_ref() != Some(parent)
            {
                return Err("track sibling/parent structural references disagree".into());
            }
            for (field, reference) in &track.animation_references {
                reference.validate()?;
                if !matches!(field.as_str(), "m_AnimClip" | "m_Curves")
                    || reference.owner != track.source
                    || reference.target.is_some()
                {
                    return Err("unsupported track animation reference".into());
                }
            }
            let mut orders = HashSet::new();
            for clip in &track.clips {
                clip.reference.validate()?;
                clip.script_reference.validate()?;
                if clip.reference.owner != track.source
                    || clip.reference.target.as_ref() != Some(&clip.source)
                    || clip.script_reference.owner != clip.source
                    || clip.script_reference.target.is_none()
                {
                    return Err("clip relation/source PPtrs disagree".into());
                }
                if clip.source_order >= track.clips.len() || !orders.insert(clip.source_order) {
                    return Err("clip source order is missing or duplicated".into());
                }
            }
        }
        if edge_count != self.tracks.len() {
            return Err("source graph contains dropped or duplicated track edges".into());
        }
        Ok(())
    }
}

impl WeatherTimelineTrack {
    fn validate(&self) -> Result<(), String> {
        // Validate authored obligations even when muted. The current value
        // mixer enumerates its serialized noise children directly.
        if !self.markers.is_empty() {
            return Err("weather timeline contains unsupported markers".into());
        }
        match self.class.as_str() {
            "MarkerTrack" => {
                if !self.clips.is_empty() {
                    return Err("MarkerTrack unexpectedly carries clips".into());
                }
                return Ok(());
            }
            "SiteEnvironmentColorTrack" => match self.target.as_deref() {
                Some("skyAdditiveColor" | "lightAdditiveColor") => {}
                other => return Err(format!("unsupported weather color target {other:?}")),
            },
            "SiteEnvironmentValueTrack" => {
                match self.target.as_deref() {
                    Some("skyAdditiveIntensity" | "lightAdditiveIntensity") => {}
                    other => return Err(format!("unsupported weather value target {other:?}")),
                }
                let scale = self
                    .scale
                    .ok_or_else(|| "weather value track missing scale".to_string())?;
                finite_f32(scale, "weather value track scale")?;
            }
            "ValueNoiseTrack" => {}
            other => return Err(format!("unsupported weather timeline track {other}")),
        }

        let expected_target = match (self.class.as_str(), self.target_value) {
            ("SiteEnvironmentColorTrack", Some(1)) => Some("skyAdditiveColor"),
            ("SiteEnvironmentColorTrack", Some(2)) => Some("lightAdditiveColor"),
            ("SiteEnvironmentValueTrack", Some(1)) => Some("skyAdditiveIntensity"),
            ("SiteEnvironmentValueTrack", Some(2)) => Some("lightAdditiveIntensity"),
            ("ValueNoiseTrack", None) => None,
            _ => return Err("unsupported numeric timeline target".into()),
        };
        if self.target.as_deref() != expected_target {
            return Err("numeric timeline target and token disagree".into());
        }

        let mut previous_start = f64::NEG_INFINITY;
        for clip in &self.clips {
            clip.validate(&self.class)?;
            if clip.start < previous_start {
                return Err(format!("unordered clips in weather track {}", self.name));
            }
            previous_start = clip.start;
        }
        if self.clips.windows(2).any(|pair| {
            pair[0].start == pair[1].start && pair[0].source_order >= pair[1].source_order
        }) {
            return Err("equal-start clips changed their authored source order".into());
        }
        Ok(())
    }
}

impl WeatherTimelineClip {
    fn validate(&self, track_class: &str) -> Result<(), String> {
        self.source.validate()?;
        for (name, value) in [
            ("start", self.start),
            ("duration", self.duration),
            ("clipIn", self.clip_in),
            ("timeScale", self.time_scale),
            ("blendInDuration", self.blend_in_duration),
            ("blendOutDuration", self.blend_out_duration),
            ("easeInDuration", self.ease_in_duration),
            ("easeOutDuration", self.ease_out_duration),
        ] {
            finite_f64(value, name)?;
        }
        if self.start < 0.0 || self.duration <= 0.0 {
            return Err(format!("invalid weather clip interval {}", self.label));
        }
        // The source mixer reads director.time directly and the current authored
        // weather corpus uses no TimelineClip remapping/blending/extrapolation.
        // Refuse future authoring until its exact source semantics are modelled.
        if self.clip_in != 0.0
            || self.time_scale != 1.0
            || self.blend_in_duration != 0.0
            || self.blend_out_duration != 0.0
            || self.ease_in_duration != 0.0
            || self.ease_out_duration != 0.0
            || self.pre_extrapolation != 0
            || self.post_extrapolation != 0
        {
            return Err(format!(
                "unsupported weather TimelineClip remapping/blend/extrapolation: {}",
                self.label
            ));
        }

        match track_class {
            "SiteEnvironmentColorTrack" => {
                if self.class != "SiteEnvironmentColorClip"
                    || self.asset.gradient.is_none()
                    || self.asset.curve.is_some()
                    || self.asset.intensity_curve.is_some()
                {
                    return Err(format!("invalid weather color clip {}", self.label));
                }
                self.asset.gradient.as_ref().expect("checked").validate()?;
            }
            "SiteEnvironmentValueTrack" => {
                if self.class != "SiteEnvironmentValueClip"
                    || self.asset.curve.is_none()
                    || self.asset.gradient.is_some()
                    || self.asset.intensity_curve.is_some()
                {
                    return Err(format!("invalid weather value clip {}", self.label));
                }
                finite_f32(
                    self.asset
                        .scale
                        .ok_or_else(|| format!("value clip {} missing scale", self.label))?,
                    "weather value clip scale",
                )?;
                self.asset.curve.as_ref().expect("checked").validate()?;
            }
            "ValueNoiseTrack" => {
                if self.class != "ValueNoiseClip"
                    || self.asset.intensity_curve.is_none()
                    || self.asset.gradient.is_some()
                    || self.asset.curve.is_some()
                {
                    return Err(format!("invalid weather noise clip {}", self.label));
                }
                finite_f32(
                    self.asset
                        .frequency
                        .ok_or_else(|| format!("noise clip {} missing frequency", self.label))?,
                    "weather noise frequency",
                )?;
                finite_f32(
                    self.asset
                        .intensity
                        .ok_or_else(|| format!("noise clip {} missing intensity", self.label))?,
                    "weather noise intensity",
                )?;
                self.asset
                    .intensity_curve
                    .as_ref()
                    .expect("checked")
                    .validate()?;
            }
            _ => {}
        }
        Ok(())
    }
}

impl SourceGradient {
    fn validate(&self) -> Result<(), String> {
        if self.mode != 0 || self.color_space != 0 {
            return Err("unsupported source weather gradient mode/color space".into());
        }
        if self.color_keys.is_empty() || self.alpha_keys.is_empty() {
            return Err("weather gradient has no keys".into());
        }
        validate_times(
            self.color_keys.iter().map(|key| key.time),
            "weather gradient color",
        )?;
        validate_times(
            self.alpha_keys.iter().map(|key| key.time),
            "weather gradient alpha",
        )?;
        for key in &self.color_keys {
            if !key.color.iter().all(|v| v.is_finite()) {
                return Err("weather gradient has non-finite color".into());
            }
        }
        for key in &self.alpha_keys {
            finite_f32(key.alpha, "weather gradient alpha")?;
        }
        Ok(())
    }
}

impl SourceAnimationCurve {
    fn compile(&self) -> moly_law::particle::value::Curve {
        use moly_law::particle::value::{Curve, CurveKey};
        Curve {
            multiplier: 1.0,
            keys: self
                .keys
                .iter()
                .map(|k| CurveKey {
                    time: k.time,
                    value: k.value,
                    in_slope: k.in_slope,
                    out_slope: k.out_slope,
                    in_weight: k.in_weight,
                    out_weight: k.out_weight,
                    weighted_mode: k.weighted_mode as u8,
                })
                .collect(),
        }
    }
    fn validate(&self) -> Result<(), String> {
        if self.keys.is_empty() {
            return Err("weather AnimationCurve has no keys".into());
        }
        if self.pre_infinity != 2 || self.post_infinity != 2 {
            return Err(format!(
                "unsupported weather AnimationCurve infinity mode {}/{}",
                self.pre_infinity, self.post_infinity
            ));
        }
        validate_times(
            self.keys.iter().map(|key| key.time),
            "weather AnimationCurve",
        )?;
        for key in &self.keys {
            for (name, value) in [
                ("value", key.value),
                ("inWeight", key.in_weight),
                ("outWeight", key.out_weight),
            ] {
                finite_f32(value, name)?;
            }
            // Current weather curves have finite slopes. A null slope from a
            // legacy exporter loses the infinity sign; it must not be guessed.
            if !key.in_slope.is_finite() || !key.out_slope.is_finite() {
                return Err("unsupported non-finite weather AnimationCurve slope".into());
            }
            if key.weighted_mode & !3 != 0 {
                return Err(format!(
                    "unsupported weather AnimationCurve weightedMode {}",
                    key.weighted_mode
                ));
            }
        }
        Ok(())
    }
}

fn finite_f32(value: f32, name: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{name} is not finite"))
    }
}

fn finite_f64(value: f64, name: &str) -> Result<(), String> {
    if value.is_finite() {
        Ok(())
    } else {
        Err(format!("{name} is not finite"))
    }
}

fn validate_times(values: impl Iterator<Item = f32>, name: &str) -> Result<(), String> {
    let mut previous = f32::NEG_INFINITY;
    for value in values {
        if !value.is_finite() || value < previous {
            return Err(format!("{name} keys are not finite and ordered"));
        }
        previous = value;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = r#"{
  "schemaVersion":3,
  "unsupported":[],
  "source":{
    "bundle":"b",
    "archive":"a",
    "pathId":"10"
  },
  "asset":"env.playable",
  "duration":2.0,
  "durationMode":1,
  "frameRate":60.0,
  "name":"env",
  "tracks":[
    {
      "id":0,
      "parentId":null,
      "source":{
        "bundle":"b",
        "archive":"a",
        "pathId":"20"
      },
      "class":"SiteEnvironmentValueTrack",
      "name":"flash",
      "target":"skyAdditiveIntensity",
      "targetValue":1,
      "scale":0.5,
      "muted":false,
      "locked":false,
      "clips":[
        {
          "source":{
            "bundle":"b",
            "archive":"a",
            "pathId":"30"
          },
          "sourceOrder":0,
          "class":"SiteEnvironmentValueClip",
          "label":"clip",
          "start":0.2,
          "duration":1.0,
          "clipIn":0.0,
          "timeScale":1.0,
          "blendInDuration":0.0,
          "blendOutDuration":0.0,
          "easeInDuration":0.0,
          "easeOutDuration":0.0,
          "preExtrapolation":0,
          "postExtrapolation":0,
          "asset":{
            "scale":1.0,
            "curve":{
              "keys":[
                {
                  "time":0.0,
                  "value":0.0,
                  "inSlope":0.0,
                  "outSlope":1.0,
                  "inWeight":0.33333334,
                  "outWeight":0.33333334,
                  "weightedMode":0
                },
                {
                  "time":1.0,
                  "value":1.0,
                  "inSlope":1.0,
                  "outSlope":0.0,
                  "inWeight":0.33333334,
                  "outWeight":0.33333334,
                  "weightedMode":0
                }
              ],
              "preInfinity":2,
              "postInfinity":2
            }
          },
          "reference":{
            "owner":{
              "bundle":"b",
              "archive":"a",
              "pathId":"20"
            },
            "pointer":{
              "fileId":0,
              "pathId":"30"
            },
            "external":null,
            "target":{
              "bundle":"b",
              "archive":"a",
              "pathId":"30"
            }
          },
          "scriptReference":{
            "owner":{
              "bundle":"b",
              "archive":"a",
              "pathId":"30"
            },
            "pointer":{
              "fileId":0,
              "pathId":"50"
            },
            "external":null,
            "target":{
              "bundle":"b",
              "archive":"a",
              "pathId":"50"
            }
          }
        }
      ],
      "siblingIndex":0,
      "reference":{
        "owner":{
          "bundle":"b",
          "archive":"a",
          "pathId":"10"
        },
        "pointer":{
          "fileId":0,
          "pathId":"20"
        },
        "external":null,
        "target":{
          "bundle":"b",
          "archive":"a",
          "pathId":"20"
        }
      },
      "parentReference":{
        "owner":{
          "bundle":"b",
          "archive":"a",
          "pathId":"20"
        },
        "pointer":{
          "fileId":0,
          "pathId":"10"
        },
        "external":null,
        "target":{
          "bundle":"b",
          "archive":"a",
          "pathId":"10"
        }
      },
      "scriptReference":{
        "owner":{
          "bundle":"b",
          "archive":"a",
          "pathId":"20"
        },
        "pointer":{
          "fileId":0,
          "pathId":"40"
        },
        "external":null,
        "target":{
          "bundle":"b",
          "archive":"a",
          "pathId":"40"
        }
      },
      "childReferences":[],
      "animationReferences":{}
    }
  ],
  "rootReferences":[
    {
      "owner":{
        "bundle":"b",
        "archive":"a",
        "pathId":"10"
      },
      "pointer":{
        "fileId":0,
        "pathId":"20"
      },
      "external":null,
      "target":{
        "bundle":"b",
        "archive":"a",
        "pathId":"20"
      }
    }
  ],
  "markerReference":{
    "owner":{
      "bundle":"b",
      "archive":"a",
      "pathId":"10"
    },
    "pointer":{
      "fileId":0,
      "pathId":"0"
    },
    "external":null,
    "target":null
  }
}"#;

    #[test]
    fn source_value_track_is_typed_and_preserved() {
        let value = WeatherTimeline::from_bytes(MINIMAL.as_bytes()).unwrap();
        assert_eq!(value.duration, 2.0);
        assert_eq!(
            value.tracks[0].target.as_deref(),
            Some("skyAdditiveIntensity")
        );
        assert_eq!(
            value.tracks[0].clips[0]
                .asset
                .curve
                .as_ref()
                .unwrap()
                .keys
                .len(),
            2
        );
    }

    #[test]
    fn unsupported_clip_remapping_fails_closed() {
        let raw = MINIMAL.replace(r#""timeScale":1.0"#, r#""timeScale":2.0"#);
        assert!(
            WeatherTimeline::from_bytes(raw.as_bytes())
                .unwrap_err()
                .contains("remapping")
        );
    }
    #[test]
    fn source_graph_gaps_cannot_be_dropped_by_deserialization() {
        let raw = MINIMAL.replace(
            "\"unsupported\":[]",
            "\"unsupported\":[{\"reason\":\"missing pointer\"}]",
        );
        assert!(
            WeatherTimeline::from_bytes(raw.as_bytes())
                .unwrap_err()
                .contains("source gaps")
        );
    }

    #[test]
    fn cyclic_parent_is_rejected_even_if_labels_look_correct() {
        let raw = MINIMAL.replace("\"parentId\":null", "\"parentId\":0");
        assert!(
            WeatherTimeline::from_bytes(raw.as_bytes())
                .unwrap_err()
                .contains("parent edge")
        );
    }

    #[test]
    fn legacy_label_based_hierarchy_is_rejected() {
        let raw = MINIMAL.replace("\"schemaVersion\":3", "\"schemaVersion\":1");
        assert!(
            WeatherTimeline::from_bytes(raw.as_bytes())
                .unwrap_err()
                .contains("structural")
        );
    }

    #[test]
    fn signed_64_bit_source_identity_is_not_rounded() {
        let raw = MINIMAL.replace("\"pathId\":\"20\"", "\"pathId\":\"-8256082717871396968\"");
        let parsed = WeatherTimeline::from_bytes(raw.as_bytes()).unwrap();
        assert_eq!(parsed.tracks[0].source.path_id, "-8256082717871396968");
    }
    #[test]
    fn incorrect_original_pointer_is_not_hidden_by_matching_target_labels() {
        let mut document: WeatherTimeline = serde_json::from_str(MINIMAL).unwrap();
        document.tracks[0].reference.pointer.path_id = "21".into();
        assert!(document.validate().unwrap_err().contains("PPtr"));
    }

    #[test]
    fn root_edges_may_not_be_silently_dropped() {
        let mut document: WeatherTimeline = serde_json::from_str(MINIMAL).unwrap();
        document
            .root_references
            .push(document.root_references[0].clone());
        assert!(
            document
                .validate()
                .unwrap_err()
                .contains("dropped or duplicated")
        );
    }

    #[test]
    fn numeric_enum_cannot_be_substituted_by_a_readable_target_name() {
        let mut document: WeatherTimeline = serde_json::from_str(MINIMAL).unwrap();
        document.tracks[0].target_value = Some(2);
        assert!(
            document
                .validate()
                .unwrap_err()
                .contains("target and token")
        );
    }

    #[test]
    fn sibling_and_clip_slots_must_match_authored_edges() {
        let mut document: WeatherTimeline = serde_json::from_str(MINIMAL).unwrap();
        document.tracks[0].sibling_index = 7;
        assert!(document.validate().unwrap_err().contains("sibling/parent"));
        document.tracks[0].sibling_index = 0;
        document.tracks[0].clips[0].source_order = 7;
        assert!(document.validate().unwrap_err().contains("source order"));
    }

    #[test]
    fn external_source_pointer_keeps_the_owning_serialized_file() {
        let mut reference: SourceReference = serde_json::from_value(serde_json::json!({
            "owner":{"bundle":"a","archive":"source.assets","pathId":"10"},
            "pointer":{"fileId":1,"pathId":"20"},"external":"archive:/other/target.assets",
            "target":{"bundle":"b","archive":"target.assets","pathId":"20"}
        }))
        .unwrap();
        reference.validate().unwrap();
        reference.external = Some("unrelated.assets".into());
        assert!(reference.validate().unwrap_err().contains("external PPtr"));
    }

    /// Supply a fresh producer output directory. No private source assets or
    /// absolute installation paths are committed as a golden fixture.
    #[test]
    #[ignore = "requires MOLY_WEATHER_SOURCE_ROOT with fresh source-qualified producer output"]
    fn current_source_graphs_compile_and_replay_deterministically() {
        let root = std::path::PathBuf::from(
            std::env::var("MOLY_WEATHER_SOURCE_ROOT").expect("source root"),
        );
        let mut files = Vec::new();
        for entry in std::fs::read_dir(&root).unwrap() {
            let file = entry.unwrap().path().join("timeline.json");
            if file.is_file() {
                files.push(file);
            }
        }
        files.sort();
        assert!(!files.is_empty(), "source corpus has no authored timeline");
        for file in files {
            let document = WeatherTimeline::from_bytes(&std::fs::read(&file).unwrap()).unwrap();
            let timeline = document.compile().unwrap();
            let mut previous = None;
            let mut dynamic = false;
            for step in 0..=(document.duration * document.frame_rate * 4.0).ceil() as usize {
                let time = step as f64 / document.frame_rate;
                let a = timeline
                    .evaluate(time.rem_euclid(document.duration))
                    .unwrap();
                let b = timeline
                    .evaluate(time.rem_euclid(document.duration))
                    .unwrap();
                assert_eq!(a, b, "seek is not deterministic for {}", file.display());
                if let Some(value) = previous {
                    dynamic |= a != value;
                }
                previous = Some(a);
            }
            assert!(
                dynamic,
                "authored corpus unexpectedly has no changing values"
            );
            println!(
                "source timeline {}: {} tracks, {} clips, duration {}",
                file.display(),
                document.tracks.len(),
                document.tracks.iter().map(|t| t.clips.len()).sum::<usize>(),
                document.duration
            );
        }
    }
}
