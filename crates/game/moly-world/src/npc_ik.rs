//! Character look-at target queue and the humanoid head solve. Target
//! ownership is separate from the animation graph: animation supplies the
//! pose, then this pass applies the look-at delta before transform propagation.

use std::collections::VecDeque;
use bevy::prelude::*;

const IK_SPEED: f32 = 1.3;
const WEIGHT: f32 = 0.78;
const HEAD_WEIGHT: f32 = 0.9;
const CLAMP_WEIGHT: f32 = 0.78;

#[derive(Component, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HeadAxes {
    pub post_q: [f32; 4],
    pub neck: Option<NeckAxes>,
}

#[derive(Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NeckAxes { pub name:String, pub post_q:[f32;4] }

#[derive(Clone, Copy)]
struct Wait { frame: u32, elapsed: f32 }

#[derive(Component)]
pub struct NpcIk {
    target: Option<Entity>,
    enabled: bool,
    current: Vec3,
    last: Vec3,
    next: Vec3,
    queue: VecDeque<Vec3>,
    weight_time: f32,
    wait: Option<Wait>,
    wait_result: Option<bool>,
    head: Entity,
    hip: Entity,
}

#[derive(Component)]
struct AppliedHead { before: Quat, after: Quat }

#[derive(SystemSet, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct HeadSolve;

pub fn install(app: &mut App) {
    app.add_systems(PreUpdate, (restore_head, load_axes))
        .add_systems(PostUpdate, solve_heads.in_set(HeadSolve)
            .after(bevy::app::AnimationSystems).before(bevy::transform::TransformSystems::Propagate));
}

fn global(world: &World, entity: Entity) -> Option<GlobalTransform> {
    let mut chain = Vec::new();
    let mut current = entity;
    loop {
        chain.push(*world.get::<Transform>(current)?);
        let Some(parent) = world.get::<ChildOf>(current) else { break; };
        current = parent.parent();
    }
    let mut pose = GlobalTransform::IDENTITY;
    for transform in chain.into_iter().rev() { pose = pose.mul_transform(transform); }
    Some(pose)
}

fn initialize(world: &mut World, actor: Entity) -> bool {
    if world.get::<NpcIk>(actor).is_some() { return true; }
    let Some(toon)=world.get::<crate::character_material::ToonMaterials>(actor) else { return false; };
    let (head,hip)=(toon.head_entity(),toon.hips_entity());
    let (Some(head_pose),Some(hip_pose))=(global(world,head),global(world,hip)) else { return false; };
    let mut start=hip_pose.translation()+hip_pose.rotation()*Vec3::Z;
    start.y=head_pose.translation().y+0.2;
    world.entity_mut(actor).insert(NpcIk { target:None, enabled:false,current:start,last:Vec3::ZERO,
        next:start,queue:VecDeque::new(),weight_time:0.0,wait:None,wait_result:None,head,hip });
    true
}

pub fn set_target(world: &mut World, actor: Entity, target: Entity) -> bool {
    if !initialize(world,actor) { return false; }
    let frame=world.get_resource::<bevy::diagnostic::FrameCount>().map_or(0,|f|f.0);
    let mut ik=world.get_mut::<NpcIk>(actor).unwrap();
    ik.enabled=true; ik.target=Some(target); ik.wait=Some(Wait {frame,elapsed:0.0}); ik.wait_result=None;
    true
}

pub fn clear(world: &mut World, actor: Entity) {
    if let Some(mut ik)=world.get_mut::<NpcIk>(actor) { ik.target=None; ik.wait=None; ik.wait_result=None; }
}

/// Completion of SetIKTargetAsync: a Yield precedes the <=0.2 distance
/// predicate, whose timeout is 0.3 seconds. A timeout is a false result.
pub fn settled(world: &World, actor: Entity) -> Option<bool> {
    world.get::<NpcIk>(actor).and_then(|ik|ik.wait_result)
}

fn cubic(t:f32)->f32 { let p=t.clamp(0.0,1.0)-1.0; p*p*p+1.0 }
fn unity_normal(v:Vec3)->Vec3 { if v.length()>0.00001 {v/v.length()} else {Vec3::ZERO} }

fn complement(next:Vec3,last:Vec3,hip:Vec3)->Vec<Vec3> {
    let a=(next.z-hip.z).atan2(next.x-hip.x)*57.296;
    let b=(last.z-hip.z).atan2(last.x-hip.x)*57.296;
    if (a-b).abs()<90.0 {return vec![next];}
    let radius=next.distance(hip);
    (1..4).map(|i| {let angle=(a+(b-a)*(i as f32*0.25))*0.017453;
        Vec3::new(radius*angle.cos(),next.y,radius*angle.sin())}).collect()
}

