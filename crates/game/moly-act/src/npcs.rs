//! The characters under a program: taken from their AI (`npc.take`), walked
//! with their own navigation and walk law, turned, shown an emoticon or a
//! tweet, playing a furniture activity at a placed fixture; a conversation
//! played at once (`npc.play_talk`) or handed to the AI to build on its next
//! decision (`npc.talk`); and a character stood at a world point at once.
//!
//! A taken character is held by the run that took it, in the hub's
//! [`moly_features::act::Taken`], and given back to its AI when the run ends.

use bevy::prelude::*;
use moly_features::act::{targets, ActCtx, ActPoll, ActionRun, ActionSpec, Fields, FixtureRef, Kinds, Leaving, Step, Taken, Target};
use moly_features::npc::{CharacterUnitId, NpcAction, NpcActions, RouteStops};
use moly_features::npc_fixture_activity::preview::{self, PreviewPhase, DIRECTOR_TICKET};
use moly_features::npc_objective::{DirectedTalks, ObjectiveMind};
use moly_features::npc_take::{self, facing, standing, walk, walk_outcome, DirectedOwned};
use moly_features::server_panel::VisitingCharacters;

use crate::companion::Showing;
use crate::Requested;

pub fn install(app: &mut App) {
    Kinds::new()
        .step(Step::new("npc.take", false, None, |fields| Ok(Box::new(Take { unit: fields.req_u32("unit")? }))))
        .step(Step::new("npc.walk_to", false, Some(60.0), |fields| Ok(Box::new(read_walk(fields)?))))
        .step(Step::new("npc.face", false, None, |fields| Ok(Box::new(read_face(fields)?))))
        .step(Step::new("npc.emote", false, None, |fields| Ok(Box::new(read_emote(fields)?))))
        .step(Step::new("npc.use", false, Some(300.0), |fields| Ok(Box::new(read_use(fields)?))))
        .step(Step::new("npc.tweet", false, None, |fields| Ok(Box::new(read_tweet(fields)?))))
        .step(Step::new("npc.talk", false, Some(60.0), |fields| Ok(Box::new(read_talk(fields)?))))
        .step(Step::new("npc.release", false, None, |fields| Ok(Box::new(Release { unit: fields.req_u32("unit")? }))))
        .step(Step::new("npc.play_talk", false, Some(20.0), |fields| Ok(Box::new(read_play_talk(fields)?))))
        .step(Step::new("npc.warp_to", false, Some(20.0), |fields| Ok(Box::new(read_warp(fields)?))))
        .install(app);
}

/// `npc.take`: the character of `unit` held, its AI's decisions ended.
#[derive(Debug)]
struct Take {
    unit: u32,
}

impl ActionSpec for Take {
    fn name(&self) -> &str {
        "npc.take"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        take_npc(world, ctx.ticket, unit)?;
        Ok(Box::new(Requested { note: format!("character {unit} taken") }))
    }
}

/// `npc.release`: the character of `unit` given back to its AI.
#[derive(Debug)]
struct Release {
    unit: u32,
}

impl ActionSpec for Release {
    fn name(&self) -> &str {
        "npc.release"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        if !Taken::untake(world, unit) {
            return Err(format!("character {unit} is not taken by this script"));
        }
        Ok(Box::new(Requested { note: format!("character {unit} released") }))
    }
}

fn is_taken(world: &World, unit: u32) -> bool {
    world.get_resource::<Taken>().is_some_and(|taken| taken.counts_any(unit))
}

/// Takes the character of `unit` for the run `ticket`; a character taken
/// already is the same take.
fn take_npc(world: &mut World, ticket: u64, unit: u32) -> Result<Entity, String> {
    Taken::take(world, ticket, unit).map(|(entity, _)| entity)
}

/// The body of `unit`, when it is taken.
fn taken(world: &mut World, unit: u32) -> Result<Entity, String> {
    if !is_taken(world, unit) {
        return Err(format!("character {unit} is not taken by this script (npc.take first)"));
    }
    Taken::body(world, unit)
}

