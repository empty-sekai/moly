//! Synthetic shared-runtime fixtures. No original-client assets.
use super::*;
use crate::billboard::{Alignment, SizeClamp};
use moly_law::particle::schema::{StartParams, EmissionParams};
use moly_law::particle::{MinMaxCurve, MinMaxGradient, RingBufferMode};

pub(crate) fn runtime() -> Runtime {
    let zero = MinMaxCurve::Constant(0.0);
    let one = MinMaxCurve::Constant(1.0);
    Runtime {
        node: "test".into(), effect: "test".into(),
        emitter: EmitterParams {
            effect:"test".into(),node:"test".into(), duration:5.0, looping:true, prewarm:true,
            play_on_awake:true, simulation_speed:1.0, simulation_space:SimulationSpace::Local,
            start_delay:zero.clone(),ring_buffer_mode:RingBufferMode::Disabled,
            ring_buffer_loop_range:[0.0,1.0], max_particles:100,
            start:StartParams { lifetime:MinMaxCurve::Constant(10.0),speed:zero.clone(),size:one,
                size_y:None,size_z:None,size3d:false,rotation:zero.clone(),rotation_x:None,rotation_y:None,rotation3d:false,
                color:MinMaxGradient::Color([1.0;4]),gravity_modifier:zero.clone() },
            emission:Some(EmissionParams { rate_over_time:MinMaxCurve::Constant(100.0),
                rate_over_distance:zero,bursts:Vec::new() }),
            shape:None,shape_enabled:Some(false),velocity_over_lifetime:None,color_over_lifetime:None,
            size_over_lifetime:None,rotation_over_lifetime:None,limit_velocity:None,
            custom_data:None,unmapped:Vec::new(),
        },
        kind:EffectKind::Camera,camera_rotation:false,node_affine:GlobalTransform::IDENTITY,
        mesh:Handle::default(),anchor:None,ring_cursor:0,
        geometry:crate::particle_runtime::Geometry::Billboard {alignment:Alignment::View,
            clamp:SizeClamp { min_size:0.0,max_screen_fraction:1.0 },pivot:[0.0;3]},
        pool:vec![Particle::born([0.0;3],[2.0,0.0,0.0],10.0)],
        side:vec![Side {rand:0.5,seed:123,rot:[0.0;3],size:[1.0;3],gravity:0.0,colour:[1.0;4],total_velocity:[0.0;3]}],
        emission:EmissionState::default(),playback_head:0.0,previous_head:0.0,emission_started:false,rng:Rng(123),
        prewarmed:false,cone_angle:None,rol:None,limit:None,velocity_law:None,size_law:None,color_law:None,
        born_total:1,died_total:0,full_total:0,refused_total:0,
    }
}
