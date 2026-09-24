//! Current JP 6.8.1 four-lane ShapeModule::Start -> EmitterStoreData boundary.
//! Transcribed from the current JP 6.8.1 libunity.
//! Hemisphere: the Random arc mode of StartHemiSphere over the envelope the
//! native receipts execute: any finite radius, shape rotation, scale and
//! position, arc of zero or more degrees, arc spread of zero or more, position
//! jitter of zero or more, thickness exactly zero or one. ConeVolume: the
//! Random arc mode of StartConeVolume over any finite radius, thickness, cone
//! angle and length, arc, arc spread, shape rotation, scale, position and
//! position jitter. SingleSidedEdge: the Random radius mode of
//! StartSingleSidedEdge (one draw, the radius-spread quantization included)
//! over any finite radius and radius spread. Circle: the Random arc mode of
//! StartCircle, plain and stepped arc, over any finite radius, thickness, arc
//! and arc spread. Donut: the Random arc mode of
//! StartDonut, plain and stepped arc, over any finite radius, torus radius,
//! thickness, arc and arc spread. These three over any finite shape rotation,
//! scale, position and position jitter.
//! Initial and Shape RNG are independent. A nonempty birth group consumes all
//! four lanes including padding; capacity, old-prefix storage, StartVelocity,
//! lifetime modules, event ownership and renderer admission remain caller work.
//! Coordinates here are native source coordinates; reflect only at the adapter.
//! Source authored scale/rotation is distinct from the explicit outer owner and
//! from the emitter-state scale the caller supplies with each group.
//! No later renormalization follows the outer owner's direction multiplication.
use super::schema::{ShapeMode, ShapeParams, ShapeTexture};
use super::seed_owner::ModuleRandom;
use super::shape::{native_rsqrt, Shell};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    UnsupportedSourceShape,
    /// The export cannot say what the kernels read. This is an export
    /// check: ShapeModule has no such member.
    ExportSchema(ExportGap),
    NonfiniteOwner,
    NonfiniteOutput,
    /// ShapeModule references a texture. The samplers read so far then call
    /// ApplyTexture for each birth group, which is not transcribed.
    ShapeTexture,
}

/// What an export lacks for the Shape law.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ExportGap {
    /// The shape controls block is missing or of another schema version.
    ControlsVersion,
    /// The shape block has no texture field (an export older than that
    /// field), so whether ShapeModule references a texture is undecided.
    Texture,
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
    /// EmitterStoreData's axis-of-rotation channel, all four lanes; present
    /// only when the particle arrays carry that channel. It draws no RNG.
    pub axis_of_rotation: Option<[[f32; 3]; 4]>,
}

#[derive(Clone, Copy, Debug)]
enum Kernel {
    Hemisphere { shell: Shell, arc_spread: f32 },
    Circle { thickness: f32, arc_spread: f32 },
    ConeVolume { thickness: f32, angle: f32, length: f32, arc_spread: f32 },
    SingleSidedEdge { spread: f32 },
    Donut { thickness: f32, donut_radius: f32, arc_spread: f32 },
}

#[derive(Clone, Copy, Debug)]
pub struct ShapeBirthLaw {
    kernel: Kernel,
    radius: f32,
    arc: f32,
    random_position: f32,
    rotation: [f32; 3],
    scale: [f32; 3],
    position: [f32; 3],
    /// Source affine for a unit emitter-state scale.
    unit_affine: [f32; 16],
}

impl ShapeBirthLaw {
    /// Only the null texture reference is admitted, for every kernel: a
    /// referenced texture is refused, and so is an export without the
    /// texture field, since it cannot say which of the two ShapeModule
    /// holds. The product admission names that missing input as well,
    /// before any birth path is chosen.
    pub fn from_params(params: &ShapeParams) -> Result<Self, Refused> {
        match params.controls.texture {
            None => return Err(Refused::ExportSchema(ExportGap::Texture)),
            Some(ShapeTexture::Reference { .. }) => return Err(Refused::ShapeTexture),
            Some(ShapeTexture::None) => {}
        }
        match params.shape_type.as_str() {
            "Hemisphere" => Self::hemisphere(params),
            "ConeVolume" => Self::cone_volume(params),
            "Circle" => Self::circle(params),
            "SingleSidedEdge" => Self::single_sided_edge(params),
            "Donut" => Self::donut(params),
            _ => Err(Refused::UnsupportedSourceShape),
        }
    }

