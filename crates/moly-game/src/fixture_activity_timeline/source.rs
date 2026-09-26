//! The formal three-table join. Object identities, not display names, select
//! playable assets and tracks. Unknown non-empty tracks remain explicit.

use super::TimelineFailure;
use serde_json::Value;
use std::{collections::HashSet, sync::Arc};

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct SourceAssetId {
    pub file: String,
    pub path_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub(crate) struct TimelineClipKey {
    pub track: SourceAssetId,
    pub clip_index: usize,
}

/// A view's authored ControlPlayableAsset -> exposed particle binding. The
/// actual particle search belongs to that instantiated view's parent, not to
/// the asset loader or a globally same-named effect.
#[derive(Clone, Debug)]
pub(crate) struct TimelineEffectBinding {
    pub playable: Option<SourceAssetId>,
    pub bind_name: String,
    pub exposed_name: String,
}

/// A Control clip's `sourceGameObject` as its director resolves it
/// (`ExposedReference.Resolve`): the director's exposed-reference table
/// holds the clip's exposed name -> that value (null included); otherwise the
/// reference's default value. Read for a director whose prefab root carries
/// no fixture timeline view; a view binds its Control clips through its own
/// effect list instead.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum ExposedSource {
    /// The document carries no exposed-reference table for this director, or
    /// the value names an object in another serialized file.
    Unreadable(String),
    /// The reference resolves to no object.
    Null,
    /// The GameObject the clip controls.
    Object(SourceAssetId),
}

#[derive(Clone, Debug)]
pub(crate) struct FixtureTimelineViewBinding {
    pub identity: SourceAssetId,
    pub game_object: SourceAssetId,
    pub effects: Vec<TimelineEffectBinding>,
}

#[derive(Clone, Debug)]
pub(crate) struct ClipTarget {
    pub target_package: String,
    pub clip_name: String,
    pub path_id: String,
    pub source_clip: Option<SourceAssetId>,
    /// Verbatim resolved target record, including source clip bounds/looping
    /// and explicit metadata-missing diagnostics. No inferred timing lives here.
    pub source_record: Value,
}

#[derive(Clone, Debug)]
pub(crate) struct AnimationPlayableSettings {
    pub position: [f64; 3],
    pub rotation: [f64; 4],
    pub euler_angles: [f64; 3],
    pub use_track_match_fields: bool,
    pub match_target_fields: u64,
    pub remove_start_offset: bool,
    pub apply_foot_ik: bool,
    /// 0 = inherit the source AnimationClip, 1 = on, 2 = off.
    pub loop_mode: u64,
}

/// Unity ControlPlayableAsset fields. These are capabilities to resolve against
/// the bound source object, not a reason to silently omit particles/directors.
#[derive(Clone, Debug)]
pub(crate) struct ControlSettings {
    pub exposed_name: String,
    pub update_particle: bool,
    pub update_director: bool,
    pub update_itime_control: bool,
    pub search_hierarchy: bool,
    pub active: bool,
    pub post_playback: u64,
    pub random_seed: u32,
}

#[derive(Clone, Debug)]
pub(crate) enum TimelinePayload {
    Animation {
        target: ClipTarget,
        settings: AnimationPlayableSettings,
    },
    /// `ChangeEyePresetClip`: the selected row's pattern name. The view
    /// looks the name up in its own eye table (`ChangeEyePattern`); the row's
    /// cells in the clip are not read.
    Eye {
        pattern: String,
    },
    Lip {
        pattern: String,
        open: i32,
        middle: i32,
        close: i32,
    },
    NoPresetChange,
    BlinkGate,
    LipGate,
    LoopFlag {
        looping: bool,
        enable_talk: bool,
    },
    Se {
        package: String,
        cue: String,
    },
    NpcIkTalkGate,
    Emoticon {
        name: String,
        use_root: bool,
    },
    /// `FadeCharacterClip`: the bound NPC's dither alpha follows the clip's
    /// curve over the clip's normalized time.
    FadeCharacter(Arc<FadeCurve>),
    Control(ControlSettings),
    /// A cut-scene track's clip. The runner keeps its clock; the cut-scene
    /// owner drives these tracks (any other owner refuses them by name).
    CutScene(CutScenePayload),
    Unsupported {
        class: String,
        fields: Value,
    },
}

/// `FadeCharacterClip.AnimationCurve`, evaluated as the engine's
/// `AnimationCurve.Evaluate` does, and the value of its last key (what the
/// track mixer writes after the clip has ended).
#[derive(Clone, Debug)]
pub(crate) struct FadeCurve {
    curve: moly_law::particle::curve::EngineCurve,
    /// `keys.LastOrDefault().value`: 0 for a curve with no key.
    pub last_value: f32,
}

impl FadeCurve {
    fn parse(value: &Value) -> Result<Self, TimelineFailure> {
        let keys = array(value, "m_Curve")?
            .iter()
            .map(|key| {
                Ok(moly_law::particle::CurveKey {
                    time: finite(key, "time")? as f32,
                    value: finite(key, "value")? as f32,
                    in_slope: number(key, "inSlope")? as f32,
                    out_slope: number(key, "outSlope")? as f32,
                    weighted_mode: u8::try_from(unsigned(key, "weightedMode")?)
                        .map_err(|_| invalid("fade curve weightedMode out of range"))?,
                    in_weight: finite(key, "inWeight")? as f32,
                    out_weight: finite(key, "outWeight")? as f32,
                })
            })
            .collect::<Result<Vec<_>, TimelineFailure>>()?;
        let wrap = |field: &str| -> Result<Option<u32>, TimelineFailure> {
            Ok(Some(
                u32::try_from(unsigned(value, field)?)
                    .map_err(|_| invalid(format!("fade curve {field} out of range")))?,
            ))
        };
        let last_value = keys.last().map_or(0.0, |key| key.value);
        let curve = moly_law::particle::Curve {
            multiplier: 1.0,
            keys,
            pre_wrap: wrap("m_PreInfinity")?,
            post_wrap: wrap("m_PostInfinity")?,
        };
        // The clip's time is normalized to [0, 1] (`(t - start) / duration`).
        let curve = moly_law::particle::curve::EngineCurve::new(
            &curve,
            moly_law::particle::curve::CurveTime::Normalized,
        )
        .map_err(|reason| invalid(format!("FadeCharacterClip curve: {reason}")))?;
        Ok(Self { curve, last_value })
    }

