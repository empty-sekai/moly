//! Runtime-side Noise phase/owner checks against the current qualified law.
//! The kernel checks use the explicit standalone Noise entry; production
//! installs Noise through install_native_birth, covered below.
use super::*;
use moly_law::particle::schema::{Effects, NoiseParams, NoiseQuality};
use moly_law::particle::MinMaxCurve;

fn params() -> NoiseParams {
    NoiseParams {
        separate_axes: false,
        strength: MinMaxCurve::Constant(0.2),
        strength_y: MinMaxCurve::Constant(0.0),
        strength_z: MinMaxCurve::Constant(0.0),
        frequency: 0.5,
        damping: false,
        octaves: 1,
        octave_multiplier: 0.5,
        octave_scale: 2.0,
        quality: NoiseQuality::High,
        dimensions: 3,
        scroll_speed: MinMaxCurve::Constant(1.0),
        remap_enabled: false,
        remap: MinMaxCurve::Constant(0.0),
        remap_y: MinMaxCurve::Constant(0.0),
        remap_z: MinMaxCurve::Constant(0.0),
        position_amount: MinMaxCurve::Constant(1.0),
        rotation_amount: MinMaxCurve::Constant(0.0),
        size_amount: MinMaxCurve::Constant(0.0),
    }
}

fn context() -> Context {
    Context {
        sky: GlobalTransform::IDENTITY,
        camera: GlobalTransform::IDENTITY,
        site: GlobalTransform::IDENTITY,
    }
}

#[test]
fn existing_range_advances_noise_once_and_adds_transient_velocity() {
    let law = NoiseLaw::from_params(&params()).unwrap();
    let mut system = test_support::runtime();
    system.emitter.emission = None;
    system.noise = Some(NoiseRuntime {
        law: law.clone(),
        state: NoiseState::default(),
        owner_seed: 0x1234_5678,
        owner: moly_law::particle::seed_owner::SeedOwner::from_serialized(0x1234_5678, false),
    });
    let before = system.pool[0].position;
    let expected = law.sample(
        NoiseState { scroll: 1.0 / 60.0 },
        crate::particle_geometry::reflect(Vec3::from_array(before)).to_array(),
        0x1234_5678,
        system.side[0].seed,
        system.pool[0].age_percent,
    );
    simulate_stopped(&mut system, 1.0 / 60.0, &context());
    let noise = system.noise.clone().unwrap();
    assert_eq!(noise.state.scroll.to_bits(), (1.0f32 / 60.0).to_bits());
    let total = system.side[0].total_velocity;
    let expected_runtime = crate::particle_geometry::reflect(Vec3::from_array(expected)).to_array();
    for axis in 0..3 {
        assert_eq!(
            total[axis].to_bits(),
            (system.pool[0].velocity[axis] + expected_runtime[axis]).to_bits(),
            "axis {axis}"
        );
    }
}

#[test]
fn noise_owner_uses_source_seed_and_keeps_manual_manager_unchanged() {
    let mut system = test_support::runtime();
    system.emitter.noise = Some(params());
    system.emitter.random_seed = Some(71);
    system.emitter.auto_random_seed = Some(false);
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(install_noise_consumer(&mut system, &mut manager).unwrap());
    let installed = system.noise.clone().unwrap();
    assert_eq!(installed.owner_seed, 71);
    assert_eq!(installed.state.scroll.to_bits(), 0);
    assert_eq!(manager.manager_words_for_test(), [17, 19, 127, 2471805022]);
}

