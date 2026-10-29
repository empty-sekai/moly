//! `TweenRotation`: a looping local-Euler tween that plays while its
//! behaviour is enabled.
//!
//! Enabling the behaviour starts the tween (start timing "on enable"): the
//! node is set to the start Euler and then eased toward the end Euler by the
//! authored animation curve. Ping-pong loops reverse the eased time, restart
//! loops jump back, and a loop count below one repeats forever. Disabling the
//! behaviour kills the tween and leaves the node where it is.

use std::sync::Arc;

use bevy::prelude::*;
use serde_json::Value;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Loop {
    Once,
    Restart,
    PingPong,
}

#[derive(Clone, Copy)]
struct Key {
    time: f32,
    value: f32,
    in_slope: f32,
    out_slope: f32,
}

/// The tween's easing curve: a cubic Hermite spline over unweighted keys.
#[derive(Clone)]
struct Ease(Vec<Key>);

impl Ease {
    fn evaluate(&self, time: f32) -> f32 {
        let first = self.0[0];
        let last = self.0[self.0.len() - 1];
        if time <= first.time { return first.value; }
        if time >= last.time { return last.value; }
        let index = self.0.windows(2).position(|pair| time < pair[1].time).unwrap_or(self.0.len() - 2);
        let (a, b) = (self.0[index], self.0[index + 1]);
        let span = b.time - a.time;
        let s = (time - a.time) / span;
        let (s2, s3) = (s * s, s * s * s);
        (2.0 * s3 - 3.0 * s2 + 1.0) * a.value + (s3 - 2.0 * s2 + s) * span * a.out_slope
            + (-2.0 * s3 + 3.0 * s2) * b.value + (s3 - s2) * span * b.in_slope
    }
}

#[derive(Clone)]
pub(super) struct Spec {
    enabled: bool,
    ease: Ease,
    duration: f32,
    delay: f32,
    kind: Loop,
    loops: Option<u32>,
    from: Vec3,
    to: Vec3,
    reversed: bool,
    keep_on_disable: bool,
}

fn flag(fields: &Value, name: &str) -> Result<bool, String> {
    match fields[name].as_u64() {
        Some(0) => Ok(false),
        Some(1) => Ok(true),
        _ => Err(format!("tween field {name} is not a serialized bool")),
    }
}

fn number(value: &Value, what: &str) -> Result<f32, String> {
    value.as_f64().map(|value| value as f32).filter(|value| value.is_finite())
        .ok_or_else(|| format!("tween {what} is not a finite number"))
}

fn vector(value: &Value, what: &str) -> Result<Vec3, String> {
    Ok(Vec3::new(number(&value["x"], what)?, number(&value["y"], what)?, number(&value["z"], what)?))
}

