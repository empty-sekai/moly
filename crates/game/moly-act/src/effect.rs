//! `effect.play`: an emoticon shown at a world point on an anchor of its own,
//! taken away after `secs`.

use bevy::prelude::*;
use moly_features::act::{ActionRun, ActionSpec, ActCtx, ActPoll, Fields};
use moly_features::effect_show::PointAnchor;

use moly_features::act::{Kinds, Step};

pub fn register(app: &mut App) {
    Kinds::new()
        .step(Step::new("effect.play", false, None, |fields| Ok(Box::new(read_effect(fields)?))))
        .install(app);
    // A world-point effect's anchor outlives its step until the emote's last
    // draws are gone.
    app.add_systems(Update, retire);
}

/// One `effect.play` step.
#[derive(Debug)]
struct EffectPlay {
    name: String,
    at: [f32; 3],
    secs: f32,
}

fn read_effect(fields: &mut Fields<'_>) -> Result<EffectPlay, String> {
    let at = fields.at().to_owned();
    let name = fields.req_str("name")?.to_owned();
    let at = moly_features::act::triple(fields.req("at")?, &format!("{at}.at"))?;
    let secs = fields.opt_f32("secs")?.unwrap_or(2.0);
    Ok(EffectPlay { name, at, secs })
}

impl ActionSpec for EffectPlay {
    fn name(&self) -> &str {
        "effect.play"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        // The anchor stands for the point; the emoticon is shown on it, and
        // the anchor is hidden until it is placed with its draws.
        let anchor = world
            .spawn((
                Transform::from_translation(Vec3::from(self.at)),
                Visibility::Hidden,
                PointAnchor,
                Name::new(format!("effect {}", self.name)),
            ))
            .id();
        match moly_features::emoticon::timeline::show(world, anchor, &self.name, false) {
            Ok(lease) => Ok(Box::new(PointEmote { anchor, lease, until: ctx.now + self.secs })),
            Err(error) => {
                world.despawn(anchor);
                Err(error)
            }
        }
    }
}

/// An emoticon at a world point, until its time.
struct PointEmote {
    anchor: Entity,
    lease: moly_features::emoticon::timeline::EmoteLease,
    until: f32,
}

impl ActionRun for PointEmote {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if ctx.now < self.until {
            return ActPoll::Running(format!("{:.1} s left", self.until - ctx.now));
        }
        let at = world.get::<Transform>(self.anchor).map_or(Vec3::ZERO, |transform| transform.translation);
        moly_features::emoticon::timeline::hide(world, self.lease, false);
        // The anchor goes once the emote's last draws have: a child is not
        // removed with its parent.
        let anchor = self.anchor;
        world.commands().queue(move |world: &mut World| deactivate(world, anchor));
        ActPoll::Done(format!("shown at ({:.2}, {:.2}, {:.2})", at.x, at.y, at.z))
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // The emote goes at once and its anchor with it.
        moly_features::emoticon::timeline::hide(world, self.lease, true);
        deactivate(world, self.anchor);
    }
}

/// Retires the anchor of a world-point effect once the step is over: its
/// emote's draws may still be in the presenter's commands, so it is hidden
/// at once (what is left under it draws next frame and not), and it is
/// despawned once its draws are gone.
fn deactivate(world: &mut World, anchor: Entity) {
    let since = world.resource::<Time>().elapsed_secs();
    let Ok(mut entity) = world.get_entity_mut(anchor) else {
        return;
    };
    if let Some(mut visibility) = entity.get_mut::<Visibility>() {
        *visibility = Visibility::Hidden;
    }
    entity.insert(Retire { since });
}

/// Seconds an anchor that has no draws yet waits before it goes.
const RETIRE_LONELY_SECS: f32 = 0.25;

/// An effect's anchor whose emote has gone, and when that was found.
#[derive(Component, Clone, Copy)]
struct Retire {
    since: f32,
}

/// Update: each retired anchor goes once no draw of an emote hangs under it
/// any more.
fn retire(mut commands: Commands, time: Res<Time>, anchors: Query<(Entity, &Retire)>, draws: Query<&ChildOf, With<moly_features::emoticon::EmoteDraw>>) {
    let held: std::collections::HashSet<Entity> = draws.iter().map(ChildOf::parent).collect();
    let now = time.elapsed_secs();
    for (entity, retire) in &anchors {
        if !held.contains(&entity) && now - retire.since >= RETIRE_LONELY_SECS {
            commands.entity(entity).despawn();
        }
    }
}