    /// `AnimationCurve.Evaluate(time)`.
    pub(crate) fn evaluate(&self, time: f32) -> f32 {
        self.curve.evaluate(time)
    }
}

/// The clip assets of the cut-scene tracks, as serialized.
#[derive(Clone, Debug)]
pub(crate) enum CutScenePayload {
    /// `CinemachineShot`: the virtual camera the brain shows, by the
    /// director's exposed name.
    CinemachineShot { exposed_name: String },
    /// `FadeInClip` / `FadeOutClip` of the fade panel track: the cut-scene
    /// screen's `SetAlpha(alpha, Color)`.
    Fade { fade_in: bool, color: [f32; 4] },
    /// `UpdateObstacleClip._siteLevel`.
    UpdateObstacle { level: i32 },
    /// `HideSiteObstacleClip._siteLevel`.
    HideSiteObstacle { level: i32 },
    /// `ShowExpansionEffectClip`.
    SiteExpansion(ExpansionEffect),
    /// `EffectClip.template`: the effect prefab it instantiates.
    Effect(EffectTemplate),
    /// `ActivationPlayableAsset` of an activation track: the bound object is
    /// active while a clip of the track has weight (the track's
    /// `m_PostPlaybackState` applies when the graph is destroyed).
    Activation,
    /// `PlayerActivationClip`: the player avatar is shown on the clip's play
    /// and hidden on its pause while the director is short of its duration.
    PlayerActivation,
    /// `SkipNextClip`: a tap jumps the director to the clip's end (the
    /// cut-scene screen's skip input).
    SkipNext,
}

/// An animation track's recorded clip (`AnimationTrack.m_InfiniteClip`),
/// decoded, with the track settings its playable reads
/// (`AnimationTrack.CreateInfiniteTrackPlayable`: the clip plays from the
/// director's time with the track's pre/post extrapolation, and its root
/// transform takes the infinite-clip offset through an offset playable).
#[derive(Clone, Debug)]
pub(crate) struct InfiniteClip {
    pub asset: SourceAssetId,
    pub name: String,
    /// `sourceStopTime - sourceStartTime` (the start is 0).
    pub length: f64,
    pub curves: Vec<InfiniteCurve>,
    pub offset_position: [f64; 3],
    pub offset_euler: [f64; 3],
    pub pre_extrapolation: u64,
    pub post_extrapolation: u64,
    pub time_offset: f64,
    /// `m_TrackOffset` (0 applies the transform offsets).
    pub track_offset: u64,
    pub apply_foot_ik: bool,
    pub remove_offset: bool,
}

/// The two root-transform channels a recorded clip binds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum InfiniteChannel {
    /// `m_LocalPosition` (Transform attribute 1).
    Position,
    /// `localEulerAngles` (Transform attribute 4), degrees.
    Euler,
}

#[derive(Clone, Debug)]
pub(crate) struct InfiniteCurve {
    pub channel: InfiniteChannel,
    /// x, y or z.
    pub component: usize,
    keys: Vec<InfiniteKey>,
}

/// A streamed key: from its time to the next key the value is
/// `((a dt + b) dt + c) dt + d`, `dt` the time since the key; a constant
/// curve's key holds its value.
#[derive(Clone, Copy, Debug)]
struct InfiniteKey {
    time: f64,
    coefficients: [f64; 4],
}

impl InfiniteCurve {
    fn evaluate(&self, t: f64) -> f64 {
        let Some(first) = self.keys.first() else {
            return 0.0;
        };
        let index = self.keys.iter().rposition(|key| key.time <= t).unwrap_or(0);
        let key = if t < first.time {
            first
        } else {
            &self.keys[index]
        };
        let dt = (t - key.time).max(0.0);
        let [a, b, c, d] = key.coefficients;
        ((a * dt + b) * dt + c) * dt + d
    }
}

impl InfiniteClip {
    fn parse(record: &Value, fields: &Value) -> Result<Self, TimelineFailure> {
        if flag(record, "legacy")? {
            return Err(invalid("a legacy recorded clip is not played"));
        }
        let start = finite(record, "sourceStartTime")?;
        let stop = finite(record, "sourceStopTime")?;
        if start != 0.0 || stop <= start {
            return Err(invalid("recorded clip bounds are not [0, stop)"));
        }
        for binding in array(record, "bindings")? {
            let type_id = unsigned(binding, "typeID")?;
            let attribute = unsigned(binding, "attribute")?;
            if type_id != 4
                || unsigned(binding, "path")? != 0
                || !matches!(attribute, 1 | 4)
                || flag(binding, "isPPtrCurve")?
            {
                return Err(invalid(format!(
                    "recorded clip binds type {type_id} attribute {attribute} off the root transform; only the root position and Euler angles are played"
                )));
            }
        }
        let mut curves = Vec::new();
        for curve in array(record, "curves")? {
            let channel = match unsigned(curve, "attribute")? {
                1 => InfiniteChannel::Position,
                4 => InfiniteChannel::Euler,
                other => return Err(invalid(format!("recorded curve attribute {other}"))),
            };
            let component = unsigned(curve, "component")? as usize;
            if component > 2 || unsigned(curve, "path")? != 0 || unsigned(curve, "typeID")? != 4 {
                return Err(invalid("recorded curve is not a root transform component"));
            }
            let kind = string(curve, "kind")?;
            let mut keys = Vec::new();
            for key in array(curve, "keys")? {
                let row = key
                    .as_array()
                    .ok_or_else(|| invalid("recorded key is not an array"))?;
                let number = |index: usize| -> Result<f64, TimelineFailure> {
                    row.get(index)
                        .and_then(Value::as_f64)
                        .filter(|value| value.is_finite())
                        .ok_or_else(|| invalid("recorded key value is not finite"))
                };
                let time = number(0)?;
                let coefficients = match (kind, row.len()) {
                    ("cubic", 5) => [number(1)?, number(2)?, number(3)?, number(4)?],
                    ("const", 2) => [0.0, 0.0, 0.0, number(1)?],
                    _ => {
                        return Err(invalid(format!(
                            "recorded key of kind {kind} has {} values",
                            row.len()
                        )))
                    }
                };
                keys.push(InfiniteKey { time, coefficients });
            }
            if keys.is_empty() || keys.windows(2).any(|pair| pair[0].time > pair[1].time) {
                return Err(invalid("recorded curve keys are empty or unordered"));
            }
            curves.push(InfiniteCurve {
                channel,
                component,
                keys,
            });
        }
        Ok(Self {
            asset: asset(&record["asset"])?,
            name: string(record, "name")?.to_owned(),
            length: stop - start,
            curves,
            offset_position: vector3(&fields["m_InfiniteClipOffsetPosition"])?,
            offset_euler: vector3(&fields["m_InfiniteClipOffsetEulerAngles"])?,
            pre_extrapolation: unsigned(fields, "m_InfiniteClipPreExtrapolation")?,
            post_extrapolation: unsigned(fields, "m_InfiniteClipPostExtrapolation")?,
            time_offset: finite(fields, "m_InfiniteClipTimeOffset")?,
            track_offset: unsigned(fields, "m_TrackOffset")?,
            apply_foot_ik: flag(fields, "m_InfiniteClipApplyFootIK")?,
            remove_offset: flag(fields, "m_InfiniteClipRemoveOffset")?,
        })
    }

