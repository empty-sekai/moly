//! Player furniture actions use Moly's real, carved field navigation.
//!
//! The bridge is deliberately explicit about fidelity: this is the same
//! single-height grid and imported surface used by normal Moly movement, not
//! Unity NavMeshAgent steering. Source locator identities, UID floor occupancy,
//! query failure, animation selection and the shared Timeline remain intact.
use std::sync::Arc;
use bevy::prelude::*;
use crate::{
    fixture_scene_inputs::{self, FixtureNavigationGeometry, FixtureSceneStamp, FixtureSceneSupply},
    npc_objective::ObjectiveFace,
    player_fixture_action::{PlayerFixtureNavigation, PlayerFixturePath, PlayerFixturePathStatus, PlayerFixturePreparationError},
    walk_face::WalkFace,
};

impl FixtureNavigationGeometry for ObjectiveFace {
    fn sample_position(&self, position: Vec3, max_distance: f32) -> Option<Vec3> {
        if !position.is_finite() || !max_distance.is_finite() || max_distance < 0. { return None; }
        self.sample(position.to_array(), max_distance).map(Vec3::from)
    }
    fn path(&self, from: Vec3, to: Vec3) -> Result<PlayerFixturePath, PlayerFixturePreparationError> {
        if !from.is_finite() || !to.is_finite() {
            return Err(PlayerFixturePreparationError::Invalid("nonfinite navigation endpoint".into()));
        }
        Ok(match self.fixture_path(from, to) {
            Some(corners) => PlayerFixturePath { query_succeeded: true, corners, status: Some(PlayerFixturePathStatus::Complete) },
            None => PlayerFixturePath { query_succeeded: false, corners: Vec::new(), status: Some(PlayerFixturePathStatus::Invalid) },
        })
    }
    fn constrain_move(&self, from: Vec3, to: Vec3) -> Option<Vec3> { self.fixture_move(from, to) }
}

#[derive(Resource)]
struct InstalledGeometry { stamp: FixtureSceneStamp, field_generation: u64 }

pub(crate) fn publish(world: &mut World) {
    let Some(inputs) = world.get_resource::<FixtureSceneSupply>().and_then(FixtureSceneSupply::current) else { return; };
    let stamp = inputs.stamp;
    let Some(face) = world.get_resource::<ObjectiveFace>().filter(|face| face.is_fresh(stamp.site_epoch)) else { return; };
    let field_generation = face.navigation_generation();
    if world.get_resource::<WalkFace>().is_none_or(|walk| walk.generation() != field_generation) { return; }
    if world.get_resource::<InstalledGeometry>().is_some_and(|installed| installed.stamp == stamp && installed.field_generation == field_generation)
        && world.get_resource::<PlayerFixtureNavigation>().is_some_and(|nav| nav.scene_stamp == stamp) { return; }
    // Arc-backed geometry stays immutable for existing owners across a rebuild.
    // Only a changed field/input generation makes a new surface snapshot.
    let geometry = Arc::new(face.clone());
    let generation = world.get_resource::<PlayerFixtureNavigation>().map_or(field_generation,
        |nav| nav.navigation_generation.saturating_add(1).max(field_generation));
    match fixture_scene_inputs::publish_navigation(world, stamp, geometry, generation, vec![
        "Moly carved single-height grid with imported surface heights; exact field paths, bounded sampling and real UID floor occupancy.".into(),
        "Unity NavMeshAgent steering and layered navigation are not emulated by this host adapter.".into(),
    ]) {
        Ok(()) => {
            world.insert_resource(InstalledGeometry { stamp, field_generation });
            info!("[player-fixture] installed real Moly navigation scene={} inputs={} field={}", stamp.site_epoch, stamp.input_revision, field_generation);
        }
        Err(reason) => warn!("[player-fixture] navigation publication refused: {reason}"),
    }
}
