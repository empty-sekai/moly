//! The NPC action-state machine per frame: the presenter's third call (the
//! state machine update), the tweet state (9) and the greeting state (10),
//! and the tweet HUD event (event 25) they publish.
//!
//! Source shape:
//! - A state change runs the old state's exit and the new state's enter
//!   inside the change itself ([`crate::npc::NpcActions::change`]). What those
//!   write on the NPC (the tweet state, the first-talk flag, the state's own
//!   latches) is written there; what needs other data (the tweet row, the
//!   greeting pick, the HUD event, the agent stop) is queued and run here, at
//!   the head of the next presenter pass, before the state updates: the same
//!   frame for a change the AI loop makes. A player talk's change runs its
//!   tweet or greeting exit (the hide event) at once, inside the change, when
//!   nothing else is queued ([`publish_exits_now`]).
//! - Per frame, while the NPC is in a registered state: the state's clock
//!   `elapsed = min(f32(elapsed + deltaTime), f32::MAX)` (skipped once it is
//!   at the maximum), then the state's update.
//! - Tweet update: outside the four home and room site types, or in the
//!   layout editor, the tweet is Done at once. Otherwise one comparison of the
//!   clock against 5.0 is read three ways: below, the motion (and the
//!   emoticon when the row names one) starts on the first update only;
//!   exactly 5.0, nothing; above, Done. The state never leaves itself: the
//!   talk objective waits for Done, and any other change runs the exit.
//! - Greeting update: complete at 5.0 s, when the player is farther than
//!   5.0 m, or when the tweet state leaves in-progress; the look-at is
//!   released from 3.0 s; one hide event on the first frame past 3.0 s while
//!   the balloon is shown.
//! - HUD event 25 carries the unit, the tweet text, show or hide, the NPC as
//!   the anchor (none on hide) and the NPC's own site type. The balloon shows
//!   on enter and hides on the hide event, so a tweet balloon lives until its
//!   state is left, not for a fixed time.
//!
//! Named gaps:
//! - Presentation calls (the tweet motion, the eye and mouth patterns, the
//!   emoticon, the look-at and its release, the facial reset, the rotation
//!   back after a tweet) are logged as calls; rendering them belongs to the
//!   presentation layer.
//! - The clocks read the NPC frame clock (`npc_clock`), which is the
//!   engine's frame clock once the extracted time settings are in.
//! - A disposed presenter never reaches an exit here: the host removes the
//!   NPC entity, and a site change resets the machine and retires its
//!   balloon.

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use moly_law::tweet::{self as tweet_law, TweetRow};

use crate::npc::{
    CharacterUnitId, MotionPhase, NpcAction, NpcActions, PathSlot, RouteStops, WalkState,
};
use crate::npc_objective::{ObjectiveMind, SequencePickSeeder};

/// The AI model's tweet state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub(crate) enum TweetState {
    #[default]
    Valid = 0,
    Pending = 1,
    InProgress = 2,
    Done = 3,
}

/// The tweet state's own fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct TweetLocal {
    pub(crate) animation_started: bool,
    pub(crate) row: Option<TweetRow>,
}

/// The greeting state's own fields.
#[derive(Debug, Clone, Default)]
pub(crate) struct GreetingLocal {
    pub(crate) animation_started: bool,
    pub(crate) show_tweet: bool,
    pub(crate) already_greeted: bool,
    pub(crate) row: Option<TweetRow>,
    /// Whether the look-at target is the player (for the release log).
    pub(crate) looking_at_player: bool,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct StateLocals {
    pub(crate) tweet: TweetLocal,
    pub(crate) greeting: GreetingLocal,
}

/// The part of a state's enter or exit that needs data outside the NPC.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StateEffect {
    TweetEnter,
    TweetExit,
    GreetingEnter,
    GreetingExit,
    /// Host only: the site changed and the machine was reset; the balloon of
    /// the old state goes with the old scene.
    Retire,
}

/// Tweet HUD event (event type 25), plus the host's retire. The balloon
/// (a UI-owned module) is its reader.
#[allow(dead_code, reason = "read by the balloon through its seam")]
#[derive(Message, Debug, Clone)]
pub(crate) enum TweetHudEvent {
    /// Show `text` over `npc`.
    Show {
        npc: Entity,
        unit: u32,
        tweet_id: i32,
        text: String,
        site_type: Option<i32>,
    },
    /// Hide the balloon of `npc` (the source sends an empty text and no
    /// anchor).
    Hide {
        npc: Entity,
        unit: u32,
        site_type: Option<i32>,
    },
    /// Host only: remove the balloon of `npc` at once.
    Retire { npc: Entity },
}

