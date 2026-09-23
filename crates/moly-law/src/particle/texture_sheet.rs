//! Render-time UVModule sampling. The source writes a normalized table
//! position, then ParticleGeomAnimateUVs expands it into the selected rectangle.
//! Constant and two-constant frame curves do not advance with particle age.
use super::{random::ParticleRandom, schema::TextureSheetParams, MinMaxCurve};

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
            _ => Err("texture-sheet curve-time sampling is not implemented".into()),
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
        if !self.uv0 { return UvRect::default(); }
        let frame = self.position(seed) * (self.tiles[0] * self.tiles[1]) as f32;
        let dx = 1.0 / self.tiles[0] as f32;
        let dy = 1.0 / self.tiles[1] as f32;
        let row = (dx * frame).floor();
        let column = frame.floor() - self.tiles[0] as f32 * row;
        UvRect { offset: [column * dx, (1.0 - dy) - row * dy], scale: [dx, dy] }
    }
}