/// `npc.walk_to`: the taken character of `unit` walking to a target with its
/// own navigation, ending within `within` metres of it.
#[derive(Debug)]
struct WalkTo {
    unit: u32,
    to: Target,
    within: f32,
    rests: bool,
}

fn read_walk(fields: &mut Fields<'_>) -> Result<WalkTo, String> {
    let at = fields.at().to_owned();
    Ok(WalkTo {
        unit: fields.req_u32("unit")?,
        to: moly_features::act::target(fields.req("to")?, &at)?,
        within: fields.opt_f32("within")?.unwrap_or(0.3),
        rests: fields.opt_bool("rests")?.unwrap_or(false),
    })
}

impl ActionSpec for WalkTo {
    fn name(&self) -> &str {
        "npc.walk_to"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = taken(world, self.unit)?;
        let goal = targets::resolve(world, &self.to)?;
        walk(world, entity, Vec2::new(goal.x, goal.z), self.rests)?;
        Ok(Box::new(WalkRun { entity, unit: self.unit, goal, within: self.within }))
    }
}

/// A character's walk until it arrives or stops short: the goal is the
/// target's place at the step's start, as a walk to a floor point goes to
/// the point it names.
struct WalkRun {
    entity: Entity,
    unit: u32,
    goal: Vec3,
    within: f32,
}

impl ActionRun for WalkRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        let at = world.get::<GlobalTransform>(self.entity).map(GlobalTransform::translation);
        let Some(at) = at else {
            return ActPoll::Failed(format!("character {} is gone", self.unit));
        };
        let distance = targets::flat(at, self.goal);
        match walk_outcome(world, self.entity) {
            Some(true) => ActPoll::Done(format!("arrived at {distance:.2} m")),
            _ if distance <= self.within => {
                // Close enough: the character stops where it is.
                moly_features::npc::stop_for_external_activity(world, self.entity);
                ActPoll::Done(format!("within {distance:.2} m; stopped there"))
            }
            Some(false) => ActPoll::Failed(format!("stopped short at {distance:.2} m")),
            None => ActPoll::Running(format!("{distance:.2} m to go")),
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        moly_features::npc::stop_for_external_activity(world, self.entity);
    }
}

/// `npc.face`: the taken character of `unit` turned toward a target over
/// `secs`, once it stands.
#[derive(Debug)]
struct Face {
    unit: u32,
    to: Target,
    secs: f32,
}

fn read_face(fields: &mut Fields<'_>) -> Result<Face, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let to = moly_features::act::target(fields.req("to")?, &at)?;
    let secs = fields.opt_f32("secs")?.unwrap_or(1.0);
    Ok(Face { unit, to, secs })
}

impl ActionSpec for Face {
    fn name(&self) -> &str {
        "npc.face"
    }

    fn hand(&self) -> bool {
        false
    }

    fn timeout(&self) -> Option<f32> {
        Some(self.secs + 5.0)
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = taken(world, self.unit)?;
        let towards = targets::resolve(world, &self.to)?;
        Ok(Box::new(FaceRun { entity, unit: self.unit, towards, secs: self.secs, started: false }))
    }
}

/// A character's turn, once it stands.
struct FaceRun {
    entity: Entity,
    unit: u32,
    towards: Vec3,
    secs: f32,
    started: bool,
}

impl ActionRun for FaceRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        if world.get_entity(self.entity).is_err() {
            return ActPoll::Failed(format!("character {} is gone", self.unit));
        }
        if !self.started {
            // The turn starts once the character stands.
            if !standing(world, self.entity) {
                return ActPoll::Running(format!("waiting for character {} to stand", self.unit));
            }
            if let Err(error) = npc_take::face(world, self.entity, self.towards, self.secs) {
                return ActPoll::Failed(error);
            }
            self.started = true;
            return ActPoll::Running(format!("character {} turning", self.unit));
        }
        if facing(world, self.entity) {
            ActPoll::Running(format!("character {} turning", self.unit))
        } else {
            ActPoll::Done(String::new())
        }
    }

    fn stop(&mut self, _world: &mut World, _ctx: &mut ActCtx) {}
}