    /// The clip's local time at director time `t`: `t - timeOffset`, then
    /// the track's extrapolation outside `[0, length]` (Hold clamps, Loop
    /// wraps, PingPong reflects, None leaves the clip unplayed).
    pub(crate) fn local_time(&self, t: f64) -> Option<f64> {
        let local = t - self.time_offset;
        if local < 0.0 {
            return match self.pre_extrapolation {
                0 => None,
                mode => Some(extrapolate(local, self.length, mode)),
            };
        }
        if local > self.length {
            return match self.post_extrapolation {
                0 => None,
                mode => Some(extrapolate(local, self.length, mode)),
            };
        }
        Some(local)
    }

    /// The recorded root position and Euler angles (source frame, degrees)
    /// at clip time `local`; a channel with no curve reads 0.
    pub(crate) fn sample(&self, local: f64) -> ([f64; 3], [f64; 3]) {
        let mut position = [0.0; 3];
        let mut euler = [0.0; 3];
        for curve in &self.curves {
            let value = curve.evaluate(local);
            match curve.channel {
                InfiniteChannel::Position => position[curve.component] = value,
                InfiniteChannel::Euler => euler[curve.component] = value,
            }
        }
        (position, euler)
    }

    /// Whether the clip carries a root channel (`AnimationClip.hasRootTransforms`
    /// for a generic clip bound at the root).
    pub(crate) fn has_root_transforms(&self) -> bool {
        !self.curves.is_empty()
    }
}

/// `ShowExpansionEffectClip`'s serialized fields.
#[derive(Clone, Copy, Debug)]
pub(crate) struct ExpansionEffect {
    pub center: [f32; 3],
    pub start_radius: f32,
    pub end_radius: f32,
    pub min_radius: f32,
    pub max_radius: f32,
    pub gradient_range: f32,
    pub edge_color: [f32; 4],
    pub fade_color: [f32; 4],
}

/// `EffectClip.template` (`EffectTemplate`).
#[derive(Clone, Debug)]
pub(crate) struct EffectTemplate {
    /// The prefab pointer; `None` for an authored null.
    pub prefab: Option<String>,
    pub parent_mode: i64,
    pub character_id: i64,
    pub fixed_starting_point: bool,
    pub offset: [f64; 3],
    pub matched_duration: bool,
    pub random_seed: bool,
}

/// A marker of a track, in serialized order. Only `SignalEmitter` carries
/// the fields below; other marker classes keep their class name.
#[derive(Clone, Debug)]
pub(crate) struct TimelineMarker {
    pub class: String,
    pub asset: SourceAssetId,
    pub time: f64,
    pub retroactive: bool,
    pub emit_once: bool,
    pub signal: Option<SourceAssetId>,
    pub signal_name: Option<String>,
}

#[derive(Clone, Debug)]
struct Keyframe {
    time: f64,
    value: f64,
    in_slope: f64,
    out_slope: f64,
    weighted: u64,
    in_weight: f64,
    out_weight: f64,
}

#[derive(Clone, Debug)]
pub(crate) struct BlendCurve {
    keys: Vec<Keyframe>,
}

impl BlendCurve {
    fn parse(value: &Value) -> Result<Self, TimelineFailure> {
        let mut keys = Vec::new();
        for key in array(value, "curve")? {
            keys.push(Keyframe {
                time: finite(key, "time")?,
                value: finite(key, "value")?,
                in_slope: number(key, "inSlope")?,
                out_slope: number(key, "outSlope")?,
                weighted: unsigned(key, "weightedMode")?,
                in_weight: finite(key, "inWeight")?,
                out_weight: finite(key, "outWeight")?,
            });
        }
        if keys.windows(2).any(|pair| pair[0].time >= pair[1].time) {
            return Err(invalid("unordered mix curve"));
        }
        Ok(Self { keys })
    }

    fn evaluate(&self, time: f64, fade_in: bool) -> f64 {
        let t = time.clamp(0.0, 1.0);
        let Some(first) = self.keys.first() else {
            // TimelineClip supplies EaseInOut when the stored curve is empty.
            let value = t * t * (3.0 - 2.0 * t);
            return if fade_in { value } else { 1.0 - value };
        };
        if t <= first.time {
            return first.value;
        }
        let last = self.keys.last().expect("nonempty curve");
        if t >= last.time {
            return last.value;
        }
        let pair = self
            .keys
            .windows(2)
            .find(|p| t <= p[1].time)
            .expect("enclosed time");
        let (a, b) = (&pair[0], &pair[1]);
        if !a.out_slope.is_finite() || !b.in_slope.is_finite() {
            return a.value;
        }
        let span = b.time - a.time;
        let x = (t - a.time) / span;
        // A non-weighted Hermite tangent is a Bezier handle of length 1/3.
        let out = if a.weighted & 2 != 0 {
            a.out_weight
        } else {
            1.0 / 3.0
        };
        let input = if b.weighted & 1 != 0 {
            b.in_weight
        } else {
            1.0 / 3.0
        };
        let (mut lo, mut hi) = (0.0, 1.0);
        for _ in 0..40 {
            let u = (lo + hi) * 0.5;
            if bezier(0.0, out, 1.0 - input, 1.0, u) < x {
                lo = u;
            } else {
                hi = u;
            }
        }
        bezier(
            a.value,
            a.value + a.out_slope * span * out,
            b.value - b.in_slope * span * input,
            b.value,
            (lo + hi) * 0.5,
        )
    }
}

fn bezier(a: f64, b: f64, c: f64, d: f64, t: f64) -> f64 {
    let s = 1.0 - t;
    s * s * s * a + 3.0 * s * s * t * b + 3.0 * s * t * t * c + t * t * t * d
}

