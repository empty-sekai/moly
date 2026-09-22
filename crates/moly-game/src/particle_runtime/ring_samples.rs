//! Current-native lifetime/integration/compaction observations through the live runtime.
use super::*;
use serde_json::{Value, json};

#[test]
#[ignore = "MOLY_PARTICLE_RING_SAMPLES must identify current-native observations"]
fn lifetime_and_compaction_match_native_in_shared_runtime() {
    let path=std::env::var("MOLY_PARTICLE_RING_SAMPLES").expect("native samples required");
    let data:Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let rows=data["step"].as_array().unwrap(); assert!(!rows.is_empty());
    let context=Context {sky:GlobalTransform::IDENTITY,camera:GlobalTransform::IDENTITY,site:GlobalTransform::IDENTITY};
    let f=|v:&Value|v.as_f64().unwrap() as f32;
    let mut failures=Vec::new(); let mut particles=0;
    for (row_index,row) in rows.iter().enumerate() {
        let mut system=test_support::runtime(); let side=system.side[0];
        system.pool.clear();system.side.clear();system.died_total=0;
        system.emitter.ring_buffer_mode=RingBufferMode::from_u32(row["mode"].as_u64().unwrap() as u32).unwrap();
        system.emitter.max_particles=row["maximum"].as_u64().unwrap() as u32;
        system.emitter.ring_buffer_loop_range=[f(&row["loopRange"][0]),f(&row["loopRange"][1])];
        for (index,age) in row["ages"].as_array().unwrap().iter().enumerate() {
            let inverse=f(&row["inverses"][index]);
            let mut p=Particle::born([index as f32,0.,0.],[1.,0.,0.],1./inverse);
            p.age_percent=f(age);p.inverse_lifetime=inverse;
            system.pool.push(p);
            system.side.push(Side {seed:index as u32,gravity:0.,..side});
        }
        particles+=system.pool.len();
        simulate_stopped(&mut system,f(&row["dt"]),&context);
        let expected=&row["expected"];
        let ages:Vec<_>=system.pool.iter().map(|p|p.age_percent.to_bits()).collect();
        let expected_ages:Vec<_>=expected["age"].as_array().unwrap().iter().map(|v|f(v).to_bits()).collect();
        let positions:Vec<_>=system.pool.iter().map(|p|p.position[0].to_bits()).collect();
        let expected_positions:Vec<_>=expected["positionX"].as_array().unwrap().iter().map(|v|f(v).to_bits()).collect();
        let owner_matches=system.pool.iter().zip(&system.side).all(|(p,s)|
            p.position[0].to_bits()==(s.seed as f32+f(&row["dt"])).to_bits());
        if ages!=expected_ages || positions!=expected_positions || !owner_matches
            || system.died_total!=expected["deaths"].as_array().unwrap().len() as u64 {
            failures.push(json!({"row":row_index,"ages":ages,"positions":positions,"ownerMatches":owner_matches}));
        }
    }
    let report=json!({"cases":rows.len(),"particles":particles,"failureCount":failures.len(),
        "sourceSha256":data["sourceSha256"],"firstFailures":failures.iter().take(12).collect::<Vec<_>>()});
    if let Ok(path)=std::env::var("MOLY_PARTICLE_RING_REPORT") {
        std::fs::write(path,serde_json::to_vec_pretty(&report).unwrap()).unwrap();
    }
    println!("{report}");assert!(failures.is_empty(),"{report}");
}
