//! Source mouth continuations and channel-zero PCM level arithmetic.
//!
//! CN 6.0.0 NPCAvatarView.LipSync gates at cycle entry, not during each
//! awaited delay. An engine adapter owns actual voice identity, PCM callback
//! blocks and Update-phase delays. This module neither plays sound nor guesses
//! a speaker from a unit id. See the voice-mouth lane's source report.

use crate::talk::UniformDraw;

/// The three authored NPCAvatarLipSyncData fields. Missing FindBy rows are
/// zero-valued structs; only an authored middle=-1 selects the two-cell arm.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LipPattern {
    pub open: i32,
    pub middle: i32,
    pub close: i32,
}

#[derive(Clone, Copy, Debug)]
pub struct LipTimes {
    /// Integer milliseconds, lower inclusive / upper exclusive.
    pub middle: (u32, u32),
    pub open: (u32, u32),
    pub close: (u32, u32),
}

impl LipTimes {
    /// NPCAvatarView static initializer; these are not fitted to speech text.
    pub const CN_6: Self = Self { middle: (30, 50), open: (100, 300), close: (50, 100) };
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct MouthDirective {
    /// Feed this index to the existing character/fixture material writer.
    pub texture: Option<i32>,
    /// Schedule exactly one Update-phase scaled-time delay, then resume once.
    /// Do not consume a large host delta through multiple continuations.
    pub wait_millis: Option<u32>,
}

#[derive(Clone, Copy, Debug)]
enum Continuation { SingleOpen, MiddleOpen, MiddleReturn, MiddleClose, End }

#[derive(Clone, Copy, Debug)]
struct Pending {
    /// ChangeMouthTextureRandomTime captures its index before awaiting.
    texture: Option<i32>,
    then: Continuation,
}

#[derive(Debug)]
pub struct LipSync {
    pattern: LipPattern,
    enabled: bool,
    pending: Option<Pending>,
}

impl LipSync {
    pub fn new(pattern: LipPattern) -> Self {
        Self { pattern, enabled: false, pending: None }
    }

    pub fn is_playing(&self) -> bool { self.pending.is_some() }

    pub fn pattern(&self) -> LipPattern { self.pattern }

    /// SetLipSyncEnabled does not cancel a running cycle or write a texture.
    pub fn set_enabled(&mut self, enabled: bool) { self.enabled = enabled; }

    /// ChangeLipSyncPattern writes the new Close immediately. A previously
    /// awaited target index remains captured; later steps read the new row.
    pub fn change_pattern(&mut self, pattern: LipPattern) -> i32 {
        self.pattern = pattern;
        pattern.close
    }

    /// Object/controller cancellation, not ordinary voice completion.
    pub fn cancel(&mut self) { self.pending = None; }

    fn wait(&mut self, texture: Option<i32>, then: Continuation, range: (u32, u32), random: &mut impl UniformDraw) -> u32 {
        let (min, max) = range;
        assert!(max > min, "source lip delay range must be nonempty");
        let span = (max - min) as usize;
        let draw = random.draw(span);
        assert!(draw < span, "uniform draw is outside its requested interval");
        self.pending = Some(Pending { texture, then });
        min + draw as u32
    }

    /// Equivalent to UpdateMouth's attempt to start LipSync. `None` means a
    /// genuinely absent source analyzer (which bypasses the low-RMS gate),
    /// not a host that has failed to obtain samples for an attached voice.
    pub fn update_mouth(&mut self, analyzer_rms: Option<f32>, times: LipTimes, random: &mut impl UniformDraw) -> MouthDirective {
        if self.is_playing() { return MouthDirective::default(); }
        if !self.enabled || analyzer_rms.is_some_and(|rms| rms < 0.1) {
            return MouthDirective { texture: Some(self.pattern.close), wait_millis: None };
        }
        let (texture, then, range) = if self.pattern.middle == -1 {
            (self.pattern.close, Continuation::SingleOpen, times.close)
        } else {
            (self.pattern.middle, Continuation::MiddleOpen, times.middle)
        };
        let wait = self.wait(Some(self.pattern.open), then, range, random);
        MouthDirective { texture: Some(texture), wait_millis: Some(wait) }
    }