    /// Gates name only what the Random-arc hemisphere kernel and the base or
    /// position-jitter Store path read. Radius mode, spread and speed, arc
    /// speed, cone angle and length, torus radius and box thickness are never
    /// loaded by that kernel, so they do not gate it. The Loop, PingPong and
    /// BurstSpread arc modes dispatch to other kernels, and the random or
    /// spherical direction and align-to-direction Store branches were not
    /// executed; they stay refused.
    fn hemisphere(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        if c.source_version != Some(1) {
            return Err(Refused::ExportSchema(ExportGap::ControlsVersion));
        }
        let finite = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
        let refused = Err(Refused::UnsupportedSourceShape);
        let Some(scale) = c.scale.filter(finite) else {
            return refused;
        };
        let Some(shell) = Shell::from_thickness(params.radius_thickness) else {
            return refused;
        };
        let Some(arc_spread) = c.arc_spread.filter(|s| s.is_finite() && *s >= 0.0) else {
            return refused;
        };
        let Some(random_position) = c.random_position.filter(|r| r.is_finite() && *r >= 0.0)
        else {
            return refused;
        };
        if !params.radius.is_finite()
            || !(params.arc.is_finite() && params.arc >= 0.0)
            || !finite(&params.rotation)
            || !finite(&params.position)
            || c.arc_mode != Some(ShapeMode::Random)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
        {
            return refused;
        }
        Ok(Self {
            kernel: Kernel::Hemisphere { shell, arc_spread },
            radius: params.radius,
            arc: params.arc,
            random_position,
            rotation: params.rotation,
            scale,
            position: params.position,
            unit_affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Gates name only what the Random-arc cone-volume kernel and the base or
    /// position-jitter Store path read, and each of those may be any finite
    /// value: both are plain f32 arithmetic whose overflow or invalid results
    /// the native code writes as they fall, which the output refusal reports.
    /// A position jitter of zero or less draws nothing, as natively. The
    /// kernel consumes only the first lane of the wide loads that also cover
    /// the radius mode and the word after the arc spread, and never loads the
    /// radius spread, torus radius or box thickness, so none of those, nor
    /// the radius or arc speed, gate it. The Loop, PingPong and BurstSpread
    /// arc modes dispatch to other kernels, and the random or spherical
    /// direction and align-to-direction Store branches were not executed; they
    /// stay refused.
    fn cone_volume(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        if c.source_version != Some(1) {
            return Err(Refused::ExportSchema(ExportGap::ControlsVersion));
        }
        let finite = |v: &f32| v.is_finite();
        let finite3 = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
        let refused = Err(Refused::UnsupportedSourceShape);
        let (Some(scale), Some(angle), Some(length), Some(arc_spread), Some(random_position)) = (
            c.scale.filter(finite3),
            c.angle.filter(finite),
            c.length.filter(finite),
            c.arc_spread.filter(finite),
            c.random_position.filter(finite),
        ) else {
            return refused;
        };
        if !params.radius.is_finite()
            || !params.radius_thickness.is_finite()
            || !params.arc.is_finite()
            || !finite3(&params.rotation)
            || !finite3(&params.position)
            || c.arc_mode != Some(ShapeMode::Random)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
        {
            return refused;
        }
        Ok(Self {
            kernel: Kernel::ConeVolume {
                thickness: params.radius_thickness,
                angle,
                length,
                arc_spread,
            },
            radius: params.radius,
            arc: params.arc,
            random_position,
            rotation: params.rotation,
            scale,
            position: params.position,
            unit_affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Gates name only what the Random-arc circle kernel and the base or
    /// position-jitter Store path read, and each of those may be any finite
    /// value; overflow or invalid results fall as the native code writes
    /// them, and the output refusal reports them. The kernel reads the arc,
    /// thickness, and only the first lane of the wide loads that also cover
    /// the radius mode and the word after the arc spread; it never loads the
    /// radius spread, cone angle or length, torus radius or box thickness, so
    /// none of those, nor the radius or arc speed, gate it. Both arc paths are
    /// the native ones: a positive arc in radians times the arc spread takes
    /// the stepped `random_arc`, anything else the continuous one. The Loop,
    /// PingPong and BurstSpread arc modes (other kernels) and the random or
    /// spherical direction and align-to-direction Store branches stay
    /// refused.
    fn circle(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        if c.source_version != Some(1) {
            return Err(Refused::ExportSchema(ExportGap::ControlsVersion));
        }
        let finite3 = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
        let refused = Err(Refused::UnsupportedSourceShape);
        let (Some(scale), Some(arc_spread), Some(random_position)) = (
            c.scale.filter(finite3),
            c.arc_spread.filter(|v| v.is_finite()),
            c.random_position.filter(|v| v.is_finite()),
        ) else {
            return refused;
        };
        if !params.radius.is_finite()
            || !params.radius_thickness.is_finite()
            || !params.arc.is_finite()
            || !finite3(&params.rotation)
            || !finite3(&params.position)
            || c.arc_mode != Some(ShapeMode::Random)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
        {
            return refused;
        }
        Ok(Self {
            kernel: Kernel::Circle {
                thickness: params.radius_thickness,
                arc_spread,
            },
            radius: params.radius,
            arc: params.arc,
            random_position,
            rotation: params.rotation,
            scale,
            position: params.position,
            unit_affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Gates name only what the Random-radius single-sided edge kernel and the
    /// base or position-jitter Store path read, each any finite value, with
    /// overflow or invalid results reported by the output refusal. The kernel
    /// reads the radius and the radius spread and nothing else of the shape:
    /// thickness, arc, arc mode, arc spread and speed, radius speed, cone
    /// angle and length, torus radius and box thickness do not gate it. The
    /// Loop, PingPong and BurstSpread radius modes dispatch to other kernels,
    /// and the random or spherical direction and align-to-direction Store
    /// branches were not executed; they stay refused.
    fn single_sided_edge(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        if c.source_version != Some(1) {
            return Err(Refused::ExportSchema(ExportGap::ControlsVersion));
        }
        let finite3 = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
        let refused = Err(Refused::UnsupportedSourceShape);
        let (Some(scale), Some(spread), Some(random_position)) = (
            c.scale.filter(finite3),
            c.radius_spread.filter(|v| v.is_finite()),
            c.random_position.filter(|v| v.is_finite()),
        ) else {
            return refused;
        };
        if !params.radius.is_finite()
            || !finite3(&params.rotation)
            || !finite3(&params.position)
            || c.radius_mode != Some(ShapeMode::Random)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
        {
            return refused;
        }
        Ok(Self {
            kernel: Kernel::SingleSidedEdge { spread },
            radius: params.radius,
            arc: params.arc,
            random_position,
            rotation: params.rotation,
            scale,
            position: params.position,
            unit_affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Gates name only what the Random-arc torus kernel and the base or
    /// position-jitter Store path read, and each of those may be any finite
    /// value: the kernel is plain f32 arithmetic whose overflow or invalid
    /// results the native code writes as they fall, which the output refusal
    /// reports. Both arc paths are the native ones: a positive arc in radians
    /// times the arc spread takes the stepped `random_arc`, anything else the
    /// continuous one. The kernel consumes only the first lane of the wide
    /// loads that also cover the radius mode, the word after the torus radius
    /// and the word after the arc spread, and never loads the radius spread,
    /// so none of those, nor the radius or arc speed, cone angle and length
    /// or box thickness, gate it. The Loop, PingPong and BurstSpread arc modes
    /// dispatch to other kernels, and the random or spherical direction and
    /// align-to-direction Store branches were not executed; they stay
    /// refused.
    fn donut(params: &ShapeParams) -> Result<Self, Refused> {
        let c = &params.controls;
        if c.source_version != Some(1) {
            return Err(Refused::ExportSchema(ExportGap::ControlsVersion));
        }
        let finite = |v: &f32| v.is_finite();
        let finite3 = |v: &[f32; 3]| v.iter().all(|x| x.is_finite());
        let refused = Err(Refused::UnsupportedSourceShape);
        let (Some(scale), Some(donut_radius), Some(arc_spread), Some(random_position)) = (
            c.scale.filter(finite3),
            c.donut_radius.filter(finite),
            c.arc_spread.filter(finite),
            c.random_position.filter(finite),
        ) else {
            return refused;
        };
        if !params.radius.is_finite()
            || !params.radius_thickness.is_finite()
            || !params.arc.is_finite()
            || !finite3(&params.rotation)
            || !finite3(&params.position)
            || c.arc_mode != Some(ShapeMode::Random)
            || c.align_to_direction != Some(false)
            || c.random_direction != Some(0.0)
            || c.spherical_direction != Some(0.0)
        {
            return refused;
        }
        Ok(Self {
            kernel: Kernel::Donut {
                thickness: params.radius_thickness,
                donut_radius,
                arc_spread,
            },
            radius: params.radius,
            arc: params.arc,
            random_position,
            rotation: params.rotation,
            scale,
            position: params.position,
            unit_affine: source_affine(params.rotation, scale, params.position, [1.0; 3]),
        })
    }

    /// Explicit owner snapshot, never a guessed source path/scene transform.
    /// Local space uses identity; World uses the supplied column-major affine
    /// (its fourth row is not read). This models a zero local Initial position;
    /// owner translation is added after outer rotation, just as Initial's
    /// already stored birth position. `emitter_scale` is the emitter-state
    /// scale ShapeModule::Start folds into the source affine;
    /// `uses_axis_of_rotation` says the particle arrays carry the
    /// axis-of-rotation channel, which EmitterStoreData then writes.
    /// A non-finite stored position or direction refuses the group. The
    /// axis channel feeds neither and is returned as written, finite or not:
    /// it does not refuse the group, and a consumer of that channel gates
    /// its finiteness itself.
    pub fn sample_group(
        &self,
        random: &mut ModuleRandom,
        outer_owner: [f32; 16],
        world_space: bool,
        emitter_scale: [f32; 3],
        uses_axis_of_rotation: bool,
    ) -> Result<ShapeBirthGroup, Refused> {
        let group = self.evaluate_group(
            *random,
            outer_owner,
            world_space,
            emitter_scale,
            uses_axis_of_rotation,
        )?;
        if group.has_nonfinite_output() {
            return Err(Refused::NonfiniteOutput);
        }
        *random = group.after_rng;
        Ok(group)
    }

    /// One group as the native boundary computes it, before the output
    /// refusal; the stream is advanced on a copy only.
    fn evaluate_group(
        &self,
        random: ModuleRandom,
        outer_owner: [f32; 16],
        world_space: bool,
        emitter_scale: [f32; 3],
        uses_axis_of_rotation: bool,
    ) -> Result<ShapeBirthGroup, Refused> {
        if world_space && outer_owner.iter().any(|x| !x.is_finite()) {
            return Err(Refused::NonfiniteOwner);
        }
        if emitter_scale.iter().any(|x| !x.is_finite()) {
            return Err(Refused::NonfiniteOwner);
        }
        let owner = if world_space { outer_owner } else { IDENTITY };
        let affine = if emitter_scale == [1.0; 3] {
            self.unit_affine
        } else {
            source_affine(self.rotation, self.scale, self.position, emitter_scale)
        };
        let before_rng = random;
        let mut next = random;
        // Draws per group: Hemisphere, ConeVolume and Donut 3, Circle 2 (arc,
        // then radial fraction), SingleSidedEdge 1.
        let first = next.next4_u32().map(super::shape::u01_from_bits);
        let second = match self.kernel {
            Kernel::SingleSidedEdge { .. } => [0.0; 4],
            _ => next.next4_u32().map(super::shape::u01_from_bits),
        };
        let third = match self.kernel {
            Kernel::Hemisphere { .. } | Kernel::ConeVolume { .. } | Kernel::Donut { .. } => {
                next.next4_u32().map(super::shape::u01_from_bits)
            }
            Kernel::Circle { .. } | Kernel::SingleSidedEdge { .. } => [0.0; 4],
        };
        let raw: [([f32; 3], [f32; 3]); 4] = std::array::from_fn(|i| match self.kernel {
            Kernel::Hemisphere { shell, arc_spread } => super::shape::hemisphere_native(
                self.radius,
                shell,
                self.arc,
                arc_spread,
                first[i],
                second[i],
                third[i],
            ),
            // Draws: arc, radial fraction.
            Kernel::Circle {
                thickness,
                arc_spread,
            } => super::shape::circle_at(
                self.radius,
                thickness,
                super::shape::random_arc(self.arc, arc_spread, first[i]),
                second[i],
            ),
            // Draws: arc, radial fraction, travelled distance.
            Kernel::ConeVolume {
                thickness,
                angle,
                length,
                arc_spread,
            } => super::shape::cone_volume_at(
                self.radius,
                thickness,
                angle,
                super::shape::random_arc(self.arc, arc_spread, first[i]),
                length,
                second[i],
                third[i],
            ),
            Kernel::SingleSidedEdge { spread } => {
                super::shape::single_sided_edge_spread(self.radius, spread, first[i])
            }
            // Draws: major arc, tube angle, tube radius.
            Kernel::Donut {
                thickness,
                donut_radius,
                arc_spread,
            } => super::shape::donut_at(
                self.radius,
                donut_radius,
                thickness,
                super::shape::random_arc(self.arc, arc_spread, first[i]),
                second[i],
                third[i],
            ),
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
        let mut axis = [[0.0; 3]; 4];
        let samples = std::array::from_fn(|i| {
            let position =
                super::shape::randomize_position(raw[i].0, self.random_position, arc[i], polar[i]);
            let rotated = vector(&owner, point(&affine, position));
            let position = std::array::from_fn(|a| rotated[a] + owner[12 + a]);
            // Normalize before source affine, then again before the final
            // owner multiply. Normalizing after owner destroys scale fidelity.
            let affine_direction = vector(&affine, normalize(raw[i].1));
            let direction = vector(&owner, normalize(affine_direction));
            if uses_axis_of_rotation {
                axis[i] = axis_of_rotation(masked(affine_direction), rotated);
            }
            ShapeSample {
                position,
                direction,
            }
        });
        let axis_of_rotation = uses_axis_of_rotation.then_some(axis);
        Ok(ShapeBirthGroup {
            samples,
            raw_position: raw.map(|v| v.0),
            raw_direction: raw.map(|v| v.1),
            before_rng,
            before_store,
            after_rng: next,
            source_affine: affine,
            axis_of_rotation,
        })
    }
}

impl ShapeBirthGroup {
    /// A non-finite stored position or direction in any of the four lanes.
    /// The axis-of-rotation channel is not part of it.
    fn has_nonfinite_output(&self) -> bool {
        self.samples.iter().any(|s| {
            s.position
                .iter()
                .chain(s.direction.iter())
                .any(|v| !v.is_finite())
        })
    }
}

pub const IDENTITY: [f32; 16] = [
    1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 1.0,
];

fn square(v: [f32; 3]) -> f32 {
    v[0] * v[0] + (v[1] * v[1] + v[2] * v[2])
}
/// Native keeps a vector only where its square compares greater than the
/// threshold; any other square, NaN included, selects +Z.
fn masked(v: [f32; 3]) -> [f32; 3] {
    if square(v) > f32::from_bits(0x0da24260) {
        v
    } else {
        [0.0, 0.0, 1.0]
    }
}
fn normalize(v: [f32; 3]) -> [f32; 3] {
    let square = square(v);
    if !(square > f32::from_bits(0x0da24260)) {
        return [0.0, 0.0, 1.0];
    }
    let r = native_rsqrt(square);
    v.map(|x| x * r)
}
/// EmitterStoreData's axis of rotation: +Z crossed with the masked source
/// direction, written with the explicit zero products of the native operand
/// order; when that cross is short (square at most 0.01) the same cross with
/// the owner-rotated position before translation; unit after the reciprocal
/// square root refinement, or +Y when the chosen cross is itself short. The
/// short test comes first, so a zero or subnormal square never reaches the
/// reciprocal square root, whose result native discards there.
fn axis_of_rotation(direction: [f32; 3], rotated: [f32; 3]) -> [f32; 3] {
    let short = f32::from_bits(0x3c23_d70a);
    let cross = |v: [f32; 3]| [v[2] * 0.0 - v[1], v[0] - v[2] * 0.0, v[1] * 0.0 - v[0] * 0.0];
    let axis = cross(direction);
    let axis = if short >= square(axis) {
        cross(rotated)
    } else {
        axis
    };
    let length = square(axis);
    if short >= length {
        return [0.0, 1.0, 0.0];
    }
    let r = native_rsqrt(length);
    axis.map(|v| v * r)
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
    // ShapeModule::Start: ZXY quaternion of the shape rotation, with the
    // engine's own sign vectors.
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
    // Each emitter-scale axis is a unit row multiplied by its scale
    // component, so a negative component makes that row's zeros -0.
    const UNIT: [[f32; 4]; 3] = [
        [1.0, 0.0, 0.0, 0.0],
        [0.0, 1.0, 0.0, 0.0],
        [0.0, 0.0, 1.0, 0.0],
    ];
    let axes: [[f32; 4]; 3] = std::array::from_fn(|k| UNIT[k].map(|u| u * emitter_scale[k]));
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

/// Diagnostic text rows exported unchanged from the current native shape birth
/// receipt. Expected native channels never feed the law.
#[cfg(test)]
pub fn replay_native_rows(text: &str) -> usize {
    use super::schema::ShapeControls;
    let mut groups = 0;
    for (case, line) in text.lines().enumerate() {
        if let Some(row) = line.strip_prefix("R ") {
            let words: Vec<u32> = row.split_whitespace().map(|v| v.parse().unwrap()).collect();
            assert_eq!(words.len(), 3);
            let value = f32::from_bits(words[0]);
            assert_eq!(
                super::shape::native_rsqrt_estimate(value).to_bits(),
                words[1],
                "native FRSQRTE {value}"
            );
            assert_eq!(
                native_rsqrt(value).to_bits(),
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
                // The native runs held the null texture reference.
                texture: Some(ShapeTexture::None),
                ..Default::default()
            },
        };
        let law = ShapeBirthLaw::from_params(&source).unwrap();
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let mut random = words(5);
        let group = law
            .sample_group(&mut random, owner, v[89] == 1, [1.0; 3], false)
            .unwrap();
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

/// What a widened-layout replay did with its native rows. The counts are a
/// report, not a contract: every refusal is tied, row by row, to what the
/// native row itself records.
#[cfg(test)]
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ReplayCount {
    /// Rows the law accepted and reproduced bit for bit.
    pub replayed: usize,
    /// Rows whose Shape block the gates refuse, each one a row flagged as
    /// carrying a branch or input the port does not transcribe.
    pub gate_refused: usize,
    /// Rows refused as a non-finite output, each one a row whose native
    /// stored position or direction is itself non-finite.
    pub output_refused: usize,
}

/// Diagnostic replay of native Hemisphere groups in the widened 174-word
/// layout: the 158-word layout above with any emitter-state scale at 86..89,
/// then the arc spread bits, the axis-of-rotation flag, the twelve axis words
/// (axis*4 + lane) and the native kernel and Store draw counts. Expected
/// native channels never feed the law. The row builder holds the arc mode
/// Random, no direction perturbation and align off, as the native runs did;
/// every other input the gates read comes from the row: the thickness, the
/// radius, arc, arc spread, position jitter, scale, rotation and position.
/// Of those, only the thickness is flagged from the row: the shell's inner
/// radius is exp2f(log2f(1 - thickness) * 3) through the device libm, which
/// libunity does not carry, and whose result is fixed only at a thickness of
/// 0 or 1. So a gate refusal must fall on a row of another thickness, and
/// such a row records the harness's libm, not the device's, so it cannot
/// vouch for an admission either. A refusal by any other gate (a non-finite
/// radius, scale, rotation or position; a negative or non-finite arc, arc
/// spread or jitter) turns the replay red, whichever side moved: a gate
/// narrowed below the recorded inputs, or a row file carrying an input past
/// those gates, which the native call executed and which the port must then
/// transcribe or flag from the row.
#[cfg(test)]
pub fn replay_native_rows_v2(text: &str) -> ReplayCount {
    use super::schema::ShapeControls;
    let nonfinite = |w: &u32| (w >> 23) & 0xff == 0xff;
    let mut count = ReplayCount::default();
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 174, "case {case} row width");
        assert_eq!(v[0], 2, "case {case}: Hemisphere rows only");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        // The native draw census is self-consistent with the recorded streams.
        let mut stepped = words(5);
        for _ in 0..v[172] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(45), "case {case} kernel draws");
        for _ in 0..v[173] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(61), "case {case} Store draws");
        let source = ShapeParams {
            shape_type: "Hemisphere".into(),
            radius: f32::from_bits(v[1]),
            radius_thickness: f32::from_bits(v[2]),
            arc: f32::from_bits(v[3]),
            rotation: floats(77),
            position: floats(83),
            controls: ShapeControls {
                source_version: Some(1),
                scale: Some(floats(80)),
                arc_mode: Some(ShapeMode::Random),
                arc_spread: Some(f32::from_bits(v[158])),
                align_to_direction: Some(false),
                random_direction: Some(0.0),
                spherical_direction: Some(0.0),
                random_position: Some(f32::from_bits(v[4])),
                // The native runs held the null texture reference.
                texture: Some(ShapeTexture::None),
                ..Default::default()
            },
        };
        let thickness = f32::from_bits(v[2]);
        let libm_dependent = thickness != 0.0 && thickness != 1.0;
        let law = match ShapeBirthLaw::from_params(&source) {
            Ok(law) => law,
            Err(refused) => {
                assert!(
                    libm_dependent,
                    "case {case}: gate refused {refused:?} a row inside the native envelope"
                );
                count.gate_refused += 1;
                continue;
            }
        };
        assert!(
            !libm_dependent,
            "case {case}: admitted a thickness whose shell radius the device libm decides"
        );
        let world = v[89] == 1;
        let uses_axis = v[159] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let native_nonfinite = v[134..158].iter().any(nonfinite);
        let mut random = words(5);
        let group = match law.sample_group(&mut random, owner, world, floats(86), uses_axis) {
            Ok(group) => group,
            Err(Refused::NonfiniteOutput) => {
                assert!(native_nonfinite, "case {case}: refused a finite native group");
                assert_eq!(random, words(5), "case {case}: refusal consumed the stream");
                count.output_refused += 1;
                continue;
            }
            Err(other) => panic!("case {case}: unexpected refusal {other:?}"),
        };
        assert_eq!(group.before_rng, words(5));
        assert_eq!(group.before_store, words(45), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        assert_eq!(random, group.after_rng);
        for i in 0..16 {
            assert_eq!(
                group.source_affine[i].to_bits(),
                v[106 + i],
                "case {case} source affine {i}"
            );
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[122 + i], "case {case} outer rotation {i}");
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
        match group.axis_of_rotation {
            Some(axes) => {
                for lane in 0..4 {
                    for axis in 0..3 {
                        // A NaN payload is not part of the contract: the
                        // channel does not refuse a group, and its consumer
                        // gates finiteness.
                        let (ours, native) = (axes[lane][axis], v[160 + axis * 4 + lane]);
                        assert!(
                            ours.to_bits() == native || (ours.is_nan() && f32::from_bits(native).is_nan()),
                            "case {case} axis of rotation {lane}/{axis}: law {:#010x} native {native:#010x}",
                            ours.to_bits()
                        );
                    }
                }
            }
            None => assert!(!uses_axis, "case {case}: axis channel not written"),
        }
        count.replayed += 1;
    }
    count
}

/// The Shape block of one native ConeVolume row (the arc mode Random, no
/// direction perturbation, align off, the null texture reference the native
/// runs held). Radius mode and spread stay absent: the law must not need
/// them.
#[cfg(test)]
fn cone_volume_row_source(v: &[u32], random_position: f32) -> ShapeParams {
    use super::schema::ShapeControls;
    let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
    ShapeParams {
        shape_type: "ConeVolume".into(),
        radius: f32::from_bits(v[0]),
        radius_thickness: f32::from_bits(v[1]),
        arc: f32::from_bits(v[4]),
        rotation: floats(6),
        position: floats(9),
        controls: ShapeControls {
            source_version: Some(1),
            angle: Some(f32::from_bits(v[2])),
            length: Some(f32::from_bits(v[3])),
            scale: Some(floats(12)),
            arc_mode: Some(ShapeMode::Random),
            arc_spread: Some(f32::from_bits(v[5])),
            align_to_direction: Some(false),
            random_direction: Some(0.0),
            spherical_direction: Some(0.0),
            random_position: Some(random_position),
            texture: Some(ShapeTexture::None),
            ..Default::default()
        },
    }
}

/// Diagnostic replay of native ConeVolume groups, 161 words per row: the
/// Shape inputs (radius, thickness, angle, length, arc, arc spread, rotation,
/// position, scale, emitter-state scale, simulation space, owner), then the
/// Shape stream at group start, before Store and after Store, the kernel's
/// local position and direction (axis*4 + lane), the Store affine and outer
/// rotation, the stored position and direction, and the case and group
/// ordinals. `#` lines are comments. Expected native channels never feed the
/// law.
#[cfg(test)]
pub fn replay_cone_volume_rows(text: &str) -> usize {
    let mut groups = 0;
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 161, "case {case} row width");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let law = ShapeBirthLaw::from_params(&cone_volume_row_source(&v, 0.0)).unwrap();
        let world = v[18] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[19 + i]));
        let mut random = words(35);
        let group = law
            .sample_group(&mut random, owner, world, floats(15), false)
            .unwrap();
        assert_eq!(group.before_rng, words(35));
        assert_eq!(group.before_store, words(51), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(67), "case {case} after Store RNG");
        assert_eq!(random, group.after_rng);
        for i in 0..16 {
            assert_eq!(group.source_affine[i].to_bits(), v[107 + i], "case {case} source affine {i}");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[123 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                assert_eq!(group.raw_position[lane][axis].to_bits(), v[83 + at], "case {case} local position {lane}/{axis}");
                assert_eq!(group.raw_direction[lane][axis].to_bits(), v[95 + at], "case {case} local direction {lane}/{axis}");
                assert_eq!(group.samples[lane].position[axis].to_bits(), v[135 + at], "case {case} stored position {lane}/{axis}");
                assert_eq!(group.samples[lane].direction[axis].to_bits(), v[147 + at], "case {case} stored direction {lane}/{axis}");
            }
        }
        groups += 1;
    }
    groups
}

/// Diagnostic replay of native ConeVolume edge groups, 177 words per row: the
/// 161-word layout above, then the position jitter bits, the
/// axis-of-rotation flag, the twelve axis words (axis*4 + lane, zero without
/// the flag) and the native kernel and Store draw counts. Every channel of
/// every row is compared, refused rows included: equal bits, or NaN where
/// native wrote NaN (a NaN payload is not part of the contract: a non-finite
/// position or direction is refused, and no product consumer reads the axis
/// channel, since the admission refuses every Mesh system without 3D
/// rotation). The refusal itself must fall exactly on the rows
/// whose native stored position or direction is non-finite, and must leave
/// the stream untouched; a non-finite written axis alone is not refused.
#[cfg(test)]
pub fn replay_cone_volume_rows_v2(text: &str) -> ReplayCount {
    let nonfinite = |w: &u32| (w >> 23) & 0xff == 0xff;
    let mut count = ReplayCount::default();
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 177, "case {case} row width");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let same = |actual: f32, at: usize, what: &str| {
            let native = v[at];
            assert!(
                actual.to_bits() == native || (actual.is_nan() && f32::from_bits(native).is_nan()),
                "case {case} {what}: law {:#010x} native {native:#010x}",
                actual.to_bits()
            );
        };
        // The native draw census is self-consistent with the recorded streams.
        let mut stepped = words(35);
        for _ in 0..v[175] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(51), "case {case} kernel draws");
        for _ in 0..v[176] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(67), "case {case} Store draws");
        // The row builder holds the arc mode Random, no direction perturbation
        // and align off, as the native runs did, and the gates admit every
        // finite value of the controls the kernel reads. The layout carries
        // no branch the port leaves untranscribed, so a gate refusal is red:
        // a narrowed gate turns the replay red.
        let law = ShapeBirthLaw::from_params(&cone_volume_row_source(&v, f32::from_bits(v[161])))
            .unwrap_or_else(|refused| {
                panic!("case {case}: gate refused {refused:?} a row inside the transcribed envelope")
            });
        let world = v[18] == 1;
        let uses_axis = v[162] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[19 + i]));
        let group = law
            .evaluate_group(words(35), owner, world, floats(15), uses_axis)
            .unwrap_or_else(|refused| panic!("case {case}: owner refusal {refused:?}"));
        assert_eq!(group.before_rng, words(35));
        assert_eq!(group.before_store, words(51), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(67), "case {case} after Store RNG");
        for i in 0..16 {
            same(group.source_affine[i], 107 + i, "source affine");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[123 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                same(group.raw_position[lane][axis], 83 + at, "local position");
                same(group.raw_direction[lane][axis], 95 + at, "local direction");
                same(group.samples[lane].position[axis], 135 + at, "stored position");
                same(group.samples[lane].direction[axis], 147 + at, "stored direction");
            }
        }
        match group.axis_of_rotation {
            Some(axes) => {
                assert!(uses_axis, "case {case}: axis channel written without the flag");
                for lane in 0..4 {
                    for axis in 0..3 {
                        same(axes[lane][axis], 163 + axis * 4 + lane, "axis of rotation");
                    }
                }
            }
            None => assert!(!uses_axis, "case {case}: axis channel not written"),
        }
        let native_nonfinite = v[135..159].iter().any(nonfinite);
        let mut random = words(35);
        match law.sample_group(&mut random, owner, world, floats(15), uses_axis) {
            Ok(_) => {
                assert!(!native_nonfinite, "case {case}: accepted a non-finite native group");
                assert_eq!(random, words(67), "case {case}: stream after the group");
                count.replayed += 1;
            }
            Err(Refused::NonfiniteOutput) => {
                assert!(native_nonfinite, "case {case}: refused a finite native group");
                assert_eq!(random, words(35), "case {case}: refusal consumed the stream");
                count.output_refused += 1;
            }
            Err(other) => panic!("case {case}: unexpected refusal {other:?}"),
        }
    }
    count
}

/// The Shape block of one native SingleSidedEdge (type 12) or Circle (type
/// 10) row of the 167-word layout, with every authored control the row
/// carries, those the kernel does not read included, and fixed non-zero
/// values for the controls the row does not carry (cone angle and length,
/// torus radius, box thickness, radius and arc speed): the gates must not
/// depend on any of them.
#[cfg(test)]
fn edge_circle_row_source(v: &[u32]) -> ShapeParams {
    use super::schema::ShapeControls;
    use super::MinMaxCurve;
    let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
    let mode = |word: u32| match word {
        0 => ShapeMode::Random,
        1 => ShapeMode::Loop,
        2 => ShapeMode::PingPong,
        3 => ShapeMode::BurstSpread,
        other => panic!("shape mode {other}"),
    };
    ShapeParams {
        shape_type: match v[0] {
            10 => "Circle",
            12 => "SingleSidedEdge",
            other => panic!("shape type {other}"),
        }
        .into(),
        radius: f32::from_bits(v[1]),
        radius_thickness: f32::from_bits(v[2]),
        arc: f32::from_bits(v[3]),
        rotation: floats(77),
        position: floats(83),
        controls: ShapeControls {
            source_version: Some(1),
            angle: Some(25.0),
            length: Some(5.0),
            donut_radius: Some(0.2),
            scale: Some(floats(80)),
            box_thickness: Some([0.5; 3]),
            arc_mode: Some(mode(v[160])),
            arc_spread: Some(f32::from_bits(v[161])),
            arc_speed: Some(MinMaxCurve::Constant(1.5)),
            radius_mode: Some(mode(v[159])),
            radius_spread: Some(f32::from_bits(v[158])),
            radius_speed: Some(MinMaxCurve::Constant(0.75)),
            align_to_direction: Some(v[166] != 0),
            random_direction: Some(f32::from_bits(v[164])),
            spherical_direction: Some(f32::from_bits(v[165])),
            random_position: Some(f32::from_bits(v[4])),
            // The native runs held the null texture reference.
            texture: Some(ShapeTexture::None),
        },
    }
}

/// Diagnostic replay of native SingleSidedEdge and Circle groups, 167 words
/// per row: the 158-word layout of `replay_native_rows` (unit emitter-state
/// scale), then the radius spread, radius mode, arc mode and arc spread
/// (modes 0 Random, 1 Loop, 2 PingPong, 3 BurstSpread), the kernel and Store
/// draw counts, and the random direction, spherical direction and align
/// words. Every row must be admitted and reproduced bit for bit, including
/// the rows whose controls vary only in fields the kernel does not read.
/// `#` lines are comments. Expected native channels never feed the law.
#[cfg(test)]
pub fn replay_edge_circle_rows(text: &str) -> usize {
    let mut groups = 0;
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 167, "case {case} row width");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let kernel_draws = match v[0] {
            12 => 1,
            10 => 2,
            other => panic!("case {case}: shape type {other}"),
        };
        assert_eq!((v[162], v[163]), (kernel_draws, 0), "case {case} draw census");
        let mut stepped = words(5);
        for _ in 0..v[162] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(45), "case {case} kernel draws");
        assert_eq!(words(45), words(61), "case {case} Store draws");
        assert_eq!(floats(86), [1.0; 3], "case {case} emitter state scale");
        let law = ShapeBirthLaw::from_params(&edge_circle_row_source(&v))
            .unwrap_or_else(|refused| panic!("case {case}: gate refused {refused:?}"));
        let world = v[89] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let mut random = words(5);
        let group = law
            .sample_group(&mut random, owner, world, [1.0; 3], false)
            .unwrap_or_else(|refused| panic!("case {case}: refused {refused:?}"));
        assert_eq!(group.before_rng, words(5));
        assert_eq!(group.before_store, words(45), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        assert_eq!(random, group.after_rng);
        for i in 0..16 {
            assert_eq!(group.source_affine[i].to_bits(), v[106 + i], "case {case} source affine {i}");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[122 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                assert_eq!(group.raw_position[lane][axis].to_bits(), v[21 + at], "case {case} local position {lane}/{axis}");
                assert_eq!(group.raw_direction[lane][axis].to_bits(), v[33 + at], "case {case} local direction {lane}/{axis}");
                assert_eq!(group.samples[lane].position[axis].to_bits(), v[134 + at], "case {case} stored position {lane}/{axis}");
                assert_eq!(group.samples[lane].direction[axis].to_bits(), v[146 + at], "case {case} stored direction {lane}/{axis}");
            }
        }
        groups += 1;
    }
    groups
}

/// Diagnostic replay of native SingleSidedEdge and Circle edge groups, 181
/// words per row: the 167-word layout above with any emitter-state scale at
/// 86..89, then the axis-of-rotation flag, the twelve axis words (axis*4 +
/// lane, zero without the flag) and the template path the native call took
/// (0 the plain kernel; 1 the edge's radius-spread path or the circle's
/// arc-spread branch). A gate refusal must fall on a row flagged as taking a
/// branch the port does not transcribe (a random or spherical direction;
/// align to direction), so a narrowed gate turns the replay red. The
/// direction perturbations write only channels the row records, so an
/// admitted row carrying them is compared like any other; the align block
/// writes the particle rotation, which the row does not record, so an aligned
/// row must not be admitted. The edge's spread path and the circle's
/// arc-spread branch must each be the native path exactly where its step (the
/// radius times the radius spread; the arc in radians times the arc spread)
/// compares above zero.
/// Every channel of every admitted row is compared: equal bits, or NaN where
/// native wrote NaN (a NaN payload is not part of the contract: a non-finite
/// position or direction is refused, and no product consumer reads the axis
/// channel, since the admission refuses every Mesh system without 3D
/// rotation). The output refusal must fall exactly on the rows
/// whose native stored position or direction is non-finite, and must leave
/// the stream untouched; a non-finite written axis alone is not refused.
#[cfg(test)]
pub fn replay_edge_circle_rows_v2(text: &str) -> ReplayCount {
    let nonfinite = |w: &u32| (w >> 23) & 0xff == 0xff;
    let mut count = ReplayCount::default();
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 181, "case {case} row width");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let same = |actual: f32, at: usize, what: &str| {
            let native = v[at];
            assert!(
                actual.to_bits() == native || (actual.is_nan() && f32::from_bits(native).is_nan()),
                "case {case} {what}: law {:#010x} native {native:#010x}",
                actual.to_bits()
            );
        };
        // The native draw census is self-consistent with the recorded streams.
        let mut stepped = words(5);
        for _ in 0..v[162] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(45), "case {case} kernel draws");
        for _ in 0..v[163] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(61), "case {case} Store draws");
        let circle = match v[0] {
            10 => true,
            12 => false,
            other => panic!("case {case}: shape type {other}"),
        };
        // Flagged on the row: the Store's random direction, spherical
        // direction and align words.
        let align = v[166] != 0;
        let outside = f32::from_bits(v[164]) != 0.0 || f32::from_bits(v[165]) != 0.0 || align;
        let law = match ShapeBirthLaw::from_params(&edge_circle_row_source(&v)) {
            Ok(law) => law,
            Err(refused) => {
                assert!(
                    outside,
                    "case {case}: gate refused {refused:?} a row inside the transcribed envelope"
                );
                count.gate_refused += 1;
                continue;
            }
        };
        assert!(!align, "case {case}: admitted align to direction, whose rotation the row does not record");
        if circle {
            let step = (f32::from_bits(v[3]) * f32::from_bits(0x3c8e_fa35)) * f32::from_bits(v[161]);
            assert_eq!(v[180] == 1, step > 0.0, "case {case}: circle arc-spread path");
        } else {
            let step = f32::from_bits(v[1]) * f32::from_bits(v[158]);
            assert_eq!(v[180] == 1, step > 0.0, "case {case}: edge spread path");
        }
        let world = v[89] == 1;
        let uses_axis = v[167] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let group = law
            .evaluate_group(words(5), owner, world, floats(86), uses_axis)
            .unwrap_or_else(|refused| panic!("case {case}: owner refusal {refused:?}"));
        assert_eq!(group.before_rng, words(5));
        assert_eq!(group.before_store, words(45), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        for i in 0..16 {
            same(group.source_affine[i], 106 + i, "source affine");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[122 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                same(group.raw_position[lane][axis], 21 + at, "local position");
                same(group.raw_direction[lane][axis], 33 + at, "local direction");
                same(group.samples[lane].position[axis], 134 + at, "stored position");
                same(group.samples[lane].direction[axis], 146 + at, "stored direction");
            }
        }
        match group.axis_of_rotation {
            Some(axes) => {
                assert!(uses_axis, "case {case}: axis channel written without the flag");
                for lane in 0..4 {
                    for axis in 0..3 {
                        same(axes[lane][axis], 168 + axis * 4 + lane, "axis of rotation");
                    }
                }
            }
            None => assert!(!uses_axis, "case {case}: axis channel not written"),
        }
        let native_nonfinite = v[134..158].iter().any(nonfinite);
        let mut random = words(5);
        match law.sample_group(&mut random, owner, world, floats(86), uses_axis) {
            Ok(_) => {
                assert!(!native_nonfinite, "case {case}: accepted a non-finite native group");
                assert_eq!(random, words(61), "case {case}: stream after the group");
                count.replayed += 1;
            }
            Err(Refused::NonfiniteOutput) => {
                assert!(native_nonfinite, "case {case}: refused a finite native group");
                assert_eq!(random, words(5), "case {case}: refusal consumed the stream");
                count.output_refused += 1;
            }
            Err(other) => panic!("case {case}: unexpected refusal {other:?}"),
        }
    }
    count
}