#[test]
fn native_install_draws_one_owner_for_birth_streams_and_noise() {
    use moly_law::particle::seed_owner::{ModuleRandom, ParticleSeedManager, ScalarRandom};
    const ENTROPY: [u32; 4] = [17, 19, 127, 2471805022];
    let mut system = test_support::runtime();
    system.pool.clear();
    system.side.clear();
    system.emitter.random_seed = Some(0);
    system.emitter.auto_random_seed = Some(true);
    system.emitter.noise = Some(params());
    let mut manager = seed::SystemSeedManager::from_entropy_words(ENTROPY);
    let mut expected = ParticleSeedManager::from_entropy_words(ENTROPY);
    let seed = expected.next_system_seed();
    assert!(matches!(
        install_native_birth(&mut system, &mut manager, &SourceRoute::Ordinary, None).unwrap(),
        BirthPath::Native
    ));
    assert_eq!(manager.manager_words_for_test(), expected.words(), "exactly one shared draw");
    let birth = system.native_birth.as_ref().unwrap();
    assert_eq!(birth.owner.unwrap().seed, seed);
    assert_eq!(birth.initial, ModuleRandom::from_owner_seed(seed));
    assert_eq!(birth.shape, ModuleRandom::from_owner_seed(seed));
    assert_eq!(birth.emission.random, ScalarRandom::from_seed(seed));
    let noise = system.noise.clone().unwrap();
    assert_eq!((noise.owner_seed, noise.owner.seed), (seed, seed));
    assert_eq!(noise.state.scroll.to_bits(), 0.0_f32.to_bits());

    // A refused Noise configuration or route stays on the legacy step: it still
    // resets its seed once (one shared draw) but installs no native owner or Noise.
    for (route, octaves) in [(SourceRoute::Ordinary, 2), (SourceRoute::Procedural, 1)] {
        let mut refused = test_support::runtime();
        refused.emitter.random_seed = Some(0);
        refused.emitter.auto_random_seed = Some(true);
        refused.emitter.noise = Some(NoiseParams { octaves, ..params() });
        if route == SourceRoute::Procedural {
            refused.emitter.simulation_space = SimulationSpace::World;
        }
        let mut manager = seed::SystemSeedManager::from_entropy_words(ENTROPY);
        assert!(matches!(
            install_native_birth(&mut refused, &mut manager, &route, None).unwrap(),
            BirthPath::Legacy(_)
        ));
        let mut one = ParticleSeedManager::from_entropy_words(ENTROPY);
        let legacy_seed = one.next_system_seed();
        assert_eq!(manager.manager_words_for_test(), one.words(), "exactly one shared draw");
        assert_eq!(refused.rng.0, u64::from(legacy_seed) | (u64::from(legacy_seed) << 32));
        assert!(refused.native_birth.is_none() && refused.noise.is_none());
    }
}

#[test]
#[ignore = "MOLY_PARTICLE_NOISE_SOURCE must identify the current JP client's effects.json"]
fn current_snow_noise_source_installs_only_the_qualified_consumer() {
    let path = std::env::var("MOLY_PARTICLE_NOISE_SOURCE").unwrap();
    let bytes = std::fs::read(path).unwrap();
    let effects = Effects::from_json_str(&bytes).unwrap();
    let emitter = effects
        .emitters
        .into_iter()
        .find(|emitter| emitter.node.ends_with("/snow_pt_01"))
        .expect("current snow_pt_01");
    let params = emitter.noise.as_ref().expect("snow NoiseModule");
    let law = NoiseLaw::from_params(params).expect("qualified snow Noise subset");
    assert_eq!(emitter.prewarm, true);
    assert_eq!(emitter.play_on_awake, false);
    assert_eq!(emitter.simulation_space, SimulationSpace::Local);
    let mut system = test_support::runtime();
    system.emitter = emitter.clone();
    system.emitter.prewarm = false;
    system.emitter.play_on_awake = true;
    system.pool = vec![Particle::born([0.25, -0.5, 1.0], [0.0; 3], 5.0)];
    system.side = vec![Side {
        seed: 0x1234_5678,
        ..system.side[0]
    }];
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    let mut expected_manager =
        moly_law::particle::seed_owner::ParticleSeedManager::from_entropy_words([
            17, 19, 127, 2471805022,
        ]);
    let expected_owner = if emitter.auto_random_seed.unwrap() {
        expected_manager.next_system_seed()
    } else {
        emitter.random_seed.unwrap()
    };
    assert!(install_noise_consumer(&mut system, &mut manager).unwrap());
    assert_eq!(system.noise.clone().unwrap().owner_seed, expected_owner);
    assert_eq!(
        system.noise.clone().unwrap().law.sample(
            NoiseState { scroll: 0.0 },
            [0.25, -0.5, 1.0],
            expected_owner,
            0,
            0.0
        ),
        law.sample(NoiseState::default(), [0.25, -0.5, 1.0], expected_owner, 0, 0.0)
    );
}