/// `npc.emote`: an emoticon shown over a character for `secs`.
#[derive(Debug)]
struct Emote {
    unit: u32,
    name: String,
    secs: f32,
    use_root: bool,
}

fn read_emote(fields: &mut Fields<'_>) -> Result<Emote, String> {
    Ok(Emote {
        unit: fields.req_u32("unit")?,
        name: fields.req_str("name")?.to_owned(),
        secs: fields.opt_f32("secs")?.unwrap_or(2.0),
        use_root: fields.opt_bool("use_root")?.unwrap_or(false),
    })
}

impl ActionSpec for Emote {
    fn name(&self) -> &str {
        "npc.emote"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = targets::npc(world, self.unit)?;
        let lease = moly_features::emoticon::timeline::show(world, entity, &self.name, self.use_root)?;
        Ok(Box::new(Showing { lease, until: ctx.now + self.secs, companion: None }))
    }
}

/// `npc.tweet`: a tweet of the master shown over a taken character for
/// `secs`.
#[derive(Debug)]
struct Tweet {
    unit: u32,
    tweet: i32,
    secs: f32,
}

fn read_tweet(fields: &mut Fields<'_>) -> Result<Tweet, String> {
    Ok(Tweet {
        unit: fields.req_u32("unit")?,
        tweet: fields.req_i32("tweet")?,
        secs: fields.opt_f32("secs")?.unwrap_or(3.0),
    })
}

impl ActionSpec for Tweet {
    fn name(&self) -> &str {
        "npc.tweet"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = taken(world, self.unit)?;
        show_tweet(world, entity, self.tweet)?;
        Ok(Box::new(TweetRun { entity, until: ctx.now + self.secs }))
    }
}

/// Shows the master's tweet `tweet` over a taken character, as its tweet
/// state shows one.
fn show_tweet(world: &mut World, entity: Entity, tweet: i32) -> Result<String, String> {
    if world.get::<DirectedOwned>(entity).is_none() {
        return Err("take the character before it tweets (npc.take)".into());
    }
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    let text = world
        .get_resource::<moly_features::npc_tweet::TweetTables>()
        .ok_or("the tweet master is not loaded")?
        .tweet(tweet)
        .map(|row| row.text.clone())
        .ok_or_else(|| format!("the tweet master has no row {tweet}"))?;
    let site_type = site_type(world, entity);
    world.write_message(moly_features::npc_state::TweetHudEvent::Show { npc: entity, unit, tweet_id: tweet, text, site_type });
    Ok(format!("tweet {tweet} shown over character {unit}"))
}

/// A character's tweet balloon until its time.
struct TweetRun {
    entity: Entity,
    until: f32,
}

impl ActionRun for TweetRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        if ctx.now >= self.until {
            hide_tweet(world, self.entity);
            ActPoll::Done(String::new())
        } else {
            ActPoll::Running(format!("{:.1} s left", self.until - ctx.now))
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        hide_tweet(world, self.entity);
    }
}

/// Shows the tweet balloon of the text `text` over the entity `entity`, with
/// the tweet id `id` for the balloon's log.
pub fn show_balloon(world: &mut World, entity: Entity, text: String, id: i32) {
    let unit = unit_of(world, entity);
    let site_type = site_type(world, entity);
    world.write_message(moly_features::npc_state::TweetHudEvent::Show { npc: entity, unit, tweet_id: id, text, site_type });
}

/// Hides the tweet balloon of a character.
pub fn hide_tweet(world: &mut World, entity: Entity) {
    if world.get_entity(entity).is_err() {
        world.write_message(moly_features::npc_state::TweetHudEvent::Retire { npc: entity });
        return;
    }
    let unit = unit_of(world, entity);
    let site_type = site_type(world, entity);
    world.write_message(moly_features::npc_state::TweetHudEvent::Hide { npc: entity, unit, site_type });
}