#[derive(Clone, Debug)]
pub(crate) struct TimelineClip {
    pub key: TimelineClipKey,
    /// The particular playable object, not its content-deduplicated payload.
    /// None is retained for legacy metadata or an authored null reference.
    pub playable: Option<SourceAssetId>,
    pub effect_binding: Option<TimelineEffectBinding>,
    /// A Control clip of a director with no fixture timeline view: its source
    /// object through the director's exposed-reference table.
    pub exposed_source: Option<ExposedSource>,
    pub start: f64,
    pub duration: f64,
    pub clip_in: f64,
    pub time_scale: f64,
    pub ease_in: f64,
    pub ease_out: f64,
    pub blend_in: f64,
    pub blend_out: f64,
    pub mix_in: BlendCurve,
    pub mix_out: BlendCurve,
    pub pre_extrapolation: u64,
    pub post_extrapolation: u64,
    pub pre_time: f64,
    pub post_time: f64,
    pub payload: TimelinePayload,
    /// Full source envelope and payload are retained for diagnostics, including
    /// curve modes and attributes not consumed by the renderer.
    pub source_envelope: Value,
    pub source_payload: Value,
}

impl TimelineClip {
    pub(crate) fn end(&self) -> f64 {
        self.start + self.duration
    }
    pub(crate) fn contains(&self, t: f64) -> bool {
        t >= self.start && t < self.end()
    }

    pub(crate) fn sample_time(&self, t: f64, final_sample: bool) -> Option<f64> {
        let relative = t - self.start;
        let relative = if t < self.start {
            if self.pre_extrapolation == 0 || t < self.start - self.pre_time {
                return None;
            }
            extrapolate(relative, self.duration, self.pre_extrapolation)
        } else if t >= self.end() && !(final_sample && t == self.end()) {
            if self.post_extrapolation == 0 || t >= self.end() + self.post_time {
                return None;
            }
            extrapolate(relative, self.duration, self.post_extrapolation)
        } else {
            relative
        };
        Some(relative * self.time_scale + self.clip_in)
    }

    pub(crate) fn weight(&self, t: f64) -> f32 {
        let input = if self.blend_in > 0.0 {
            self.blend_in
        } else {
            self.ease_in
        };
        let output = if self.blend_out > 0.0 {
            self.blend_out
        } else {
            self.ease_out
        };
        let a = if input > 0.0 {
            self.mix_in
                .evaluate((t - self.start) / input, true)
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        let b = if output > 0.0 {
            self.mix_out
                .evaluate((t - (self.end() - output)) / output, false)
                .clamp(0.0, 1.0)
        } else {
            1.0
        };
        (a * b) as f32
    }
}

fn extrapolate(time: f64, duration: f64, mode: u64) -> f64 {
    match mode {
        1 => time.clamp(0.0, duration),
        2 if time < 0.0 => duration - (-time % duration),
        2 if time > duration => time % duration,
        3 => duration - (time.rem_euclid(2.0 * duration) - duration).abs(),
        _ => time,
    }
}

#[derive(Clone, Debug)]
pub(crate) struct TimelineTrack {
    pub identity: SourceAssetId,
    pub class: String,
    pub name: String,
    pub clips: Vec<TimelineClip>,
    /// The track's markers (`TrackAsset.m_Markers`); empty in documents
    /// exported before markers were.
    pub markers: Vec<TimelineMarker>,
    /// The track class's own serialized settings, verbatim (`Null` in
    /// documents exported before they were).
    pub settings: Value,
    /// The decoded recorded clip of an animation track that has one.
    pub infinite: Option<Arc<InfiniteClip>>,
}

#[derive(Clone, Debug)]
pub(crate) struct TimelineDefinition {
    pub package: String,
    pub prefab: String,
    pub fixture_view: Option<FixtureTimelineViewBinding>,
    pub director: SourceAssetId,
    pub timeline: SourceAssetId,
    pub duration: f64,
    pub tracks: Vec<TimelineTrack>,
}

pub(crate) struct TimelinePackage {
    tracks: Value,
    clips: Value,
    targets: Value,
}

impl TimelinePackage {
    pub(crate) fn from_jsons(
        tracks: &str,
        clips: &str,
        targets: &str,
    ) -> Result<Self, TimelineFailure> {
        let parse =
            |text| serde_json::from_str(text).map_err(|e| invalid(format!("timeline JSON: {e}")));
        let result = Self {
            tracks: parse(tracks)?,
            clips: parse(clips)?,
            targets: parse(targets)?,
        };
        let package = string(&result.tracks, "package")?;
        if string(&result.clips, "package")? != package
            || string(&result.targets, "package")? != package
        {
            return Err(invalid("timeline three-table package mismatch"));
        }
        Ok(result)
    }

