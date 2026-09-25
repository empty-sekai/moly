//! Explicit current-native child commands into one shared Runtime pool.
//! The caller supplies persistent Initial RNG and captured update inputs. This
//! entry never advances old particles or autonomous clocks and has no weather
//! admission route. Qualified: Local identity owner, neutral inheritance, zero
//! speed/gravity, Initial/Color/CustomData, no overflow/death/recursive events.
use super::*;
use moly_law::particle::{
    initial::{InitialContext, InitialGroupInput, InitialLaw},
    seed_owner::ModuleRandom,
    sub_emission::{BirthDistribution, CatchUp},
    MinMaxCurve,
};

const MAX_EXACT_COUNT: u64 = 16_777_215;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ChildCommand {
    pub count: u64,
    pub rate_count: u64,
    pub distribution: BirthDistribution,
    /// Source-world coordinates, not yet reflected to runtime coordinates.
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub inherited_words: [u32; 13],
    pub dt: f32,
    pub previous: f32,
    pub current: f32,
    pub catch_up: f32,
}

#[derive(Clone, Copy, Debug)]
pub(super) struct ChildUpdate {
    pub flags: u32,
    pub frame_dt: f32,
    pub world_playing: bool,
    /// Captured Initial upper-lifetime cache, not the system duration.
    pub upper_lifetime: f32,
}

