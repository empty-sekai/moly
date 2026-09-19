//! Lifecycle regression fixtures; not original-client visual goldens.
use super::*;
use moly_law::particle::schema::EmissionParams;
use moly_law::particle::MinMaxCurve;
use std::time::Duration;

use crate::particle_runtime::test_support::runtime;

fn lifecycle(delay: f64) -> WeatherEffectLifecycle {
    WeatherEffectLifecycle::from_effect(&serde_json::json!({"lifecycle":{
        "stopBehavior":"stopEmitting","timeUntilDestroy":delay,"delaySource":"serializedRoot"
    }})).unwrap()
}

#[test]
fn disabled_shape_matches_engine_origin_and_forward_motion() {
    // Unity 2022.3.62f2 ParticleSystem.Simulate: disabled Shape, speed 2,
    // owner (2,3,4), Euler (0,90,0). Runtime world coordinates reflect X.
    for space in [SimulationSpace::Local, SimulationSpace::World] {
        let mut system = runtime();
        system.pool.clear();
        system.side.clear();
        system.born_total = 0;
        system.emitter.simulation_space = space;
        system.emitter.start.speed = MinMaxCurve::Constant(2.0);
        // Use the same single time-zero burst as the engine probe. A rate
        // surrogate would conceal a broken initial emission boundary.
        system.emitter.emission = Some(EmissionParams {
            rate_over_time: MinMaxCurve::Constant(0.0),
            rate_over_distance: MinMaxCurve::Constant(0.0),
            bursts: vec![moly_law::particle::Burst {
                time: 0.0, count: MinMaxCurve::Constant(1.0),
                cycles: moly_law::particle::emit::BurstCycles::from_serialized(1),
                repeat_interval: 0.01, probability: 1.0,
            }],
        });
        system.node_affine = GlobalTransform::from(
            Transform::from_xyz(-2.0, 3.0, 4.0)
                .with_rotation(Quat::from_rotation_y(-std::f32::consts::FRAC_PI_2)),
        );
        let ctx = Context { sky: GlobalTransform::IDENTITY,
            camera: GlobalTransform::IDENTITY, site: GlobalTransform::IDENTITY };
        crate::particle_runtime::simulate(&mut system, 0.01, &ctx);
        assert_eq!(system.pool.len(), 1);
        let expected_velocity = if space == SimulationSpace::Local {
            [0.0, 0.0, 2.0]
        } else { [-2.0, 0.0, 0.0] };
        for (elapsed, dt) in [(0.01, 0.0), (0.25, 0.24)] {
            if dt > 0.0 {
                crate::particle_runtime::simulate(&mut system, dt, &ctx);
            }
            assert_eq!(system.born_total, 1, "initial burst must fire exactly once");
            assert_eq!(system.pool.len(), 1);
            let expected_position = if space == SimulationSpace::Local {
                [0.0, 0.0, elapsed * 2.0]
            } else { [-2.0 - elapsed * 2.0, 3.0, 4.0] };
            for axis in 0..3 {
                assert!((system.pool[0].position[axis] - expected_position[axis]).abs() < 1e-5);
                assert!((system.pool[0].velocity[axis] - expected_velocity[axis]).abs() < 1e-5);
            }
        }
    }
}

fn active(world: &mut World, delay: f64) -> (WeatherFxState, Entity) {
    let draw=world.spawn(WeatherFxDraw).id();
    (WeatherFxState { selection:None, global_identity:None,sky_stopped:false,live:vec![LiveWeatherEmitter {runtime:runtime(),draw,lifecycle:lifecycle(delay)}],
        tier:"old".into(),env_site:"home".into(),admitted:1,records:1 }, draw)
}