    pub(crate) fn select_prefab(
        &self,
        asset_name: &str,
    ) -> Result<Arc<TimelineDefinition>, TimelineFailure> {
        let wanted = asset_name.strip_suffix(".prefab").unwrap_or(asset_name);
        let prefabs: Vec<_> = array(&self.tracks, "prefabs")?
            .iter()
            .filter(|p| {
                p.get("container")
                    .and_then(Value::as_str)
                    .is_some_and(|name| {
                        name == asset_name
                            || name
                                .rsplit('/')
                                .next()
                                .and_then(|s| s.strip_suffix(".prefab"))
                                == Some(wanted)
                    })
            })
            .collect();
        if prefabs.len() != 1 {
            return Err(invalid(format!(
                "prefab selection is not unique: {asset_name}"
            )));
        }
        let prefab = prefabs[0];
        // NPCFixtureTimelineController.SetUpView calls GetComponent on the
        // instantiated prefab root, then uses that view's own director. Child
        // views and another otherwise unique director are not substitutes.
        let root = asset(&prefab["asset"])?;
        let views = match prefab.get("fixtureTimelineViews") {
            Some(value) => value
                .as_array()
                .ok_or_else(|| invalid("fixtureTimelineViews is not an array"))?
                .iter()
                .filter_map(|view| match asset(&view["gameObject"]) {
                    Ok(owner) if owner == root => Some(Ok(view)),
                    Ok(_) => None,
                    Err(error) => Some(Err(error)),
                })
                .collect::<Result<Vec<_>, _>>()?,
            None => Vec::new(), // Older documents retain the previous reader.
        };
        if views.len() > 1 {
            return Err(invalid("multiple NPC timeline views on the prefab root"));
        }
        let view_director = views
            .first()
            .map(|view| asset(&view["director"]))
            .transpose()?;
        let fixture_view = views
            .first()
            .map(
                |view| -> Result<FixtureTimelineViewBinding, TimelineFailure> {
                    Ok(FixtureTimelineViewBinding {
                        identity: asset(&view["asset"])?,
                        game_object: asset(&view["gameObject"])?,
                        effects: array(view, "effectBindings")?
                            .iter()
                            .map(|row| {
                                Ok(TimelineEffectBinding {
                                    playable: if row["playable"].is_null() {
                                        None
                                    } else {
                                        Some(asset(&row["playable"])?)
                                    },
                                    bind_name: string(row, "bindName")?.to_owned(),
                                    exposed_name: string(row, "exposedName")?.to_owned(),
                                })
                            })
                            .collect::<Result<Vec<_>, TimelineFailure>>()?,
                    })
                },
            )
            .transpose()?;
        let directors: Vec<_> = array(prefab, "directors")?
            .iter()
            .filter(|d| match &view_director {
                Some(wanted) => asset(&d["asset"]).ok().as_ref() == Some(wanted),
                // Non-NPC prefabs (e.g. FixtureIdleAnimationController) and
                // legacy documents still require the existing unique director.
                None => !d["timeline"].is_null(),
            })
            .collect();
        if directors.len() != 1 {
            return Err(invalid("prefab director selection is not unique"));
        }
        let director = directors[0];
        let exposed = exposed_table(director);
        let identity = asset(&director["timeline"])?;
        let timelines: Vec<_> = array(&self.tracks, "timelines")?
            .iter()
            .filter(|t| asset(&t["asset"]).ok().as_ref() == Some(&identity))
            .collect();
        if timelines.len() != 1 {
            return Err(invalid(
                "director's exact playable asset is missing or duplicated",
            ));
        }
        let timeline = timelines[0];
        let mut selected = Vec::new();
        flatten(array(timeline, "tracks")?, &mut selected)?;
        if !timeline["marker"].is_null() {
            flatten(std::slice::from_ref(&timeline["marker"]), &mut selected)?;
        }
        let mut seen = HashSet::new();
        let mut tracks = Vec::new();
        for track in selected {
            let identity = asset(&track["asset"])?;
            if !seen.insert(identity.clone()) {
                return Err(invalid("duplicate selected track identity"));
            }
            let matches: Vec<_> = array(&self.clips, "tracks")?
                .iter()
                .filter(|row| asset(&row["asset"]).ok().as_ref() == Some(&identity))
                .collect();
            let class = string(track, "class")?.to_owned();
            if matches.is_empty() && matches!(class.as_str(), "GroupTrack" | "MarkerTrack") {
                continue;
            }
            if matches.len() != 1 {
                return Err(invalid(format!("missing selected track {identity:?}")));
            }
            let row = matches[0];
            if let Some(id) = row.get("m_AnimClip").and_then(|v| v.get("m_PathID")) {
                if path_id(id)? != "0" {
                    return Err(invalid("infinite animation clip needs an explicit binding"));
                }
            }
            // A recorded clip is played only from its decoded record; a row
            // that names one without the record is refused as before.
            let infinite = match row.get("m_InfiniteClip").and_then(|v| v.get("m_PathID")) {
                Some(id) if path_id(id)? != "0" => match row.get("infiniteClip") {
                    Some(record) if !record.is_null() => Some(Arc::new(
                        InfiniteClip::parse(record, &row["fields"]).map_err(|error| {
                            invalid(format!(
                                "track {}: recorded clip: {}",
                                row["name"].as_str().unwrap_or("?"),
                                error.message
                            ))
                        })?,
                    )),
                    _ => return Err(invalid("infinite animation clip needs an explicit binding")),
                },
                _ => None,
            };
            let mut clips = Vec::new();
            for (index, envelope) in array(row, "clips")?.iter().enumerate() {
                let key = TimelineClipKey {
                    track: identity.clone(),
                    clip_index: index,
                };
                let payload = array(&self.clips, "assets")?
                    .get(unsigned(envelope, "assetRef")? as usize)
                    .ok_or_else(|| invalid("clip assetRef is out of range"))?;
                let body = self.payload(payload, &key)?;
                let playable = envelope
                    .get("playableAsset")
                    .filter(|value| !value.is_null())
                    .map(asset)
                    .transpose()?;
                let effect_binding = playable.as_ref().and_then(|playable| {
                    fixture_view
                        .as_ref()?
                        .effects
                        .iter()
                        .find(|binding| binding.playable.as_ref() == Some(playable))
                        .cloned()
                });
                let exposed_source = match (&body, &fixture_view) {
                    (TimelinePayload::Control(settings), None) => Some(resolve_exposed(
                        &exposed,
                        &settings.exposed_name,
                        &payload["fields"]["sourceGameObject"]["defaultValue"],
                        playable.as_ref(),
                    )),
                    _ => None,
                };
                let result = TimelineClip {
                    key,
                    playable,
                    effect_binding,
                    exposed_source,
                    start: finite(envelope, "m_Start")?,
                    duration: finite(envelope, "m_Duration")?,
                    clip_in: finite(envelope, "m_ClipIn")?,
                    time_scale: finite(envelope, "m_TimeScale")?,
                    ease_in: finite(envelope, "m_EaseInDuration")?,
                    ease_out: finite(envelope, "m_EaseOutDuration")?,
                    blend_in: finite(envelope, "m_BlendInDuration")?,
                    blend_out: finite(envelope, "m_BlendOutDuration")?,
                    mix_in: BlendCurve::parse(&envelope["m_MixInCurve"])?,
                    mix_out: BlendCurve::parse(&envelope["m_MixOutCurve"])?,
                    pre_extrapolation: unsigned(&envelope["m_PreExtrapolationMode"], "value")?,
                    post_extrapolation: unsigned(&envelope["m_PostExtrapolationMode"], "value")?,
                    pre_time: number(envelope, "m_PreExtrapolationTime")?,
                    post_time: number(envelope, "m_PostExtrapolationTime")?,
                    payload: body,
                    source_envelope: envelope.clone(),
                    source_payload: payload.clone(),
                };
                if result.start < 0.0
                    || result.duration <= 0.0
                    || result.time_scale <= 0.0
                    || result.pre_time < 0.0
                    || result.post_time < 0.0
                    || result.pre_extrapolation > 4
                    || result.post_extrapolation > 4
                {
                    return Err(invalid("invalid timeline envelope"));
                }
                clips.push(result);
            }
            let markers = match row.get("markers") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::Array(rows)) => {
                    rows.iter().map(marker).collect::<Result<Vec<_>, _>>()?
                }
                Some(_) => return Err(invalid("markers is not an array")),
            };
            tracks.push(TimelineTrack {
                identity,
                class,
                name: string(track, "name")?.to_owned(),
                clips,
                markers,
                settings: row.get("fields").cloned().unwrap_or(Value::Null),
                infinite,
            });
        }
        let settings = &timeline["settings"];
        let duration = match unsigned(settings, "m_DurationMode")? {
            0 => tracks
                .iter()
                .flat_map(|t| &t.clips)
                .map(TimelineClip::end)
                .fold(0.0, f64::max),
            1 => finite(settings, "m_FixedDuration")?,
            _ => return Err(invalid("unknown timeline duration mode")),
        };
        if duration <= 0.0 {
            return Err(invalid("empty timeline"));
        }
        Ok(Arc::new(TimelineDefinition {
            package: string(&self.tracks, "package")?.to_owned(),
            prefab: string(prefab, "container")?.to_owned(),
            fixture_view,
            director: asset(&director["asset"])?,
            timeline: identity,
            duration,
            tracks,
        }))
    }

    fn payload(
        &self,
        value: &Value,
        key: &TimelineClipKey,
    ) -> Result<TimelinePayload, TimelineFailure> {
        let class = string(value, "class")?;
        let f = &value["fields"];
        Ok(match class {
            "AnimationPlayableAsset" => {
                let rows: Vec<_> = array(&self.targets, "keyedClips")?
                    .iter()
                    .filter(|v| {
                        asset(&v["track"]).ok().as_ref() == Some(&key.track)
                            && v["clipIndex"].as_u64() == Some(key.clip_index as u64)
                    })
                    .collect();
                if rows.len() != 1 {
                    return Err(invalid("missing or duplicated full-identity animation target row; legacy pathId-only rows need re-extraction"));
                }
                let target = &rows[0]["target"];
                TimelinePayload::Animation {
                    target: ClipTarget {
                        target_package: string(target, "targetPackage")?.to_owned(),
                        clip_name: string(target, "clipName")?.to_owned(),
                        path_id: path_id(&f["m_Clip"]["m_PathID"])?,
                        source_clip: if target["sourceClip"].is_null() {
                            None
                        } else {
                            Some(asset(&target["sourceClip"])?)
                        },
                        source_record: target.clone(),
                    },
                    settings: AnimationPlayableSettings {
                        position: vector3(&f["m_Position"])?,
                        euler_angles: vector3(&f["m_EulerAngles"])?,
                        rotation: [
                            finite(&f["m_Rotation"], "x")?,
                            finite(&f["m_Rotation"], "y")?,
                            finite(&f["m_Rotation"], "z")?,
                            finite(&f["m_Rotation"], "w")?,
                        ],
                        use_track_match_fields: flag(f, "m_UseTrackMatchFields")?,
                        match_target_fields: unsigned(f, "m_MatchTargetFields")?,
                        remove_start_offset: flag(f, "m_RemoveStartOffset")?,
                        apply_foot_ik: flag(f, "m_ApplyFootIK")?,
                        loop_mode: unsigned(f, "m_Loop")?,
                    },
                }
            }
            "ChangeEyePresetClip" | "ChangeLipSyncPresetClip" => {
                let index = integer(f, "SelectIndex")?;
                let list_key = if class == "ChangeEyePresetClip" {
                    "EyeDataList"
                } else {
                    "LipSyncDataList"
                };
                let rows = match f.get(list_key) {
                    Some(Value::Null) => None,
                    Some(Value::Array(rows)) => Some(rows),
                    _ => return Err(invalid(format!("missing array {list_key}"))),
                };
                // IsChangeEye/LipSyncPreset first gates on a non-empty list.
                // Empty authored clips are no-ops even with SelectIndex == 0.
                // Only a populated list reaches the source indexed getter.
                if index < 0 || rows.is_none_or(|rows| rows.is_empty()) {
                    TimelinePayload::NoPresetChange
                } else if class == "ChangeEyePresetClip" {
                    let row = rows
                        .unwrap()
                        .get(index as usize)
                        .ok_or_else(|| invalid("eye SelectIndex out of range"))?;
                    TimelinePayload::Eye {
                        pattern: string(row, "PatternName")?.into(),
                    }
                } else {
                    let row = rows
                        .unwrap()
                        .get(index as usize)
                        .ok_or_else(|| invalid("lip SelectIndex out of range"))?;
                    TimelinePayload::Lip {
                        pattern: string(row, "Name")?.into(),
                        open: integer(row, "OpenLipSyncIndex")?,
                        middle: integer(row, "MiddleLipSyncIndex")?,
                        close: integer(row, "CloseLipSyncIndex")?,
                    }
                }
            }
            "LoopFlagClip" => TimelinePayload::LoopFlag {
                looping: flag(f, "_loopFlag")?,
                enable_talk: flag(f, "_enableTalkFlag")?,
            },
            "SEClip" => TimelinePayload::Se {
                package: string(f, "_assetBundleName")?.replace('/', "__"),
                cue: string(f, "_cueName")?.into(),
            },
            "ChangeBlinkStateClip" => TimelinePayload::BlinkGate,
            "ChangeLipSyncStateClip" => TimelinePayload::LipGate,
            "EnableIKTalkClip" => TimelinePayload::NpcIkTalkGate,
            "EmoticonClip" => emoticon_payload(f)?,
            "FadeCharacterClip" => {
                TimelinePayload::FadeCharacter(Arc::new(FadeCurve::parse(&f["AnimationCurve"])?))
            }
            "ControlPlayableAsset" => TimelinePayload::Control(control_payload(f)?),
            "CinemachineShot" => TimelinePayload::CutScene(CutScenePayload::CinemachineShot {
                exposed_name: string(&f["VirtualCamera"], "exposedName")?.to_owned(),
            }),
            "FadeInClip" | "FadeOutClip" => TimelinePayload::CutScene(CutScenePayload::Fade {
                fade_in: class == "FadeInClip",
                color: color(&f["Color"])?,
            }),
            "UpdateObstacleClip" => TimelinePayload::CutScene(CutScenePayload::UpdateObstacle {
                level: integer(f, "_siteLevel")?,
            }),
            "HideSiteObstacleClip" => {
                TimelinePayload::CutScene(CutScenePayload::HideSiteObstacle {
                    level: integer(f, "_siteLevel")?,
                })
            }
            "ShowExpansionEffectClip" => {
                let center = vector3(&f["centerPosition"])?;
                TimelinePayload::CutScene(CutScenePayload::SiteExpansion(ExpansionEffect {
                    center: [center[0] as f32, center[1] as f32, center[2] as f32],
                    start_radius: finite(f, "startRadius")? as f32,
                    end_radius: finite(f, "endRadius")? as f32,
                    min_radius: finite(f, "minRadius")? as f32,
                    max_radius: finite(f, "maxRadius")? as f32,
                    gradient_range: finite(f, "gradientRange")? as f32,
                    edge_color: color(&f["edgeColor"])?,
                    fade_color: color(&f["fadeColor"])?,
                }))
            }
            "ActivationPlayableAsset" => TimelinePayload::CutScene(CutScenePayload::Activation),
            "PlayerActivationClip" => TimelinePayload::CutScene(CutScenePayload::PlayerActivation),
            "SkipNextClip" => TimelinePayload::CutScene(CutScenePayload::SkipNext),
            "EffectClip" => {
                let t = &f["template"];
                let prefab = path_id(&t["prefab"]["m_PathID"])?;
                TimelinePayload::CutScene(CutScenePayload::Effect(EffectTemplate {
                    prefab: (prefab != "0").then_some(prefab),
                    parent_mode: t["parentMode"]
                        .as_i64()
                        .ok_or_else(|| invalid("missing integer parentMode"))?,
                    character_id: t["characterID"]
                        .as_i64()
                        .ok_or_else(|| invalid("missing integer characterID"))?,
                    fixed_starting_point: flag(t, "fixedStartingPoint")?,
                    offset: vector3(&t["offset"])?,
                    matched_duration: flag(t, "isMatchedDuration")?,
                    random_seed: flag(t, "isEnabledRandomSeed")?,
                }))
            }
            _ => TimelinePayload::Unsupported {
                class: class.into(),
                fields: f.clone(),
            },
        })
    }
}

