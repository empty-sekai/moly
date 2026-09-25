//! Per-particle trails of an installed native system. The TrailModule keeps
//! a clock and, per particle, an ordered run of recorded points. The engine
//! updates it at two points of every slice: once per new four-lane group of a
//! birth with dt 0, after the newborn pre-simulation modules and before the
//! newborn age, emitter backtrack and integration (so the birth point is the
//! Shape position at age 0); and after the simulation and death pass over the
//! whole pool with the slice dt. A particle's points move with it wherever
//! the particle arrays move (death compaction, birth packing). The renderer
//! builds the trail strip from the points after the frame's update, as a
//! second draw of the same renderer with the trail material.
use super::*;
use moly_law::particle::trail::{self as recording, Refused, TrailClock, TrailParticle, TrailRing, TrailSize};
use moly_law::particle::trail_geometry::{self as strip, GeometryParticle, TrailGeometryLaw, TrailMesh, TrailView};

/// The installed trail of one native system.
#[derive(Clone, Debug)]
pub(crate) struct TrailState {
    pub(crate) law: TrailGeometryLaw,
    pub(crate) clock: TrailClock,
    /// One ring per particle, parallel to the pool and the side array.
    pub(crate) rings: Vec<TrailRing>,
    /// The first refusal of the geometry pass, if any (the draw is empty then).
    pub(crate) draw_refusal: Option<Refused>,
    pub(crate) vertices: usize,
    /// The owner matrix the trail job of a Local simulation composes with the
    /// view: the engine's owner local-to-world words (source axes, column
    /// major), attached at install. `None` for a World simulation.
    pub(crate) owner: Option<[f32; 16]>,
}

impl TrailState {
    fn new(law: TrailGeometryLaw, particles: usize) -> Self {
        Self { law, clock: TrailClock::default(), rings: vec![TrailRing::default(); particles], draw_refusal: None, vertices: 0,
            owner: None }
    }

    /// One module update over `[from, to)`: the clock advances by `dt` once
    /// (an empty range still advances it), then each particle of the range
    /// ages its tail and may record its position.
    pub(super) fn update(&mut self, pool: &[Particle], side: &[Side], from: usize, to: usize, dt: f32, size3d: bool) {
        self.clock.advance(dt);
        let to = to.min(pool.len()).min(side.len()).min(self.rings.len());
        for index in from..to {
            let particle = &pool[index];
            let input = TrailParticle {
                seed: side[index].seed,
                age_percent: particle.age_percent,
                inverse_lifetime: particle.inverse_lifetime,
                position: source_position(particle.position),
                size: TrailSize { components: side[index].size, size3d },
            };
            recording::record(&self.law.recording, &mut self.rings[index], self.clock, &input, None);
        }
    }

    /// Points currently recorded over every ring.
    pub(crate) fn points(&self) -> usize {
        self.rings.iter().map(TrailRing::len).sum()
    }
}

/// The runtime stores positions reflected in X; the module reads source
/// coordinates. A sign flip is exact.
fn source_position(p: [f32; 3]) -> [f32; 3] {
    [-p[0], p[1], p[2]]
}

/// What this runtime qualifies of an authored TrailModule: the transcribed
/// recording and geometry laws, restricted to the inputs the runtime supplies
/// exactly. `None` without a TrailModule.
pub(crate) fn qualify(emitter: &EmitterParams) -> Result<Option<TrailGeometryLaw>, &'static str> {
    let Some(params) = emitter.trails.as_ref() else { return Ok(None) };
    let law = TrailGeometryLaw::from_params(params).map_err(refusal)?;
    if !params.die_with_particles {
        // The kill pass then keeps an over-age particle while its ring holds
        // points; the death compaction here is the plain law.
        return Err("trail outliving its particle (dieWithParticles false) is not transcribed");
    }
    if params.world_space {
        return Err("world-space trail: the matrix the recording transforms with is not identified");
    }
    if params.size_affects_width || params.size_affects_lifetime {
        // The module reads the start or the current size array by a flag;
        // this runtime keeps neither the flag's writer nor a current-size array.
        return Err("size-affected trail: the size array the module reads is not kept by this runtime");
    }
    if params.split_sub_emitter_ribbons || params.attach_ribbons_to_transform {
        return Err("ribbon trail controls on a per-particle trail are not executed");
    }
    if emitter.simulation_space == SimulationSpace::Custom {
        return Err("trail of a Custom simulation: the owner matrix the geometry composes is not produced here");
    }
    // A Local simulation's geometry composes the view with the engine's own
    // owner matrix (this runtime's composed transform is not that matrix bit
    // for bit): the installer attaches the owner words, and a trail without
    // them draws nothing.
    Ok(Some(law))
}

