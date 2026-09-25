//! When a footstep falls: the player avatar's `PublishFootSE` animation
//! events, carried onto the SD locomotion clip the product plays.
//!
//! In the source the events live on the player avatar's own site clips: the
//! Move state plays the walk clip and the Dash state the run clip, each with
//! two `PublishFootSE` events. The product's player body is an SD character
//! whose walk and run clips have no events (the SD motion library carries
//! none), so each event is placed at the same fraction of the SD clip that
//! is playing: event time divided by the avatar clip's length, read at run
//! time from the avatar motion manifest (never typed in). That placement is
//! a product adaptation, named as such.

use serde_json::Value;

/// The avatar walk clip the Move state plays.
pub(crate) const WALK_CLIP: &str = "mov_u000_site_walk001_O";
/// The avatar run clip the Dash state plays.
pub(crate) const RUN_CLIP: &str = "mov_u000_site_run001_O";
const EVENT: &str = "PublishFootSE";

/// The avatar motion manifest the phases are read from.
pub(crate) const MANIFEST: &str =
    "moly://avatar/motion/mysekai__player_avatar.motion-manifest.json";

/// Event phases, as fractions of a clip cycle in `(0, 1]`, in clip order.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct FootPhases {
    pub walk: Vec<f32>,
    pub run: Vec<f32>,
}

impl FootPhases {
    pub(crate) fn parse(manifest: &Value) -> Result<Self, String> {
        let clips = manifest["clips"]
            .as_array()
            .ok_or("avatar motion manifest has no clips list")?;
        let phases = |name: &str| -> Result<Vec<f32>, String> {
            let clip = clips
                .iter()
                .find(|clip| clip["name"] == name)
                .ok_or_else(|| format!("avatar motion manifest has no clip {name}"))?;
            let duration = clip["durationSeconds"]
                .as_f64()
                .filter(|d| d.is_finite() && *d > 0.0)
                .ok_or_else(|| format!("clip {name} has no positive durationSeconds"))?;
            let mut out = Vec::new();
            for event in clip["events"]
                .as_array()
                .ok_or_else(|| format!("clip {name} has no events list"))?
            {
                if event["functionName"] != EVENT {
                    continue;
                }
                let time = event["time"]
                    .as_f64()
                    .filter(|t| t.is_finite())
                    .ok_or_else(|| format!("clip {name} has a {EVENT} event without a time"))?;
                let fraction = (time / duration) as f32;
                if !(fraction > 0.0 && fraction <= 1.0) {
                    // An event at 0 would need the clip's start edge, which the
                    // crossing test below does not model; none exists today.
                    return Err(format!(
                        "clip {name} {EVENT} at {time}s of {duration}s lies outside (0, 1] of the cycle"
                    ));
                }
                out.push(fraction);
            }
            if out.is_empty() {
                return Err(format!("clip {name} has no {EVENT} event"));
            }
            Ok(out)
        };
        Ok(Self {
            walk: phases(WALK_CLIP)?,
            run: phases(RUN_CLIP)?,
        })
    }
}

/// The events crossed when a clip's unwrapped position (completed cycles
/// plus the fraction of the current one) moves from `previous` (exclusive)
/// to `current` (inclusive): `(event index, fraction)` in time order.
pub(crate) fn crossed(phases: &[f32], previous: f32, current: f32) -> Vec<(usize, f32)> {
    let mut out = Vec::new();
    if !(current > previous) || !previous.is_finite() || !current.is_finite() {
        return out;
    }
    let first = previous.floor() as i64;
    let last = current.floor() as i64;
    for cycle in first..=last {
        for (index, fraction) in phases.iter().enumerate() {
            let at = cycle as f32 + fraction;
            if previous < at && at <= current {
                out.push((index, *fraction));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Reads the release avatar motion manifest (path in
    /// `MOLY_FOOT_MANIFEST`) and checks the phase table against the event
    /// times and clip lengths it carries.
    #[test]
    #[ignore = "needs the release avatar motion manifest (MOLY_FOOT_MANIFEST)"]
    fn phases_come_from_the_manifest() {
        let path = std::env::var("MOLY_FOOT_MANIFEST").expect("MOLY_FOOT_MANIFEST");
        let text = std::fs::read_to_string(&path).expect("manifest readable");
        let value: Value = serde_json::from_str(&text).expect("manifest json");
        let phases = FootPhases::parse(&value).expect("phases");
        // Reference: the manifest's own numbers, divided in f64 and rounded
        // once to f32 (walk 0.15/0.8333..., 0.5667/0.8333...; run 0.1/0.5,
        // 0.3667/0.5).
        let expect = |clip: &str| -> Vec<f32> {
            let clip = value["clips"]
                .as_array()
                .unwrap()
                .iter()
                .find(|c| c["name"] == clip)
                .unwrap();
            let duration = clip["durationSeconds"].as_f64().unwrap();
            clip["events"]
                .as_array()
                .unwrap()
                .iter()
                .filter(|e| e["functionName"] == EVENT)
                .map(|e| (e["time"].as_f64().unwrap() / duration) as f32)
                .collect()
        };
        assert_eq!(phases.walk, expect(WALK_CLIP));
        assert_eq!(phases.run, expect(RUN_CLIP));
        eprintln!("walk {:?} run {:?}", phases.walk, phases.run);
        assert_eq!(phases.walk.len(), 2);
        assert_eq!(phases.run.len(), 2);
        assert!((phases.walk[0] - 0.18).abs() < 1e-6 && (phases.walk[1] - 0.68).abs() < 1e-6);
        assert!((phases.run[0] - 0.2).abs() < 1e-6 && (phases.run[1] - 0.733_333_4).abs() < 1e-6);
    }
}
