//! Initial product framing for an isolated furniture/activity selection.
//! The real FieldCamera remains the only camera owner. This applies a single
//! bounded initial model update after the ordinary preview snapshot is taken;
//! gestures, authored talk cameras and restoration retain their existing paths.
use super::*;
use moly_law::fixture::position::TILE_SIZE;

fn distance_for(radius: f32, vertical_fov: f32, aspect: f32, minimum: f32, maximum: f32) -> f32 {
    let half_vertical = vertical_fov.to_radians() * 0.5;
    let half_horizontal = (half_vertical.tan() * aspect.max(0.25)).atan();
    let half_angle = half_vertical.min(half_horizontal).max(0.05);
    // A small composition margin avoids touching canvas edges; never expand
    // source camera limits or replace source-scale models to make them fit.
    (radius * 1.08 / half_angle.sin()).clamp(minimum, maximum)
}

pub(super) fn prepare(world: &mut World, choice: &PlaybackChoice, player_pose: Transform) {
    if choice.mode != ExperienceMode::Independent
        || !world.contains_resource::<crate::browser_stage::BrowserStage>()
    {
        return;
    }
    let fixture_id = match choice.key {
        EntryKey::Fixture(id) => id,
        EntryKey::Activity(key) => {
            let Some(row) = world.resource::<LibraryCatalog>().activity(key) else {
                return;
            };
            row.spec.fixture_id
        }
        // Authored talk cameras already own their initial and subsequent shots.
        _ => return,
    };
    let Some(row) = world.resource::<LibraryCatalog>().fixture(fixture_id) else {
        return;
    };
    if matches!(row.presentation, FixturePresentation::Surface { .. }) {
        return;
    }
    let Some(source) = row.source.as_ref() else {
        return;
    };
    let dimensions = Vec3::new(
        source.grid_size.x as f32,
        source.grid_size.y as f32,
        source.grid_size.z as f32,
    ) * TILE_SIZE;
    let Some(position) = world
        .query::<(&FixtureActivityIdentity, &Transform)>()
        .iter(world)
        .find(|(identity, _)| identity.master_id == fixture_id)
        .map(|(_, pose)| pose.translation)
    else {
        return;
    };
    let aspect = world
        .query::<&Window>()
        .iter(world)
        .next()
        .map(|window| window.width() / window.height().max(1.0))
        .unwrap_or(1.0);
    let delta = position - player_pose.translation;
    // Retain both the selected furniture footprint and its real player/actor
    // approach zone. Source-authored cell dimensions are already validated.
    let radius = Vec3::new(
        delta.x.abs() * 0.5 + dimensions.x * 0.5 + 0.12,
        dimensions.y.max(0.9) * 0.5 + 0.12,
        delta.z.abs() * 0.5 + dimensions.z * 0.5 + 0.12,
    )
    .length();
    let Some(mut camera) = world.get_resource_mut::<crate::camera::FieldCameraModel>() else {
        return;
    };
    let distance = distance_for(
        radius,
        camera.fov,
        aspect,
        camera.min_distance,
        camera.max_distance,
    );
    camera.distance = distance;
    camera.gestured_distance = distance;
    camera.look_at = player_pose.translation;
    camera.offset.x = delta.x * 0.5;
    camera.offset.z = delta.z * 0.5;
    camera.offset.y = (dimensions.y * 0.5).clamp(0.45, 2.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn small_furniture_is_not_left_at_the_full_site_overview_distance() {
        let distance = distance_for(0.9, 35., 1.6, 1.7, 8.);
        assert!(distance > 1.7 && distance < 4.0);
    }
    #[test]
    fn portrait_framing_and_large_furniture_respect_the_source_camera_limits() {
        let wide = distance_for(0.9, 35., 1.6, 1.7, 8.);
        let narrow = distance_for(0.9, 35., 0.56, 1.7, 8.);
        assert!(narrow > wide && narrow <= 8.);
        assert_eq!(distance_for(20., 35., 0.56, 1.7, 8.), 8.);
        assert_eq!(distance_for(0.01, 35., 1.6, 1.7, 8.), 1.7);
    }
}
