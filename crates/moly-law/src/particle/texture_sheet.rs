//! Render-time UVModule sampling. The source writes a normalized table
//! position, then ParticleGeomAnimateUVs expands it into the selected rectangle.
//! Constant and two-constant frame curves do not advance with particle age;
//! [`CurveTimeSheet`] is the Lifetime kernel of a frame curve.
use super::{random::ParticleRandom, schema::TextureSheetParams, Curve, MinMaxCurve};
use super::curve::{arm_fmax, CurveSampler, CurveTime, EngineCurve};

#[derive(Clone, Copy, Debug)]
enum Sample {
    Constant(f32),
    TwoConstants { min: f32, max: f32 },
}
impl Sample {
    fn prepare(curve: &MinMaxCurve) -> Result<Self, String> {
        match *curve {
            MinMaxCurve::Constant(value) => Ok(Self::Constant(value)),
            MinMaxCurve::TwoConstants { min, max } => Ok(Self::TwoConstants { min, max }),
            _ => Err("texture-sheet frame curve: TextureSheet takes constant frames; the curve-time kernel is CurveTimeSheet".into()),
        }
    }
    fn evaluate(self, random: f32) -> f32 {
        match self {
            Self::Constant(value) => value,
            Self::TwoConstants { min, max } => min + random * (max - min),
        }
    }
}

/// Bottom-left offset and size in source UV coordinates. The source's first
/// sheet row is the top row. Applying this to an authored UV preserves its V
/// convention; no texture orientation conversion belongs in this operation.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvRect {
    pub offset: [f32; 2],
    pub scale: [f32; 2],
}
impl Default for UvRect {
    fn default() -> Self { Self { offset: [0.0; 2], scale: [1.0; 2] } }
}
impl UvRect {
    pub fn apply(self, uv: [f32; 2]) -> [f32; 2] {
        [self.offset[0] + uv[0] * self.scale[0], self.offset[1] + uv[1] * self.scale[1]]
    }
}

#[derive(Clone, Copy, Debug)]
enum Row { WholeSheet, Custom(u32), Random }

#[derive(Clone, Copy, Debug)]
pub struct TextureSheet {
    frame: Sample,
    start: Sample,
    tiles: [u32; 2],
    row: Row,
    uv0: bool,
}
impl TextureSheet {
    pub fn from_params(p: &TextureSheetParams) -> Result<Self, String> {
        if p.mode != 0 || p.time_mode != 0 {
            return Err("texture-sheet Sprites, Speed and FPS consumers are not implemented".into());
        }
        if p.tiles.iter().any(|n| *n == 0 || *n > 511) {
            return Err("texture-sheet tile count is outside the source geometry range".into());
        }
        if p.flip != [0.0; 2] {
            return Err("texture-sheet UV flip consumer is not implemented".into());
        }
        let row = match (p.animation_type, p.row_mode) {
            (0, _) => Row::WholeSheet,
            (1, 0) => Row::Custom(p.row_index.clamp(0, p.tiles[1] as i32 - 1) as u32),
            (1, 1) => Row::Random,
            _ => return Err("texture-sheet animation or mesh-index row consumer is not implemented".into()),
        };
        Ok(Self { frame: Sample::prepare(&p.frame)?, start: Sample::prepare(&p.start)?,
            tiles: p.tiles, row, uv0: p.uv_channel_mask & 1 != 0 })
    }

    pub fn position(self, seed: u32) -> f32 {
        let frame = self.frame.evaluate(ParticleRandom::sample(seed, 0x1374_0583));
        let start = self.start.evaluate(ParticleRandom::sample(seed, 0x56b3_dbb0));
        let value = start + frame;
        let fraction = value - value.floor();
        // Preserve the native multiply/divide order and the rounded row span.
        let span = (1.0 / (self.tiles[0] as f32 * self.tiles[1] as f32)) * self.tiles[0] as f32;
        let row = match self.row {
            Row::WholeSheet => return fraction,
            // The source converts the custom row through normalized space and
            // floors it again. Cancelling these factors changes non-power-of-two grids.
            Row::Custom(row) => (self.tiles[1] as f32 * (span * row as f32)).floor(),
            Row::Random => (ParticleRandom::sample(seed, 0xaf50_2044) * self.tiles[1] as f32)
                .floor().min((self.tiles[1] - 1) as f32),
        };
        let lower = span * row;
        let width = (span + lower) - lower;
        lower + width * fraction
    }