/// The unit a character or a companion body stands for.
fn unit_of(world: &World, entity: Entity) -> u32 {
    world
        .get::<CharacterUnitId>(entity)
        .map(|unit| unit.0)
        .or_else(|| world.get::<moly_features::companion::Companion>(entity).map(|companion| companion.unit))
        .unwrap_or(0)
}

fn site_type(world: &World, entity: Entity) -> Option<i32> {
    world
        .get::<NpcActions>(entity)
        .and_then(|actions| moly_features::npc::residency::site_type_value(&actions.site_type))
}

/// `npc.use`: one of the taken character's authored no-talk activities at a
/// placed fixture.
#[derive(Debug)]
struct Use {
    unit: u32,
    fixture: FixtureRef,
    activity: Option<i32>,
    variant: Option<u32>,
    rests: bool,
}

fn read_use(fields: &mut Fields<'_>) -> Result<Use, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let fixture = moly_features::act::fixture_ref(fields.req("fixture")?, &at)?;
    let activity = fields.opt_i32("activity")?;
    let variant = fields.opt_u32("variant")?;
    if variant == Some(0) {
        return Err(format!("{at}: variant counts from 1"));
    }
    let rests = fields.opt_bool("rests")?.unwrap_or(false);
    Ok(Use { unit, fixture, activity, variant, rests })
}

impl ActionSpec for Use {
    fn name(&self) -> &str {
        "npc.use"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = taken(world, self.unit)?;
        let ticket = begin_activity(world, entity, &self.fixture, self.activity, self.variant, self.rests)?;
        Ok(Box::new(UseRun { ticket }))
    }
}

/// Starts one of a taken character's authored no-talk activities at a placed
/// fixture through the furniture owner its AI uses (the walk there, the
/// timeline, the exit). Without `rests` the approach drops the walk law's
/// rest stops, as `npc.walk_to` does. Answers the activity's ticket.
fn begin_activity(
    world: &mut World,
    entity: Entity,
    fixture: &FixtureRef,
    activity: Option<i32>,
    variant: Option<u32>,
    rests: bool,
) -> Result<u64, String> {
    use moly_features::fixture_activity_data::{ActivityOrigin, FixtureActivityTables};
    use moly_features::fixture_activity_state::{FixtureActivityIdentity, FixtureTarget};

    if world.get::<DirectedOwned>(entity).is_none() {
        return Err("take the character before it uses a fixture (npc.take)".into());
    }
    let ticket = next_ticket(world);
    let unit = world.get::<CharacterUnitId>(entity).map_or(0, |unit| unit.0);
    let from = world.get::<GlobalTransform>(entity).map_or(Vec3::ZERO, GlobalTransform::translation);
    let mut placed: Vec<(Entity, String, i32, f32)> = world
        .query::<(Entity, &FixtureActivityIdentity, Option<&GlobalTransform>)>()
        .iter(world)
        .filter(|(_, identity, _)| match fixture {
            FixtureRef::Uid(uid) => identity.uid == *uid,
            FixtureRef::Id(id) => identity.master_id == *id,
        })
        .map(|(instance, identity, at)| {
            let distance = at.map_or(f32::INFINITY, |at| targets::flat(at.translation(), from));
            (instance, identity.uid.clone(), identity.master_id, distance)
        })
        .collect();
    placed.sort_by(|a, b| a.3.total_cmp(&b.3).then_with(|| a.1.cmp(&b.1)));
    let Some((fixture_entity, uid, master, _)) = placed.into_iter().next() else {
        return Err(format!("no placed fixture {fixture:?} on this site"));
    };
    let (tables, store) = {
        let tables = world.get_resource::<FixtureActivityTables>().ok_or("the furniture activity tables are not loaded")?;
        let store = world.get_resource::<moly_features::talk::TalkStore>().ok_or("the talk store is not loaded")?;
        (tables, store)
    };
    let mine: Vec<_> = tables
        .catalogue_activities(store)
        .into_iter()
        .filter(|spec| spec.unit == unit && matches!(spec.key.origin, ActivityOrigin::NoTalk(_)))
        .collect();
    let spec = mine
        .iter()
        .filter(|spec| spec.fixture_id == master)
        .filter(|spec| activity.is_none_or(|id| spec.key.source_id() == id))
        .find(|spec| variant.is_none_or(|variant| spec.variant == variant as usize))
        .cloned();
    let Some(spec) = spec else {
        let mut offered: Vec<String> = mine
            .iter()
            .map(|spec| format!("{} at {} ({} variants)", spec.key.source_id(), spec.fixture_id, spec.variants))
            .collect();
        offered.dedup();
        offered.truncate(24);
        return Err(format!(
            "character {unit} has no activity {} variant {} at fixture {master}; its activities: [{}]",
            activity.map_or("(any)".into(), |id| id.to_string()),
            variant.map_or("(first)".into(), |variant| variant.to_string()),
            offered.join(", ")
        ));
    };
    let target = FixtureTarget { entity: fixture_entity, uid: uid.clone() };
    preview::start(world, spec.key, target, ticket);
    if let Some((PreviewPhase::Failed, error)) = activity_phase(world, ticket) {
        end_activity(world, ticket, false);
        return Err(error.unwrap_or_else(|| "the activity was refused".into()));
    }
    let dropped = match world.get_mut::<RouteStops>(entity) {
        Some(mut route) if !rests => route.drop_rests(),
        _ => 0,
    };
    info!(
        "[director] character {unit}: activity {} variant {}/{} at fixture {master} ({uid}); {dropped} rest stops dropped",
        spec.key.source_id(),
        spec.variant,
        spec.variants
    );
    Ok(ticket)
}

