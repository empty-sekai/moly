//! The original's cues for a companion's build: the hit cue of a hammer
//! blow and the put cue of the finished fixture.
//!
//! A build is a hammer against a fixture the layout holds, so its cues are
//! the ones the original plays for those two things:
//!
//! - each hit plays the pickaxe hit sound (`se_pickaxe1`, the harvest
//!   class) and the stone hit effect (`EffectManager` type 111), placed as
//!   the stone's hit effect is: 0.2 m back toward the swinger and 0.35 m up,
//!   facing it;
//! - the finished fixture plays the editor's own put cue for it
//!   (`FixtureController.ShowPutEffect`, the `fx_fixture_put_01` effect and
//!   the fixture's put sound) and then the decide's sound
//!   (`se_housing_finish`).
//!
//! The companion writes a [`BuildCue`] per hit and one when it has built
//! the fixture; this module reads them, so the cues play even when no build
//! of the director's own asked for them.

use bevy::prelude::*;

use crate::audio::{SeClass, SeRequest, SeRequests};
use crate::companion::BuildCue;
use crate::fixture::FixturePlacements;
use crate::harvest::damage::push_se;
use crate::harvest::{EffectHook, HarvestEffectHooks};

/// The hit sound of a stone's pickaxe blow (`MysekaiAreaStoneView`).
const HIT_SE: &str = "se_pickaxe1";
/// The hit effect of that blow (`PlayDamageNormalEffect`, tool level 1).
const HIT_EFFECT: u16 = 111;
/// The editor's decide sound, played after a fixture's put cue.
const FINISH_SE: &str = "se_housing_finish";

/// The plugin: the build cues, played.
pub struct CompanionCuePlugin;

impl Plugin for CompanionCuePlugin {
    fn build(&self, app: &mut App) {
        // The cues are read the frame the swings write them, before the SE
        // queue drains, so a hit's sound starts on its frame.
        app.add_systems(
            Update,
            play_cues
                .after(crate::companion::CompanionSteps)
                .before(crate::audio::SeDrainSet::Drain),
        );
    }
}

/// Update: each build cue of this frame, played.
fn play_cues(
    mut commands: Commands,
    mut cues: MessageReader<BuildCue>,
    mut se: ResMut<SeRequests>,
    mut effects: ResMut<HarvestEffectHooks>,
    placements: Option<Res<FixturePlacements>>,
) {
    for cue in cues.read() {
        match cue {
            BuildCue::Hit { unit, uid, hit, of, at, from } => {
                let (position, rotation) = hit_pose(*at, *from);
                effects.pending.push(EffectHook { kind: HIT_EFFECT, position, rotation });
                push_se(&mut se, HIT_SE, "companion-build");
                info!(
                    "[companion-cue] unit {unit} build hit {hit}/{of} on fixture {uid}: {HIT_SE} and effect {HIT_EFFECT}"
                );
            }
            BuildCue::Built { unit, uid } => {
                // The put effect queues the fixture's own put sound once its
                // instance stands; the decide's sound follows at once.
                match placements.as_ref().and_then(|placements| placed(placements, uid)) {
                    Some(item) => commands.queue(move |world: &mut World| {
                        crate::fixture_edit::show_put_effect(world, &item, "companion-build");
                    }),
                    None => info!(
                        "[companion-cue] unit {unit}: fixture {uid} is built but no placement row holds it; no put cue"
                    ),
                }
                se.0.push(SeRequest {
                    owner: None,
                    cue: FINISH_SE.to_owned(),
                    class: SeClass::Ui,
                    source: "companion-build",
                });
                info!("[companion-cue] unit {unit}: fixture {uid} built: put cue and {FINISH_SE}");
            }
        }
    }
}

/// The placed row of the layout `uid` names.
fn placed(placements: &FixturePlacements, uid: &str) -> Option<crate::fixture::EditableFixture> {
    placements.editor_rows().into_iter().find(|row| row.uid == uid)
}

/// The hit effect's pose for a blow at `at` swung from `from`: 0.2 m back
/// toward the swinger and 0.35 m up, facing the swinger (the stone's hit
/// effect, `damage_effect`).
fn hit_pose(at: Vec3, from: Vec3) -> (Vec3, Quat) {
    let back = Vec2::new(at.x - from.x, at.z - from.z).normalize_or_zero();
    let position = at + Vec3::new(-0.2 * back.x, 0.35, -0.2 * back.y);
    let towards = Vec2::new(from.x - at.x, from.z - at.z);
    let rotation = if towards.length_squared() > 0.0 {
        Quat::from_rotation_y(towards.x.atan2(towards.y))
    } else {
        Quat::IDENTITY
    };
    (position, rotation)
}