fn queue_position(ik:&mut NpcIk,target:Vec3,hip:Vec3,hip_forward:Vec3,head_y:f32,dt:f32) {
    if let Some(last)=ik.queue.back().copied() {
        if target.distance(last)>=0.1 {ik.queue.extend(complement(target,last,hip));}
    } else {ik.queue.push_back(target);}
    if let Some(next)=ik.queue.pop_front() {ik.next=next;}
    if ik.next.distance(hip)!=1.0 {
        let dir=unity_normal(ik.next-hip); ik.next.x=hip.x+dir.x;ik.next.z=hip.z+dir.z;
    }
    ik.last=ik.queue.back().copied().unwrap_or(ik.next);
    ik.current=ik.current.lerp(ik.next,cubic(dt*IK_SPEED));
    let forward=unity_normal(Vec3::new(hip_forward.x,0.0,hip_forward.z));
    let to=unity_normal(ik.current-hip);
    let flat=Vec3::new(to.x,0.0,to.z);
    let denominator=forward.length()*flat.length();
    let angle=if denominator>=1.0e-15 {(forward.dot(flat)/denominator).clamp(-1.0,1.0).acos()*57.296} else {0.0};
    if angle>45.0 {
        let signed=if forward.z*flat.x-forward.x*flat.z>0.0 {45.0_f32} else {-45.0};
        let direction=Quat::from_rotation_y(signed.to_radians())*forward;
        ik.current=hip+direction*ik.current.distance(hip);
    }
    if (ik.current.distance(hip)-1.0).abs()>=ik.current.distance(hip).abs().max(1.0)*0.000001 {
        let dir=unity_normal(ik.current-hip);ik.current.x=hip.x+dir.x;ik.current.z=hip.z+dir.z;
    }
    ik.current.y=head_y+0.2;
}

/// Presenter update maintains the target and async wait; the bone solve is
/// scheduled after animation instead of being overwritten by it.
pub fn update(world:&mut World) {
    let dt=world.get_resource::<crate::npc_clock::NpcClock>().map_or(0.0,|clock|clock.delta());
    let frame=world.get_resource::<bevy::diagnostic::FrameCount>().map_or(0,|f|f.0);
    let actors:Vec<Entity>=world.query_filtered::<Entity,With<NpcIk>>().iter(world).collect();
    for actor in actors {
        let Some(mut ik)=world.entity_mut(actor).take::<NpcIk>() else {continue;};
        let (Some(hip),Some(head))=(global(world,ik.hip),global(world,ik.head)) else {world.entity_mut(actor).insert(ik);continue;};
        let target=ik.target.and_then(|target|global(world,target)).map(|t|t.translation());
        let active=ik.enabled && target.is_some();
        ik.weight_time=(ik.weight_time.clamp(0.0,1.0)+if active {dt} else {-dt}).clamp(0.0,1.0);
        let forward=hip.rotation()*Vec3::Z;
        let desired=if let Some(target)=target.filter(|_|active) {
            let flat=unity_normal(Vec3::new(target.x-hip.translation().x,0.0,target.z-hip.translation().z));
            let root=global(world,actor).map_or(hip.translation(),|t|t.translation());
            Vec3::new(root.x+flat.x,(head.translation().y+0.2+target.y).clamp(hip.translation().y+0.5,hip.translation().y+0.6),root.z+flat.z)
        } else {let mut at=hip.translation()+forward;at.y=head.translation().y+0.2;at};
        queue_position(&mut ik,desired,hip.translation(),forward,head.translation().y,dt);
        if let Some(wait)=ik.wait.as_mut() {
            if wait.frame!=frame {
                wait.elapsed+=dt;
                let result=if ik.current.distance(ik.last)<=0.2 {Some(true)} else if wait.elapsed>=0.3 {Some(false)} else {None};
                if let Some(result)=result {ik.wait=None;ik.wait_result=Some(result);}
            }
        }
        world.entity_mut(actor).insert(ik);
    }
}

pub fn load_axes(mut commands:Commands,packs:Query<(Entity,&crate::character::CharacterPack),Without<HeadAxes>>,
    documents:Res<Assets<moly_assets::json::JsonAsset>>) {
    for (entity,pack) in &packs {
        let Some(document)=documents.get(&pack.rig) else {continue;};
        let Ok(value)=serde_json::from_str::<serde_json::Value>(&document.0) else {continue;};
        let Some(head)=value.get("headIk") else {continue;};
        match serde_json::from_value::<HeadAxes>(head.clone()) {
            Ok(axes)=> {commands.entity(entity).insert(axes);}
            Err(error)=>error!("head IK axes are malformed: {error}"),
        }
    }
}