/// The Shape block of one native Donut (type 17) row: the torus radius, arc
/// spread, radius mode and radius spread the row carries (the rows vary the
/// last two on purpose; the kernel never reads them), the given arc mode and
/// Store controls, and fixed non-zero values for the controls the row does
/// not carry (cone angle and length, box thickness, radius and arc speed):
/// the gates must not depend on any of them.
#[cfg(test)]
fn donut_row_source(
    v: &[u32],
    arc_mode: ShapeMode,
    random_direction: f32,
    spherical_direction: f32,
    align_to_direction: bool,
) -> ShapeParams {
    use super::schema::ShapeControls;
    use super::MinMaxCurve;
    let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
    ShapeParams {
        shape_type: "Donut".into(),
        radius: f32::from_bits(v[1]),
        radius_thickness: f32::from_bits(v[2]),
        arc: f32::from_bits(v[3]),
        rotation: floats(77),
        position: floats(83),
        controls: ShapeControls {
            source_version: Some(1),
            angle: Some(25.0),
            length: Some(5.0),
            donut_radius: Some(f32::from_bits(v[158])),
            scale: Some(floats(80)),
            box_thickness: Some([0.5; 3]),
            arc_mode: Some(arc_mode),
            arc_spread: Some(f32::from_bits(v[159])),
            arc_speed: Some(MinMaxCurve::Constant(1.5)),
            radius_mode: Some(replay_mode(v[160])),
            radius_spread: Some(f32::from_bits(v[161])),
            radius_speed: Some(MinMaxCurve::Constant(0.75)),
            align_to_direction: Some(align_to_direction),
            random_direction: Some(random_direction),
            spherical_direction: Some(spherical_direction),
            random_position: Some(f32::from_bits(v[4])),
            // The native runs held the null texture reference.
            texture: Some(ShapeTexture::None),
        },
    }
}