/// Attach the owner words a Local simulation's trail job composes with.
pub(crate) fn attach_owner(system: &mut Runtime, owner: [f32; 16]) -> Result<(), &'static str> {
    if system.emitter.simulation_space != SimulationSpace::Local {
        return Err("trail owner words on a system that is not a Local simulation");
    }
    let Some(trail) = system.trail.as_mut() else { return Err("trail owner words without an installed trail") };
    trail.owner = Some(owner);
    Ok(())
}

/// Whether an installed trail has every input its draw reads: a Local
/// simulation's owner words.
pub(crate) fn owner_ready(system: &Runtime) -> bool {
    system.trail.as_ref().is_none_or(|trail|
        system.emitter.simulation_space == SimulationSpace::World || trail.owner.is_some())
}

fn refusal(refused: Refused) -> &'static str {
    match refused {
        Refused::Ribbon => "ribbon trail mode is not transcribed",
        Refused::CurveMode(_) => "curve-mode trail lifetime or width is not transcribed",
        Refused::OutsideLoadClamp(_) => "trail control outside the engine's load clamp",
        Refused::LightingData => "trail lighting data selects a vertex layout that is not transcribed",
        Refused::PerceptualGradient => "trail gradient pairing evaluated through the device libm",
        Refused::SingularView => "view matrix too close to singular for the trail job",
        Refused::LinearColour => "linear colour space trail colours use the device libm",
        Refused::EmitterScale => "trail width scale of a non-unit emitter scale uses the device libm",
        Refused::MeshSizeWidth => "size-affected width of a mesh renderer uses the device libm",
    }
}

/// Install the qualified trail of a system whose native birth owner was
/// just installed.
pub(super) fn install(system: &mut Runtime, law: TrailGeometryLaw) {
    system.trail = Some(TrailState::new(law, system.pool.len()));
}

/// The emitter scale the trail job reads: the emitter's own local scale in
/// Local scaling mode. The Hierarchy mode's value is not produced here.
fn emitter_scale(system: &Runtime) -> Result<[f32; 3], &'static str> {
    match system.geometry.shape_evidence().map(|evidence| evidence.scaling) {
        Some(crate::particle_geometry::Scaling::Local { scale, .. }) => Ok(scale.to_array()),
        Some(crate::particle_geometry::Scaling::Hierarchy) => Err("Hierarchy scaling: the trail job's emitter scale is not produced here"),
        None => Err("legacy billboard carries no authored scaling mode"),
    }
}

/// The renderer-side conditions of a trail draw that the admission can check
/// before the instance exists.
pub(crate) fn draw_eligible(emitter: &EmitterParams, evidence: Option<ShapeEmitterEvidence>) -> Result<(), &'static str> {
    if emitter.trails.is_none() {
        return Ok(());
    }
    match evidence.map(|evidence| evidence.scaling) {
        Some(crate::particle_geometry::Scaling::Local { .. }) => Ok(()),
        Some(crate::particle_geometry::Scaling::Hierarchy) => Err("Hierarchy scaling: the trail job's emitter scale is not produced here"),
        None => Err("legacy billboard carries no authored scaling mode"),
    }
}

/// Build this frame's trail strip into `mesh`. `camera` is the rendering
/// camera's world transform (runtime space); the view the trail job reads is
/// the source world-to-camera matrix the source programs also receive.
pub(crate) fn write_mesh(mesh: &mut Mesh, system: &mut Runtime, owner: &GlobalTransform, camera: &GlobalTransform) {
    let result = build(system, owner, camera);
    let Some(trail) = system.trail.as_mut() else { return };
    let strip = match result {
        Ok(strip) => strip,
        Err(refused) => {
            if trail.draw_refusal.is_none() {
                error!(?refused, effect=%system.effect, node=%system.node, "trail draw refused");
            }
            trail.draw_refusal = Some(refused);
            TrailMesh::default()
        }
    };
    trail.vertices = strip.vertices.len();
    let positions: Vec<[f32; 3]> = strip.vertices.iter().map(|v| source_position(v.position)).collect();
    let colours: Vec<[f32; 4]> = strip.vertices.iter()
        .map(|v| moly_law::particle::gradient::rgba8_to_float(v.colour.to_le_bytes()))
        .collect();
    let uv: Vec<[f32; 2]> = strip.vertices.iter().map(|v| [v.u, v.v]).collect();
    let count = positions.len();
    mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
    // The trail vertex carries no normal and no custom channels; the program
    // reads their zero default.
    mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, vec![[0.0f32; 3]; count]);
    mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uv);
    mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
    mesh.insert_attribute(crate::billboard::ATTRIBUTE_CUSTOM1, vec![[0.0f32; 4]; count]);
    mesh.insert_attribute(crate::billboard::ATTRIBUTE_CUSTOM2, vec![[0.0f32; 4]; count]);
    mesh.insert_indices(bevy::mesh::Indices::U32(strip.indices));
}

