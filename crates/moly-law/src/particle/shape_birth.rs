//! Current JP 6.8.1 four-lane Shape.Start -> EmitterStoreData boundary.
//! Current libunity SHA 937c6d28...75badd9; entries 0xefcd6c, 0xf01d50,
//! 0xf06dac, 0xf08e90. Qualified authored shapes: current snow Hemisphere and
//! rain Circle, radius 50/full thickness/random 360-degree arc, Euler (-90,0,0),
//! source scales (1,1,.16)/(1,1,1), randomPosition 0/2, no direction perturbation.
//! Initial and Shape RNG are independent. A nonempty birth group consumes all
//! four lanes including padding; capacity, old-prefix storage, StartVelocity,
//! lifetime modules, event ownership and renderer admission remain caller work.
//! Coordinates here are native source coordinates; reflect only at the adapter.
//! Source authored scale/rotation is distinct from the explicit outer owner.
//! This boundary assumes emitter state scale=(1,1,1); no later renormalization
//! follows the outer owner's nonuniform direction multiplication.
use super::schema::{ShapeMode, ShapeParams};
use super::seed_owner::ModuleRandom;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    UnsupportedSourceShape,
    NonfiniteOwner,
    NonfiniteOutput,
}

#[derive(Clone, Copy, Debug)]
pub struct ShapeSample {
    pub position: [f32; 3],
    pub direction: [f32; 3],
}

#[derive(Clone, Copy, Debug)]
pub struct ShapeBirthGroup {
    /// All four padded native lanes, not only the accepted logical particles.
    pub samples: [ShapeSample; 4],
    pub raw_position: [[f32; 3]; 4],
    pub raw_direction: [[f32; 3]; 4],
    pub before_rng: ModuleRandom,
    pub before_store: ModuleRandom,
    pub after_rng: ModuleRandom,
    pub source_affine: [f32; 16],
}

#[derive(Clone, Copy, Debug)]
pub struct ShapeBirthLaw {
    hemisphere: bool,
    radius: f32,
    thickness: f32,
    arc: f32,
    random_position: f32,
    affine: [f32; 16],
}

