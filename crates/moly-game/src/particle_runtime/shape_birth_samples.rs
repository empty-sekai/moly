//! Current native Initial -> Shape -> StartVelocity boundary in the real pool.
//! The reference probe disables lifetime modules. This matches that explicit
//! boundary, not a complete snow/rain system or its prewarm/lifecycle.
use super::*;
use moly_law::particle::{Effects, seed_owner::ModuleRandom, sub_emission::{BirthBatch, BirthDistribution}};
use serde_json::{json, Value};

fn number(v: &Value) -> f32 { v.as_f64().unwrap() as f32 }
fn stream(v: &Value) -> ModuleRandom {
    assert_eq!(v.as_array().unwrap().len(),16);
    ModuleRandom { words: std::array::from_fn(|w|std::array::from_fn(|l|v[w*4+l].as_u64().unwrap() as u32)) }
}
fn source_vector(v: [f32;3]) -> [f32;3] {
    crate::particle_geometry::reflect(Vec3::from_array(v)).to_array()
}
fn source_runtime(source: &Value, old: usize, simulation: Option<SimulationSpace>, owner: Option<&Value>) -> Runtime {
    let selected=json!({"effects":{"shape-boundary":{"particles":[{"node":source["node"],"system":source["system"]}]}}});
    let mut effects=Effects::from_json_str(&serde_json::to_vec(&selected).unwrap()).unwrap();
    let mut emitter=effects.emitters.remove(0);
    // Match ShapeNative.configure: source Initial and Shape unchanged; its
    // other native module enable bytes are zero at this boundary.
    emitter.velocity_over_lifetime=None; emitter.rotation_over_lifetime=None;
    emitter.size_over_lifetime=None; emitter.color_over_lifetime=None;
    emitter.custom_data=None; emitter.noise=None; emitter.force=None;
    emitter.limit_velocity=None; emitter.inherit_velocity=None;
    emitter.collision=None; emitter.trails=None; emitter.sub_emitters.clear();
    emitter.max_particles=300;
    if let Some(space)=simulation { emitter.simulation_space=space; }
    let mut runtime=test_support::runtime();
    let mut side=runtime.side[0]; side.seed=0; side.total_velocity=[0.0;3];
    runtime.emitter=emitter; runtime.kind=EffectKind::Site;
    // The probe writes the owner matrix straight into the emitter state and
    // leaves the axis-of-rotation channel off: the full composed matrix that
    // Hierarchy scaling stores, on a non-Mesh renderer.
    runtime.geometry=test_support::source_billboard(crate::particle_geometry::Scaling::Hierarchy);
    if let Some(owner)=owner {
        let source: [f32;16]=std::array::from_fn(|i|number(&owner[i]));
        let reflected=std::array::from_fn(|i|if (i%4==0)^(i/4==0) {-source[i]} else {source[i]});
        runtime.node_affine=GlobalTransform::from(Mat4::from_cols_array(&reflected));
    }
    // Native setup_child input: old X=index, zero velocities/seeds, age10,
    // inverse lifetime0.5. No expected output is used to seed the runtime.
    runtime.pool=(0..old).map(|i|Particle {position:source_vector([i as f32,0.0,0.0]),
        velocity:[0.0;3],start_lifetime:2.0,inverse_lifetime:0.5,age_percent:10.0}).collect();
    runtime.side=vec![side;old];runtime.born_total=old as u64;
    runtime
}

fn replay(runtime: &mut Runtime, initial: &mut ModuleRandom, shape: &mut ModuleRandom,
    row: &Value, case: usize) -> usize {
    assert_eq!(*initial,stream(&row["beforeRng"]["initial"]));
    assert_eq!(*shape,stream(&row["beforeRng"]["shape"]));
    let count=row["requested"].as_u64().unwrap() as u32;
    let context=Context{sky:GlobalTransform::IDENTITY,camera:GlobalTransform::IDENTITY,site:GlobalTransform::IDENTITY};
    super::birth::start_explicit_with_shape(runtime,initial,shape,BirthBatch{count,rate_count:count,
        distribution:BirthDistribution{spacing:0.25,offset:0.0,burst_fraction:0.0}},
        number(&row["dt"]),0.25,0.25,&context).unwrap();
    assert_eq!(*initial,stream(&row["afterRng"]["initial"]),"case {case} Initial stream");
    assert_eq!(*shape,stream(&row["afterRng"]["shape"]),"case {case} Shape stream");
    let after=&row["after"];
    assert_eq!(runtime.pool.len(),after["count"].as_u64().unwrap() as usize,"case {case} count");
    for (i,particle) in runtime.pool.iter().enumerate() {
        for (field,actual) in [("positions",source_vector(particle.position)),("velocities",source_vector(particle.velocity))] {
            for axis in 0..3 {
                let expected=number(&after[field][i][axis]);
                assert!(actual[axis].to_bits()==expected.to_bits() || (actual[axis]==0.0 && expected==0.0),
                    "case {case} particle {i} {field}[{axis}] actual {:?} native {:?}",actual[axis],expected);
            }
        }
        assert_eq!(particle.age_percent.to_bits(),number(&after["age"][i]).to_bits(),"case {case} age {i}");
        assert_eq!(particle.inverse_lifetime.to_bits(),number(&after["inverseLifetime"][i]).to_bits(),"case {case} inverse {i}");
        assert_eq!(runtime.side[i].seed,after["seeds"][i].as_u64().unwrap() as u32,"case {case} seed {i}");
    }
    runtime.pool.len()
}