/// The director's `m_ExposedReferences` as read by the track table: exposed
/// name -> the object (None for an authored null). `Err` when the document
/// has no table for this director.
type ExposedTable = Result<Vec<(String, Result<Option<SourceAssetId>, String>)>, String>;

fn exposed_table(director: &Value) -> ExposedTable {
    let rows = director
        .get("exposedReferences")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "the director's exposed-reference table is not in the track table".to_owned()
        })?;
    rows.iter()
        .map(|row| {
            let name = row
                .get("name")
                .and_then(Value::as_str)
                .ok_or_else(|| "an exposed-reference row has no name".to_owned())?
                .to_owned();
            let value = match row.get("value") {
                None | Some(Value::Null) => Ok(None),
                Some(value) if value.get("unresolved").is_some() => Err(format!(
                    "exposed reference {name} names an object outside this package"
                )),
                Some(value) => asset(value).map(Some).map_err(|error| error.to_string()),
            };
            Ok((name, value))
        })
        .collect()
}

/// `ExposedReference<GameObject>.Resolve(director)`: a name the table holds
/// resolves to its value, null included; any other name to the default value
/// (`m_FileID` 0 is the playable asset's own file).
fn resolve_exposed(
    table: &ExposedTable,
    name: &str,
    default: &Value,
    playable: Option<&SourceAssetId>,
) -> ExposedSource {
    let rows = match table {
        Ok(rows) => rows,
        Err(reason) => return ExposedSource::Unreadable(reason.clone()),
    };
    if let Some((_, value)) = rows.iter().find(|(key, _)| key == name) {
        return match value {
            Ok(Some(object)) => ExposedSource::Object(object.clone()),
            Ok(None) => ExposedSource::Null,
            Err(reason) => ExposedSource::Unreadable(reason.clone()),
        };
    }
    let path = match path_id(&default["m_PathID"]) {
        Ok(path) => path,
        Err(_) => return ExposedSource::Unreadable("the default value is unreadable".into()),
    };
    if path == "0" {
        return ExposedSource::Null;
    }
    match (default["m_FileID"].as_i64(), playable) {
        (Some(0), Some(playable)) => ExposedSource::Object(SourceAssetId {
            file: playable.file.clone(),
            path_id: path,
        }),
        _ => ExposedSource::Unreadable("the default value names another serialized file".into()),
    }
}