/// The per-frame presenter calls, in source order (the third call, the state
/// machine update, then the tenth, the greeting gate).
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub(crate) struct NpcPresenterSet;

/// Registration of the resources and messages these systems use.
pub(crate) struct NpcStatePlugin;

impl Plugin for NpcStatePlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<TweetHudEvent>()
            .init_resource::<SequencePickSeeder>()
            .init_resource::<crate::npc_clock::NpcClock>()
            .add_systems(Startup, (crate::npc_tweet::load, crate::npc_clock::load))
            .add_systems(
                First,
                crate::npc_clock::advance.after(bevy::time::TimeSystems),
            )
            .add_systems(Update, crate::npc_tweet::parse);
    }
}

/// Tweet rows missing on enter end the NPC's AI: the source dereferences the
/// row, and that exception is not one the AI loop catches.
fn stop_ai(mind: &mut ObjectiveMind, unit: u32, reason: String) {
    error!("[npc-state] unit={unit}: {reason}; this character's AI loop ends");
    mind.ai_stopped = Some(reason);
}

/// Eye and mouth of a tweet row: an empty name is logged and leaves the
/// pattern as it is; an empty eye still lets the mouth apply, an empty mouth
/// ends the call.
fn set_expressions(unit: u32, row: &TweetRow) {
    if row.eye_name.is_empty() {
        error!("[npc-state] unit={unit} tweet={} has no eye pattern", row.id);
    } else {
        info!(
            "[npc-state] unit={unit} call=ChangeEyePattern eye={}",
            crate::balloon::ascii_or(&row.eye_name)
        );
    }
    if row.mouth_name.is_empty() {
        error!("[npc-state] unit={unit} tweet={} has no mouth pattern", row.id);
        return;
    }
    info!(
        "[npc-state] unit={unit} call=ChangeLipSyncPattern mouth={}",
        crate::balloon::ascii_or(&row.mouth_name)
    );
}

/// The motion start of the tweet and greeting states' first update.
/// The tweet state tests the emoticon name for null only; the greeting
/// state also skips an empty name (`require_nonempty`).
fn start_motion(unit: u32, row: &TweetRow, require_nonempty: bool) {
    info!(
        "[npc-state] unit={unit} call=ChangeAnimation motion={} blend=0.5 speed=1.0 hasExitTime=true playEnd=true",
        row.motion_name.as_deref().map(crate::balloon::ascii_or).unwrap_or("<none>")
    );
    let emoticon = row
        .emoticon_name
        .as_deref()
        .filter(|name| !require_nonempty || !name.is_empty());
    if let Some(name) = emoticon {
        info!(
            "[npc-state] unit={unit} call=ShowEmoticon emoticon={} playSe=true",
            crate::balloon::ascii_or(name)
        );
    }
}

/// The global current site's type value (the site the player is on).
fn global_site_type(
    selection: Option<&crate::site::SiteSelection>,
    catalog: &crate::player_talk::TalkCatalog,
) -> Option<i32> {
    selection.and_then(|site| catalog.site_type_value(site.site_type()))
}

/// A change made outside the presenter pass (the player's talk): the source
/// runs the old state's exit inside the change, so the tweet or greeting
/// exit's hide event goes out now instead of at the next presenter pass.
/// Only when the change queued exits alone onto an empty queue; otherwise
/// the queue keeps its order and the pass runs it.
pub(crate) fn publish_exits_now(world: &mut World, entity: Entity, queued_before: usize) {
    if queued_before != 0 {
        return;
    }
    let Some(unit) = world.get::<CharacterUnitId>(entity).map(|unit| unit.0) else {
        return;
    };
    let frame = world.get_resource::<FrameCount>().map_or(0, |frame| frame.0);
    let Some(mut actions) = world.get_mut::<NpcActions>(entity) else {
        return;
    };
    if actions.effects.is_empty()
        || !actions
            .effects
            .iter()
            .all(|effect| matches!(effect, StateEffect::TweetExit | StateEffect::GreetingExit))
    {
        return;
    }
    let effects = std::mem::take(&mut actions.effects);
    let site_type = actions.site_type.clone();
    let own_site = world
        .get_resource::<crate::player_talk::PlayerTalkSites>()
        .and_then(|sites| sites.type_value_of_name(&site_type));
    for effect in effects {
        let word = match effect {
            StateEffect::TweetExit => "tweet exit: state=Done hide; call=HideEmoticon",
            _ => "greeting exit: state=Done first_talk_complete; call=ResetFacial call=HideEmoticon call=SetIKTarget(null)",
        };
        info!("[npc-state] unit={unit} frame={frame} {word} (in the talk change)");
        world.write_message(TweetHudEvent::Hide {
            npc: entity,
            unit,
            site_type: own_site,
        });
    }
}