    pub fn rect(self, seed: u32) -> UvRect {
        expand(self.position(seed), self.tiles, self.uv0)
    }
}

/// ParticleGeomAnimateUVs: the table position times the tile count, split
/// into a row (from the top) and a column of the grid.
fn expand(position: f32, tiles: [u32; 2], uv0: bool) -> UvRect {
    if !uv0 { return UvRect::default(); }
    let frame = position * (tiles[0] * tiles[1]) as f32;
    let dx = 1.0 / tiles[0] as f32;
    let dy = 1.0 / tiles[1] as f32;
    let row = (dx * frame).floor();
    let column = frame.floor() - tiles[0] as f32 * row;
    UvRect { offset: [column * dx, (1.0 - dy) - row * dy], scale: [dx, dy] }
}

/// The source's floor: truncation toward zero (saturating, NaN to 0), minus
/// one where the truncation lies above the value; the fraction is the value
/// minus that floor.
fn fraction(x: f32) -> f32 {
    let truncated = (x as i32) as f32;
    x - (truncated - if truncated > x { 1.0 } else { 0.0 })
}

#[derive(Clone, Debug)]
enum FrameCurve {
    Curve { max: EngineCurve, multiplier: f32 },
    TwoCurves { min: EngineCurve, max: EngineCurve, multiplier: f32 },
}

/// Grid mode, Lifetime time mode, whole-sheet animation, a frame-over-time
/// curve the asset reader leaves unoptimized (curve or two curves) and a
/// constant start frame: the one curve-time kernel with users.
///
/// Per particle, from its age percent and seed:
/// - `t = fraction(cycles * fmax(agePercent * 0.01, +0))`, the maximum
///   keeping a NaN age NaN;
/// - `frame = curveMax.Evaluate(t) * multiplier`; for two curves the minimum
///   likewise, and `frame = min + r * (max - min)` with the draw `r` of the
///   frame stream (the constant-frame kernels' salt);
/// - `position = fraction(start + frame)`, expanded into the grid like the
///   constant kernels.
///
/// The evaluation reads each curve object's own cache; [`EngineCurve::new`]
/// on the normalized clock admits only lanes whose value that history cannot
/// change, so each time is evaluated as from a reset cache.
///
/// Refused by name (no data row uses them, and their kernels are separate
/// bodies not transcribed here): Sprites mode, the Speed and FPS time modes,
/// UV flip, single-row animation with a curve, a start frame other than a
/// constant, and a frame curve the reader optimizes into its polynomial form;
/// and a cycle count that is not positive and finite (see `from_params`).
#[derive(Clone, Debug)]
pub struct CurveTimeSheet {
    frame: FrameCurve,
    start: f32,
    cycles: f32,
    tiles: [u32; 2],
    uv0: bool,
}