impl ShapeBirthLaw {
    pub fn from_params(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        let hemisphere = match params.shape_type.as_str() {
            "Hemisphere" => true,
            "Circle" => false,
            _ => return Err(Refused::UnsupportedSourceShape),
        };
        let scale = if hemisphere {
            [1.0, 1.0, 0.16]
        } else {
            [1.0; 3]
        };
        let random_position = if hemisphere { 0.0 } else { 2.0 };
        if params.radius != 50.0
            || params.radius_thickness != 1.0
            || params.arc != 360.0
            || params.rotation != [-90.0, 0.0, 0.0]
            || params.position != [0.0; 3]
            || c.source_version != Some(1)
            || c.scale != Some(scale)
            || c.arc_mode != Some(ShapeMode::Random)
            || c.radius_mode != Some(ShapeMode::Random)
            || c.arc_spread != Some(0.0)
            || c.radius_spread != Some(0.0)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
            || c.random_position != Some(random_position)
        {
            return Err(Refused::UnsupportedSourceShape);
        }
        Ok(Self {
            hemisphere,
            radius: params.radius,
            thickness: params.radius_thickness,
            arc: params.arc,
            random_position,
            affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Explicit owner snapshot, never a guessed source path/scene transform.
    /// Local space uses identity; World uses the supplied column-major affine.
    /// Current receipt tests identity and a rotated nonuniform World owner.
    /// This models a zero local Initial position; owner translation is added
    /// after outer rotation, just as Initial's already stored birth position.
    pub fn sample_group(
        &self,
        random: &mut ModuleRandom,
        outer_owner: [f32; 16],
        world_space: bool,
    ) -> Result<ShapeBirthGroup, Refused> {
        if world_space && outer_owner.iter().any(|x| !x.is_finite()) {
            return Err(Refused::NonfiniteOwner);
        }
        let owner = if world_space { outer_owner } else { IDENTITY };
        let before_rng = *random;
        let mut next = *random;
        let first = next.next4_u32().map(super::shape::u01_from_bits);
        let second = next.next4_u32().map(super::shape::u01_from_bits);
        let third = if self.hemisphere {
            next.next4_u32().map(super::shape::u01_from_bits)
        } else {
            [0.0; 4]
        };
        let raw: [([f32; 3], [f32; 3]); 4] = std::array::from_fn(|i| {
            if self.hemisphere {
                super::shape::hemisphere_position(
                    self.radius,
                    self.thickness,
                    self.arc,
                    first[i],
                    second[i],
                    third[i],
                )
            } else {
                super::shape::circle_base(
                    self.radius,
                    self.thickness,
                    self.arc,
                    first[i],
                    second[i],
                )
            }
        });
        let before_store = next;
        let (arc, polar) = if self.random_position > 0.0 {
            (
                next.next4_u32().map(super::shape::u01_from_bits),
                next.next4_u32().map(super::shape::u01_from_bits),
            )
        } else {
            ([0.0; 4], [0.0; 4])
        };
        let samples = std::array::from_fn(|i| {
            let position =
                super::shape::randomize_position(raw[i].0, self.random_position, arc[i], polar[i]);
            let position = vector(&owner, point(&self.affine, position));
            let position = std::array::from_fn(|axis| position[axis] + owner[12 + axis]);
            // Normalize before source affine, then again before the final
            // owner multiply. Normalizing after owner destroys scale fidelity.
            let direction = vector(&owner, normalize(vector(&self.affine, normalize(raw[i].1))));
            ShapeSample {
                position,
                direction,
            }
        });
        if samples.iter().any(|s| {
            s.position
                .iter()
                .chain(s.direction.iter())
                .any(|v| !v.is_finite())
        }) {
            return Err(Refused::NonfiniteOutput);
        }
        *random = next;
        Ok(ShapeBirthGroup {
            samples,
            raw_position: raw.map(|v| v.0),
            raw_direction: raw.map(|v| v.1),
            before_rng,
            before_store,
            after_rng: next,
            source_affine: self.affine,
        })
    }
}

pub const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

const fn rsqrt_estimates() -> [u16; 256] {
    let mut table = [0_u16; 256];
    let mut i = 0;
    while i < 256 {
        let midpoint = (257_u64 + 2 * (i % 128) as u64) << (i / 128);
        let mut estimate = 256_u64;
        while midpoint * (2 * estimate + 1) * (2 * estimate + 1) < (1_u64 << 28) {
            estimate += 1;
        }
        table[i] = estimate as u16;
        i += 1;
    }
    table
}
const RSQRT_ESTIMATE: [u16; 256] = rsqrt_estimates();

// Current ARM FRSQRTE normalized positive domain. Integer midpoint/table
// quantization, not a host reciprocal sqrt. Native refinement FRSQRTS uses
// fused rounding of (3-a*b)/2 after the preceding separate f32 FMUL.
fn rsqrt_estimate(value: f32) -> f32 {
    assert!(value.is_normal() && value > 0.0);
    let bits = value.to_bits();
    let exponent = ((bits >> 23) & 255) as i32;
    let index = ((bits & 0x7fffff) >> 16) as usize + if exponent & 1 == 0 { 128 } else { 0 };
    let estimate = RSQRT_ESTIMATE[index] as u32;
    let estimate_bits = ((((380 - exponent) / 2) as u32) << 23) | (((estimate as u32) - 256) << 15);
    f32::from_bits(estimate_bits)
}
fn rsqrt(value: f32) -> f32 {
    let r0 = rsqrt_estimate(value);
    let step = |a: f32, b: f32| ((3.0_f64 - (a as f64) * (b as f64)) * 0.5) as f32;
    let r1 = r0 * step(r0 * value, r0);
    r1 * step(value * r1, r1)
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let square = v[0] * v[0] + (v[1] * v[1] + v[2] * v[2]);
    if square <= f32::from_bits(0x0da24260) {
        return [0.0, 0.0, 1.0];
    }
    let r = rsqrt(square);
    v.map(|x| x * r)
}
fn vector(matrix: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    std::array::from_fn(|a| matrix[a] * v[0] + (matrix[4 + a] * v[1] + matrix[8 + a] * v[2]))
}
fn point(matrix: &[f32; 16], v: [f32; 3]) -> [f32; 3] {
    // StoreData translation is added to Z term before Y and X.
    std::array::from_fn(|a| {
        matrix[a] * v[0] + (matrix[4 + a] * v[1] + (matrix[8 + a] * v[2] + matrix[12 + a]))
    })
}
fn source_affine(
    rotation: [f32; 3],
    scale: [f32; 3],
    position: [f32; 3],
    emitter_scale: [f32; 3],
) -> [f32; 16] {
    let trig =
        rotation.map(|v| super::shape::engine_sincos((v * f32::from_bits(0x3c8efa35)) * 0.5));
    let [sx, sy, sz] = trig.map(|t| t.0);
    let [cx, cy, cz] = trig.map(|t| t.1);
    // Shape.Start 0xefcf10..0xefcf60: ZXY quaternion with actual sign vectors.
    let b = [cz * sx, sx * sz, cx * sz, cx * cz];
    let shift = [b[2], b[3], b[0], b[1]];
    let sign_a = [1.0, -1.0, 1.0, 1.0];
    let sign_b = [1.0, 1.0, -1.0, 1.0];
    let q: [f32; 4] =
        std::array::from_fn(|i| sign_a[i] * (b[i] * cy) + (sign_b[i] * sy) * shift[i]);
    let [x, y, z, _w] = q;
    let rev = [q[1], q[0], q[3], q[2]];
    let ext = [q[2], q[3], q[0], q[1]];
    let rev_ext = [q[3], q[2], q[1], q[0]];
    let col0: [f32; 4] = std::array::from_fn(|i| {
        (rev[i] * ([-2.0, 2.0, -2.0, 0.0][i] * y) + ext[i] * ([-2.0, 2.0, 2.0, 0.0][i] * z))
            + [1.0, 0.0, 0.0, 0.0][i]
    });
    let col1: [f32; 4] = std::array::from_fn(|i| {
        (rev_ext[i] * ([-2.0, -2.0, 2.0, 0.0][i] * z) + rev[i] * ([2.0, -2.0, 2.0, 0.0][i] * x))
            + [0.0, 1.0, 0.0, 0.0][i]
    });
    let col2: [f32; 4] = std::array::from_fn(|i| {
        (ext[i] * ([2.0, -2.0, -2.0, 0.0][i] * x) + rev_ext[i] * ([2.0, 2.0, -2.0, 0.0][i] * y))
            + [0.0, 0.0, 1.0, 0.0][i]
    });
    let axes = [
        [emitter_scale[0], 0.0, 0.0, 0.0],
        [0.0, emitter_scale[1], 0.0, 0.0],
        [0.0, 0.0, emitter_scale[2], 0.0],
    ];
    let mut out = [0.0; 16];
    for (col, values) in [col0, col1, col2].iter().enumerate() {
        let scaled = values.map(|v| v * scale[col]);
        for i in 0..4 {
            out[col * 4 + i] =
                axes[0][i] * scaled[0] + (axes[1][i] * scaled[1] + axes[2][i] * scaled[2]);
        }
    }
    for i in 0..4 {
        out[12 + i] = (axes[0][i] * position[0]
            + (axes[1][i] * position[1] + axes[2][i] * position[2]))
            + 0.0;
    }
    out
}

/// Diagnostic text fixture emitted directly from shape-birth-current.json by
/// verify-shape-geometry-exact.py. Expected native channels never feed the law.
#[cfg(any(test, moly_shape_replay))]
pub fn replay_native_rows(text: &str) -> usize {
    use super::schema::ShapeControls;
    let mut groups = 0;
    for (case, line) in text.lines().enumerate() {
        if let Some(row) = line.strip_prefix("R ") {
            let words: Vec<u32> = row.split_whitespace().map(|v| v.parse().unwrap()).collect();
            assert_eq!(words.len(), 3);
            let value = f32::from_bits(words[0]);
            assert_eq!(
                rsqrt_estimate(value).to_bits(),
                words[1],
                "native FRSQRTE {value}"
            );
            assert_eq!(
                rsqrt(value).to_bits(),
                words[2],
                "native FRSQRTE/FRSQRTS {value}"
            );
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 158);
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        assert_eq!(floats(86), [1.0; 3], "unqualified emitter state scale");
        let source = ShapeParams {
            shape_type: match v[0] {
                2 => "Hemisphere",
                10 => "Circle",
                _ => panic!("shape kind"),
            }
            .into(),
            radius: f32::from_bits(v[1]),
            radius_thickness: f32::from_bits(v[2]),
            arc: f32::from_bits(v[3]),
            rotation: floats(77),
            position: floats(83),
            controls: ShapeControls {
                source_version: Some(1),
                scale: Some(floats(80)),
                arc_mode: Some(ShapeMode::Random),
                radius_mode: Some(ShapeMode::Random),
                arc_spread: Some(0.0),
                radius_spread: Some(0.0),
                align_to_direction: Some(false),
                random_direction: Some(0.0),
                spherical_direction: Some(0.0),
                random_position: Some(f32::from_bits(v[4])),
                ..Default::default()
            },
        };
        let law = ShapeBirthLaw::from_params(&source).unwrap();
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let mut random = words(5);
        let group = law.sample_group(&mut random, owner, v[89] == 1).unwrap();
        assert_eq!(group.before_rng, words(5));
        assert_eq!(
            group.before_store,
            words(45),
            "case {case} before Store RNG"
        );
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        assert_eq!(random, group.after_rng);
        for i in 0..16 {
            assert_eq!(
                group.source_affine[i].to_bits(),
                v[106 + i],
                "case {case} source affine {i}"
            );
        }
        let outer = if v[89] == 1 { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(
                outer[i].to_bits(),
                v[122 + i],
                "case {case} outer rotation {i}"
            );
        }
        for (field, values) in [group.raw_position, group.raw_direction].iter().enumerate() {
            for lane in 0..4 {
                for axis in 0..3 {
                    assert_eq!(
                        values[lane][axis].to_bits(),
                        v[21 + field * 12 + axis * 4 + lane],
                        "case {case} raw {field}/{lane}/{axis}"
                    );
                }
            }
        }
        for lane in 0..4 {
            for (field, values) in [group.samples[lane].position, group.samples[lane].direction]
                .iter()
                .enumerate()
            {
                for axis in 0..3 {
                    assert_eq!(
                        values[axis].to_bits(),
                        v[134 + field * 12 + axis * 4 + lane],
                        "case {case} store {field}/{lane}/{axis}"
                    );
                }
            }
        }
        groups += 1;
    }
    groups
}

#[cfg(test)]
mod tests {
    use super::super::schema::ShapeControls;
    use super::*;

    #[test]
    fn overflowing_finite_world_owner_refuses_without_consuming_shape_rng() {
        let source = ShapeParams {
            shape_type: "Hemisphere".into(),
            radius: 50.0,
            radius_thickness: 1.0,
            arc: 360.0,
            rotation: [-90.0, 0.0, 0.0],
            position: [0.0; 3],
            controls: ShapeControls {
                source_version: Some(1),
                scale: Some([1.0, 1.0, 0.16]),
                arc_mode: Some(ShapeMode::Random),
                radius_mode: Some(ShapeMode::Random),
                arc_spread: Some(0.0),
                radius_spread: Some(0.0),
                align_to_direction: Some(false),
                random_direction: Some(0.0),
                spherical_direction: Some(0.0),
                random_position: Some(0.0),
                ..Default::default()
            },
        };
        let law = ShapeBirthLaw::from_params(&source).unwrap();
        let mut random = ModuleRandom::from_owner_seed(1729);
        let before = random;
        let mut owner = IDENTITY;
        owner[0] = f32::MAX;
        owner[5] = f32::MAX;
        owner[10] = f32::MAX;
        assert_eq!(
            law.sample_group(&mut random, owner, true).unwrap_err(),
            Refused::NonfiniteOutput
        );
        assert_eq!(random, before);
    }
    #[test]
    #[ignore = "set MOLY_SHAPE_BIRTH_NATIVE_ROWS to native rows exported by verify-shape-geometry-exact.py"]
    fn current_source_shape_store_all_padded_lanes_bit_exact() {
        let path =
            std::env::var_os("MOLY_SHAPE_BIRTH_NATIVE_ROWS").expect("current native row file");
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(super::replay_native_rows(&text), 92);
    }
}