/// The NPC's own visit count: the visitor row that holds the unit. A unit
/// outside the visitor rows (the full showcase) reads the model default 0.
fn visit_count(
    panel: Option<&crate::server_panel::ServerPanel>,
    visitors: Option<&crate::server_panel::VisitingCharacters>,
    unit: u32,
) -> i32 {
    match (panel, visitors) {
        (Some(panel), Some(visitors)) => {
            crate::server_panel::visit_count_of(panel, visitors, unit).unwrap_or(0)
        }
        _ => 0,
    }
}

/// A greeting pick draws one System.Random over the filtered greetings.
struct GreetingDraw<'a> {
    seeder: &'a mut SequencePickSeeder,
}

impl tweet_law::UniformDraw for GreetingDraw<'_> {
    fn draw(&mut self, len: usize) -> usize {
        self.seeder
            .pick(len)
            .expect("the greeting pool is not empty when it draws")
    }
}

/// Presenter call 3: queued enter and exit work, the presenter clock, the
/// state clock and the tweet and greeting updates. NPCs in the avatar list
/// order.
#[allow(clippy::type_complexity, clippy::too_many_arguments)]
pub(crate) fn on_update(
    clock: Res<crate::npc_clock::NpcClock>,
    frame: Res<FrameCount>,
    editor: Res<crate::fixture_edit::EditSessionActive>,
    selection: Option<Res<crate::site::SiteSelection>>,
    catalog: crate::player_talk::TalkCatalog,
    tables: Option<Res<crate::npc_tweet::TweetTables>>,
    panel: (
        Option<Res<crate::server_panel::ServerPanel>>,
        Option<Res<crate::server_panel::VisitingCharacters>>,
    ),
    mut seeder: ResMut<SequencePickSeeder>,
    mut hud: MessageWriter<TweetHudEvent>,
    visible: Query<
        (&CharacterUnitId, &InheritedVisibility),
        With<crate::character::MotionDriver>,
    >,
    players: Query<&Transform, With<crate::player::PlayerControlled>>,
    mut npcs: Query<
        (
            Entity,
            &CharacterUnitId,
            &Transform,
            &mut NpcActions,
            &mut ObjectiveMind,
            &mut RouteStops,
            &mut PathSlot,
            &mut WalkState,
            &mut MotionPhase,
        ),
        Without<crate::player::PlayerControlled>,
    >,
) {
    let dt = clock.delta();
    let frame = frame.0;
    let global_site = global_site_type(selection.as_deref(), &catalog);
    let player = players.single().ok().map(|transform| transform.translation);
    for (entity, unit, transform, mut actions, mut mind, mut route, mut path, mut walk, mut phase) in
        &mut npcs
    {
        let unit = unit.0;
        let own_site = catalog.site_type_value(&actions.site_type);
        // Queued enter and exit work, in the order the changes ran.
        let effects = std::mem::take(&mut actions.effects);
        for (index, effect) in effects.iter().enumerate() {
            let needs_tables = matches!(effect, StateEffect::TweetEnter | StateEffect::GreetingEnter);
            if needs_tables && tables.is_none() {
                // The tables load before any objective can start a tweet.
                // Keep this and the later work queued for the next frame.
                actions.effects.extend_from_slice(&effects[index..]);
                break;
            }
            match effect {
                StateEffect::TweetEnter => {
                    let tables = tables.as_deref().expect("checked above");
                    crate::npc::stop_agent(&mut route, &mut path, &mut walk, &mut phase);
                    let id = actions.tweet_id;
                    let Some(row) = tables.tweet(id).cloned() else {
                        stop_ai(
                            &mut mind,
                            unit,
                            format!("tweet state entered with tweet {id}, which has no master row"),
                        );
                        continue;
                    };
                    set_expressions(unit, &row);
                    info!(
                        "[npc-state] unit={unit} frame={frame} tweet enter: tweet={id} state=InProgress show",
                    );
                    hud.write(TweetHudEvent::Show {
                        npc: entity,
                        unit,
                        tweet_id: id,
                        text: row.text.clone(),
                        site_type: own_site,
                    });
                    actions.locals.tweet.row = Some(row);
                }
                StateEffect::TweetExit => {
                    info!(
                        "[npc-state] unit={unit} frame={frame} tweet exit: state=Done hide; call=HideEmoticon",
                    );
                    hud.write(TweetHudEvent::Hide {
                        npc: entity,
                        unit,
                        site_type: own_site,
                    });
                }
                StateEffect::GreetingEnter => {
                    let tables = tables.as_deref().expect("checked above");
                    let visits = visit_count(panel.0.as_deref(), panel.1.as_deref(), unit);
                    let phenomena = catalog.phenomena_id();
                    let mut draw = GreetingDraw {
                        seeder: &mut seeder,
                    };
                    let picked = tweet_law::pick_greeting(
                        &tables.greetings,
                        &tables.wrt,
                        &tables.conditions,
                        unit as i32,
                        visits,
                        phenomena,
                        &mut draw,
                    );
                    let Some(greeting) = picked else {
                        // The pick builds its generator, counts nothing and
                        // raises inside the greeting objective.
                        seeder.fresh();
                        stop_ai(
                            &mut mind,
                            unit,
                            format!(
                                "greeting pool is empty (visit count {visits}, phenomenon {phenomena})"
                            ),
                        );
                        continue;
                    };
                    let Some(tweet_id) = tweet_law::greeting_tweet_id(greeting, &tables.wrt) else {
                        stop_ai(
                            &mut mind,
                            unit,
                            format!("greeting {} has no owner row", greeting.id),
                        );
                        continue;
                    };
                    actions.tweet_id = tweet_id;
                    crate::npc::stop_agent(&mut route, &mut path, &mut walk, &mut phase);
                    actions.locals.greeting.looking_at_player = player.is_some();
                    info!(
                        "[npc-state] unit={unit} frame={frame} greeting enter: visit={visits} phenomenon={phenomena} greeting={} tweet={tweet_id}; call=SetIKTarget(player) call=DoLookAt(player, 1.0, Linear)",
                        greeting.id
                    );
                    if tweet_id < 1 {
                        error!("[npc-state] unit={unit} greeting tweet id {tweet_id} is below 1");
                        continue;
                    }
                    let Some(row) = tables.tweet(tweet_id).cloned() else {
                        error!("[npc-state] unit={unit} greeting tweet {tweet_id} has no master row");
                        continue;
                    };
                    set_expressions(unit, &row);
                    hud.write(TweetHudEvent::Show {
                        npc: entity,
                        unit,
                        tweet_id,
                        text: row.text.clone(),
                        site_type: own_site,
                    });
                    actions.locals.greeting.row = Some(row);
                    actions.locals.greeting.show_tweet = true;
                }
                StateEffect::GreetingExit => {
                    info!(
                        "[npc-state] unit={unit} frame={frame} greeting exit: state=Done first_talk_complete; call=ResetFacial call=HideEmoticon call=SetIKTarget(null)",
                    );
                    hud.write(TweetHudEvent::Hide {
                        npc: entity,
                        unit,
                        site_type: own_site,
                    });
                }
                StateEffect::Retire => {
                    hud.write(TweetHudEvent::Retire { npc: entity });
                }
            }
        }
        if !actions.ready() {
            continue;
        }
        // The presenter updates only while its NPC is visible; its clock
        // since initialization advances first.
        let shown = visible
            .iter()
            .any(|(id, visibility)| id.0 == unit && visibility.get());
        if shown {
            actions.since_initialized += dt;
        }
        // The state machine: nothing in None; otherwise the state clock,
        // then the state's update.
        if actions.current == NpcAction::None {
            continue;
        }
        if actions.elapsed < f32::MAX {
            actions.elapsed = (actions.elapsed + dt).min(f32::MAX);
        }
        match actions.current {
            NpcAction::Tweet => update_tweet(unit, frame, global_site, editor.is_active(), &mut actions),
            NpcAction::Greeting => update_greeting(
                unit,
                frame,
                entity,
                own_site,
                transform.translation,
                player,
                &mut actions,
                &mut hud,
            ),
            _ => {}
        }
    }
}