impl CurveTimeSheet {
    pub fn from_params(p: &TextureSheetParams) -> Result<Self, String> {
        if p.mode != 0 {
            return Err("texture-sheet Sprites mode is not implemented".into());
        }
        if p.time_mode != 0 {
            return Err("texture-sheet Speed and FPS time modes are not implemented".into());
        }
        if p.tiles.iter().any(|n| *n == 0 || *n > 511) {
            return Err("texture-sheet tile count is outside the source geometry range".into());
        }
        if p.flip != [0.0; 2] {
            return Err("texture-sheet UV flip consumer is not implemented".into());
        }
        if p.animation_type != 0 {
            return Err("texture-sheet single-row animation with a frame curve is not implemented".into());
        }
        // A positive finite cycle count keeps the curve time on the normalized
        // clock. With a negative one the time can be -0 or -inf, and the pre-wrap
        // window an infinite time writes into the curve object's cache turns the
        // next evaluations at -0 into NaN in the source: the cache history then
        // decides the frame, which a reset-cache evaluation does not follow.
        if !(p.cycles > 0.0 && p.cycles.is_finite()) {
            return Err("texture-sheet cycles not positive and finite with a frame curve: the curve cache history can decide the frame".into());
        }
        let &MinMaxCurve::Constant(start) = &p.start else {
            return Err("texture-sheet start frame other than a constant with a frame curve is not implemented".into());
        };
        if CurveSampler::engine_optimized(&p.frame) {
            return Err("texture-sheet frame curve in the reader's optimized polynomial form is not implemented".into());
        }
        let lane = |curve: &Curve| EngineCurve::new(curve, CurveTime::Normalized)
            .map_err(|reason| format!("texture-sheet frame curve: {reason}"));
        let frame = match &p.frame {
            MinMaxCurve::Curve { multiplier, max } => FrameCurve::Curve { max: lane(max)?, multiplier: *multiplier },
            MinMaxCurve::TwoCurves { multiplier, min, max } =>
                FrameCurve::TwoCurves { min: lane(min)?, max: lane(max)?, multiplier: *multiplier },
            _ => return Err("texture-sheet frame curve expected; constant frames take TextureSheet".into()),
        };
        Ok(Self { frame, start, cycles: p.cycles, tiles: p.tiles, uv0: p.uv_channel_mask & 1 != 0 })
    }

    /// The normalized table position of one particle.
    pub fn position(&self, seed: u32, age_percent: f32) -> f32 {
        let t = fraction(self.cycles * arm_fmax(age_percent * f32::from_bits(0x3c23_d70a), 0.0));
        let frame = match &self.frame {
            FrameCurve::Curve { max, multiplier } => max.evaluate(t) * *multiplier,
            FrameCurve::TwoCurves { min, max, multiplier } => {
                let high = max.evaluate(t) * *multiplier;
                let low = min.evaluate(t) * *multiplier;
                let random = ParticleRandom::sample(seed, 0x1374_0583);
                low + random * (high - low)
            }
        };
        fraction(self.start + frame)
    }