/// The ticket a new activity takes: the director's own, above every record
/// the previews hold.
fn next_ticket(world: &mut World) -> u64 {
    let held: u64 = world
        .get_resource::<preview::DirectorPreviews>()
        .map_or(0, |ledger| ledger.0.len() as u64);
    DIRECTOR_TICKET | (held + 1)
}

/// The phase of the activity started under `ticket`, with its last note.
fn activity_phase(world: &World, ticket: u64) -> Option<(PreviewPhase, Option<String>)> {
    world.get_resource::<preview::DirectorPreviews>()?.0.get(&ticket).cloned()
}

/// Forgets the activity of `ticket`; `cancel` also ends its session if it
/// still runs.
fn end_activity(world: &mut World, ticket: u64, cancel: bool) {
    if cancel {
        preview::cancel(world, ticket);
    }
    if let Some(mut ledger) = world.get_resource_mut::<preview::DirectorPreviews>() {
        ledger.0.remove(&ticket);
    }
}

/// A furniture activity until its phase ends.
struct UseRun {
    ticket: u64,
}

impl ActionRun for UseRun {
    fn poll(&mut self, world: &mut World, ctx: &mut ActCtx) -> ActPoll {
        let ticket = self.ticket;
        let poll = match activity_phase(world, ticket) {
            None => ActPoll::Failed("the activity's record is gone".into()),
            Some((PreviewPhase::Completed, _)) => ActPoll::Done("the activity completed".into()),
            Some((PreviewPhase::Cancelled, _)) => ActPoll::Failed("the activity was cancelled".into()),
            Some((PreviewPhase::Failed, error)) => ActPoll::Failed(error.unwrap_or_else(|| "the activity failed".into())),
            Some((phase, note)) => {
                return ActPoll::Running(match note {
                    Some(note) => format!("{phase:?}: {note}"),
                    None => format!("{phase:?}"),
                });
            }
        };
        // The card is what the run's end ends, should the step be stopped
        // first; an activity that ended here is forgotten at once.
        ctx.card.lay(Leaving::Activity(ticket));
        end_activity(world, ticket, false);
        poll
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        end_activity(world, self.ticket, true);
    }
}

/// `npc.talk`: the character of `unit` handed to its AI with a talk to open
/// on its next decision; the step ends once that talk is built or refused.
#[derive(Debug)]
struct Talk {
    unit: u32,
    talk: i32,
}

fn read_talk(fields: &mut Fields<'_>) -> Result<Talk, String> {
    Ok(Talk { unit: fields.req_u32("unit")?, talk: fields.req_i32("talk")? })
}