/// The tweet state's update.
fn update_tweet(
    unit: u32,
    frame: u32,
    global_site: Option<i32>,
    editing: bool,
    actions: &mut NpcActions,
) {
    if !global_site.is_some_and(|site| (0..=3).contains(&site)) || editing {
        if actions.tweet_state != TweetState::Done {
            info!(
                "[npc-state] unit={unit} frame={frame} tweet update: site {:?} editing {editing} -> Done",
                global_site
            );
        }
        actions.tweet_state = TweetState::Done;
        return;
    }
    let elapsed = actions.elapsed;
    if elapsed < tweet_law::TWEET_DISPLAY_SECONDS {
        if actions.locals.tweet.animation_started {
            return;
        }
        if let Some(row) = actions.locals.tweet.row.as_ref() {
            start_motion(unit, row, false);
        }
        actions.locals.tweet.animation_started = true;
    } else if elapsed > tweet_law::TWEET_DISPLAY_SECONDS {
        if actions.tweet_state != TweetState::Done {
            info!(
                "[npc-state] unit={unit} frame={frame} tweet update: elapsed {elapsed} ({:#x}) > 5.0 -> Done",
                elapsed.to_bits()
            );
        }
        actions.tweet_state = TweetState::Done;
    }
    // Exactly 5.0 (and NaN): nothing.
}

