//! Authored environment mixers recovered from current JP 6.8.1.
//!
//! Clip selection is first-in-source-order with inclusive endpoints. Mixer
//! time is director.time, not accumulated particle time. Weighted curves reuse
//! the native-derived Curve evaluator; there is no weather-name dispatch.
//! ValueNoiseMixer's external call was resolved through ARM64 .plt/.rela.plt
//! to the dynamic symbol `sinf` in the current player, not inferred visually.
use crate::particle::value::{Curve, Gradient};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target { SkyColor, LightColor, SkyIntensity, LightIntensity, Noise }

#[derive(Debug, Clone)]
pub enum ClipValue {
    Color(Gradient),
    Value { curve: Curve, scale: f32 },
    Noise { frequency: f32, intensity: f32, curve: Curve },
}

#[derive(Debug, Clone)]
pub struct Clip { pub start: f64, pub duration: f64, pub value: ClipValue }
impl Clip {
    fn normalized(&self, time: f64) -> f32 {
        ((time - self.start) / self.duration).clamp(0.0, 1.0) as f32
    }
}

#[derive(Debug, Clone)]
pub struct Track {
    pub target: Target,
    pub muted: bool,
    pub scale: f32,
    pub noise_track: Option<usize>,
    pub clips: Vec<Clip>,
}
impl Track {
    fn active_clip(&self, time: f64) -> Option<&Clip> {
        self.clips.iter().find(|clip| clip.start <= time && time <= clip.start + clip.duration)
    }
}

#[derive(Debug, Clone)]
pub struct Timeline { pub duration: f64, pub tracks: Vec<Track> }

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Values {
    pub sky_color: [f32; 4],
    pub light_color: [f32; 4],
    pub sky_intensity: f32,
    pub light_intensity: f32,
}

impl Timeline {
    /// The caller owns Play/Stop/wrap/clock semantics; this function is an
    /// allocation-free evaluation of a validated authored graph at one time.
    pub fn evaluate(&self, time: f64) -> Result<Values, &'static str> {
        if !time.is_finite() { return Err("non-finite environment timeline time"); }
        let mut values = Values::default();
        for track in &self.tracks {
            if track.muted || track.target == Target::Noise { continue; }
            match track.target {
                Target::SkyColor => values.sky_color = [0.0; 4],
                Target::LightColor => values.light_color = [0.0; 4],
                Target::SkyIntensity => values.sky_intensity = 0.0,
                Target::LightIntensity => values.light_intensity = 0.0,
                Target::Noise => {}
            }
            let Some(clip) = track.active_clip(time) else { continue; };
            match (&clip.value, track.target) {
                (ClipValue::Color(gradient), Target::SkyColor) => values.sky_color = gradient.evaluate(clip.normalized(time)),
                (ClipValue::Color(gradient), Target::LightColor) => values.light_color = gradient.evaluate(clip.normalized(time)),
                (ClipValue::Value { curve, scale }, Target::SkyIntensity | Target::LightIntensity) => {
                    let mut value = curve.evaluate(clip.normalized(time)) * scale * track.scale;
                    if let Some(noise_index) = track.noise_track {
                        let noise = self.tracks.get(noise_index).ok_or("unresolved noise parent edge")?;
                        // GetSubTrack enumerates serialized children, independent
                        // of whether the child generated its own playable.
                        if let Some(noise_clip) = noise.active_clip(time) {
                            let ClipValue::Noise { frequency, intensity, curve } = &noise_clip.value else {
                                return Err("noise track carries a non-noise clip");
                            };
                            let local_seconds = (time - noise_clip.start) as f32;
                            let phase = frequency * local_seconds * std::f32::consts::PI * 2.0;
                            let noise_value = phase.sin() * 0.5 + 0.5;
                            let weight = (intensity * curve.evaluate(noise_clip.normalized(time))).clamp(0.0, 1.0);
                            value += (value * noise_value - value) * weight;
                        }
                    }
                    if track.target == Target::SkyIntensity { values.sky_intensity = value; }
                    else { values.light_intensity = value; }
                }
                _ => return Err("incompatible authored environment clip and target"),
            }
        }
        Ok(values)
    }
}