impl ActionSpec for Talk {
    fn name(&self) -> &str {
        "npc.talk"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let entity = open_talk(world, self.unit, self.talk)?;
        // The character is its AI's again now; the card covers a stop before
        // its decision.
        Taken::hand_back(world, entity);
        ctx.card.lay(Leaving::Talk(entity));
        Ok(Box::new(TalkRun { entity }))
    }
}

/// Hands the character of `unit` the talk `talk` to open on its next
/// decision, which builds that talk instead of drawing one (see
/// `npc_objective::DirectedTalks`). A taken character goes back to its AI
/// with no forced update owed; any other has its current objective
/// cancelled. Either way the next decision skips the Rest pause. The talk's
/// other speakers must be free, so one the director holds is refused.
fn open_talk(world: &mut World, unit: u32, talk: i32) -> Result<Entity, String> {
    let entity = targets::npc(world, unit)?;
    let speakers = {
        let tables = world
            .get_resource::<moly_features::fixture_activity_data::FixtureActivityTables>()
            .ok_or("the furniture activity tables are not loaded")?;
        let master = tables.talk_master(talk).ok_or_else(|| format!("talk {talk} has no master row"))?;
        tables.unit_ids_of_group(master.unit_group_id)?
    };
    if speakers.first() != Some(&unit) {
        return Err(format!("talk {talk} is opened by its first speaker, not by character {unit} (speakers {speakers:?})"));
    }
    for &speaker in &speakers[1..] {
        let Ok(other) = targets::npc(world, speaker) else { continue };
        if world.get::<DirectedOwned>(other).is_some() {
            return Err(format!("character {speaker} speaks in talk {talk} and the director holds it (npc.release first)"));
        }
        // A member busy with its own activity keeps CanBeRecruited false for
        // as long as that activity lasts, so the gathering would wait out its
        // clock and the talk would never start. The step asked for this
        // talk, so the member's activity is cancelled the way the entry's
        // own placement cancels one; its next decision joins the plan.
        moly_features::npc::gate_entries::try_cancel_current_objective(world, other);
        moly_features::npc::stop_for_external_activity(world, other);
    }
    // Only the conversation's cast stands on the site: the roster is set to
    // this talk's speakers and rebuilt, so what is on screen is the two or
    // three characters that speak and nobody else. The caller asks again once
    // they stand.
    let cast: Vec<u32> = speakers.to_vec();
    info!(
        "[director] play_talk {talk}: roster {:?} vs cast {:?}",
        world.get_resource::<VisitingCharacters>().map(|visitors| visitors.units().to_vec()),
        cast
    );
    let mut wanted = cast.clone();
    wanted.sort_unstable();
    let stale = world.get_resource::<VisitingCharacters>().is_none_or(|visitors| {
        let mut there = visitors.units().to_vec();
        there.sort_unstable();
        there != wanted
    });
    if stale {
        world.insert_resource(VisitingCharacters::of_units(cast.clone()));
        moly_features::npc::dispose::dispose_npc_all(world, "the conversation's cast");
        // The roster is created again the way the scene restores one: the
        // arrival runs for the new bodies, which stay hidden until it has.
        if let Err(reason) = moly_features::npc::restore::begin(world) {
            warn!("[director] play_talk: the roster is not restored: {reason}");
        }
        return Err(format!("the conversation's cast {cast:?} is being placed on the site; ask again once they stand"));
    }
    let mind = world.get::<ObjectiveMind>(entity).ok_or_else(|| format!("character {unit} has no AI"))?;
    if let Some(reason) = mind.ai_stopped.as_deref().filter(|reason| *reason != npc_take::OWNED) {
        return Err(format!("character {unit}'s AI loop has ended: {reason}"));
    }
    if world.get::<NpcActions>(entity).is_some_and(|actions| actions.current == NpcAction::Talk) {
        return Err(format!("character {unit} is in a conversation"));
    }
    if world.get::<DirectedOwned>(entity).is_some() {
        moly_features::npc::stop_for_external_activity(world, entity);
        world.entity_mut(entity).remove::<DirectedOwned>();
    } else {
        moly_features::npc::gate_entries::try_cancel_current_objective(world, entity);
    }
    if let Some(mut mind) = world.get_mut::<ObjectiveMind>(entity) {
        if mind.ai_stopped.as_deref() == Some(npc_take::OWNED) {
            mind.ai_stopped = None;
        }
        mind.skip_next_rest = true;
    }
    let mut talks = world.get_resource_or_insert_with(DirectedTalks::default);
    talks.outcomes.remove(&entity);
    talks.requests.insert(entity, talk);
    info!("[director] character {unit} asked to open talk {talk} on its next decision");
    Ok(entity)
}