fn control_payload(fields: &Value) -> Result<ControlSettings, TimelineFailure> {
    if path_id(&fields["prefabGameObject"]["m_PathID"])? != "0" {
        return Err(invalid(
            "ControlPlayableAsset prefab instantiation is not supported",
        ));
    }
    let exposed_name = string(&fields["sourceGameObject"], "exposedName")?.to_owned();
    if exposed_name.is_empty() {
        return Err(invalid(
            "ControlPlayableAsset exposed source binding is empty",
        ));
    }
    let post_playback = unsigned(fields, "postPlayback")?;
    if post_playback > 2 {
        return Err(invalid("unknown ControlPlayableAsset postPlayback state"));
    }
    Ok(ControlSettings {
        exposed_name,
        update_particle: flag(fields, "updateParticle")?,
        update_director: flag(fields, "updateDirector")?,
        update_itime_control: flag(fields, "updateITimeControl")?,
        search_hierarchy: flag(fields, "searchHierarchy")?,
        active: flag(fields, "active")?,
        post_playback,
        random_seed: u32::try_from(unsigned(fields, "particleRandomSeed")?)
            .map_err(|_| invalid("ControlPlayableAsset seed exceeds uint32"))?
            .max(1),
    })
}

fn emoticon_payload(fields: &Value) -> Result<TimelinePayload, TimelineFailure> {
    let index = integer(fields, "SelectIndex")?;
    let name = usize::try_from(index)
        .ok()
        .and_then(|index| {
            fields
                .get("EmoticonNames")
                .and_then(Value::as_array)
                .and_then(|names| names.get(index))
        })
        .and_then(Value::as_str)
        .filter(|name| !name.is_empty())
        .ok_or_else(|| invalid("emoticon SelectIndex has no authored name"))?;
    Ok(TimelinePayload::Emoticon {
        name: name.to_owned(),
        use_root: flag(fields, "UseRootTransform")?,
    })
}