#[test]
#[ignore="MOLY_SHAPE_BIRTH_CURRENT must identify the current private native receipt"]
fn initial_shape_and_start_velocity_match_native_in_shared_pool() {
    let path=std::env::var_os("MOLY_SHAPE_BIRTH_CURRENT").unwrap();
    let receipt: Value=serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt["sourceSha256"],"937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9");
    assert_eq!(receipt["failureCount"],0);
    let mut cases=0;let mut snapshots=0;
    for row in receipt["rows"].as_array().unwrap() {
        let source=&receipt["sources"][row["sourceIndex"].as_u64().unwrap() as usize];
        let mut runtime=source_runtime(source,row["old"].as_u64().unwrap() as usize,None,None);
        let batches=row["batches"].as_array().unwrap();
        let mut initial=stream(&batches[0]["beforeRng"]["initial"]);
        let mut shape=stream(&batches[0]["beforeRng"]["shape"]);
        for batch in batches { snapshots+=replay(&mut runtime,&mut initial,&mut shape,batch,cases);cases+=1; }
    }
    for row in receipt["independence"].as_array().unwrap() {
        let source=&receipt["sources"][row["sourceIndex"].as_u64().unwrap() as usize];
        let mut runtime=source_runtime(source,0,None,None);let batch=&row["batch"];
        snapshots+=replay(&mut runtime,&mut stream(&batch["beforeRng"]["initial"]),
            &mut stream(&batch["beforeRng"]["shape"]),batch,cases);cases+=1;
    }
    for row in receipt["transformRows"].as_array().unwrap() {
        let space=if row["simulationSpace"]==0 {SimulationSpace::Local} else {SimulationSpace::World};
        let mut runtime=source_runtime(&receipt["sources"][0],0,Some(space),Some(&row["owner"]));
        let batch=&row["batch"];
        snapshots+=replay(&mut runtime,&mut stream(&batch["beforeRng"]["initial"]),
            &mut stream(&batch["beforeRng"]["shape"]),batch,cases);cases+=1;
    }
    assert_eq!(cases,76);
    // Refused composition must not consume either stream, advance old pool
    // age or commit the system clock. These are deliberate invalid variations.
    let mut guarded=source_runtime(&receipt["sources"][0],1,None,None);
    let mut state=super::birth::NativeBirthState {owner:None,
        initial:ModuleRandom::from_owner_seed(1729),shape:ModuleRandom::from_owner_seed(1729),
        emission:moly_law::particle::autonomous_emission::AutonomousEmissionState::initialized(
            moly_law::particle::seed_owner::ScalarRandom::from_seed(1729))};
    let state_before=state.clone();let age_before=guarded.pool[0].age_percent;
    let context=Context{sky:GlobalTransform::IDENTITY,camera:GlobalTransform::IDENTITY,site:GlobalTransform::IDENTITY};
    // Shape now rides the full slice; ring replacement still has no newborn
    // consumer and must refuse before any clock or stream is touched.
    guarded.emitter.ring_buffer_mode=moly_law::particle::RingBufferMode::LoopUntilReplaced;
    assert_eq!(super::birth::step_explicit(&mut guarded,&mut state,0.25,false,&context),
        Err(super::birth::BirthRefused::Unsupported("newborn ring replacement composition")));
    guarded.emitter.ring_buffer_mode=moly_law::particle::RingBufferMode::Disabled;
    assert_eq!(guarded.pool.len(),1);assert_eq!(guarded.pool[0].age_percent,age_before);
    assert_eq!(guarded.playback_head,0.0);assert_eq!(state.initial,state_before.initial);
    assert_eq!(state.shape,state_before.shape);assert_eq!(state.emission,state_before.emission);
    let batch=BirthBatch{count:1,rate_count:1,distribution:BirthDistribution{spacing:0.25,offset:0.0,burst_fraction:0.0}};
    // A shaped system without its independent Shape stream is refused whole.
    assert_eq!(super::birth::start_explicit(&mut guarded,&mut state.initial,
        batch,0.25,0.25,0.25,&context),Err(super::birth::BirthRefused::Unsupported("missing independent Shape stream")));
    assert_eq!(state.initial,state_before.initial);assert_eq!(state.shape,state_before.shape);
    let owner=&receipt["transformRows"][3]["owner"];
    let mut overflow=source_runtime(&receipt["sources"][0],0,Some(SimulationSpace::World),Some(owner));
    overflow.emitter.start.speed=moly_law::particle::MinMaxCurve::Constant(f32::MAX);
    assert_eq!(super::birth::start_explicit_with_shape(&mut overflow,&mut state.initial,&mut state.shape,
        batch,0.0,0.25,0.25,&context),Err(super::birth::BirthRefused::Unsupported("nonfinite birth position or velocity")));
    assert!(overflow.pool.is_empty() && overflow.side.is_empty());
    assert_eq!(overflow.born_total,0);assert_eq!(state.initial,state_before.initial);assert_eq!(state.shape,state_before.shape);
    println!("{}",json!({"birthCalls":cases,"particleSnapshots":snapshots,"failureCount":0,
        "atomicRefusalCases":3,
        "scope":"Explicit Initial/Shape/StartVelocity boundary only; all other native modules disabled by the reference probe. No weather admission/prewarm claim."}));
}