fn build(system: &Runtime, owner: &GlobalTransform, camera: &GlobalTransform) -> Result<TrailMesh, Refused> {
    let Some(trail) = system.trail.as_ref() else { return Ok(TrailMesh::default()) };
    // Admission and install already refuse these; a system that reaches here
    // otherwise draws no trail rather than a wrong one.
    let Ok(emitter_scale) = emitter_scale(system) else { return Err(Refused::EmitterScale) };
    let view = (camera.to_matrix().inverse() * Mat4::from_scale(Vec3::new(-1.0, 1.0, 1.0))).to_cols_array();
    let identity = Mat4::IDENTITY.to_cols_array();
    let simulation_world = system.emitter.simulation_space == SimulationSpace::World;
    // World simulation: the job composes the view with the identity; Local:
    // with the engine's owner words attached at install (none: no draw).
    let job_owner = match (simulation_world, trail.owner) {
        (true, _) => identity,
        (false, Some(words)) if system.emitter.simulation_space == SimulationSpace::Local => words,
        _ => return Err(Refused::SingularView),
    };
    let evidence = system.geometry.shape_evidence();
    let trail_view = TrailView {
        view,
        owner: job_owner,
        // Read only by a world-space trail, which is refused.
        local_to_world: birth::source_owner_matrix(owner),
        simulation_world,
        emitter_scale,
        // Particle draws cast no shadow here; the bias applies only to a
        // shadow pass.
        shadow_pass: false,
        // The player's colour space is gamma.
        linear_colour: false,
        mesh_renderer: evidence.is_some_and(|evidence| evidence.mesh_renderer),
        size3d: system.emitter.start.size3d,
        colour_module: system.emitter.color_over_lifetime.as_ref(),
    };
    let particles = system.pool.iter().zip(&system.side).zip(&trail.rings).map(|((particle, side), ring)| GeometryParticle {
        position: source_position(particle.position),
        age_percent: particle.age_percent,
        inverse_lifetime: particle.inverse_lifetime,
        seed: side.seed,
        // The particle colour array holds the birth colour bytes; the side
        // keeps them as floats that convert back exactly.
        colour: u32::from_le_bytes(moly_law::particle::gradient::quantize_rgba8(side.colour)),
        size: side.size,
        ring,
    });
    strip::build(&trail.law, &trail_view, trail.clock, particles)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn bits(v: &Value) -> f32 {
        f32::from_bits(v.as_u64().expect("bits") as u32)
    }

    /// The native recording rows driven through the product's trail state:
    /// positions enter reflected in X as the runtime stores them, ages,
    /// seeds and inverse lifetimes through the pool and side arrays, each
    /// call as the product issues it (a newborn call resets its rings by
    /// appending fresh ones). Compares every call's clock, each ring's count,
    /// path length and points, and the trail lifetimes the call evaluated.
    /// Rows with size-affected lifetimes or world-space trails are outside
    /// this runtime's qualification and are counted, not driven.
    #[test]
    #[ignore = "needs MOLY_TRAIL_RECORD_ROWS"]
    fn product_trail_state_matches_native_recording_rows() {
        let path = std::env::var("MOLY_TRAIL_RECORD_ROWS").expect("MOLY_TRAIL_RECORD_ROWS");
        let doc: Value = serde_json::from_slice(&std::fs::read(&path).expect("read rows")).expect("parse rows");
        let (mut cases, mut calls, mut points, mut skipped) = (0usize, 0usize, 0usize, 0usize);
        for row in doc["rows"].as_array().expect("rows") {
            let case = &row["case"];
            let id = case["id"].as_str().unwrap();
            let t = &case["trail"];
            if t["sizeAffectsLifetime"].as_bool().unwrap() || t["worldSpace"].as_bool().unwrap() {
                skipped += 1;
                continue;
            }
            let lifetime = match t["lifetime"]["mode"].as_str().unwrap() {
                "constant" => recording::TrailLifetime::Constant(bits(&t["lifetime"]["value"])),
                _ => recording::TrailLifetime::TwoConstants { min: bits(&t["lifetime"]["min"]), max: bits(&t["lifetime"]["max"]) },
            };
            let lifetime = match lifetime {
                recording::TrailLifetime::Constant(v) => moly_law::particle::MinMaxCurve::Constant(v),
                recording::TrailLifetime::TwoConstants { min, max } => moly_law::particle::MinMaxCurve::TwoConstants { min, max },
            };
            // Only the recording half is read; the geometry fields take the
            // exported 009 trail values and are never evaluated here.
            let params = moly_law::particle::schema::TrailParams {
                mode: moly_law::particle::schema::TrailMode::PerParticle,
                ratio: bits(&t["ratio"]), lifetime, min_vertex_distance: bits(&t["minVertexDistance"]),
                texture_mode: moly_law::particle::schema::TrailTextureMode::Stretch, texture_scale: [1.0, 1.0],
                ribbon_count: 1, shadow_bias: 0.5, world_space: false, die_with_particles: true,
                size_affects_width: false, size_affects_lifetime: false, inherit_particle_color: true,
                generate_lighting_data: false, split_sub_emitter_ribbons: false, attach_ribbons_to_transform: false,
                color_over_lifetime: moly_law::particle::MinMaxGradient::Color([1.0; 4]),
                width_over_trail: moly_law::particle::MinMaxCurve::Constant(0.09),
                color_over_trail: moly_law::particle::MinMaxGradient::Color([1.0; 4]),
            };
            let law = TrailGeometryLaw::from_params(&params).unwrap_or_else(|e| panic!("{id}: recording law refused: {e:?}"));
            let n = case["n"].as_u64().unwrap() as usize;
            let parts = &case["particles"];
            let seeds: Vec<u32> = parts["seeds"].as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
            let inv: Vec<f32> = parts["inv"].as_array().unwrap().iter().map(bits).collect();
            let mut state = TrailState {
                law,
                clock: TrailClock::default(),
                rings: vec![TrailRing::default(); n],
                draw_refusal: None,
                vertices: 0,
                owner: None,
            };
            let mut pool: Vec<Particle> = (0..n).map(|i| Particle {
                position: [0.0; 3], velocity: [0.0; 3], start_lifetime: 1.0,
                inverse_lifetime: inv[i], age_percent: 0.0,
            }).collect();
            let side: Vec<Side> = (0..n).map(|i| Side {
                rand: 0.0, seed: seeds[i], rot: [0.0; 3], size: [1.0; 3], gravity: 0.0, colour: [1.0; 4],
                total_velocity: [0.0; 3], custom_data: [[0.0; 4]; 2], emit_carry: [0.0; 2], animated: [0.0; 3], current_size: 0.0,
                axis: [0.0, 0.0, 1.0],
            }).collect();
            let natives = row["native"].as_array().unwrap();
            for (k, call) in case["calls"].as_array().unwrap().iter().enumerate() {
                for i in call["resetRings"].as_array().unwrap() {
                    state.rings[i.as_u64().unwrap() as usize] = TrailRing::default();
                }
                for i in 0..n {
                    let p = &call["positions"][i];
                    // Enter the runtime's reflected storage.
                    pool[i].position = [-bits(&p[0]), bits(&p[1]), bits(&p[2])];
                    pool[i].age_percent = bits(&call["ages"][i]);
                }
                let (from, to) = (call["from"].as_u64().unwrap() as usize, call["to"].as_u64().unwrap() as usize);
                state.update(&pool, &side, from, to, bits(&call["dt"]), false);
                let native = &natives[k];
                let time = u64::from_str_radix(native["timeBits"].as_str().unwrap().trim_start_matches("0x"), 16).unwrap();
                assert_eq!(state.clock.time.to_bits(), time, "{id} call {k} clock");
                for (i, native_ring) in native["rings"].as_array().unwrap().iter().enumerate() {
                    let ring = &state.rings[i];
                    assert_eq!(ring.len(), native_ring["count"].as_u64().unwrap() as usize, "{id} call {k} ring {i} count");
                    assert_eq!(ring.length().to_bits(), native_ring["lenBits"].as_u64().unwrap() as u32, "{id} call {k} ring {i} length");
                    for (p, q) in ring.points().zip(native_ring["points"].as_array().unwrap()) {
                        let words = [p.position[0].to_bits(), p.position[1].to_bits(), p.position[2].to_bits(), p.time.to_bits()];
                        let expected: Vec<u32> = q.as_array().unwrap().iter().map(|v| v.as_u64().unwrap() as u32).collect();
                        assert_eq!(words.to_vec(), expected, "{id} call {k} ring {i} point");
                        points += 1;
                    }
                }
                calls += 1;
            }
            cases += 1;
        }
        println!("product trail replay: {cases} cases, {calls} calls, {points} points compared, {skipped} cases outside \
            the runtime's qualification");
        assert!(cases > 0 && points > 0);
    }
}