#[test]
#[ignore = "MOLY_PARTICLE_NOISE_SOURCE must identify the current JP client's effects.json"]
fn source_snow_legacy_composition_smoke_is_not_native_prewarm() {
    let path = std::env::var("MOLY_PARTICLE_NOISE_SOURCE").unwrap();
    let effects = Effects::from_json_str(&std::fs::read(path).unwrap()).unwrap();
    let emitter = effects
        .emitters
        .into_iter()
        .find(|emitter| emitter.node.ends_with("/snow_pt_01"))
        .expect("current snow_pt_01");
    NoiseLaw::from_params(emitter.noise.as_ref().expect("snow NoiseModule"))
        .expect("qualified snow Noise subset");
    assert!(emitter.prewarm && emitter.looping && emitter.duration > 0.0);
    let mut system = test_support::runtime();
    system.emitter = emitter;
    system.emitter.prewarm = true;
    system.emitter.play_on_awake = true;
    system.gravity_law =
        moly_law::particle::gravity::Gravity::new(&system.emitter.start.gravity_modifier).expect("curves validated during admission");
    system.velocity_law = system
        .emitter
        .velocity_over_lifetime
        .as_ref()
        .map(|p| moly_law::particle::velocity::VelocityOverLifetime::from_params(p).expect("curves validated during admission"));
    system.rol = system
        .emitter
        .rotation_over_lifetime
        .as_ref()
        .map(|params| {
            RotationOverLifetime::from_parts(
                params.separate_axes,
                params.x.as_ref(),
                params.y.as_ref(),
                &params.curve,
            )
            .expect("snow rotation law")
        });
    system.size_law = system
        .emitter
        .size_over_lifetime
        .as_ref()
        .map(|p| moly_law::particle::size::SizeOverLifetime::from_params(p).expect("curves validated during admission"));
    system.color_law = system
        .emitter
        .color_over_lifetime
        .as_ref()
        .map(moly_law::particle::color::ColorOverLifetime::from_params);
    system.custom_law = system
        .emitter
        .custom_data
        .as_ref()
        .map(|p| moly_law::particle::custom_data::CustomData::from_params(p).expect("curves validated during admission"));
    system.pool.clear();
    system.side.clear();
    system.native_birth = None;
    let mut manager = seed::SystemSeedManager::from_entropy_words([17, 19, 127, 2471805022]);
    assert!(install_noise_consumer(&mut system, &mut manager).unwrap());
    let ctx = context();
    let steps = (12.0 / PREWARM_STEP).ceil() as usize;
    for _ in 0..steps {
        simulate(&mut system, PREWARM_STEP, &ctx);
    }
    let noise = system.noise.clone().expect("snow Noise owner");
    assert!(noise.state.scroll.is_finite());
    assert!(
        !system.pool.is_empty(),
        "prewarm should create snow particles"
    );
    assert_eq!(system.pool.len(), system.side.len());
    assert!(system.pool.iter().all(|particle| {
        particle
            .position
            .iter()
            .chain(particle.velocity.iter())
            .all(|value| value.is_finite())
    }));
    if let Some(output) = std::env::var_os("MOLY_PARTICLE_SNOW_RUNTIME_REPORT") {
        let report = serde_json::json!({
            "owner": "fx_env_sky_010_snow/root/snow_pt_01",
            "prewarmSteps": steps,
            "alive": system.pool.len(),
            "noiseScroll": noise.state.scroll,
            "ownerSeed": noise.owner_seed,
            "failureCount": 0,
            "scope": "Smoke only: 720 manual 1/60 steps with legacy Rng(123), native_birth=None and forced play_on_awake=true. Neither the production one-second weather warmup nor the current native 174-slice prewarm is exercised. No native particle oracle, Shape RNG, live owner, or renderer equivalence is proven."
        });
        std::fs::write(output, serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
}
