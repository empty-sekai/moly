//! The event stream: what happened in the world, in order, for a client to
//! read with `moly.events`.
//!
//! Every event carries a sequence number, the frame and the game time it
//! happened at, a `kind` and its fields. The newest [`KEPT`] events are kept;
//! a client passes the `next` number of its last read as `since` and gets
//! what happened after it, and `missed` tells it how many events it was too
//! late to read.
//!
//! The domains that decide an event emit it where it is decided (an item
//! collected, a craft answered, a delivery started or ended, a tweet shown).
//! The events that follow from state (weather, site, conversations, NPC
//! arrivals and departures) are read once per frame by [`watch`], which
//! compares with the frame before.
//!
//! The exchange with the server process is emitted by the client's reader:
//! `server.call` when a call is sent, `server.reply` and `server.push` when
//! the server answers or sends on its own, and `server.link` when the
//! connection's state changes.

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

use bevy::diagnostic::FrameCount;
use bevy::prelude::*;
use serde_json::{json, Map, Value};

use crate::npc::{CharacterUnitId, NpcActions};

/// Events kept for late readers.
const KEPT: usize = 4096;

/// The most events one read answers.
pub const MOST_PER_READ: usize = 1000;

struct Log {
    next: u64,
    events: VecDeque<Value>,
}

static LOG: Mutex<Log> = Mutex::new(Log { next: 1, events: VecDeque::new() });
static FRAME: AtomicU64 = AtomicU64::new(0);
static ELAPSED_BITS: AtomicU64 = AtomicU64::new(0);

/// Records one event of `kind` with `fields` (an object; anything else is
/// kept under `value`).
pub fn emit(kind: &str, fields: Value) {
    let mut event = match fields {
        Value::Object(fields) => fields,
        Value::Null => Map::new(),
        other => Map::from_iter([("value".to_owned(), other)]),
    };
    let mut log = LOG.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let seq = log.next;
    log.next += 1;
    event.insert("seq".into(), json!(seq));
    event.insert("frame".into(), json!(FRAME.load(Ordering::Relaxed)));
    event.insert("time".into(), json!(f64::from_bits(ELAPSED_BITS.load(Ordering::Relaxed))));
    event.insert("kind".into(), json!(kind));
    log.events.push_back(Value::Object(event));
    while log.events.len() > KEPT {
        log.events.pop_front();
    }
}

/// The events after `since`, at most `limit`, only of `kinds` when given.
pub fn read(since: u64, limit: usize, kinds: Option<&[String]>) -> Value {
    let log = LOG.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let oldest = log.events.front().and_then(|event| event["seq"].as_u64()).unwrap_or(log.next);
    let missed = oldest.saturating_sub(since + 1);
    let mut next = since.max(oldest.saturating_sub(1));
    let mut events = Vec::new();
    for event in log.events.iter().filter(|event| event["seq"].as_u64().is_some_and(|seq| seq > since)) {
        if events.len() == limit {
            break;
        }
        next = event["seq"].as_u64().unwrap_or(next);
        if kinds.is_none_or(|kinds| kinds.iter().any(|kind| event["kind"] == kind.as_str())) {
            events.push(event.clone());
        }
    }
    if events.len() < limit {
        next = next.max(log.next - 1);
    }
    json!({ "events": events, "next": next, "missed": missed, "latest": log.next - 1 })
}

/// What [`watch`] saw last frame.
#[derive(Default)]
pub struct Seen {
    started: bool,
    weather: Option<i32>,
    site: Option<(u32, String)>,
    talk: Option<i32>,
    player_talk: Option<(i32, u32)>,
    away: BTreeMap<u32, bool>,
}

/// First: the frame and game time the events of this frame are stamped with.
pub fn stamp(frame: Res<FrameCount>, time: Res<Time<Virtual>>) {
    FRAME.store(u64::from(frame.0), Ordering::Relaxed);
    ELAPSED_BITS.store(time.elapsed_secs_f64().to_bits(), Ordering::Relaxed);
}

/// Last: the events that follow from state, against the frame before. The
/// first frame records the state it starts from without events.
pub fn watch(world: &mut World, mut seen: Local<Seen>) {
    let weather = world.get_resource::<crate::weather::CurrentPhenomenonId>().map(|id| id.0);
    let site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| (site.site_id, site.site_type.clone()));
    let talk = world.get_resource::<crate::talk::ActiveTalk>().map(crate::talk::ActiveTalk::talk_id);
    let player_talk = world
        .get_resource::<crate::player_talk::PlayerTalkSession>()
        .map(|session| (session.talk_id(), session.unit()));
    let mut roster =
        world.query_filtered::<(&CharacterUnitId, Has<crate::npc::residency::Away>), With<NpcActions>>();
    let away: BTreeMap<u32, bool> = roster.iter(world).map(|(unit, away)| (unit.0, away)).collect();
    let started = seen.started;
    seen.started = true;

    if started && weather != seen.weather {
        if let Some(phenomenon) = weather {
            let name = world
                .get_resource::<crate::weather::PhenomenonCatalogue>()
                .and_then(|catalogue| catalogue.0.iter().find(|option| option.id == phenomenon))
                .map(|option| option.name.clone());
            emit("weather_changed", json!({ "phenomenon": phenomenon, "name": name, "from": seen.weather }));
        }
    }
    if started && site != seen.site {
        if let Some((site_id, site_type)) = &site {
            emit(
                "site_entered",
                json!({ "site_id": site_id, "site_type": site_type, "from": seen.site.as_ref().map(|(_, from)| from) }),
            );
        }
    }
    if started && talk != seen.talk {
        if let Some(talk_id) = seen.talk {
            emit("talk_ended", json!({ "talk_id": talk_id, "with_player": false }));
        }
        if let Some(talk_id) = talk {
            emit("talk_started", json!({ "talk_id": talk_id, "with_player": false }));
        }
    }
    if started && player_talk != seen.player_talk {
        if let Some((talk_id, unit)) = seen.player_talk {
            emit("talk_ended", json!({ "talk_id": talk_id, "with_player": true, "unit": unit }));
        }
        if let Some((talk_id, unit)) = player_talk {
            emit("talk_started", json!({ "talk_id": talk_id, "with_player": true, "unit": unit }));
        }
    }
    if started {
        let site_type = site.as_ref().map(|(_, site_type)| site_type.as_str());
        for (unit, now_away) in &away {
            match seen.away.get(unit) {
                Some(was_away) if was_away == now_away => {}
                None if *now_away => {}
                _ => emit(
                    if *now_away { "character_left" } else { "character_arrived" },
                    json!({ "unit": unit, "site_type": site_type }),
                ),
            }
        }
    }
    seen.weather = weather;
    seen.site = site;
    seen.talk = talk;
    seen.player_talk = player_talk;
    seen.away = away;
}