#[cfg(test)]
mod emoticon_tests {
    use super::*;
    #[test]
    fn authored_emoticon_selection_and_root_mode_are_exact() {
        let fields = serde_json::json!({"EmoticonNames":["fx_emote_001","fx_emote_005_loop"],"SelectIndex":1,"UseRootTransform":0});
        assert!(
            matches!(emoticon_payload(&fields).unwrap(),TimelinePayload::Emoticon{name,use_root:false} if name=="fx_emote_005_loop")
        );
        for index in [-1, 2] {
            let mut bad = fields.clone();
            bad["SelectIndex"] = serde_json::json!(index);
            assert!(emoticon_payload(&bad).is_err());
        }
    }

    #[test]
    fn control_settings_preserve_source_features_and_reject_unknown_policy() {
        let fields = serde_json::json!({
            "sourceGameObject":{"exposedName":"source-id","defaultValue":{"m_PathID":"0"}},
            "prefabGameObject":{"m_PathID":"0"},"updateParticle":1,"updateDirector":1,
            "updateITimeControl":1,"searchHierarchy":0,"active":1,"postPlayback":2,
            "particleRandomSeed":8270,
        });
        let parsed = control_payload(&fields).unwrap();
        assert_eq!(parsed.exposed_name, "source-id");
        assert_eq!(parsed.random_seed, 8270);
        assert!(parsed.update_particle && parsed.update_director && parsed.update_itime_control);
        assert!(!parsed.search_hierarchy);
        assert!(parsed.active);
        assert_eq!(parsed.post_playback, 2);
        let mut bad = fields.clone();
        bad["postPlayback"] = serde_json::json!(3);
        assert!(control_payload(&bad).is_err());
        bad = fields.clone();
        bad["prefabGameObject"]["m_PathID"] = serde_json::json!("123");
        assert!(control_payload(&bad).is_err());
        bad = fields.clone();
        bad["updateDirector"] = Value::Null;
        assert!(control_payload(&bad).is_err());
        let mut zero_seed = fields;
        zero_seed["particleRandomSeed"] = serde_json::json!(0);
        assert_eq!(control_payload(&zero_seed).unwrap().random_seed, 1);
    }
}

fn flatten<'a>(rows: &'a [Value], output: &mut Vec<&'a Value>) -> Result<(), TimelineFailure> {
    for row in rows {
        output.push(row);
        flatten(array(row, "children")?, output)?;
    }
    Ok(())
}

pub(super) fn invalid(message: impl Into<String>) -> TimelineFailure {
    TimelineFailure {
        message: message.into(),
        retryable: false,
    }
}
pub(super) fn string<'a>(v: &'a Value, key: &str) -> Result<&'a str, TimelineFailure> {
    v.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| invalid(format!("missing string {key}")))
}
pub(super) fn array<'a>(v: &'a Value, key: &str) -> Result<&'a Vec<Value>, TimelineFailure> {
    v.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| invalid(format!("missing array {key}")))
}
pub(super) fn number(v: &Value, key: &str) -> Result<f64, TimelineFailure> {
    match &v[key] {
        Value::Number(n) => n
            .as_f64()
            .ok_or_else(|| invalid(format!("invalid number {key}"))),
        Value::String(s) if s == "Infinity" => Ok(f64::INFINITY),
        Value::String(s) if s == "-Infinity" => Ok(f64::NEG_INFINITY),
        _ => Err(invalid(format!("missing number {key}"))),
    }
}
pub(super) fn finite(v: &Value, key: &str) -> Result<f64, TimelineFailure> {
    let value = number(v, key)?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(invalid(format!("nonfinite {key}")))
    }
}
pub(super) fn unsigned(v: &Value, key: &str) -> Result<u64, TimelineFailure> {
    v[key]
        .as_u64()
        .ok_or_else(|| invalid(format!("missing unsigned {key}")))
}
fn integer(v: &Value, key: &str) -> Result<i32, TimelineFailure> {
    v[key]
        .as_i64()
        .and_then(|v| i32::try_from(v).ok())
        .ok_or_else(|| invalid(format!("missing integer {key}")))
}
pub(super) fn flag(v: &Value, key: &str) -> Result<bool, TimelineFailure> {
    match &v[key] {
        Value::Bool(b) => Ok(*b),
        Value::Number(n) if n.as_u64() == Some(0) => Ok(false),
        Value::Number(n) if n.as_u64() == Some(1) => Ok(true),
        _ => Err(invalid(format!("invalid flag {key}"))),
    }
}
fn vector3(v: &Value) -> Result<[f64; 3], TimelineFailure> {
    Ok([finite(v, "x")?, finite(v, "y")?, finite(v, "z")?])
}
fn color(v: &Value) -> Result<[f32; 4], TimelineFailure> {
    Ok([
        finite(v, "r")? as f32,
        finite(v, "g")? as f32,
        finite(v, "b")? as f32,
        finite(v, "a")? as f32,
    ])
}
fn marker(row: &Value) -> Result<TimelineMarker, TimelineFailure> {
    let class = string(row, "class")?.to_owned();
    let asset = asset(&row["asset"])?;
    if class != "SignalEmitter" {
        return Ok(TimelineMarker {
            class,
            asset,
            time: f64::NAN,
            retroactive: false,
            emit_once: false,
            signal: None,
            signal_name: None,
        });
    }
    Ok(TimelineMarker {
        class,
        asset,
        time: finite(row, "time")?,
        retroactive: flag(row, "retroactive")?,
        emit_once: flag(row, "emitOnce")?,
        signal: if row["signal"].is_null() || row["signal"].get("unresolved").is_some() {
            None
        } else {
            Some(asset_of(&row["signal"])?)
        },
        signal_name: row["signalName"].as_str().map(str::to_owned),
    })
}
fn asset_of(v: &Value) -> Result<SourceAssetId, TimelineFailure> {
    asset(v)
}
pub(super) fn path_id(v: &Value) -> Result<String, TimelineFailure> {
    if let Some(s) = v.as_str() {
        s.parse::<i64>().map_err(|_| invalid("invalid pathId"))?;
        Ok(s.into())
    } else {
        v.as_i64()
            .map(|n| n.to_string())
            .ok_or_else(|| invalid("invalid pathId"))
    }
}
pub(super) fn asset(v: &Value) -> Result<SourceAssetId, TimelineFailure> {
    Ok(SourceAssetId {
        file: string(v, "file")?.into(),
        path_id: path_id(&v["pathId"])?,
    })
}