/// Unity's humanoid delta keeps quaternion w while scaling xyz by weight,
/// then normalizes again. It is not an angle multiplied by that weight.
fn look_delta(pivot:Vec3,reference:Vec3,reference_rotation:Quat,target:Vec3,weight:f32)->Quat {
    let source=reference-reference_rotation*(Vec3::Y*target.distance(reference))-pivot;
    let destination=target-pivot;
    let xyz=source.cross(destination);
    let w=(source.length_squared()*destination.length_squared()).sqrt()+source.dot(destination);
    let raw=Vec4::new(xyz.x,xyz.y,xyz.z,w);
    if raw.length_squared()<f32::MIN_POSITIVE {return Quat::IDENTITY;}
    let q=raw.normalize();
    let weighted=Vec4::new(q.x*weight,q.y*weight,q.z*weight,q.w).normalize();
    Quat::from_xyzw(weighted.x,weighted.y,weighted.z,weighted.w)
}

fn restore_head(mut commands:Commands,mut heads:Query<(Entity,&AppliedHead,&mut Transform)>) {
    for (entity,applied,mut transform) in &mut heads {
        if transform.rotation==applied.after {transform.rotation=applied.before;}
        commands.entity(entity).remove::<AppliedHead>();
    }
}

fn solve_heads(world:&mut World) {
    let actors:Vec<Entity>=world.query_filtered::<Entity,(With<NpcIk>,With<HeadAxes>)>().iter(world).collect();
    for actor in actors {
        let Some(ik)=world.get::<NpcIk>(actor) else {continue;};
        let (head,target,weight)=(ik.head,ik.current,WEIGHT*HEAD_WEIGHT*cubic(ik.weight_time));
        if weight<=0.0 {continue;}
        let Some(axes)=world.get::<HeadAxes>(actor).cloned() else {continue;};
        let Some(head_world)=global(world,head) else {continue;};
        let Some(local)=world.get::<Transform>(head).copied() else {continue;};
        let mut rotation=head_world.rotation();
        let post=Quat::from_array(axes.post_q);
        let mut origin=head_world.translation();
        let mut target=target;
        // Clamp weight limits the angular target before the head's three
        // squared-weight passes. The cubic transition keeps the limit soft.
        let forward=-(rotation*post*Vec3::Y);
        let desired=unity_normal(target-origin);
        let q=Quat::from_rotation_arc(forward.normalize_or_zero(),desired);
        let limit=((1.0-CLAMP_WEIGHT)*std::f32::consts::PI*0.5).cos();
        let q=if q.w.abs()<limit {
            let amount=1.0-(1.0-limit)*q.w.abs()/(limit*(1.0-q.w.abs()));
            q.lerp(if q.w<0.0 {-Quat::IDENTITY} else {Quat::IDENTITY},amount.powi(3)).normalize()
        } else {q};
        target=origin+(q*forward)*target.distance(origin);
        if let Some(neck)=axes.neck.as_ref() {
            // The optional neck is an actual humanoid ancestor; identify
            // the authored name only along the already bound head chain.
            let mut parent=world.get::<ChildOf>(head).map(ChildOf::parent);
            let mut found=None;
            while let Some(entity)=parent {
                if world.get::<Name>(entity).is_some_and(|name|name.as_str()==neck.name) {found=Some(entity);break;}
                parent=world.get::<ChildOf>(entity).map(ChildOf::parent);
            }
            if let Some(neck_entity)=found {
                if let (Some(neck_world),Some(neck_local))=(global(world,neck_entity),world.get::<Transform>(neck_entity).copied()) {
                    let delta=look_delta(neck_world.translation(),origin,rotation*post,target,weight*0.5);
                    let parent=world.get::<ChildOf>(neck_entity).and_then(|p|global(world,p.parent())).map_or(Quat::IDENTITY,|t|t.rotation());
                    let after=(parent.inverse()*delta*neck_world.rotation()).normalize();
                    world.entity_mut(neck_entity).insert(AppliedHead {before:neck_local.rotation,after});
                    world.get_mut::<Transform>(neck_entity).unwrap().rotation=after;
                    if let Some(next)=global(world,head) {origin=next.translation();rotation=next.rotation();}
                }
            }
        }
        for _ in 0..3 {rotation=(look_delta(origin,origin,rotation*post,target,weight*weight)*rotation).normalize();}
        let parent=world.get::<ChildOf>(head).and_then(|p|global(world,p.parent())).map_or(Quat::IDENTITY,|t|t.rotation());
        // The humanoid workspace delivers this solved pose. A scene-only
        // Transform write in OnAnimatorIK is overwritten by the later IK
        // output; adding an Euler clamp here would constrain a different pose.
        let after=parent.inverse()*rotation;
        world.entity_mut(head).insert(AppliedHead {before:local.rotation,after});
        world.get_mut::<Transform>(head).unwrap().rotation=after;
    }
}
