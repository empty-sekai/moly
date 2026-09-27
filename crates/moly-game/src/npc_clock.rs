//! The frame delta time the NPC timers read: the engine's frame clock
//! (`moly_law::frame_time`) run on this host's real clock with the project's
//! extracted time settings (`time-manager.json`).
//!
//! Every NPC accumulator reads [`NpcClock::delta`]: the agent step, the Rest
//! delay, the objective waits, the state clocks and the presenter clock, and
//! the idle scripts' waits. Bevy's own virtual clock is not changed; other
//! domains keep reading it.
//!
//! The engine clock starts with the player's start-up reset on the first
//! frame after the settings are in, so its first two frames read 0.02.
//!
//! Until the settings file is in, the NPC timers read 0 and the AI loop
//! holds before its first decision. A settings file that fails to load is
//! refused: the error is logged once and the NPC AI never starts; no host
//! clock stands in for the engine's.
//!
//! Named gap:
//! - A scene load and a resume from the background each restart the engine
//!   clock's first frames in the source; which host events count as those is
//!   not mapped, so neither is modelled.

use bevy::asset::{AssetPath, LoadState};
use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use moly_law::frame_time::{FrameClock, TimeSettings};

/// The NPC frame clock.
#[derive(Resource, Default)]
pub struct NpcClock {
    engine: Option<FrameClock>,
    settings: Option<Handle<JsonAsset>>,
}

impl NpcClock {
    /// This frame's delta time for every NPC timer, in seconds (0 until the
    /// engine clock runs).
    pub(crate) fn delta(&self) -> f32 {
        self.engine.as_ref().map_or(0.0, FrameClock::delta)
    }

    /// Whether the engine clock runs (the extracted settings are in).
    pub(crate) fn ready(&self) -> bool {
        self.engine.is_some()
    }
}

fn settings_path() -> AssetPath<'static> {
    AssetPath::from("moly://time-manager.json".to_owned())
}

/// Startup: request the extracted time settings.
pub(crate) fn load(server: Res<AssetServer>, mut clock: ResMut<NpcClock>) {
    clock.settings = Some(server.load::<JsonAsset>(settings_path()));
}

/// Reads the settings document. A malformed document is data damage and
/// panics at the asset boundary.
fn parse_settings(text: &str) -> TimeSettings {
    let value: serde_json::Value = serde_json::from_str(text)
        .unwrap_or_else(|err| panic!("time-manager.json is not valid JSON: {err}"));
    let version = value.get("version").and_then(|v| v.as_i64());
    assert_eq!(version, Some(1), "time-manager.json has version {version:?}, not 1");
    let settings = value
        .get("timeManager")
        .unwrap_or_else(|| panic!("time-manager.json has no timeManager object"));
    let float = |key: &str| -> f32 {
        settings
            .get(key)
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| panic!("time-manager.json has no number {key:?}")) as f32
    };
    TimeSettings {
        maximum_allowed_timestep: float("Maximum Allowed Timestep"),
        time_scale: float("m_TimeScale"),
    }
}

/// First, after the host clocks update: one engine-clock frame.
pub(crate) fn advance(
    real: Res<Time<Real>>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut clock: ResMut<NpcClock>,
) {
    let Some(now) = real
        .last_update()
        .map(|instant| instant.duration_since(real.startup()).as_secs_f64())
    else {
        return;
    };
    if let Some(engine) = clock.engine.as_mut() {
        engine.update(now);
        return;
    }
    let Some(handle) = clock.settings.clone() else {
        return;
    };
    if let LoadState::Failed(err) = server.load_state(&handle) {
        error!(
            "[npc-clock] time-manager.json (the extracted engine time settings) did not load ({err:?}); refused: the NPC AI does not start without the engine's frame clock"
        );
        clock.settings = None;
        return;
    }
    let Some(document) = jsons.get(&handle) else {
        return;
    };
    let settings = parse_settings(&document.0);
    let mut engine = FrameClock::reset_with_first_delta(settings, now);
    engine.update(now);
    info!(
        "[npc-clock] engine frame clock started: maximum allowed timestep {} ({:#x}), time scale {}; first delta {}",
        settings.maximum_allowed_timestep,
        settings.maximum_allowed_timestep.to_bits(),
        settings.time_scale,
        engine.delta()
    );
    clock.engine = Some(engine);
    clock.settings = None;
}