/// The greeting state's update.
#[allow(clippy::too_many_arguments)]
fn update_greeting(
    unit: u32,
    frame: u32,
    npc: Entity,
    own_site: Option<i32>,
    position: Vec3,
    player: Option<Vec3>,
    actions: &mut NpcActions,
    hud: &mut MessageWriter<TweetHudEvent>,
) {
    if actions.locals.greeting.already_greeted {
        actions.complete_greeting();
        return;
    }
    let elapsed = actions.elapsed;
    // The look-at target, every frame: none without a player or from 3.0 s.
    let look = player.is_some() && elapsed < tweet_law::GREETING_LOOK_AT_SECONDS;
    if actions.locals.greeting.looking_at_player && !look {
        info!(
            "[npc-state] unit={unit} frame={frame} greeting: call=SetIKTarget(null) at elapsed {elapsed} ({:#x})",
            elapsed.to_bits()
        );
    }
    actions.locals.greeting.looking_at_player = look;
    if elapsed >= tweet_law::GREETING_STATE_SECONDS {
        info!(
            "[npc-state] unit={unit} frame={frame} greeting complete: elapsed {elapsed} ({:#x}) >= 5.0",
            elapsed.to_bits()
        );
        actions.complete_greeting();
    }
    if actions.tweet_state != TweetState::InProgress {
        actions.complete_greeting();
        return;
    }
    let Some(row) = actions.locals.greeting.row.clone() else {
        error!("[npc-state] unit={unit} greeting update has no tweet row");
        actions.tweet_state = TweetState::Done;
        return;
    };
    if !actions.locals.greeting.animation_started {
        start_motion(unit, &row, true);
        actions.locals.greeting.animation_started = true;
    }
    let Some(player) = player else {
        // The gate never opens without a player; the source faults here.
        error!("[npc-state] unit={unit} greeting update has no player");
        actions.complete_greeting();
        return;
    };
    let (dx, dy, dz) = (
        position.x - player.x,
        position.y - player.y,
        position.z - player.z,
    );
    let distance = ((dx * dx + dy * dy) + dz * dz).sqrt();
    if !(distance <= tweet_law::GREETING_ABORT_DISTANCE) {
        info!(
            "[npc-state] unit={unit} frame={frame} greeting complete: player at {distance} m > 5.0"
        );
        actions.complete_greeting();
        return;
    }
    if elapsed <= tweet_law::GREETING_HUD_SECONDS || !actions.locals.greeting.show_tweet {
        return;
    }
    info!(
        "[npc-state] unit={unit} frame={frame} greeting hide: elapsed {elapsed} ({:#x}) > 3.0",
        elapsed.to_bits()
    );
    hud.write(TweetHudEvent::Hide {
        npc,
        unit,
        site_type: own_site,
    });
    actions.locals.greeting.show_tweet = false;
}

#[cfg(test)]
#[path = "npc_harness/state.rs"]
mod harness;