impl Spec {
    pub(super) fn parse(fields: &Value) -> Result<Self, String> {
        let int = |name: &str| fields[name].as_i64().ok_or_else(|| format!("tween field {name} is not an integer"));
        if int("behaviour")? != 0 {
            return Err("a reference tween needs its master tween's lifecycle".into());
        }
        if int("componentType")? != 0 {
            return Err("a UI tween target needs another consumer".into());
        }
        if int("startTiming")? != 2 {
            return Err("only play-on-enable tween start timing has a consumer".into());
        }
        if flag(fields, "isReflesh")? || flag(fields, "syncGameTime")? {
            return Err("tween refresh and game-time sync have no consumer".into());
        }
        let reference = &fields["reference"]["m_PathID"];
        if reference.as_i64() != Some(0) && reference.as_str() != Some("0") {
            return Err("a tween reference has no consumer".into());
        }
        if !fields["onFinished"]["m_PersistentCalls"]["m_Calls"].as_array().is_some_and(Vec::is_empty) {
            return Err("tween completion callbacks have no consumer".into());
        }
        let duration = number(&fields["duration"], "duration")?;
        let delay = number(&fields["delay"], "delay")?;
        if duration <= 0.0 || delay < 0.0 {
            return Err("tween duration must be positive and its delay nonnegative".into());
        }
        let kind = match int("loopType")? {
            0 => Loop::Once,
            1 => Loop::Restart,
            2 => Loop::PingPong,
            _ => return Err("unknown tween loop type".into()),
        };
        let count = int("loopCount")?;
        let loops = match kind {
            Loop::Once => Some(1),
            _ if count < 1 => None,
            _ => Some(u32::try_from(count).map_err(|_| "tween loop count is out of range")?),
        };
        let reversed = match int("direction")? {
            0 => false,
            1 => true,
            _ => return Err("unknown tween direction".into()),
        };
        let curve = &fields["curve"];
        let mut keys = Vec::new();
        for key in curve["m_Curve"].as_array().ok_or("tween curve has no keys")? {
            if key["weightedMode"].as_i64() != Some(0) {
                return Err("a weighted tween curve key has no consumer".into());
            }
            keys.push(Key {
                time: number(&key["time"], "curve time")?,
                value: number(&key["value"], "curve value")?,
                in_slope: number(&key["inSlope"], "curve slope")?,
                out_slope: number(&key["outSlope"], "curve slope")?,
            });
        }
        if keys.len() < 2 || keys.windows(2).any(|pair| pair[0].time >= pair[1].time) {
            return Err("tween curve keys are not strictly increasing".into());
        }
        if keys[0].time > 0.0 || keys[keys.len() - 1].time < 1.0 {
            return Err("tween curve does not cover its normalized time".into());
        }
        Ok(Self {
            enabled: flag(fields, "m_Enabled")?,
            ease: Ease(keys),
            duration,
            delay,
            kind,
            loops,
            from: vector(&fields["from"], "from")?,
            to: vector(&fields["to"], "to")?,
            reversed,
            keep_on_disable: flag(fields, "dontKillIfDisable")?,
        })
    }

    pub(super) fn enabled(&self) -> bool {
        self.enabled
    }

    fn endpoints(&self) -> (Vec3, Vec3) {
        if self.reversed { (self.to, self.from) } else { (self.from, self.to) }
    }
}

pub(super) struct State {
    spec: Arc<Spec>,
    elapsed: f64,
    running: bool,
}

impl State {
    pub(super) fn new(spec: Arc<Spec>) -> Self {
        Self { spec, elapsed: 0.0, running: false }
    }

    /// OnEnable: kill any previous tween, set the start Euler and start over.
    pub(super) fn enable(&mut self) -> Quat {
        self.elapsed = 0.0;
        self.running = true;
        super::lane::euler_rotation(self.spec.endpoints().0)
    }

    /// OnDisable: the tween dies unless it is authored to survive.
    pub(super) fn disable(&mut self) {
        if !self.spec.keep_on_disable { self.running = false; }
    }

    /// The node rotation after `delta` more seconds, while the tween runs.
    pub(super) fn step(&mut self, delta: f64) -> Option<Quat> {
        if !self.running { return None; }
        let spec = &*self.spec;
        self.elapsed += delta;
        let duration = f64::from(spec.duration);
        let time = (self.elapsed - f64::from(spec.delay)).max(0.0);
        let mut finished = false;
        let mut loop_index = (time / duration).floor() as u64;
        let mut position = time - loop_index as f64 * duration;
        if let Some(loops) = spec.loops {
            if loop_index >= u64::from(loops) {
                finished = true;
                loop_index = u64::from(loops) - 1;
                position = duration;
            }
        }
        if spec.kind == Loop::PingPong && loop_index % 2 == 1 {
            position = duration - position;
        }
        let (start, end) = spec.endpoints();
        let eased = spec.ease.evaluate((position / duration) as f32);
        if finished { self.running = false; }
        Some(super::lane::euler_rotation(start + (end - start) * eased))
    }
}
