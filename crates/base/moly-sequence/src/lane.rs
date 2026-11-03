//! A lane: the resolved clips of one track and their weight over time.

use crate::doc::Extend;
use crate::ease::Ease;
use crate::Error;

/// A clip on the time axis, with its content.
#[derive(Clone, Debug)]
pub(crate) struct Span<P> {
    pub start: f32,
    pub end: f32,
    pub shape: Shape,
    /// The clip is present from here (before `start` when it holds before).
    pub from: f32,
    /// The clip is present until here (after `end` when it extends after).
    pub until: f32,
    pub looped: bool,
    pub payload: P,
}

/// The timing and blending numbers of a clip.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shape {
    pub clip_in: f32,
    pub speed: f32,
    pub blend_in: f32,
    pub blend_out: f32,
    pub ease_in: Ease,
    pub ease_out: Ease,
    pub before: Option<Extend>,
    pub after: Option<Extend>,
}

impl Shape {
    /// The numbers as a document writes them.
    pub(crate) fn written(&self) -> crate::doc::Shape {
        crate::doc::Shape {
            clip_in: self.clip_in,
            speed: self.speed,
            blend_in: self.blend_in,
            blend_out: self.blend_out,
            ease_in: self.ease_in,
            ease_out: self.ease_out,
            before: self.before,
            after: self.after,
        }
    }
}

/// A clip's state at one time, as a clip reports it at a time and the
/// sampler reads it.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Presence {
    /// 0..=1 from the blend in and blend out; 1 where the clip extends.
    pub weight: f32,
    /// Content seconds: `clip_in` at the start, advancing at `speed`.
    pub local: f32,
    /// Share of the span, 0 at the start and 1 at the end.
    pub progress: f32,
    /// Seconds since the start, not clamped.
    pub elapsed: f32,
}

impl<P> Span<P> {
    fn length(&self) -> f32 {
        self.end - self.start
    }

    pub(crate) fn presence(&self, t: f32) -> Option<Presence> {
        if t < self.from || t > self.until {
            return None;
        }
        let length = self.length();
        let shape = &self.shape;
        let elapsed = t - self.start;
        let (weight, inside) = if t < self.start {
            (1.0, 0.0)
        } else if t > self.end {
            let inside = if self.looped { elapsed.rem_euclid(length) } else { length };
            (1.0, inside)
        } else {
            let fade_in = if shape.blend_in > 0.0 { shape.ease_in.apply(elapsed / shape.blend_in) } else { 1.0 };
            let fade_out = if shape.blend_out > 0.0 { shape.ease_out.apply((self.end - t) / shape.blend_out) } else { 1.0 };
            (fade_in * fade_out, elapsed)
        };
        Some(Presence { weight, local: shape.clip_in + inside * shape.speed, progress: inside / length, elapsed })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Lane<P> {
    /// Ordered by start.
    pub spans: Vec<Span<P>>,
}

impl<P> Default for Lane<P> {
    fn default() -> Self {
        Self { spans: Vec::new() }
    }
}

/// A resolved clip waiting to join a lane, with the document path used in
/// refusals.
pub(crate) struct Entry<P> {
    pub path: String,
    pub start: f32,
    pub end: f32,
    pub shape: Shape,
    pub payload: P,
}

impl<P> Lane<P> {
    /// Orders the clips and works out how far each extends. `after` gives
    /// the lane's extension for a clip that does not write one.
    pub fn new(mut entries: Vec<Entry<P>>, after: fn(&Shape) -> Extend) -> Result<Lane<P>, Error> {
        entries.sort_by(|a, b| a.start.total_cmp(&b.start));
        for entry in &entries {
            if entry.end - entry.start < 1e-6 {
                return Err(Error::new(&entry.path, "the clip has no length; give it a duration or an end"));
            }
            if entry.shape.before == Some(Extend::Loop) {
                return Err(Error::new(&entry.path, "\"before\" is \"none\" or \"hold\"; a clip loops only after its end"));
            }
            if entry.shape.before == Some(Extend::Hold) && entry.shape.blend_in > 0.0 {
                return Err(Error::new(&entry.path, "a clip that holds before its start cannot also blend in"));
            }
            if matches!(entry.shape.after, Some(Extend::Hold | Extend::Loop)) && entry.shape.blend_out > 0.0 {
                return Err(Error::new(&entry.path, "a clip that extends after its end cannot also blend out"));
            }
        }
        let afters: Vec<Extend> = entries.iter().map(|e| e.shape.after.unwrap_or_else(|| after(&e.shape))).collect();
        let count = entries.len();
        let mut bounds: Vec<(f32, f32)> = Vec::with_capacity(count);
        for index in 0..count {
            let entry = &entries[index];
            // An extension after the end lasts until the next clip is fully in.
            let until = match afters[index] {
                Extend::None => entry.end,
                Extend::Hold | Extend::Loop => match entries.get(index + 1) {
                    Some(next) => entry.end.max(next.start + next.shape.blend_in),
                    None => f32::INFINITY,
                },
            };
            // A hold before the start reaches back to where the previous clip
            // stops being present.
            let from = match entry.shape.before {
                Some(Extend::Hold) => match index.checked_sub(1) {
                    Some(previous) => bounds[previous].1.min(entry.start),
                    None => f32::NEG_INFINITY,
                },
                _ => entry.start,
            };
            bounds.push((from, until));
        }
        let mut spans = Vec::with_capacity(count);
        for ((entry, extend), (from, until)) in entries.into_iter().zip(afters).zip(bounds) {
            spans.push(Span {
                start: entry.start,
                end: entry.end,
                shape: entry.shape,
                from,
                until,
                looped: extend == Extend::Loop,
                payload: entry.payload,
            });
        }
        Ok(Lane { spans })
    }

    /// The clips present at `t`, in start order.
    pub fn present(&self, t: f32) -> impl Iterator<Item = (&Span<P>, Presence)> {
        self.spans.iter().filter_map(move |span| span.presence(t).map(|p| (span, p)))
    }

    /// Clips present at `t` with the weight each keeps after the later
    /// clips are laid over it: a later clip of weight `w` leaves `1 - w` of
    /// everything beneath it.
    pub fn layered(&self, t: f32) -> Vec<(&Span<P>, Presence, f32)> {
        let present: Vec<_> = self.present(t).collect();
        let mut out = Vec::with_capacity(present.len());
        let mut cover = 1.0f32;
        for (span, presence) in present.into_iter().rev() {
            let share = presence.weight * cover;
            cover *= 1.0 - presence.weight;
            if share > 1e-5 {
                out.push((span, presence, share));
            }
            if cover <= 1e-5 {
                break;
            }
        }
        out.reverse();
        out
    }
}

/// The lane extension for a clip that writes none: absent after its end.
pub(crate) fn absent_after(_: &Shape) -> Extend {
    Extend::None
}

/// The camera keeps its last shot unless that shot blends out.
pub(crate) fn camera_after(shape: &Shape) -> Extend {
    if shape.blend_out > 0.0 { Extend::None } else { Extend::Hold }
}