#[cfg(test)]
fn replay_mode(word: u32) -> ShapeMode {
    match word {
        0 => ShapeMode::Random,
        1 => ShapeMode::Loop,
        2 => ShapeMode::PingPong,
        3 => ShapeMode::BurstSpread,
        other => panic!("shape mode {other}"),
    }
}

/// Diagnostic replay of native Donut groups, 162 words per row: the 158-word
/// layout of `replay_native_rows` (emitter-state scale at 86..89), then the
/// torus radius, arc spread, radius mode (0 Random, 1 Loop, 2 PingPong, 3
/// BurstSpread) and radius spread. Arc mode Random, no direction
/// perturbation, align off. Every row must be admitted and reproduced bit for
/// bit, including the rows whose radius mode and spread vary. `#` lines are
/// comments. Expected native channels never feed the law.
#[cfg(test)]
pub fn replay_donut_rows(text: &str) -> usize {
    let mut groups = 0;
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 162, "case {case} row width");
        assert_eq!(v[0], 17, "case {case}: Donut rows only");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let law = ShapeBirthLaw::from_params(&donut_row_source(&v, ShapeMode::Random, 0.0, 0.0, false))
            .unwrap_or_else(|refused| panic!("case {case}: gate refused {refused:?}"));
        let world = v[89] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let mut random = words(5);
        let group = law
            .sample_group(&mut random, owner, world, floats(86), false)
            .unwrap_or_else(|refused| panic!("case {case}: refused {refused:?}"));
        assert_eq!(group.before_rng, words(5));
        assert_eq!(group.before_store, words(45), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        assert_eq!(random, group.after_rng);
        for i in 0..16 {
            assert_eq!(group.source_affine[i].to_bits(), v[106 + i], "case {case} source affine {i}");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[122 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                assert_eq!(group.raw_position[lane][axis].to_bits(), v[21 + at], "case {case} local position {lane}/{axis}");
                assert_eq!(group.raw_direction[lane][axis].to_bits(), v[33 + at], "case {case} local direction {lane}/{axis}");
                assert_eq!(group.samples[lane].position[axis].to_bits(), v[134 + at], "case {case} stored position {lane}/{axis}");
                assert_eq!(group.samples[lane].direction[axis].to_bits(), v[146 + at], "case {case} stored direction {lane}/{axis}");
            }
        }
        groups += 1;
    }
    groups
}

