//! Room removal waits for its NPCs to leave before releasing the site's tree.
//! The renderer may load the next room meanwhile; all callbacks retain the
//! exact actor entities they began with.
use bevy::ecs::system::SystemState;
use bevy::prelude::*;
use super::{CharacterUnitId, NpcActions};

enum Step {
    Cancel,
    WaitFixture { real_start: f64 },
    Yield { frame: u32 },
    Delay(crate::tween_clock::Delay),
    Pick,
    Place { position: [f32; 3] },
}
struct Removal {
    site: String,
    roots: Vec<Entity>,
    actors: Vec<Entity>,
    next: usize,
    step: Step,
}
#[derive(Resource, Default)]
struct Removals(Vec<Removal>);
#[derive(Resource)]
pub struct RemovalPending;
#[derive(Resource)]
struct Random(moly_law::objective::appearance::EngineRand);
#[derive(Resource)]
struct Walkable(SystemState<super::walkable::WalkableSource<'static, 'static>>);

pub fn begin(world: &mut World, site: &str, roots: Vec<Entity>) {
    let actors = world.query::<(Entity, &NpcActions)>().iter(world)
        .filter(|(_, a)| a.site_type == site).map(|(e, _)| e).collect();
    world.init_resource::<Removals>();
    let mut removal = Removal {
        site: site.to_owned(), roots, actors, next: 0, step: Step::Cancel,
    };
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    if advance(world, &mut removal, frame, 0.0) {
        release(world, removal);
    } else {
        world.resource_mut::<Removals>().0.push(removal);
        world.insert_resource(RemovalPending);
    }
}

fn state(world: &World, actor: Entity) -> Option<u8> {
    world.get::<NpcActions>(actor).map(|a| a.current as u8)
}

fn advance(world: &mut World, run: &mut Removal, frame: u32, dt: f32) -> bool {
    loop {
        if run.actors.is_empty() { return true; }
        match &mut run.step {
            Step::Cancel => {
                if run.next == run.actors.len() {
                    run.next = 0;
                    // WARP_DELAY_TIME (1s) plus the room controller's 0.5s.
                    run.step = Step::Delay(crate::tween_clock::Delay::new(1.5, u64::from(frame)));
                    return false;
                }
                let actor = run.actors[run.next];
                if world.get_entity(actor).is_err() { run.next += 1; continue; }
                if state(world, actor).is_some_and(|s| matches!(s, 11 | 12 | 16)) {
                    if !super::gate_entries::try_cancel_current_objective(world, actor) {
                        crate::npc_objective::controller::force_stop_in_world(world, actor);
                    }
                    let real_start = world.resource::<Time<Real>>().elapsed_secs_f64();
                    run.step = Step::WaitFixture { real_start };
                } else {
                    run.step = Step::Yield { frame };
                    return false;
                }
            }
            Step::WaitFixture { real_start } => {
                let actor = run.actors[run.next];
                let attached = world.get_resource::<crate::fixture_activity_timeline::FixtureActivityTimelines>()
                    .is_some_and(|t| t.has_npc_fixture_actor(actor));
                if attached && world.resource::<Time<Real>>().elapsed_secs_f64() - *real_start < 2.0 { return false; }
                run.step = Step::Yield { frame };
                return false;
            }
            Step::Yield { frame: started } => {
                if *started == frame { return false; }
                let actor = run.actors[run.next];
                if world.get_entity(actor).is_ok() {
                    super::gate_entries::hide_and_cancel_objective(world, actor);
                    if let Some(mut actions) = world.get_mut::<NpcActions>(actor) {
                        actions.site_type = "home_site".to_owned();
                    }
                }
                run.next += 1;
                run.step = Step::Cancel;
            }
            Step::Delay(delay) => {
                if !delay.tick(u64::from(frame), dt) { return false; }
                run.step = Step::Pick;
            }
            Step::Pick => {
                if run.next == run.actors.len() { return true; }
                let actor = run.actors[run.next];
                if world.get_entity(actor).is_err() { run.next += 1; continue; }
                let cells = world.resource_scope(|world, cache: Mut<Walkable>| cache.into_inner().0.get_mut(world).cells_for("home_site"));
                let Ok(cells) = cells else { return false; };
                let position = if cells.is_empty() { [0.0; 3] } else {
                    let pick = world.resource_mut::<Random>().0.range_int(0, cells.len() as i32) as usize;
                    moly_law::objective::appearance::field_position(cells[pick])
                };
                run.step = Step::Place { position };
            }
            Step::Place { position } => {
                let actor = run.actors[run.next];
                if !world.get::<NpcActions>(actor).is_some_and(NpcActions::ready) { return false; }
                let Some(face) = super::navigation::face(world, actor) else { return false; };
                let warped = face.walk_face().sample([position[0], position[2]], moly_law::carve::AGENT_QUERY_HALF_EXTENT)
                    .and_then(|xz| face.navigation_point_at(xz));
                if let Some(at) = warped { super::force_set_position(world, actor, at); }
                super::gate_entries::show(world, actor);
                if super::gate_entries::try_cancel_current_objective(world, actor) {
                    super::gate_entries::force_update_objective(world, actor);
                }
                if let Some(unit) = world.get::<CharacterUnitId>(actor) {
                    info!("[site-remove] home placement unit={} position={:?}", unit.0, position);
                }
                run.next += 1;
                run.step = Step::Pick;
            }
        }
    }
}

fn release(world: &mut World, run: Removal) {
    crate::fixture::residency::remove_site(world, &run.site);
    world.resource_mut::<super::residency::SiteNavigation>().forget(&run.site);
    for root in run.roots { if let Ok(entity) = world.get_entity_mut(root) { entity.despawn(); } }
}

pub fn update(world: &mut World) {
    if !world.contains_resource::<Removals>() { return; }
    if !world.contains_resource::<Walkable>() {
        let cache = SystemState::<super::walkable::WalkableSource>::new(world);
        world.insert_resource(Walkable(cache));
    }
    if !world.contains_resource::<Random>() {
        let a = crate::npc_objective::platform_seed();
        let b = crate::npc_objective::platform_seed();
        world.insert_resource(Random(moly_law::objective::appearance::EngineRand { state: [a as u32, (a >> 32) as u32, b as u32, (b >> 32) as u32] }));
    }
    let frame = world.resource::<bevy::diagnostic::FrameCount>().0;
    let dt = world.resource::<crate::npc_clock::NpcClock>().delta();
    let mut pending = world.remove_resource::<Removals>().unwrap();
    let mut keep = Vec::new();
    for mut run in pending.0.drain(..) {
        if advance(world, &mut run, frame, dt) {
            release(world, run);
        } else { keep.push(run); }
    }
    pending.0 = keep;
    if pending.0.is_empty() { world.remove_resource::<RemovalPending>(); }
    world.insert_resource(pending);
}
