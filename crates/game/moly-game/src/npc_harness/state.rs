//! Acceptance harness, tweet and greeting states: the product's tweet-state
//! update on the executed grid of the source's update (elapsed, site type,
//! game state, first-update latch), and the product's greeting-state update
//! frame by frame on the frame sequences the source's greeting state was
//! executed on. See the parent harness module for the protocol.

use bevy::ecs::message::Messages;
use bevy::ecs::system::RunSystemOnce;
use bevy::prelude::*;
use serde_json::{json, Value};

use super::{update_greeting, update_tweet, TweetHudEvent, TweetState};
use crate::npc::{NpcAction, NpcActions};
use crate::npc_harness::{case, emit};
use moly_law::tweet::TweetRow;

#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR"]
fn a07_tweet_update_direct() {
    let input = case("a07");
    let mut rows = Vec::new();
    for row in input["direct"].as_array().expect("direct") {
        let bits = row["elapsed_bits"].as_u64().unwrap() as u32;
        let latch = row["latch"].as_u64() == Some(1);
        let game_state = row["game_state"].as_u64().unwrap();
        let site = row["site"].as_i64().unwrap() as i32;
        let mut actions = NpcActions::default();
        actions.current = NpcAction::Tweet;
        actions.elapsed = f32::from_bits(bits);
        actions.tweet_state = TweetState::InProgress;
        actions.locals.tweet.animation_started = latch;
        // The layout editor is the game state that ends a tweet at once; the
        // other game states are not an input of the update.
        update_tweet(1, 0, Some(site), game_state == 2, &mut actions);
        let outcome = if actions.tweet_state == TweetState::Done {
            "Done"
        } else if actions.locals.tweet.animation_started && !latch {
            "Start"
        } else {
            "nothing"
        };
        rows.push(json!(outcome));
    }
    emit("a07", &Value::Array(rows));
}

/// The tweet state frame by frame on the executed delta-time sequences: the
/// state clock advances by the frame delta (the per-frame line of the state
/// update, as in `on_update`), then the product's tweet update runs.
#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR"]
fn a07_tweet_frames() {
    let input = case("a07s");
    let mut out = Vec::new();
    for sequence in input["sequences"].as_array().expect("sequences") {
        let mut actions = NpcActions::default();
        actions.current = NpcAction::Tweet;
        actions.elapsed = 0.0;
        actions.tweet_state = TweetState::InProgress;
        actions.locals.tweet.animation_started = false;
        let mut frames = Vec::new();
        for (index, bits) in sequence["dt_bits"].as_array().unwrap().iter().enumerate() {
            let dt = f32::from_bits(bits.as_u64().unwrap() as u32);
            let started_before = actions.locals.tweet.animation_started;
            if actions.elapsed < f32::MAX {
                actions.elapsed = (actions.elapsed + dt).min(f32::MAX);
            }
            update_tweet(1, index as u32 + 1, Some(0), false, &mut actions);
            let outcome = if actions.tweet_state == TweetState::Done {
                "Done"
            } else if actions.locals.tweet.animation_started && !started_before {
                "Start"
            } else {
                "nothing"
            };
            frames.push(json!([index + 1, actions.elapsed.to_bits(), outcome]));
            if outcome == "Done" {
                break;
            }
        }
        out.push(json!({"name": sequence["name"], "frames": frames}));
    }
    emit("a07s", &Value::Array(out));
}

/// One greeting-state frame: the state clock advances by the frame delta
/// (the presenter's per-frame accumulation), then the product's update runs.
fn greeting_frame(world: &mut World, actions: NpcActions, frame: u32, dt: f32, player: Option<Vec3>) -> NpcActions {
    world
        .run_system_once_with(
            |In((mut actions, frame, dt, player)): In<(NpcActions, u32, f32, Option<Vec3>)>,
             mut hud: MessageWriter<TweetHudEvent>| {
                if actions.elapsed < f32::MAX {
                    actions.elapsed = (actions.elapsed + dt).min(f32::MAX);
                }
                update_greeting(1, frame, Entity::PLACEHOLDER, Some(0), Vec3::ZERO, player, &mut actions, &mut hud);
                actions
            },
            (actions, frame, dt, player),
        )
        .expect("greeting frame")
}

#[test]
#[ignore = "requires MOLY_NPC_HARNESS_DIR"]
fn a09_greeting_state_frames() {
    let input = case("a09s");
    let mut out = Vec::new();
    for scenario in input["scenarios"].as_array().expect("scenarios") {
        let mut world = World::new();
        world.init_resource::<Messages<TweetHudEvent>>();
        let player = scenario["player"].as_array().map(|p| {
            Vec3::new(p[0].as_f64().unwrap() as f32, p[1].as_f64().unwrap() as f32, p[2].as_f64().unwrap() as f32)
        });
        let row = TweetRow {
            id: 3,
            motion_name: Some("greet_motion".to_owned()),
            emoticon_name: Some(scenario["emoticon"].as_str().unwrap().to_owned()),
            eye_name: String::new(),
            mouth_name: String::new(),
            text: String::new(),
        };
        // The state right after the greeting enter.
        let mut actions = NpcActions::default();
        actions.current = NpcAction::Greeting;
        actions.elapsed = 0.0;
        actions.tweet_state = match scenario["tweet_state"].as_u64().unwrap() {
            1 => TweetState::Pending,
            2 => TweetState::InProgress,
            3 => TweetState::Done,
            other => panic!("tweet state {other}"),
        };
        actions.first_talk_complete = false;
        actions.locals.greeting.animation_started = false;
        actions.locals.greeting.show_tweet = true;
        actions.locals.greeting.already_greeted = scenario["greeted"].as_bool().unwrap();
        actions.locals.greeting.row = scenario["has_row"].as_bool().unwrap().then_some(row);
        actions.locals.greeting.looking_at_player = player.is_some();
        let mut frames = Vec::new();
        let mut hud_before = 0usize;
        for (index, dt) in scenario["dts"].as_array().unwrap().iter().enumerate() {
            let dt = dt.as_f64().unwrap() as f32;
            let greeted_before = actions.locals.greeting.already_greeted;
            let started_before = actions.locals.greeting.animation_started;
            actions = greeting_frame(&mut world, actions, index as u32 + 1, dt, player);
            let hud_total = world.resource::<Messages<TweetHudEvent>>().len();
            frames.push(json!({
                "el_bits": actions.elapsed.to_bits(),
                "tweet": actions.tweet_state as u8,
                "first": actions.first_talk_complete,
                "greeted": actions.locals.greeting.already_greeted,
                "show": actions.locals.greeting.show_tweet,
                "look": (!greeted_before).then_some(actions.locals.greeting.looking_at_player),
                "animation": actions.locals.greeting.animation_started && !started_before,
                "hud": hud_total - hud_before,
            }));
            hud_before = hud_total;
        }
        out.push(json!({"name": scenario["name"], "frames": frames}));
    }
    emit("a09s", &Value::Array(out));
}
