use super::*;
use moly_law::particle::{MinMaxCurve, gravity::Gravity};
use serde_json::{Value,json};

#[test]
#[ignore = "MOLY_PARTICLE_GRAVITY_SAMPLES must identify external native observations"]
fn gravity_matches_native_in_shared_runtime() {
    let path=std::env::var("MOLY_PARTICLE_GRAVITY_SAMPLES").expect("native samples required");
    let data:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows=data["rows"].as_array().unwrap();assert!(!rows.is_empty());
    let f=|v:&Value|v.as_f64().unwrap() as f32;
    let context=Context {sky:GlobalTransform::IDENTITY,camera:GlobalTransform::IDENTITY,site:GlobalTransform::IDENTITY};
    let reflection=Mat4::from_scale(Vec3::new(-1.,1.,1.));
    let mut failures=Vec::new();let mut max_error=0.0_f32;
    for (index,row) in rows.iter().enumerate() {
        let mut system=test_support::runtime();let side=system.side[0];
        system.pool.clear();system.side.clear();
        let curve=if row["mode"]==0 {MinMaxCurve::Constant(f(&row["high"]))}
            else {MinMaxCurve::TwoConstants {min:f(&row["low"]),max:f(&row["high"])}};
        system.gravity_law=Gravity::new(&curve).unwrap();
        // Native samples supply the time seen by InitialModule after Tick.
        // The runtime fixture starts one step earlier; reference vectors stay
        // unchanged when the stopped path advances its system clock.
        let dt=f(&row["dt"]);
        system.playback_head=f(&row["time"])-dt;system.emitter.duration=f(&row["duration"]);
        system.emitter.simulation_space=if row["simulation"]==1 {SimulationSpace::World}else{SimulationSpace::Local};
        let inverse=Mat4::from_cols_array(&std::array::from_fn(|i|f(&row["inverse"][i])));
        system.node_affine=GlobalTransform::from(reflection*inverse.inverse()*reflection);
        for seed in row["seeds"].as_array().unwrap() {
            system.pool.push(Particle::born([0.;3],[0.;3],100.));
            system.side.push(Side {seed:seed.as_u64().unwrap() as u32,..side});
        }
        simulate_stopped(&mut system,dt,&context);
        for (lane,p) in system.pool.iter().enumerate() {
            let velocity=crate::particle_geometry::reflect(Vec3::from_array(p.velocity));
            let position=crate::particle_geometry::reflect(Vec3::from_array(p.position));
            let expected:Vec3=Vec3::from_array(std::array::from_fn(|axis|f(&row["expected"][lane][axis])));
            let error=(velocity-expected).abs().max_element().max((position-expected*dt).abs().max_element());
            max_error=max_error.max(error);
            if !error.is_finite() || error>0.00002 {
                failures.push(json!({"row":index,"lane":lane,"error":error,"velocity":velocity.to_array(),"expected":expected.to_array()}));
            }
        }
    }
    let report=json!({"cases":rows.len(),"particles":rows.len()*4,"maximumError":max_error,
        "failureCount":failures.len(),"firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path)=std::env::var("MOLY_PARTICLE_GRAVITY_REPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");assert!(failures.is_empty(),"{report}");
}