#[test]
fn stop_emitting_preserves_and_advances_existing_particles_without_rng_or_births() {
    let mut system=runtime();
    let before=system.pool[0].remaining_lifetime;
    let ctx=Context {sky:GlobalTransform::IDENTITY,camera:GlobalTransform::IDENTITY,site:GlobalTransform::IDENTITY};
    crate::particle_runtime::simulate_stopped(&mut system,0.25,&ctx);
    assert_eq!(system.pool.len(),1);
    assert_eq!(system.born_total,1);
    assert_eq!(system.rng.0,123);
    assert!(system.pool[0].remaining_lifetime<before);
    assert!((system.pool[0].position[0]-0.5).abs()<1e-6);
    assert!(!system.prewarmed,"Stop must not implicitly prewarm");
}

#[test]
fn retirement_keeps_particles_past_the_sky_fade_but_destroys_on_the_authored_deadline() {
    let mut app=App::new();
    app.init_resource::<Time>().init_resource::<WeatherFxRetirements>()
        .add_systems(Update,expire_retirements);
    let (mut old,draw)=active(app.world_mut(),2.0);
    // The source deadline is not scaled by this emitter's simulation speed.
    old.live[0].emitter.simulation_speed=0.1;
    app.world_mut().resource_mut::<WeatherFxRetirements>().stop(&mut old,0.0);
    assert!(old.live.is_empty());
    for delta in [0.25,1.749] {
        app.world_mut().resource_mut::<Time>().advance_by(Duration::from_secs_f64(delta));
        app.update();
        assert!(app.world().get_entity(draw).is_ok());
        assert_eq!(app.world().resource::<WeatherFxRetirements>().live.len(),1);
    }
    app.world_mut().resource_mut::<Time>().advance_by(Duration::from_millis(1));
    app.update();
    assert!(app.world().get_entity(draw).is_err());
    assert!(app.world().resource::<WeatherFxRetirements>().live.is_empty());
    // There was no camera at any point, and the particle had a 10s lifetime.
}

#[test]
fn overlapping_retirements_keep_their_own_stop_instants() {
    let mut world=World::new();
    let (mut first,_)=active(&mut world,2.0);
    let (mut second,_)=active(&mut world,3.75);
    let mut retiring=WeatherFxRetirements::default();
    retiring.stop(&mut first,10.0);
    retiring.stop(&mut second,10.5);
    assert_eq!(retiring.live.iter().map(|r|r.destroy_at).collect::<Vec<_>>(),vec![12.0,14.25]);
    assert_eq!(retiring.live.iter().map(|r|r.emitter.pool.len()).sum::<usize>(),2);
}

#[test]
fn session_disposal_clears_active_and_retired_draws() {
    let mut app=App::new();
    app.init_resource::<WeatherFxRetirements>();
    let (mut old,old_draw)=active(app.world_mut(),2.0);
    let (new,new_draw)=active(app.world_mut(),2.0);
    app.world_mut().resource_mut::<WeatherFxRetirements>().stop(&mut old,0.0);
    app.insert_resource(new);
    app.add_systems(Update,|mut commands:Commands| teardown(&mut commands));
    app.update();
    assert!(app.world().get_entity(old_draw).is_err());
    assert!(app.world().get_entity(new_draw).is_err());
    assert!(!app.world().contains_resource::<WeatherFxState>());
    assert!(app.world().resource::<WeatherFxRetirements>().live.is_empty());
}

#[test]
fn site_replacement_preserves_global_and_retiring_effects() {
    let mut app=App::new();
    app.init_resource::<WeatherFxRetirements>().init_resource::<WeatherTransition>();
    let (mut old,old_draw)=active(app.world_mut(),2.0);
    let (new,new_draw)=active(app.world_mut(),2.0);
    app.world_mut().resource_mut::<WeatherFxRetirements>().stop(&mut old,0.0);
    app.insert_resource(new);
    app.add_systems(Update,|mut commands:Commands| invalidate_site(&mut commands));
    app.update();
    assert!(app.world().get_entity(old_draw).is_ok());
    assert!(app.world().get_entity(new_draw).is_ok());
    assert!(app.world().contains_resource::<WeatherFxState>());
    assert_eq!(app.world().resource::<WeatherFxRetirements>().live.len(),1);
    assert_eq!(app.world().resource::<WeatherTransition>().site_generation,1);
}