/// Diagnostic replay of native Donut edge groups, 182 words per row: the
/// 162-word layout above, then the arc mode, random direction, spherical
/// direction and align words, the axis-of-rotation flag, the twelve axis
/// words (axis*4 + lane, zero without the flag), the template path the
/// native call took (0 the plain arc, 1 the stepped arc) and the native
/// kernel and Store draw counts. A gate refusal must fall on a row flagged as
/// taking a branch the port does not transcribe (another arc mode, a random
/// or spherical direction, align to direction), so a narrowed gate turns the
/// replay red. The other arc kernels and the direction perturbations write
/// only channels the row records, so an admitted row carrying them is
/// compared like any other; the align block writes the particle rotation,
/// which the row does not record, so an aligned row must not be admitted.
/// The stepped arc must be the native path exactly where the arc in radians
/// times the spread compares above zero. Every channel of every admitted row
/// is compared: equal bits, or NaN where native wrote NaN (a NaN payload is not
/// part of the contract: a non-finite position or direction is refused, and
/// no product consumer reads the axis channel, since the admission refuses
/// every Mesh system without 3D rotation). The output refusal must fall
/// exactly on the rows whose native stored position or direction is
/// non-finite, and must leave the stream untouched; a non-finite written
/// axis alone is not refused.
#[cfg(test)]
pub fn replay_donut_rows_v2(text: &str) -> ReplayCount {
    let nonfinite = |w: &u32| (w >> 23) & 0xff == 0xff;
    let mut count = ReplayCount::default();
    for (case, line) in text.lines().enumerate() {
        if line.trim().is_empty() || line.starts_with('#') {
            continue;
        }
        let v: Vec<u32> = line
            .split_whitespace()
            .map(|x| x.parse().unwrap())
            .collect();
        assert_eq!(v.len(), 182, "case {case} row width");
        assert_eq!(v[0], 17, "case {case}: Donut rows only");
        let floats = |at: usize| std::array::from_fn::<_, 3, _>(|a| f32::from_bits(v[at + a]));
        let words = |at: usize| ModuleRandom {
            words: std::array::from_fn(|w| std::array::from_fn(|l| v[at + w * 4 + l])),
        };
        let same = |actual: f32, at: usize, what: &str| {
            let native = v[at];
            assert!(
                actual.to_bits() == native || (actual.is_nan() && f32::from_bits(native).is_nan()),
                "case {case} {what}: law {:#010x} native {native:#010x}",
                actual.to_bits()
            );
        };
        // The native draw census is self-consistent with the recorded streams.
        let mut stepped = words(5);
        for _ in 0..v[180] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(45), "case {case} kernel draws");
        for _ in 0..v[181] {
            stepped.next4_u32();
        }
        assert_eq!(stepped, words(61), "case {case} Store draws");
        let (random_direction, spherical_direction) = (f32::from_bits(v[163]), f32::from_bits(v[164]));
        // Flagged on the row: an arc mode other than Random (another kernel),
        // and the Store's random direction, spherical direction and align
        // words.
        let align = v[165] != 0;
        let outside = v[162] != 0 || random_direction != 0.0 || spherical_direction != 0.0 || align;
        let source = donut_row_source(
            &v,
            replay_mode(v[162]),
            random_direction,
            spherical_direction,
            align,
        );
        let law = match ShapeBirthLaw::from_params(&source) {
            Ok(law) => law,
            Err(refused) => {
                assert!(
                    outside,
                    "case {case}: gate refused {refused:?} a row inside the transcribed envelope"
                );
                count.gate_refused += 1;
                continue;
            }
        };
        assert!(!align, "case {case}: admitted align to direction, whose rotation the row does not record");
        let step = (f32::from_bits(v[3]) * f32::from_bits(0x3c8e_fa35)) * f32::from_bits(v[159]);
        assert_eq!(v[179] == 1, step > 0.0, "case {case}: stepped arc path");
        let world = v[89] == 1;
        let uses_axis = v[166] == 1;
        let owner = std::array::from_fn(|i| f32::from_bits(v[90 + i]));
        let group = law
            .evaluate_group(words(5), owner, world, floats(86), uses_axis)
            .unwrap_or_else(|refused| panic!("case {case}: owner refusal {refused:?}"));
        assert_eq!(group.before_rng, words(5));
        assert_eq!(group.before_store, words(45), "case {case} before Store RNG");
        assert_eq!(group.after_rng, words(61), "case {case} after Store RNG");
        for i in 0..16 {
            same(group.source_affine[i], 106 + i, "source affine");
        }
        let outer = if world { owner } else { IDENTITY };
        for i in 0..12 {
            assert_eq!(outer[i].to_bits(), v[122 + i], "case {case} outer rotation {i}");
        }
        for lane in 0..4 {
            for axis in 0..3 {
                let at = axis * 4 + lane;
                same(group.raw_position[lane][axis], 21 + at, "local position");
                same(group.raw_direction[lane][axis], 33 + at, "local direction");
                same(group.samples[lane].position[axis], 134 + at, "stored position");
                same(group.samples[lane].direction[axis], 146 + at, "stored direction");
            }
        }
        match group.axis_of_rotation {
            Some(axes) => {
                assert!(uses_axis, "case {case}: axis channel written without the flag");
                for lane in 0..4 {
                    for axis in 0..3 {
                        same(axes[lane][axis], 167 + axis * 4 + lane, "axis of rotation");
                    }
                }
            }
            None => assert!(!uses_axis, "case {case}: axis channel not written"),
        }
        let native_nonfinite = v[134..158].iter().any(nonfinite);
        let mut random = words(5);
        match law.sample_group(&mut random, owner, world, floats(86), uses_axis) {
            Ok(_) => {
                assert!(!native_nonfinite, "case {case}: accepted a non-finite native group");
                assert_eq!(random, words(61), "case {case}: stream after the group");
                count.replayed += 1;
            }
            Err(Refused::NonfiniteOutput) => {
                assert!(native_nonfinite, "case {case}: refused a finite native group");
                assert_eq!(random, words(5), "case {case}: refusal consumed the stream");
                count.output_refused += 1;
            }
            Err(other) => panic!("case {case}: unexpected refusal {other:?}"),
        }
    }
    count
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
                texture: Some(ShapeTexture::None),
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
            law.sample_group(&mut random, owner, true, [1.0; 3], false)
                .unwrap_err(),
            Refused::NonfiniteOutput
        );
        assert_eq!(random, before);
    }
    fn rows(variable: &str) -> String {
        let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} names the native row file"));
        std::fs::read_to_string(path).unwrap()
    }
    /// Every native Hemisphere group of the item receipt that the 158-word
    /// layout can express (unit emitter-state scale, zero arc spread),
    /// including all corpus configurations.
    #[test]
    #[ignore = "set MOLY_SHAPE_HEMISPHERE_ROWS to the current native Hemisphere rows, 158-word layout"]
    fn current_hemisphere_item_rows_bit_exact() {
        assert_eq!(super::replay_native_rows(&rows("MOLY_SHAPE_HEMISPHERE_ROWS")), 1087);
    }
    /// Every native group of the item receipt: any emitter-state scale, the
    /// quantized arc, both shells, position jitter and the axis channel.
    /// Each refusal is tied to its row inside the replay; the counts are
    /// reported.
    #[test]
    #[ignore = "set MOLY_SHAPE_HEMISPHERE_ROWS_V2 to the current native Hemisphere rows, 174-word layout"]
    fn current_hemisphere_widened_rows_bit_exact() {
        let text = rows("MOLY_SHAPE_HEMISPHERE_ROWS_V2");
        let total = text.lines().filter(|l| !l.trim().is_empty()).count();
        let count = super::replay_native_rows_v2(&text);
        println!("{count:?} of {total}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    /// Independent native edge executions and finite inputs whose f32
    /// intermediates overflow or underflow: every row either reproduces the
    /// native bits (NaN where native wrote NaN in the axis channel) or,
    /// exactly where native writes a non-finite position or direction, is
    /// refused as a non-finite output without consuming the stream; a gate
    /// refusal must fall on a row whose thickness is neither 0 nor 1 (its
    /// shell radius comes from the device libm), and a refusal by any other
    /// gate is red. The counts are reported.
    #[test]
    #[ignore = "set MOLY_SHAPE_HEMISPHERE_ROWS_V3 to the current native Hemisphere edge rows, 174-word layout"]
    fn current_hemisphere_edge_rows_bit_exact_or_refused() {
        let text = rows("MOLY_SHAPE_HEMISPHERE_ROWS_V3");
        let total = text.lines().filter(|l| !l.trim().is_empty()).count();
        let count = super::replay_native_rows_v2(&text);
        println!("{count:?} of {total}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    /// Every native ConeVolume group of the first recorded native run: all
    /// corpus configurations and the randomized envelope, unit emitter-state
    /// scale.
    #[test]
    #[ignore = "set MOLY_CONE_VOLUME_NATIVE_ROWS to the current native ConeVolume rows, 161-word layout"]
    fn current_cone_volume_store_all_padded_lanes_bit_exact() {
        assert_eq!(super::replay_cone_volume_rows(&rows("MOLY_CONE_VOLUME_NATIVE_ROWS")), 401);
    }
    /// Independent native ConeVolume executions over the widened gates: any
    /// emitter-state scale, position jitter, the axis channel, and finite
    /// inputs whose f32 intermediates overflow, underflow or turn invalid.
    #[test]
    #[ignore = "set MOLY_CONE_VOLUME_NATIVE_ROWS_V2 to the current native ConeVolume edge rows, 177-word layout"]
    fn current_cone_volume_edge_rows_bit_exact_or_refused() {
        let text = rows("MOLY_CONE_VOLUME_NATIVE_ROWS_V2");
        let total = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();
        let count = super::replay_cone_volume_rows_v2(&text);
        println!("{count:?} of {total}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    /// Every native SingleSidedEdge and Circle group of the first recorded
    /// native run: every corpus configuration of the two Random kernels, the
    /// controls the kernels do not read, the edge's radius-spread path,
    /// degenerate authored scales and the randomized envelope, unit
    /// emitter-state scale.
    #[test]
    #[ignore = "set MOLY_SHAPE_EDGE_CIRCLE_NATIVE_ROWS to the current native SingleSidedEdge and Circle rows, 167-word layout"]
    fn current_edge_and_circle_store_all_padded_lanes_bit_exact() {
        assert_eq!(super::replay_edge_circle_rows(&rows("MOLY_SHAPE_EDGE_CIRCLE_NATIVE_ROWS")), 1004);
    }
    /// Independent native SingleSidedEdge and Circle executions over the
    /// widened gates: any emitter-state scale, position jitter, the axis
    /// channel, the gate boundaries, and finite inputs whose f32
    /// intermediates overflow, underflow or turn invalid.
    #[test]
    #[ignore = "set MOLY_SHAPE_EDGE_CIRCLE_NATIVE_ROWS_V2 to the current native SingleSidedEdge and Circle edge rows, 181-word layout"]
    fn current_edge_and_circle_edge_rows_bit_exact_or_refused() {
        let text = rows("MOLY_SHAPE_EDGE_CIRCLE_NATIVE_ROWS_V2");
        let total = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();
        let count = super::replay_edge_circle_rows_v2(&text);
        let circle_spread_rows = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .map(|l| l.split_whitespace().collect::<Vec<_>>())
            .filter(|w| w[0] == "10" && w[180] == "1")
            .count();
        println!("{count:?} of {total}; circle rows on the arc-spread branch {circle_spread_rows}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    /// Independent native Circle arc-spread executions the edge rows above do
    /// not hold: the arc in radians times the spread overflowing to infinity,
    /// a step of one count, step counts at and around an integer ratio and
    /// saturating ones, each with position jitter, World owners, emitter-state
    /// scale and the axis channel, same layout and checks.
    #[test]
    #[ignore = "set MOLY_SHAPE_CIRCLE_SPREAD_ROWS to the current native Circle arc-spread rows, 181-word layout"]
    fn current_circle_arc_spread_rows_bit_exact_or_refused() {
        let text = rows("MOLY_SHAPE_CIRCLE_SPREAD_ROWS");
        let total = text.lines().filter(|l| !l.trim().is_empty()).count();
        let count = super::replay_edge_circle_rows_v2(&text);
        println!("{count:?} of {total}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    /// Every native Donut group of the first recorded native run inside its
    /// envelope file: every corpus configuration (each member re-run, World
    /// owners), the radius modes and spreads the kernel does not read, and
    /// the randomized envelope, unit emitter-state scale and zero arc spread.
    #[test]
    #[ignore = "set MOLY_SHAPE_DONUT_NATIVE_ROWS to the current native Donut rows, 162-word layout"]
    fn current_source_donut_store_all_padded_lanes_bit_exact() {
        assert_eq!(super::replay_donut_rows(&rows("MOLY_SHAPE_DONUT_NATIVE_ROWS")), 1206);
    }
    /// Every native Donut group of that run: adds its non-unit emitter-state
    /// scales and its stepped-arc groups.
    #[test]
    #[ignore = "set MOLY_SHAPE_DONUT_NATIVE_ROWS_EXTENDED to every current native Donut row of that run, 162-word layout"]
    fn current_source_donut_extended_rows_bit_exact() {
        assert_eq!(super::replay_donut_rows(&rows("MOLY_SHAPE_DONUT_NATIVE_ROWS_EXTENDED")), 1414);
    }
    /// Independent native Donut executions over the widened gates: any
    /// emitter-state scale, position jitter, the axis channel, the stepped
    /// arc and its boundaries, the gate boundaries, and finite inputs whose
    /// f32 intermediates overflow, underflow or turn invalid.
    #[test]
    #[ignore = "set MOLY_SHAPE_DONUT_NATIVE_ROWS_V2 to the current native Donut edge rows, 182-word layout"]
    fn current_donut_edge_rows_bit_exact_or_refused() {
        let text = rows("MOLY_SHAPE_DONUT_NATIVE_ROWS_V2");
        let total = text
            .lines()
            .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
            .count();
        let count = super::replay_donut_rows_v2(&text);
        println!("{count:?} of {total}");
        assert_eq!(count.replayed + count.gate_refused + count.output_refused, total);
        assert!(count.replayed > 0);
    }
    #[test]
    #[ignore = "set MOLY_SHAPE_BIRTH_NATIVE_ROWS to the diagnostic native rows exported from the current shape birth receipt"]
    fn current_source_shape_store_all_padded_lanes_bit_exact() {
        let path =
            std::env::var_os("MOLY_SHAPE_BIRTH_NATIVE_ROWS").expect("current native row file");
        let text = std::fs::read_to_string(path).unwrap();
        assert_eq!(super::replay_native_rows(&text), 92);
    }
}
