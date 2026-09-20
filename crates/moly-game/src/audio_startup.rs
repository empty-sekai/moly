//! Browser loading is silent without consuming the beginning of a cue.
//!
//! The existing output stream is still opened synchronously in the user's
//! gesture. Its actual players are prepared paused, then released together
//! after scene preparation, sink construction and completed render passes.
//! This is a host loading policy, not another mixer or a script clock.

use std::sync::{
    atomic::{AtomicBool, AtomicU64, Ordering},
    Arc,
};

use bevy::{
    audio::{AudioSink, AudioSinkPlayback, PlaybackSettings},
    prelude::*,
    render::{render_resource::PipelineCache, Render, RenderApp, RenderSystems},
    transform::TransformSystems,
};

#[derive(Resource, Default)]
pub(crate) struct BrowserAudioStartup {
    pub(crate) prepared: bool,
    released: bool,
    prepared_frame: Option<u64>,
}

impl BrowserAudioStartup {
    pub(crate) fn released(&self) -> bool { self.released }

    fn can_release(&mut self, audio_ready: bool, pipelines_ready: bool, frame: u64) -> bool {
        if !self.prepared || !audio_ready || !pipelines_ready {
            self.prepared_frame = None;
            return false;
        }
        let prepared_frame = *self.prepared_frame.get_or_insert(frame);
        frame.saturating_sub(prepared_frame) >= 2
    }
}

#[derive(Default)]
struct RenderProgress {
    frame: AtomicU64,
    pipelines_ready: AtomicBool,
}

#[derive(Resource, Clone, Default)]
struct StartupRenderProgress(Arc<RenderProgress>);

#[derive(Component)]
struct StartupPaused {
    originally_paused: bool,
}

pub(crate) fn configure(app: &mut App) {
    let progress = StartupRenderProgress::default();
    app.init_resource::<BrowserAudioStartup>()
        .insert_resource(progress.clone())
        .add_systems(PostUpdate, hold_players.before(TransformSystems::Propagate))
        .add_systems(Last, release_prepared_players);
    if let Some(render) = app.get_sub_app_mut(RenderApp) {
        render.insert_resource(progress).add_systems(
            Render,
            observe_render.in_set(RenderSystems::Cleanup),
        );
    }
}

fn observe_render(cache: Res<PipelineCache>, progress: Res<StartupRenderProgress>) {
    progress.0.pipelines_ready.store(
        cache.waiting_pipelines().next().is_none(),
        Ordering::Release,
    );
    progress.0.frame.fetch_add(1, Ordering::Release);
}

/// Bevy constructs audio sinks after transform propagation. Set the original
/// PlaybackSettings before that system can start a newly loaded source.
fn hold_players(
    mut commands: Commands,
    startup: Res<BrowserAudioStartup>,
    mut players: Query<(Entity, &mut PlaybackSettings), Without<StartupPaused>>,
) {
    if startup.released { return; }
    for (entity, mut settings) in &mut players {
        commands.entity(entity).insert(StartupPaused {
            originally_paused: settings.paused,
        });
        settings.paused = true;
    }
}

/// Prepare environment players only after the world they belong to is ready.
/// Their encoded resources and decoder/loop initialization complete silently.
pub(crate) fn can_prepare(startup: Option<Res<BrowserAudioStartup>>) -> bool {
    startup.is_none_or(|startup| startup.prepared || startup.released)
}

pub(crate) fn can_play(startup: Option<Res<BrowserAudioStartup>>) -> bool {
    startup.is_none_or(|startup| startup.released)
}

fn release_prepared_players(
    mut commands: Commands,
    mut startup: ResMut<BrowserAudioStartup>,
    progress: Res<StartupRenderProgress>,
    mut players: Query<(Entity, &mut PlaybackSettings, &StartupPaused, Option<&AudioSink>)>,
) {
    if startup.released { return; }
    // A scene can become ready before BGM/ambient assets have materialized as
    // real sinks. Paused sinks include the original loop's synchronous seek.
    let audio_ready = !players.is_empty() && players.iter().all(|(_, _, _, sink)| sink.is_some());
    let frame = progress.0.frame.load(Ordering::Acquire);
    // Let the prepared world actually pass extraction and render before audio
    // starts. This is measured render completion, not an elapsed-time sleep.
    if !startup.can_release(audio_ready, progress.0.pipelines_ready.load(Ordering::Acquire), frame) {
        return;
    }
    for (entity, mut settings, held, sink) in &mut players {
        settings.paused = held.originally_paused;
        if !held.originally_paused {
            sink.expect("all prepared audio players have sinks").play();
        }
        commands.entity(entity).remove::<StartupPaused>();
    }
    startup.released = true;
    info!("[audio-startup] scene, audio sinks and renderer prepared; original players released");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loading_does_not_consume_an_originally_playing_source() {
        let mut app = App::new();
        app.init_resource::<BrowserAudioStartup>()
            .add_systems(PostUpdate, hold_players);
        let intro = app.world_mut().spawn(PlaybackSettings::ONCE).id();
        let loop_sink = app.world_mut().spawn(PlaybackSettings::LOOP.paused()).id();
        app.update();
        assert!(app.world().get::<PlaybackSettings>(intro).unwrap().paused);
        assert!(!app.world().get::<StartupPaused>(intro).unwrap().originally_paused);
        assert!(app.world().get::<StartupPaused>(loop_sink).unwrap().originally_paused);
        // A second loading frame must not overwrite the original pause state.
        app.update();
        assert!(!app.world().get::<StartupPaused>(intro).unwrap().originally_paused);
    }

    #[test]
    fn actual_render_progress_is_required_after_scene_and_audio_preparation() {
        let mut startup = BrowserAudioStartup::default();
        assert!(!startup.can_release(true, true, 100));
        startup.prepared = true;
        assert!(!startup.can_release(false, true, 101));
        assert!(!startup.can_release(true, false, 102));
        assert!(!startup.can_release(true, true, 103));
        assert!(!startup.can_release(true, true, 103));
        assert!(!startup.can_release(true, true, 104));
        assert!(startup.can_release(true, true, 105));
    }

    #[test]
    fn new_preparation_work_restarts_render_confirmation() {
        let mut startup = BrowserAudioStartup { prepared: true, ..default() };
        assert!(!startup.can_release(true, true, 20));
        assert!(!startup.can_release(true, false, 21));
        assert!(!startup.can_release(true, true, 22));
        assert!(!startup.can_release(true, true, 23));
        assert!(startup.can_release(true, true, 24));
    }
}