    pub fn rect(&self, seed: u32, age_percent: f32) -> UvRect {
        expand(self.position(seed, age_percent), self.tiles, self.uv0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn word(v: &Value) -> u32 { v.as_f64().unwrap() as u32 }

    fn params(sheet: &Value) -> TextureSheetParams {
        let number = |key: &str| sheet.get(key).unwrap().as_f64().unwrap() as f32;
        TextureSheetParams { mode: 0, time_mode: 0, animation_type: 0,
            tiles: [number("tilesX") as u32, number("tilesY") as u32], row_mode: 1, row_index: 0,
            cycles: number("cycles"), fps: 30.0, speed_range: [0.0, 1.0], uv_channel_mask: -1, flip: [0.0; 2],
            frame: crate::particle::schema::min_max_curve(sheet.get("frameOverTime"), "frameOverTime").unwrap(),
            start: crate::particle::schema::min_max_curve(sheet.get("startFrame"), "startFrame").unwrap() }
    }

    /// A deliberate misreading of the kernel, for the mutant arms.
    #[derive(Clone, Copy, Debug, PartialEq)]
    enum Mutant { None, NoCycleFraction, NanAgeToZero, CyclesIgnored, MultiplierDropped, NoFinalFraction,
        StartStreamDraw, SidesSwapped }

    fn mutant_position(sheet: &CurveTimeSheet, seed: u32, age: f32, mutant: Mutant) -> f32 {
        let scaled = age * f32::from_bits(0x3c23_d70a);
        let base = if mutant == Mutant::NanAgeToZero { scaled.max(0.0) } else { arm_fmax(scaled, 0.0) };
        let t = match mutant {
            Mutant::NoCycleFraction => sheet.cycles * base,
            Mutant::CyclesIgnored => fraction(base),
            _ => fraction(sheet.cycles * base),
        };
        let scale = |m: f32| if mutant == Mutant::MultiplierDropped { 1.0 } else { m };
        let frame = match &sheet.frame {
            FrameCurve::Curve { max, multiplier } => max.evaluate(t) * scale(*multiplier),
            FrameCurve::TwoCurves { min, max, multiplier } => {
                let (mut high, mut low) = (max.evaluate(t) * scale(*multiplier), min.evaluate(t) * scale(*multiplier));
                if mutant == Mutant::SidesSwapped { std::mem::swap(&mut high, &mut low); }
                let salt = if mutant == Mutant::StartStreamDraw { 0x56b3_dbb0 } else { 0x1374_0583 };
                let random = ParticleRandom::sample(seed, salt);
                low + random * (high - low)
            }
        };
        if mutant == Mutant::NoFinalFraction { sheet.start + frame } else { fraction(sheet.start + frame) }
    }

    // Native rows of the Lifetime curve-time kernel, executed through the UV
    // dispatcher with each case's module and curve objects kept across batches
    // (their caches carried as in the engine). Every admitted row must match
    // bit for bit (a NaN matches a NaN); every mutant arm must turn rows red.
    #[test]
    #[ignore = "MOLY_UV_CURVE_TIME_NATIVE must identify the native curve-time kernel rows"]
    fn curve_time_kernel_matches_native() {
        let path = std::env::var_os("MOLY_UV_CURVE_TIME_NATIVE").expect("MOLY_UV_CURVE_TIME_NATIVE");
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("summary").unwrap().get("sourceSha256").unwrap().as_str(),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let arms = [Mutant::NoCycleFraction, Mutant::NanAgeToZero, Mutant::CyclesIgnored, Mutant::MultiplierDropped,
            Mutant::NoFinalFraction, Mutant::StartStreamDraw, Mutant::SidesSwapped];
        let (mut cases, mut refused, mut optimized, mut rows, mut nan_rows, mut corpus_rows) = (0, 0, 0, 0, 0, 0);
        let mut red = vec![0usize; arms.len()];
        let mut first_refusals = Vec::new();
        for case in receipt.get("cases").unwrap().as_array().unwrap() {
            cases += 1;
            let name = case.get("name").unwrap().as_str().unwrap();
            let p = params(case.get("sheet").unwrap());
            let native_optimized = case.get("optimized").unwrap().as_bool().unwrap();
            assert_eq!(CurveSampler::engine_optimized(&p.frame), native_optimized, "{name}: optimized bit");
            let sheet = match CurveTimeSheet::from_params(&p) {
                Ok(sheet) => sheet,
                Err(reason) => {
                    assert!(!name.starts_with("corpus"), "{name} refused: {reason}");
                    optimized += usize::from(native_optimized);
                    refused += 1;
                    if first_refusals.len() < 6 { first_refusals.push(format!("{name}: {reason}")); }
                    continue;
                }
            };
            for row in case.get("rows").unwrap().as_array().unwrap() {
                let row: Vec<u32> = row.as_array().unwrap().iter().map(word).collect();
                let (seed, age, native) = (row[0], f32::from_bits(row[1]), row[2]);
                let actual = sheet.position(seed, age);
                let same = |v: f32| (v.is_nan() && f32::from_bits(native).is_nan()) || v.to_bits() == native;
                assert!(same(actual), "{name} seed {seed:#x} age {:#x}: {:#x} vs native {native:#x}", row[1], actual.to_bits());
                assert!(same(mutant_position(&sheet, seed, age, Mutant::None)));
                for (arm, count) in arms.iter().zip(red.iter_mut()) {
                    *count += usize::from(!same(mutant_position(&sheet, seed, age, *arm)));
                }
                rows += 1;
                nan_rows += usize::from(actual.is_nan());
                if name.starts_with("corpus") { corpus_rows += 1; }
            }
        }
        let tally: Vec<String> = arms.iter().zip(&red).map(|(arm, n)| format!("{arm:?} {n}")).collect();
        println!("curve-time kernel: cases {cases}, refused {refused} (optimized {optimized}), rows {rows} \
            (corpus {corpus_rows}, NaN {nan_rows}), mismatches 0; mutants red: {}; first refusals: {first_refusals:?}",
            tally.join(", "));
        assert!(corpus_rows > 0 && rows > corpus_rows);
        assert!(red.iter().all(|n| *n > 0), "a mutant arm stayed green: {tally:?}");
    }
}