    /// Call once when the previously returned wait has actually completed.
    /// RMS/enable are intentionally not rechecked halfway through the cycle.
    pub fn resume_delay(&mut self, times: LipTimes, random: &mut impl UniformDraw) -> MouthDirective {
        let Some(pending) = self.pending.take() else { return MouthDirective::default(); };
        let wait = match pending.then {
            Continuation::SingleOpen => Some(self.wait(None, Continuation::End, times.open, random)),
            // The source middle-present arm holds Open using CloseTime,
            // not the longer OpenTime used by the middle=-1 arm.
            Continuation::MiddleOpen => Some(self.wait(Some(self.pattern.middle), Continuation::MiddleReturn, times.close, random)),
            Continuation::MiddleReturn => Some(self.wait(Some(self.pattern.close), Continuation::MiddleClose, times.middle, random)),
            Continuation::MiddleClose => Some(self.wait(None, Continuation::End, times.close, random)),
            Continuation::End => None,
        };
        MouthDirective { texture: pending.texture, wait_millis: wait }
    }
}

/// RMS accumulator belonging to an attached output analyzer, not to text.
/// The source voice factory configures rate=48000, hence 2400 frames minimum.
/// A whole PCM callback is included before checking the limit; overshoot is
/// not carried to a second window and GetRms holds the last published value.
#[derive(Debug)]
pub struct ChannelRms {
    window_frames: u32,
    frames: u32,
    sum_squares: f32,
    latest: f32,
}

impl ChannelRms {
    pub fn new(configured_rate: u32) -> Option<Self> {
        let window_frames = configured_rate / 20;
        (window_frames != 0).then_some(Self { window_frames, frames: 0, sum_squares: 0.0, latest: 0.0 })
    }

    /// Input must be one real, normalized float PCM callback's channel zero
    /// at the matching analyzer input stage. Do not replace the configured
    /// rate with a file's rate without proving the resampling position.
    /// This is not interleaved stereo,
    /// a mix of BGM and voice, or PCM sliced by the UI frame delta.
    pub fn submit_channel_zero(&mut self, samples: &[f32]) -> Result<Option<f32>, &'static str> {
        let count = u32::try_from(samples.len()).map_err(|_| "PCM callback is too large")?;
        self.frames = self.frames.checked_add(count).ok_or("RMS sample count overflow")?;
        self.sum_squares = source_sum_squares(samples) + self.sum_squares;
        if self.frames < self.window_frames { return Ok(None); }
        // Keep the source operation order: reciprocal count, multiply, sqrt.
        self.latest = ((1.0 / self.frames as f32) * self.sum_squares).sqrt();
        self.frames = 0;
        self.sum_squares = 0.0;
        Ok(Some(self.latest))
    }

    /// CriAtomExOutputAnalyzer.GetRms returns zero without attachment/handle,
    /// or when its player is neither Preparing(1) nor Playing(2). Reading a
    /// stopped player does not itself reset the analyzer's stored samples.
    pub fn get_for_player(&self, attached: bool, preparing_or_playing: bool) -> f32 {
        if attached && preparing_or_playing { self.latest } else { 0.0 }
    }
}

fn source_sum_squares(samples: &[f32]) -> f32 {
    // The ARM64 helper uses four f32 lanes on 16-aligned blocks, then a scalar
    // tail. This preserves its grouping for the same block/alignment, without
    // claiming different codecs or host callback partitions are bit-identical.
    if samples.as_ptr() as usize & 15 != 0 {
        return samples.iter().fold(0.0, |sum, value| sum + value * value);
    }
    let mut lanes = [0.0_f32; 4];
    let mut chunks = samples.chunks_exact(16);
    for chunk in &mut chunks {
        for group in chunk.chunks_exact(4) {
            for lane in 0..4 { lanes[lane] += group[lane] * group[lane]; }
        }
    }
    let sum = ((lanes[0] + lanes[1]) + lanes[2]) + lanes[3];
    chunks.remainder().iter().fold(sum, |sum, value| sum + value * value)
}