/// EnvironmentShaderView.BlendAdditiveColor: accumulate RGB in linear space,
/// preserve the source blend alpha. Intensity is clamped only at zero.
pub fn additive_light(mut base: [f32; 4], color: [f32; 4], intensity: f32) -> [f32; 4] {
    use super::sky::{gamma_to_linear, linear_to_gamma};
    for i in 0..3 {
        base[i] = linear_to_gamma(gamma_to_linear(base[i])
            + intensity.max(0.0) * gamma_to_linear(color[i]));
    }
    base
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::value::CurveKey;
    fn constant(value: f32) -> Curve {
        Curve { multiplier: 1.0, keys: vec![CurveKey { time: 0.0, value, in_slope: 0.0, out_slope: 0.0,
            in_weight: 0.0, out_weight: 0.0, weighted_mode: 0 }] }
    }
    fn value_track(value: f32) -> Track {
        Track { target: Target::SkyIntensity, muted: false, scale: 2.0, noise_track: None,
            clips: vec![Clip { start: 0.0, duration: 1.0, value: ClipValue::Value { curve: constant(value), scale: 0.5 } }] }
    }
    #[test]
    fn first_inclusive_clip_wins_at_shared_boundary() {
        let mut track = value_track(2.0);
        track.clips.push(Clip { start: 1.0, duration: 1.0, value: ClipValue::Value { curve: constant(9.0), scale: 1.0 } });
        let timeline = Timeline { duration: 2.0, tracks: vec![track] };
        assert_eq!(timeline.evaluate(1.0).unwrap().sky_intensity, 2.0);
        assert_eq!(timeline.evaluate(1.00001).unwrap().sky_intensity, 18.0);
    }
    #[test]
    fn outside_clips_and_muted_parent_reset_all_contributions() {
        let mut timeline = Timeline { duration: 2.0, tracks: vec![value_track(3.0)] };
        assert_eq!(timeline.evaluate(1.5).unwrap(), Values::default());
        timeline.tracks[0].muted = true;
        assert_eq!(timeline.evaluate(0.5).unwrap(), Values::default());
        assert!(timeline.evaluate(f64::NAN).is_err());
    }
    #[test]
    fn noise_uses_local_seconds_and_the_structural_parent() {
        let mut first = value_track(2.0); first.noise_track = Some(2);
        let mut second = value_track(7.0); second.target = Target::LightIntensity;
        let noise = Track { target: Target::Noise, muted: true, scale: 1.0, noise_track: None,
            clips: vec![Clip { start: 0.0, duration: 1.0, value: ClipValue::Noise {
                frequency: 1.0, intensity: 1.0, curve: constant(1.0) } }] };
        let timeline = Timeline { duration: 2.0, tracks: vec![first, second, noise] };
        assert_eq!(timeline.evaluate(0.0).unwrap().sky_intensity, 1.0);
        assert!((timeline.evaluate(0.25).unwrap().sky_intensity - 2.0).abs() < 1e-6);
        assert!(timeline.evaluate(0.75).unwrap().sky_intensity.abs() < 1e-6);
        assert_eq!(timeline.evaluate(0.75).unwrap().light_intensity, 7.0);
    }
    #[test]
    fn direct_seek_has_no_history_or_accumulated_flash() {
        let timeline = Timeline { duration: 2.0, tracks: vec![value_track(3.0)] };
        let first = timeline.evaluate(0.3).unwrap();
        let _ = timeline.evaluate(0.8).unwrap();
        let _ = timeline.evaluate(1.8).unwrap();
        assert_eq!(timeline.evaluate(0.3).unwrap(), first);
    }
}