#[derive(Debug, PartialEq)]
pub(super) enum Refused {
    InvalidCommand(&'static str),
    Unsupported(&'static str),
    Initial(moly_law::particle::initial::Refused),
}

#[derive(Debug, PartialEq)]
pub(super) struct Applied {
    pub born: usize,
    pub catch_up_steps: usize,
    pub catch_up_remainder: f32,
}

impl ChildCommand {
    /// Current ARM64 layout. The pointer at +0 is not dereferenced: the caller
    /// supplies its separately captured twelve-byte emission payload. Padding
    /// at +0x54 is not a semantic field. Preserve all inherited parameter bits.
    pub fn from_native_bytes(raw: &[u8], emission: &[u8]) -> Result<Self, Refused> {
        if raw.len() != 0x78 || emission.len() != 12 {
            return Err(Refused::InvalidCommand("native command/payload length"));
        }
        let word = |offset| u32::from_le_bytes(raw[offset..offset + 4].try_into().unwrap());
        let float = |offset| f32::from_bits(word(offset));
        Ok(Self {
            count: u64::from_le_bytes(raw[0x58..0x60].try_into().unwrap()),
            rate_count: u64::from_le_bytes(raw[0x60..0x68].try_into().unwrap()),
            distribution: BirthDistribution {
                spacing: f32::from_le_bytes(emission[0..4].try_into().unwrap()),
                offset: f32::from_le_bytes(emission[4..8].try_into().unwrap()),
                burst_fraction: f32::from_le_bytes(emission[8..12].try_into().unwrap()),
            },
            position: std::array::from_fn(|a| float(8 + a * 4)),
            velocity: std::array::from_fn(|a| float(20 + a * 4)),
            inherited_words: std::array::from_fn(|i| word(0x20 + i * 4)),
            dt: float(0x68),
            previous: float(0x6c),
            current: float(0x70),
            catch_up: float(0x74),
        })
    }

    fn validate(&self, update: ChildUpdate) -> Result<(), Refused> {
        if self.count > MAX_EXACT_COUNT || self.rate_count > self.count {
            return Err(Refused::InvalidCommand("count outside exact range"));
        }
        if self
            .position
            .iter()
            .chain(self.velocity.iter())
            .any(|v| !v.is_finite())
            || !self.catch_up.is_finite()
            || self.catch_up < 0.0
            || !update.upper_lifetime.is_finite()
            || update.upper_lifetime <= 0.0
        {
            return Err(Refused::InvalidCommand(
                "nonfinite position or lifetime/timing",
            ));
        }
        if !matches!(update.flags, 0 | 1 | 4 | 5) {
            return Err(Refused::Unsupported("update flags"));
        }
        // Parent command seed is retained separately; it is not child RandN.
        // Native Math initialization supplies Vector3.forward, including the
        // +1 at command +0x44. Zero there came from an uninitialized probe VM.
        let neutral = [
            u32::MAX,
            0x3f800000,
            0x3f800000,
            0x3f800000,
            0,
            0,
            0,
            0,
            0,
            0x3f800000,
            0x3f800000,
            0x7f800000,
        ];
        if self.inherited_words[..12] != neutral {
            return Err(Refused::Unsupported("non-neutral inherited context"));
        }
        self.distribution
            .timing(
                0,
                self.rate_count as u32,
                self.dt,
                self.previous,
                self.current,
            )
            .map_err(|_| Refused::InvalidCommand("birth distribution"))?;
        Ok(())
    }
}

/// Execute one complete command before the next arrival. Source admission has
/// no caller here; the same actual Runtime/pool code is used by native replay.
pub(super) fn apply_command(
    system: &mut Runtime,
    random: &mut ModuleRandom,
    command: &ChildCommand,
    update: ChildUpdate,
    ctx: &Context,
) -> Result<Applied, Refused> {
    let noop = || Applied {
        born: 0,
        catch_up_steps: 0,
        catch_up_remainder: command.catch_up,
    };
    if command.count == 0 {
        return Ok(noop());
    }
    command.validate(update)?;
    if update.world_playing && command.catch_up >= update.upper_lifetime {
        return Ok(noop());
    }
    let mut catch_up = CatchUp::new(command.catch_up, update.frame_dt, update.flags & 5 != 0)
        .map_err(|_| Refused::Unsupported("catch-up range"))?;
    // CatchUp is Copy: consuming by value leaves the original remainder intact.
    let steps: Vec<_> = catch_up.by_ref().collect();
    let emitter = &system.emitter;
    let zero = |curve: &MinMaxCurve| {
        matches!(curve, MinMaxCurve::Constant(0.0))
            || matches!(curve, MinMaxCurve::TwoConstants { min: 0.0, max: 0.0 })
    };
    if emitter.simulation_space != SimulationSpace::Local
        || compose_to_world(system, ctx) != GlobalTransform::IDENTITY
        || emitter.simulation_speed != 1.0
        || emitter.prewarm
        || emitter.shape_enabled != Some(false)
        || emitter.shape.is_some()
        || emitter.ring_buffer_mode != RingBufferMode::Disabled
        || !zero(&emitter.start.speed)
        || !zero(&emitter.start.gravity_modifier)
        || emitter.velocity_over_lifetime.is_some()
        || emitter.rotation_over_lifetime.is_some()
        || emitter.size_over_lifetime.is_some()
        || emitter.force.is_some()
        || emitter.limit_velocity.is_some()
        || emitter.inherit_velocity.is_some()
        || emitter.noise.is_some()
        || emitter.collision.is_some()
        || emitter.trails.is_some()
        || !emitter.sub_emitters.is_empty()
        || system.noise.is_some()
        || system.velocity_law.is_some()
        || system.rol.is_some()
        || system.limit.is_some()
        || system.force_law.is_some()
        || system.size_law.is_some()
    {
        return Err(Refused::Unsupported("child module/owner composition"));
    }
    // A child's axis of rotation on this birth path (the Initial module's +Z
    // or a record inherited from the parent) was not read, and the mesh
    // renderer turns a child without 3D rotation about it.
    if matches!(system.geometry, super::Geometry::Mesh(_)) && super::uses_rotation_3d(emitter, true) != Some(true) {
        return Err(Refused::Unsupported("child Mesh particle axis of rotation"));
    }
    assert_eq!(system.pool.len(), system.side.len());
    let old_count = system.pool.len();
    let requested = command.count as usize;
    let accepted = birth_capacity(
        old_count,
        emitter.ring_buffer_mode,
        emitter.max_particles as usize,
        requested,
    );
    if accepted != requested {
        return Err(Refused::Unsupported("child capacity overflow"));
    }
    let law = InitialLaw::from_params(&emitter.start, 0.0).map_err(Refused::Initial)?;
    let mut next = *random;
    let mut particles = Vec::with_capacity(accepted.next_multiple_of(4));
    let mut sides = Vec::with_capacity(particles.capacity());
    let mut partial_dts = Vec::with_capacity(particles.capacity());
    for offset in (0..accepted).step_by(4) {
        let timing = (0..4)
            .map(|lane| {
                command
                    .distribution
                    .timing(
                        (offset + lane) as u32,
                        command.rate_count as u32,
                        command.dt,
                        command.previous,
                        command.current,
                    )
                    .map_err(|_| Refused::InvalidCommand("birth timing"))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let group = law
            .start_group_native(
                &mut next,
                InitialGroupInput {
                    active_lanes: (accepted - offset).min(4),
                    storage_size_3d: emitter.start.size3d,
                    storage_rotation_3d: emitter.start.rotation3d,
                    birth_fraction: std::array::from_fn(|lane| timing[lane].fraction),
                    // Initial receives current broadcast; StartVelocity's separate
                    // per-lane curve time must not replace this input.
                    curve_time: [command.current; 4],
                    context: InitialContext::Autonomous,
                },
            )
            .map_err(Refused::Initial)?;
        for (i, lane) in group.lanes.into_iter().enumerate() {
            let mut age = (timing[i].dt * 100.0) * lane.inverse_lifetime;
            for step in &steps {
                age += (*step * 100.0) * lane.inverse_lifetime;
            }
            if !age.is_finite() || age > 100.0 {
                return Err(Refused::Unsupported("death during child birth/catch-up"));
            }
            // There are no position-sensitive pre modules or nonzero born
            // speed in this subset. Backtracking before pre is equivalent only
            // under these gates; keep native division/multiply operation order.
            let backtrack = (command.dt / emitter.simulation_speed) * timing[i].fraction;
            let position = std::array::from_fn(|axis| {
                command.position[axis] - command.velocity[axis] * backtrack
            });
            if position.iter().any(|v| !v.is_finite()) {
                return Err(Refused::InvalidCommand("birth position overflow"));
            }
            particles.push(Particle {
                position: crate::particle_geometry::reflect(Vec3::from_array(position)).to_array(),
                velocity: [0.0; 3],
                start_lifetime: lane.lifetime,
                inverse_lifetime: lane.inverse_lifetime,
                age_percent: 0.0,
            });
            sides.push(Side {
                rand: 0.0,
                seed: lane.seed,
                rot: lane.rotation.map(|v| v.unwrap_or(0.0)),
                size: lane.size.map(|v| v.unwrap_or(lane.size[0].unwrap())),
                gravity: 0.0,
                colour: moly_law::particle::gradient::rgba8_to_float(lane.color),
                total_velocity: [0.0; 3],
                custom_data: [[0.0; 4]; 2],
                axis: [0.0, 0.0, 1.0],
            });
            partial_dts.push(timing[i].dt);
        }
    }
    // All padded lanes, including possible catch-up death, were checked before
    // mutation. The old pool, clocks, carries and RNG have not been advanced.
    system.pool.extend(particles);
    system.side.extend(sides);
    let survivors = simulate_birth_span(system, old_count, accepted, &partial_dts, ctx);
    assert_eq!(survivors, accepted, "qualified child span must not die");
    for step in &steps {
        simulate_range(system, old_count, old_count + accepted, *step, None, ctx);
    }
    system.pool.truncate(old_count + accepted);
    system.side.truncate(old_count + accepted);
    finish_births(
        &mut system.pool,
        &mut system.side,
        &mut system.ring_cursor,
        RingBufferMode::Disabled,
        system.emitter.max_particles as usize,
        old_count,
        |_, _| {},
    );
    *random = next;
    system.born_total += accepted as u64;
    Ok(Applied {
        born: accepted,
        catch_up_steps: steps.len(),
        catch_up_remainder: catch_up.remainder(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex_bytes(value: &str) -> Vec<u8> {
        assert_eq!(value.len() % 2, 0, "hex fixture has odd length");
        value
            .as_bytes()
            .chunks_exact(2)
            .map(|pair| {
                let digit = |byte: u8| match byte {
                    b'0'..=b'9' => byte - b'0',
                    b'a'..=b'f' => byte - b'a' + 10,
                    b'A'..=b'F' => byte - b'A' + 10,
                    _ => panic!("invalid fixture hex digit"),
                };
                (digit(pair[0]) << 4) | digit(pair[1])
            })
            .collect()
    }

    fn raw(count: u64) -> (Vec<u8>, Vec<u8>) {
        let mut bytes = vec![0_u8; 0x78];
        let put32 = |bytes: &mut [u8], at: usize, value: u32| {
            bytes[at..at + 4].copy_from_slice(&value.to_le_bytes());
        };
        bytes[0x58..0x60].copy_from_slice(&count.to_le_bytes());
        bytes[0x60..0x68].copy_from_slice(&0_u64.to_le_bytes());
        for (at, value) in [
            (0x20, u32::MAX),
            (0x24, 0x3f800000),
            (0x28, 0x3f800000),
            (0x2c, 0x3f800000),
            (0x44, 0x3f800000),
            (0x48, 0x3f800000),
            (0x4c, 0x7f800000),
        ] {
            put32(&mut bytes, at, value);
        }
        for (at, value) in [
            (0x08, 1.0_f32),
            (0x0c, 2.0),
            (0x10, 3.0),
            (0x14, 0.0),
            (0x18, 0.0),
            (0x1c, 0.0),
            (0x68, 0.25),
            (0x6c, 0.25),
            (0x70, 0.5),
            (0x74, 0.0),
        ] {
            put32(&mut bytes, at, value.to_bits());
        }
        (
            bytes,
            0_f32
                .to_le_bytes()
                .into_iter()
                .chain(0_f32.to_le_bytes())
                .chain(1_f32.to_le_bytes())
                .collect(),
        )
    }

    #[test]
    fn zero_count_native_command_is_parseable_noop_metadata() {
        let (raw, emission) = raw(0);
        let command = ChildCommand::from_native_bytes(&raw, &emission).unwrap();
        assert_eq!(command.count, 0);
        assert_eq!(
            command.distribution.burst_fraction.to_bits(),
            1.0_f32.to_bits()
        );
    }

    #[test]
    fn neutral_context_requires_initialized_native_forward_vector() {
        let (mut bytes, emission) = raw(1);
        let update = ChildUpdate {
            flags: 0,
            frame_dt: 0.125,
            world_playing: true,
            upper_lifetime: 1.0,
        };
        let command = ChildCommand::from_native_bytes(&bytes, &emission).unwrap();
        assert_eq!(command.inherited_words[9], 1.0_f32.to_bits());
        assert_eq!(command.validate(update), Ok(()));
        bytes[0x44..0x48].fill(0);
        let uninitialized = ChildCommand::from_native_bytes(&bytes, &emission).unwrap();
        assert_eq!(uninitialized.validate(update),
            Err(Refused::Unsupported("non-neutral inherited context")));
    }

    #[test]
    fn parser_retains_inherited_words_and_rejects_non_exact_count() {
        let (mut raw, emission) = raw(1);
        raw[0x58..0x60].copy_from_slice(&(MAX_EXACT_COUNT + 1).to_le_bytes());
        let command = ChildCommand::from_native_bytes(&raw, &emission).unwrap();
        assert_eq!(command.inherited_words[0], u32::MAX);
        let update = ChildUpdate {
            flags: 0,
            frame_dt: 0.125,
            world_playing: true,
            upper_lifetime: 1.0,
        };
        assert_eq!(
            command.validate(update),
            Err(Refused::InvalidCommand("count outside exact range"))
        );
    }

    #[test]
    #[ignore = "set MOLY_CHILD_COMMAND_CURRENT or run from the weather-complete lane"]
    fn replays_current_flash_child_commands_into_shared_pool() {
        use serde_json::{json, Value};
        use std::path::PathBuf;

        fn number(value: &Value) -> f32 {
            let n = value.as_f64().expect("native receipt number") as f32;
            assert!(n.is_finite(), "finite native output required");
            n
        }
        fn words(value: &Value) -> ModuleRandom {
            let flat = value.as_array().expect("Initial ModuleRandom words");
            assert_eq!(flat.len(), 16);
            ModuleRandom {
                words: std::array::from_fn(|word| {
                    std::array::from_fn(|lane| {
                        u32::try_from(flat[word * 4 + lane].as_u64().unwrap()).unwrap()
                    })
                }),
            }
        }
        fn exact(actual: f32, expected: f32, label: &str) {
            assert_eq!(
                actual.to_bits(),
                expected.to_bits(),
                "{label}: {actual:?} vs native {expected:?}"
            );
        }
        fn physical_vector(actual: [f32; 3], native: &Value, label: &str) {
            assert_eq!(native.as_array().unwrap().len(), 3);
            for axis in 0..3 {
                // Source->runtime handedness changes X only. A zero direction may
                // be represented as +0 after integration or -0 after reflection;
                // this equivalence applies ONLY to position/velocity zero lanes.
                let expected = number(&native[axis]) * if axis == 0 { -1.0 } else { 1.0 };
                if actual[axis] == 0.0 && expected == 0.0 {
                    continue;
                }
                exact(actual[axis], expected, &format!("{label}[{axis}]"));
            }
        }
        fn check_pool(system: &Runtime, row: &Value, label: &str) {
            let output = &row["output"];
            let draw = &row["draw"];
            let count = output["count"].as_u64().unwrap() as usize;
            assert_eq!(system.pool.len(), count, "{label} pool count");
            assert_eq!(system.side.len(), count, "{label} side count");
            for field in ["position", "velocity", "age", "inverseLifetime", "seeds"] {
                assert_eq!(
                    output[field].as_array().unwrap().len(),
                    count,
                    "{label} native {field} length"
                );
            }
            for field in ["rotation", "size", "colors", "custom"] {
                assert_eq!(
                    draw[field].as_array().unwrap().len(),
                    count,
                    "{label} native draw.{field} length"
                );
            }
            // Exercise the existing presentation consumer too. Native draw.colors
            // is ColorModule's byte result; CustomData must be retained pre-output,
            // never recomputed at the final presentation age inside this test.
            let quads = build_quads(system, &GlobalTransform::IDENTITY);
            assert_eq!(quads.len(), count);
            for index in 0..count {
                let p = &system.pool[index];
                let side = &system.side[index];
                let lane = format!("{label} particle {index}");
                physical_vector(
                    p.position,
                    &output["position"][index],
                    &format!("{lane} position"),
                );
                physical_vector(
                    p.velocity,
                    &output["velocity"][index],
                    &format!("{lane} velocity"),
                );
                exact(
                    p.age_percent,
                    number(&output["age"][index]),
                    &format!("{lane} age"),
                );
                exact(
                    p.inverse_lifetime,
                    number(&output["inverseLifetime"][index]),
                    &format!("{lane} inverseLifetime"),
                );
                assert_eq!(
                    side.seed as u64,
                    output["seeds"][index].as_u64().unwrap(),
                    "{lane} seed"
                );
                assert_eq!(draw["rotation"][index].as_array().unwrap().len(), 3);
                for axis in 0..3 {
                    exact(
                        side.rot[axis],
                        number(&draw["rotation"][index][axis]),
                        &format!("{lane} rotation[{axis}]"),
                    );
                    // This source has size3D=false. Native draw stores one size
                    // channel; Runtime expands that one authored value to XYZ.
                    exact(
                        side.size[axis],
                        number(&draw["size"][index]),
                        &format!("{lane} expanded size[{axis}]"),
                    );
                }
                let actual_color =
                    moly_law::particle::gradient::quantize_rgba8(quads[index].colour);
                assert_eq!(draw["colors"][index].as_array().unwrap().len(), 4);
                for channel in 0..4 {
                    assert_eq!(
                        actual_color[channel] as u64,
                        draw["colors"][index][channel].as_u64().unwrap(),
                        "{lane} rendered color[{channel}]"
                    );
                }
                assert_eq!(draw["custom"][index].as_array().unwrap().len(), 8);
                let [custom1, custom2] = [quads[index].custom1, quads[index].custom2];
                for stream in 0..2 {
                    for channel in 0..4 {
                        let expected = number(&draw["custom"][index][stream * 4 + channel]);
                        exact(
                            side.custom_data[stream][channel],
                            expected,
                            &format!("{lane} persistent custom[{stream}][{channel}]"),
                        );
                        let rendered = if stream == 0 {
                            custom1[channel]
                        } else {
                            custom2[channel]
                        };
                        exact(
                            rendered,
                            expected,
                            &format!("{lane} presented custom[{stream}][{channel}]"),
                        );
                    }
                }
            }
        }
        fn source_runtime(particle: &Value, effect: &str) -> Runtime {
            // Retain the whole original particle. Select it out of the source
            // document only to avoid unrelated emitters' unsupported schema arms.
            let selected = json!({"effects": {effect: {"particles": [particle]}}});
            let mut decoded = moly_law::particle::schema::Effects::from_json_str(
                &serde_json::to_vec(&selected).unwrap(),
            )
            .expect("decode original flash source");
            assert_eq!(decoded.emitters.len(), 1);
            let emitter = decoded.emitters.remove(0);
            assert_eq!(emitter.simulation_space, SimulationSpace::Local);
            assert_eq!(emitter.shape_enabled, Some(false));
            assert!(!emitter.prewarm && !emitter.start.size3d && emitter.start.rotation3d);
            assert_eq!(emitter.max_particles, 30);
            assert_eq!(emitter.simulation_speed.to_bits(), 1.0_f32.to_bits());
            assert!(emitter.color_over_lifetime.is_some());
            assert!(emitter.custom_data.is_some());
            // The probe's catchupGravity is nonzero but the actual flash authored
            // modifier is zero. Do not overwrite the source to fit a synthetic row.
            assert!(matches!(
                emitter.start.gravity_modifier,
                MinMaxCurve::Constant(0.0)
            ));
            let mut system = test_support::runtime();
            system.node = emitter.node.clone();
            system.effect = emitter.effect.clone();
            system.kind = EffectKind::Site;
            system.gravity_law =
                moly_law::particle::gravity::Gravity::new(&emitter.start.gravity_modifier).expect("curves validated during admission");
            system.color_law = emitter
                .color_over_lifetime
                .as_ref()
                .map(moly_law::particle::color::ColorOverLifetime::from_params);
            system.custom_law = emitter
                .custom_data
                .as_ref()
                .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission"));
            system.emitter = emitter;
            system.pool.clear();
            system.side.clear();
            system.born_total = 0;
            system.died_total = 0;
            system.full_total = 0;
            system.refused_total = 0;
            system
        }
        fn replay(
            system: &mut Runtime,
            random: &mut ModuleRandom,
            row: &Value,
            update: ChildUpdate,
            context: &Context,
            label: &str,
        ) -> Applied {
            let value = &row["command"];
            let command = ChildCommand::from_native_bytes(
                &hex_bytes(value["rawHex"].as_str().unwrap()),
                &hex_bytes(value["emissionHex"].as_str().unwrap()),
            )
            .unwrap();
            assert_eq!(command.count, value["count"].as_u64().unwrap());
            for i in 0..13 {
                assert_eq!(
                    command.inherited_words[i] as u64,
                    value["inheritedWords"][i].as_u64().unwrap()
                );
            }
            assert_eq!(
                *random,
                words(&row["beforeWords"]),
                "{label} Initial RNG before"
            );
            let old_rng = system.rng.0;
            let old_clock = (
                system.playback_head.to_bits(),
                system.previous_head.to_bits(),
            );
            let old_born = system.born_total;
            let applied = apply_command(system, random, &command, update, context)
                .unwrap_or_else(|e| panic!("{label} apply: {e:?}"));
            assert_eq!(applied.born as u64, command.count, "{label} born count");
            assert_eq!(
                system.born_total - old_born,
                command.count,
                "{label} cumulative births"
            );
            assert_eq!(
                *random,
                words(&row["afterWords"]),
                "{label} Initial RNG after"
            );
            assert_eq!(system.rng.0, old_rng, "{label} legacy RNG untouched");
            assert_eq!(
                (
                    system.playback_head.to_bits(),
                    system.previous_head.to_bits()
                ),
                old_clock,
                "{label} child entry leaves autonomous clock unchanged"
            );
            assert_eq!(
                (system.died_total, system.full_total, system.refused_total),
                (0, 0, 0)
            );
            check_pool(system, row, label);
            if command.count == 0 {
                assert_eq!(
                    words(&row["beforeWords"]),
                    *random,
                    "{label} zero-count RNG no-op"
                );
                assert!(row["initialCalls"].as_array().unwrap().is_empty());
            }
            applied
        }

        let fixture_path = std::env::var_os("MOLY_CHILD_COMMAND_CURRENT")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                    .join("../../..")
                    .join("child-command-current.json")
            });
        let fixture: Value = serde_json::from_slice(&std::fs::read(fixture_path).unwrap()).unwrap();
        assert_eq!(
            fixture["sourceSha256"],
            "937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"
        );
        let rows = fixture["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 8);
        assert_eq!(fixture["summary"]["failureCount"].as_u64(), Some(0));
        // Keep the recorded source path intact; cloud replays opt into relocation.
        let effects_path = std::env::var_os("MOLY_CHILD_COMMAND_EFFECTS")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(fixture["source"]["effects"].as_str().unwrap()));
        let effects: Value =
            serde_json::from_slice(&std::fs::read(&effects_path).unwrap_or_else(|error| {
                panic!("read child effects {}: {error}", effects_path.display())
            }))
            .unwrap();
        let effect_name = fixture["source"]["effect"].as_str().unwrap();
        let node_name = fixture["source"]["node"].as_str().unwrap();
        let particle = effects["effects"][effect_name]["particles"]
            .as_array()
            .unwrap()
            .iter()
            .find(|p| p["node"].as_str() == Some(node_name))
            .expect("selected original flash particle");
        assert_eq!(particle["systemPathId"], fixture["source"]["systemPathId"]);
        let mut modules: Vec<_> = particle["system"]["sourceModules"]["enabled"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        modules.sort_unstable();
        assert_eq!(
            modules,
            [
                "ColorModule",
                "CustomDataModule",
                "EmissionModule",
                "InitialModule"
            ]
        );
        let identity = json!([1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1, 0, 0, 0, 0, 1]);
        assert_eq!(fixture["probeInputs"]["owner"], identity);
        assert_eq!(fixture["probeInputs"]["inverseChildOwner"], identity);
        assert_eq!(number(&fixture["probeInputs"]["simulationSpeed"]), 1.0);
        let context = Context {
            sky: GlobalTransform::IDENTITY,
            camera: GlobalTransform::IDENTITY,
            site: GlobalTransform::IDENTITY,
        };
        let update = ChildUpdate {
            flags: fixture["probeInputs"]["chainUpdateFlags"].as_u64().unwrap() as u32,
            frame_dt: number(&fixture["probeInputs"]["frameDt"]),
            world_playing: true,
            upper_lifetime: number(&fixture["probeInputs"]["upperLifetimeSlot"]),
        };
        let mut system = source_runtime(particle, effect_name);
        let mut random = words(&fixture["initialWords"]);
        let mut nonzero = 0;
        for (i, row) in rows.iter().enumerate() {
            let applied = replay(
                &mut system,
                &mut random,
                row,
                update,
                &context,
                &format!("row {i}"),
            );
            nonzero += usize::from(applied.born > 0);
            assert_eq!(applied.catch_up_steps, 0);
        }
        assert_eq!(nonzero, 4);
        assert_eq!(system.pool.len(), 4);
        assert_eq!(random, words(&fixture["finalWords"]));

        // Independently captured synthetic command, not a ninth RecordEmit arrival.
        // Fresh source target/RNG; this is the nonzero CustomData phase regression.
        let catchup = &fixture["explicitCatchup"];
        assert!(catchup["commandOrigin"]
            .as_str()
            .unwrap()
            .contains("not captured RecordEmit"));
        assert!(catchup["draw"]["custom"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|v| v.as_array().unwrap())
            .any(|v| number(v) != 0.0));
        let mut catchup_system = source_runtime(particle, effect_name);
        let mut catchup_random = words(&catchup["beforeWords"]);
        let applied = replay(
            &mut catchup_system,
            &mut catchup_random,
            catchup,
            ChildUpdate {
                flags: fixture["probeInputs"]["explicitCatchupUpdateFlags"]
                    .as_u64()
                    .unwrap() as u32,
                ..update
            },
            &context,
            "explicitCatchup",
        );
        assert_eq!(applied.born, 4);
        assert_eq!(applied.catch_up_steps, 1);
        exact(applied.catch_up_remainder, 0.0, "explicitCatchup remainder");
    }
}