/// A requested talk until its character decides.
struct TalkRun {
    entity: Entity,
}

impl ActionRun for TalkRun {
    fn poll(&mut self, world: &mut World, _ctx: &mut ActCtx) -> ActPoll {
        match take_talk_outcome(world, self.entity) {
            Some(Ok(note)) => ActPoll::Done(note),
            Some(Err(reason)) => ActPoll::Failed(reason),
            None => ActPoll::Running("waiting for the character's next decision".into()),
        }
    }

    fn stop(&mut self, world: &mut World, _ctx: &mut ActCtx) {
        // A talk request its character has not decided on yet is withdrawn.
        if let Some(mut talks) = world.get_resource_mut::<DirectedTalks>() {
            talks.requests.remove(&self.entity);
            talks.outcomes.remove(&self.entity);
        }
    }
}

/// What the character's decision built for its requested talk, once it has
/// decided.
fn take_talk_outcome(world: &mut World, entity: Entity) -> Option<Result<String, String>> {
    world.get_resource_mut::<DirectedTalks>()?.outcomes.remove(&entity)
}

/// `npc.play_talk`: a conversation played at once, without the cast's
/// gathering.
#[derive(Debug)]
struct PlayTalk {
    unit: u32,
    talk: i32,
    fixture: Option<FixtureRef>,
}

fn read_play_talk(fields: &mut Fields<'_>) -> Result<PlayTalk, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let talk = fields.req_i32("talk")?;
    let fixture = fields.opt("fixture").map(|value| moly_features::act::fixture_ref(value, &at)).transpose()?;
    Ok(PlayTalk { unit, talk, fixture })
}

impl ActionSpec for PlayTalk {
    fn name(&self) -> &str {
        "npc.play_talk"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, _ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        play_talk(world, self.unit, self.talk, self.fixture.as_ref())?;
        Ok(Box::new(Requested { note: format!("talk {} asked to play on character {}", self.talk, self.unit) }))
    }
}

/// Plays a conversation at once: the request carries the row itself
/// (`exact`), which is the content library's own path, so no gathering runs
/// and no cast has to be recruited. The row's own backend decides how it
/// plays; a fixture conversation plays at `fixture`, a placed instance of one
/// of its fixtures (by UID, or the instance of that master id nearest the
/// speaker). The speakers' own activities are cancelled first so the
/// conversation is theirs.
fn play_talk(world: &mut World, unit: u32, talk: i32, fixture: Option<&FixtureRef>) -> Result<Entity, String> {
    let content = {
        let mut catalog = bevy::ecs::system::SystemState::<moly_features::player_talk::TalkCatalog>::new(world);
        let catalog = catalog.get(world).map_err(|error| format!("the talk catalog is not ready: {error}"))?;
        catalog.resolve(talk).map(|resolved| resolved.content())
    }
    .ok_or_else(|| format!("talk {talk} has no script"))?;
    let speakers = {
        let tables = world
            .get_resource::<moly_features::fixture_activity_data::FixtureActivityTables>()
            .ok_or("the furniture activity tables are not loaded")?;
        let master = tables.talk_master(talk).ok_or_else(|| format!("talk {talk} has no master row"))?;
        tables.unit_ids_of_group(master.unit_group_id)?
    };
    // Only the conversation's cast stands on the site: a full roster crawls
    // the sim and puts strangers in frame. The roster is rebuilt in place —
    // no site reload, so no door transition plays over the shot.
    let cast: Vec<u32> = speakers.to_vec();
    info!(
        "[director] play_talk {talk}: roster {:?} vs cast {:?}",
        world.get_resource::<VisitingCharacters>().map(|visitors| visitors.units().to_vec()),
        cast
    );
    let mut wanted = cast.clone();
    wanted.sort_unstable();
    let stale = world.get_resource::<VisitingCharacters>().is_none_or(|visitors| {
        let mut there = visitors.units().to_vec();
        there.sort_unstable();
        there != wanted
    });
    if stale {
        world.insert_resource(VisitingCharacters::of_units(cast.clone()));
        moly_features::npc::dispose::dispose_npc_all(world, "the conversation's cast");
        if let Err(reason) = moly_features::npc::restore::begin(world) {
            warn!("[director] play_talk: the roster is not restored: {reason}");
        }
        return Err(format!("the conversation's cast {cast:?} is being placed; ask again once they stand"));
    }
    let entity = targets::npc(world, unit)?;
    for &speaker in &speakers {
        if let Ok(other) = targets::npc(world, speaker) {
            moly_features::npc::gate_entries::try_cancel_current_objective(world, other);
            moly_features::npc::stop_for_external_activity(world, other);
        }
    }
    let target_fixture = fixture.map(|fixture| placed_fixture(world, fixture, entity)).transpose()?;
    world.write_message(moly_features::player_talk::PlayerTalkRequest { entity, unit, exact: Some(content), target_fixture });
    info!("[director] talk {talk} handed to the talk player on character {unit} (no gathering)");
    Ok(entity)
}

/// The placed fixture `fixture` names: the instance of that UID, or the
/// instance of that master id nearest the character `near`.
fn placed_fixture(world: &mut World, fixture: &FixtureRef, near: Entity) -> Result<Entity, String> {
    let from = world.get::<GlobalTransform>(near).map_or(Vec3::ZERO, GlobalTransform::translation);
    let mut placed = world.query_filtered::<(
        Entity,
        &moly_features::fixture_activity_state::FixtureActivityIdentity,
        &GlobalTransform,
    ), With<moly_features::fixture::FixtureRoot>>();
    match fixture {
        FixtureRef::Uid(uid) => placed
            .iter(world)
            .find(|(_, identity, _)| identity.uid == *uid)
            .map(|(entity, _, _)| entity)
            .ok_or_else(|| format!("no placed fixture with UID {uid}")),
        FixtureRef::Id(id) => placed
            .iter(world)
            .filter(|(_, identity, _)| identity.master_id == *id)
            .min_by(|(_, _, a), (_, _, b)| {
                a.translation().distance_squared(from).total_cmp(&b.translation().distance_squared(from))
            })
            .map(|(entity, _, _)| entity)
            .ok_or_else(|| format!("no placed fixture of master id {id}")),
    }
}

/// `npc.warp_to`: a character stood at a world point at once.
#[derive(Debug)]
struct Warp {
    unit: u32,
    to: [f32; 3],
}

fn read_warp(fields: &mut Fields<'_>) -> Result<Warp, String> {
    let at = fields.at().to_owned();
    let unit = fields.req_u32("unit")?;
    let to = moly_features::act::triple(fields.req("to")?, &format!("{at}.to"))?;
    Ok(Warp { unit, to })
}

impl ActionSpec for Warp {
    fn name(&self) -> &str {
        "npc.warp_to"
    }

    fn hand(&self) -> bool {
        false
    }

    fn start(&self, world: &mut World, ctx: &mut ActCtx) -> Result<Box<dyn ActionRun>, String> {
        let unit = self.unit;
        let entity = targets::npc(world, unit)?;
        // A warped character stands where it was put: its AI makes no
        // decision until released.
        take_npc(world, ctx.ticket, unit)?;
        moly_features::npc::stop_for_external_activity(world, entity);
        moly_features::npc::force_set_position(world, entity, self.to);
        Ok(Box::new(Requested { note: format!("character {unit} warped to {:?}", self.to) }))
    }
}
