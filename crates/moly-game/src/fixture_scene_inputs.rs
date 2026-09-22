//! Generation-bound floor identities and player parameters for fixture actions.
//!
//! This module does not supply NavMesh samples, paths, steering or collision.
//! A geometry owner installs those separately, bound to one immutable input
//! snapshot. Layout changes invalidate new admission, not an existing action's
//! ownership or its already-installed navigation snapshot.

use std::{
    collections::{HashMap, HashSet},
    sync::Arc,
};

use bevy::prelude::*;
use moly_law::fixture::{
    areas::{rotated_center_grid, GridAreaData},
    position::{direction_yaw_degrees, field_position, layout_type, WALL_LAYOUT_MASK},
    GridPosition,
};
use serde_json::Value;

use crate::{
    fixture::{FixtureRoot, FixtureScenesReady, OccupancyRow},
    fixture_activity_data::FixtureActivityTables,
    fixture_activity_state::FixtureActivityIdentity,
    fixture_tiles::{
        FixtureFloorTiles, TileOccupancyEntry, TileOccupancyIdentity, TileOccupancySnapshot,
    },
    player::PlayerControlled,
    player_fixture_action::{
        FixtureTileKnowledge, PlayerFixtureNavWorld, PlayerFixtureNavigation, PlayerFixturePath,
        PlayerFixturePreparationError,
    },
    site::{
        FloorGridLayout, GroundEpoch, SiteActive, SiteReady, SiteScenesReady, SiteSelection, Sites,
    },
};

/// The main scene wrapper, not an arbitrary ground mesh or the master world's
/// distant site offset. The scene owner puts this on exactly one SiteRoot.
#[derive(Component)]
pub(crate) struct SiteCoordinateOrigin;

/// Carried by the actual placed entity. Editing code updates this same record
/// together with its pose; despawning the entity removes its occupancy source.
#[derive(Component, Clone)]
pub(crate) struct FixtureScenePlacement(pub OccupancyRow);

/// These values belong to the logical player, not its replaceable SD body.
/// The constructor sets the speed weight once. Camera/manual movement scales
/// are separate values and must not be copied into this field.
#[derive(Component, Clone, Copy, Debug, PartialEq)]
pub(crate) struct PlayerFixtureAgentParameters {
    move_speed_weight: f32,
}

impl Default for PlayerFixtureAgentParameters {
    fn default() -> Self {
        Self {
            move_speed_weight: 1.0,
        }
    }
}

impl PlayerFixtureAgentParameters {
    pub(crate) fn radius(self) -> f32 {
        0.15
    }
    pub(crate) fn default_move_speed(self) -> f32 {
        2.5
    }
    pub(crate) fn active_speed(self) -> f32 {
        self.default_move_speed() * self.move_speed_weight
    }
    /// For the owner corresponding to SetNavMeshAgentSpeed; not an animation
    /// speed setter. A zero weight remains a supplied value, not a default.
    pub(crate) fn set_move_speed_weight(&mut self, weight: f32) -> Result<(), &'static str> {
        if !weight.is_finite() {
            return Err("player speed weight is not finite");
        }
        self.move_speed_weight = weight;
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct FixtureSceneStamp {
    pub site_epoch: u64,
    pub input_revision: u64,
    pub player: Entity,
}

pub(crate) struct FixtureSceneInputs {
    pub stamp: FixtureSceneStamp,
    pub site_type: String,
    pub floor: Arc<FixtureFloorTiles>,
    pub agent: PlayerFixtureAgentParameters,
    pub gaps: Vec<String>,
}

#[derive(Resource, Default)]
pub(crate) struct FixtureSceneSupply {
    current: Option<Arc<FixtureSceneInputs>>,
    signature: Option<Signature>,
    next_revision: u64,
    pending: Option<String>,
    last_report: Vec<String>,
}

impl FixtureSceneSupply {
    pub(crate) fn current(&self) -> Option<&Arc<FixtureSceneInputs>> {
        self.current.as_ref()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct ObservedRow {
    entity: Option<Entity>,
    row: OccupancyRow,
    identity: Option<FixtureActivityIdentity>,
    world_matrix: Option<[f32; 16]>,
}

#[derive(Clone, Debug, PartialEq)]
struct Signature {
    epoch: u64,
    site: String,
    floor: FloorGridLayout,
    origin_entity: Entity,
    origin: Vec3,
    player: Entity,
    agent: PlayerFixtureAgentParameters,
    areas_revision: u64,
    rows: Vec<ObservedRow>,
}

/// Populated by the existing fixture-areas parser. It neither reloads the
/// document nor substitutes motion/cutscene arrays for AddUsingGrid.
#[derive(Resource, Default)]
pub(crate) struct FixtureFloorAreas {
    revision: u64,
    entries: HashMap<String, Result<GridAreaData, String>>,
}

impl FixtureFloorAreas {
    pub(crate) fn replace_document(&mut self, document: &Value) -> Result<(), String> {
        let packages = document["packages"]
            .as_object()
            .ok_or("area document lacks packages")?;
        let mut entries = HashMap::new();
        for (name, row) in packages {
            let parsed = (|| {
                if row["hasMeta"].as_bool() != Some(true) {
                    return Err("fixture metadata has not been supplied".to_owned());
                }
                if !row["readError"].is_null() {
                    return Err("fixture metadata could not be read".to_owned());
                }
                let grid = &row["AddUsingGrid"];
                if grid["ragged"].as_bool() != Some(false)
                    || grid["rowsWithoutColumns"].as_u64() != Some(0)
                    || !grid["anomaly"].is_null()
                {
                    return Err("AddUsingGrid has unresolved row structure".to_owned());
                }
                let rows = grid["dims"]["rows"]
                    .as_u64()
                    .ok_or("AddUsingGrid row count is missing")?
                    as usize;
                let cols = grid["dims"]["cols"]
                    .as_u64()
                    .ok_or("AddUsingGrid column count is missing")?
                    as usize;
                let cells = grid["cells"]
                    .as_array()
                    .ok_or("AddUsingGrid cells are missing")?;
                if cells.len() != rows {
                    return Err("AddUsingGrid row count disagrees".to_owned());
                }
                let mut values = Vec::new();
                for row in cells {
                    let row = row.as_array().ok_or("AddUsingGrid row is not an array")?;
                    if row.len() != cols {
                        return Err("AddUsingGrid column count disagrees".to_owned());
                    }
                    for cell in row {
                        values.push(cell.as_bool().ok_or("AddUsingGrid cell is not boolean")?);
                    }
                }
                Ok(GridAreaData::from_meta(rows, cols, |r, c| {
                    values[r * cols + c]
                }))
            })();
            entries.insert(name.clone(), parsed);
        }
        self.entries = entries;
        self.revision = self
            .revision
            .checked_add(1)
            .expect("fixture area revision exhausted");
        Ok(())
    }
}

fn live_matrix(world: &World, entity: Entity) -> Option<Mat4> {
    let mut chain = Vec::new();
    let mut seen = HashSet::new();
    let mut current = entity;
    loop {
        if !seen.insert(current) {
            return None;
        }
        chain.push(*world.get::<Transform>(current)?);
        match world.get::<ChildOf>(current) {
            Some(parent) => current = parent.parent(),
            None => break,
        }
    }
    let mut global = GlobalTransform::IDENTITY;
    for local in chain.into_iter().rev() {
        global = global.mul_transform(local);
    }
    Some(global.to_matrix())
}

fn discover(world: &mut World) -> Result<Signature, String> {
    if world
        .get_resource::<crate::fixture_edit::EditSessionActive>()
        .is_some_and(|editor| editor.is_active())
    {
        return Err(
            "layout editing owns scene poses; draft geometry is not a playable snapshot".into(),
        );
    }
    if !world.contains_resource::<FixtureScenesReady>() {
        return Err("placed fixture scenes are still loading".into());
    }
    if !world.contains_resource::<SiteScenesReady>() || world.contains_resource::<SiteReady>() {
        return Err("active site scene has not installed its new epoch".into());
    }
    let epoch = world
        .get_resource::<GroundEpoch>()
        .ok_or("site epoch is not installed")?
        .0;
    let active = world
        .get_resource::<SiteActive>()
        .ok_or("active site is not installed")?
        .clone();
    let selection = world
        .get_resource::<SiteSelection>()
        .ok_or("site selection is not installed")?;
    if selection.site_type() != active.site_type {
        return Err("site selection and active scene disagree".into());
    }
    let floor = selection.fixture_floor_grid(
        world
            .get_resource::<Sites>()
            .ok_or("site master table is not installed")?,
    )?;
    let origin_entities: Vec<_> = world
        .query_filtered::<Entity, With<SiteCoordinateOrigin>>()
        .iter(world)
        .collect();
    let [origin_entity] = origin_entities.as_slice() else {
        return Err("one active site origin is required".into());
    };
    let origin = live_matrix(world, *origin_entity)
        .ok_or("site origin transform is not installed")?
        .transform_point3(Vec3::ZERO);
    if !origin.is_finite() {
        return Err("site origin is nonfinite".into());
    }
    let players: Vec<_> = world
        .query_filtered::<Entity, With<PlayerControlled>>()
        .iter(world)
        .collect();
    let [player] = players.as_slice() else {
        return Err("one logical player is required".into());
    };
    if world.get::<PlayerFixtureAgentParameters>(*player).is_none() {
        world
            .entity_mut(*player)
            .insert(PlayerFixtureAgentParameters::default());
    }
    let agent = *world
        .get::<PlayerFixtureAgentParameters>(*player)
        .expect("installed player parameters");
    if !agent.active_speed().is_finite() {
        return Err("player active speed is nonfinite".into());
    }
    let rows: Vec<_> = world
        .query_filtered::<(
            Entity,
            Option<&FixtureScenePlacement>,
            Option<&FixtureActivityIdentity>,
        ), With<FixtureRoot>>()
        .iter(world)
        .map(|(entity, row, identity)| (entity, row.cloned(), identity.cloned()))
        .collect();
    let mut observed = Vec::new();
    for (entity, row, identity) in rows {
        let row = row.ok_or_else(|| format!("fixture {entity:?} has no layout-owner record"))?;
        observed.push(ObservedRow {
            entity: Some(entity),
            row: row.0,
            identity,
            world_matrix: live_matrix(world, entity).map(|matrix| matrix.to_cols_array()),
        });
    }
    if let Some(edit) = world.get_resource::<crate::fixture_edit::EditSession>() {
        for row in crate::fixture_edit::decided_scene_rows(edit, &active.site_type) {
            // The current editor has data rows but no persistent placed entity.
            // Keep this identity/revision visible; do not pretend its preview is
            // a playable fixture instance or a complete geometry installation.
            observed.push(ObservedRow {
                entity: None,
                row,
                identity: None,
                world_matrix: None,
            });
        }
    }
    observed.sort_by(|a, b| {
        a.row
            .uid
            .cmp(&b.row.uid)
            .then_with(|| a.entity.cmp(&b.entity))
    });
    let areas_revision = world
        .get_resource::<FixtureFloorAreas>()
        .ok_or("floor area supplier is not installed")?
        .revision;
    Ok(Signature {
        epoch,
        site: active.site_type,
        floor,
        origin_entity: *origin_entity,
        origin,
        player: *player,
        agent,
        areas_revision,
        rows: observed,
    })
}

fn build_floor(
    world: &World,
    signature: &Signature,
) -> Result<(FixtureFloorTiles, Vec<String>), String> {
    let tables = world
        .get_resource::<FixtureActivityTables>()
        .ok_or("fixture masters are not installed")?;
    let areas = world
        .get_resource::<FixtureFloorAreas>()
        .ok_or("fixture areas are not installed")?;
    let mut entries = Vec::new();
    let mut gaps = Vec::new();
    let domain = FixtureFloorTiles::new(
        signature.floor,
        signature.origin,
        TileOccupancySnapshot::complete(Vec::new()),
    )?;
    let mut uid_counts = HashMap::new();
    for observed in &signature.rows {
        *uid_counts
            .entry(observed.row.uid.as_str())
            .or_insert(0usize) += 1;
    }
    for observed in &signature.rows {
        let row = &observed.row;
        if uid_counts.get(row.uid.as_str()) != Some(&1) || row.uid.is_empty() {
            gaps.push(format!("layout UID is empty or duplicated: {:?}", row.uid));
            continue;
        }
        // GridData keeps layout layers separate. Keep all authored Y cells;
        // the fixture-interaction world lookup selects y=0, without flattening
        // elevated fixture bodies or AddUsing into the ground layer.
        if row.layout & layout_type::FLOOR == 0 {
            continue;
        }
        if row.layout & WALL_LAYOUT_MASK != 0 {
            gaps.push(format!("{} has a mixed wall/floor layout", row.uid));
            continue;
        }
        let result = (|| -> Result<Vec<TileOccupancyEntry>, String> {
            let identity = observed
                .identity
                .as_ref()
                .ok_or("live fixture master identity is not installed")?;
            if identity.uid != row.uid || identity.model_package != row.package {
                return Err("layout and live fixture identities disagree".into());
            }
            let master = tables
                .fixture_master(identity.master_id)
                .ok_or("fixture master is missing")?;
            if !domain.contains_grid(GridPosition::new(row.min.x, row.center_y, row.min.z)) {
                return Err("fixture minimum cell is outside the selected authored floor".into());
            }
            if row.layout_grid_size.x != master.grid_size.x
                || row.layout_grid_size.z != master.grid_size.z
            {
                return Err(format!(
                    "offline layout footprint {}x{} differs from source master {}x{}",
                    row.layout_grid_size.x,
                    row.layout_grid_size.z,
                    master.grid_size.x,
                    master.grid_size.z
                ));
            }
            let position = field_position(row.min, row.max, row.center_y, row.layout)?;
            let expected = crate::fixture::source_transform(
                (signature.origin + Vec3::from(position)).to_array(),
                direction_yaw_degrees(row.direction).to_radians(),
            )
            .to_matrix();
            let actual = observed
                .world_matrix
                .ok_or("layout row has no live fixture pose")?;
            // Input agreement check, not a movement/arrival tolerance. Both
            // values originate in the placement owner; a transform-only edit
            // must not silently leave the old tile record active.
            if actual.iter().any(|value| !value.is_finite())
                || expected
                    .to_cols_array()
                    .iter()
                    .zip(actual)
                    .any(|(a, b)| (*a - b).abs() > f32::EPSILON * 16.0 * a.abs().max(1.0))
            {
                return Err("fixture pose changed without its layout-owner record".into());
            }
            if row.min.x > row.max.x || row.min.z > row.max.z || master.grid_size.y <= 0 {
                return Err("fixture grid dimensions are invalid".into());
            }
            let mut result = Vec::new();
            for x in row.min.x as i32..=row.max.x as i32 {
                for z in row.min.z as i32..=row.max.z as i32 {
                    for y in row.center_y as i32..row.center_y as i32 + master.grid_size.y {
                        result.push(TileOccupancyEntry {
                            position: GridPosition::new(x as i8, y as i8, z as i8),
                            identity: TileOccupancyIdentity::Uid(row.uid.clone()),
                        });
                    }
                }
            }
            let mut add_using = areas
                .entries
                .get(&row.package)
                .ok_or("AddUsingGrid package record is missing")?
                .clone()?;
            let (source_center, source_direction, _) =
                moly_assets::player_data::mirror_fixture_layout(
                    row.layout_center,
                    master.grid_size,
                    row.direction,
                    row.layout,
                )?;
            add_using.rotate(source_direction, true);
            let center = rotated_center_grid(source_center, master.grid_size, source_direction);
            for cell in add_using.enable_tiles {
                let source = center + cell;
                let position = GridPosition::new(
                    i8::try_from(-i16::from(source.x) - 1)
                        .map_err(|_| "AddUsing X exceeds grid domain")?,
                    source.y,
                    source.z,
                );
                result.push(TileOccupancyEntry {
                    position,
                    identity: TileOccupancyIdentity::Uid(row.uid.clone()),
                });
            }
            Ok(result)
        })();
        match result {
            Ok(row_entries) => entries.extend(row_entries),
            Err(reason) => gaps.push(format!("{}: {reason}", row.uid)),
        }
    }
    // Unknown source rows may also contribute AddUsing outside their display
    // bounds, and may precede another row in GridData's insertion order. Until
    // those inputs close, even a known row cannot prove a tile's final UID.
    // Keep tile existence usable; never turn incomplete occupancy into empty
    // or trust a healthy-looking subset of the rows as the final dictionary.
    let snapshot = if gaps.is_empty() {
        TileOccupancySnapshot::complete(entries)
    } else {
        TileOccupancySnapshot::unavailable()
    };
    let mut floor = FixtureFloorTiles::new(signature.floor, signature.origin, snapshot)?;
    if floor.ignored_out_of_grid > 0 {
        gaps.push(format!(
            "{} supplied cells lie outside the selected authored floor",
            floor.ignored_out_of_grid
        ));
    }
    if floor.conflicting_cells > 0 {
        gaps.push(format!(
            "{} cells have conflicting layout identities; source insertion outcome is not resolved",
            floor.conflicting_cells
        ));
        floor = FixtureFloorTiles::new(
            signature.floor,
            signature.origin,
            TileOccupancySnapshot::unavailable(),
        )?;
    }
    Ok((floor, gaps))
}

/// Install after instance records, editor input and player reseeding, before
/// fixture admission. This never constructs a navigation-query implementation.
pub(crate) fn advance(world: &mut World) {
    world.init_resource::<FixtureSceneSupply>();
    let mut supply = world
        .remove_resource::<FixtureSceneSupply>()
        .expect("initialized scene supply");
    match discover(world) {
        Err(reason) => {
            supply.current = None;
            supply.signature = None;
            supply.pending = Some(reason);
        }
        Ok(signature) => {
            if supply.signature.as_ref() != Some(&signature) {
                match build_floor(world, &signature) {
                    Err(reason) => {
                        supply.current = None;
                        supply.signature = None;
                        supply.pending = Some(reason);
                    }
                    Ok((floor, gaps)) => {
                        supply.next_revision = supply
                            .next_revision
                            .checked_add(1)
                            .expect("fixture input revision exhausted");
                        supply.current = Some(Arc::new(FixtureSceneInputs {
                            stamp: FixtureSceneStamp {
                                site_epoch: signature.epoch,
                                input_revision: supply.next_revision,
                                player: signature.player,
                            },
                            site_type: signature.site.clone(),
                            floor: Arc::new(floor),
                            agent: signature.agent,
                            gaps,
                        }));
                        supply.signature = Some(signature);
                        supply.pending = None;
                    }
                }
            }
        }
    }
    let mut report = Vec::new();
    if let Some(current) = &supply.current {
        report.push(format!(
            "{:?} site={} floor-level={} coverage={:?}",
            current.stamp,
            current.site_type,
            current.floor.layout.level,
            current.floor.coverage()
        ));
        report.extend(current.gaps.iter().cloned());
        if world
            .get_resource::<PlayerFixtureNavigation>()
            .is_none_or(|nav| nav.scene_stamp != current.stamp)
        {
            report.push("same-generation NavMesh query/agent backend is not installed; scene input readiness is not seat readiness".into());
        }
    } else if let Some(pending) = &supply.pending {
        report.push(pending.clone());
    }
    if report != supply.last_report {
        for line in &report {
            info!("[fixture-scene-inputs] {line}");
        }
        supply.last_report = report;
    }
    world.insert_resource(supply);
}

/// Called after the action owner's site-change cancellation and before scene
/// teardown. Do not use this for an ordinary layout revision or a save event.
pub(crate) fn invalidate_for_site_change(world: &mut World) {
    if let Some(mut supply) = world.get_resource_mut::<FixtureSceneSupply>() {
        supply.current = None;
        supply.signature = None;
        supply.pending = Some("site transition is replacing the scene".into());
    }
    world.remove_resource::<PlayerFixtureNavigation>();
}

pub(crate) fn validate_admission(
    world: &World,
    navigation: &PlayerFixtureNavigation,
) -> Result<(), PlayerFixturePreparationError> {
    let current = world
        .get_resource::<FixtureSceneSupply>()
        .and_then(FixtureSceneSupply::current)
        .ok_or(PlayerFixturePreparationError::Missing(
            "current fixture scene inputs",
        ))?;
    if current.stamp != navigation.scene_stamp {
        return Err(PlayerFixturePreparationError::Missing(
            "NavMesh installed for the current fixture scene input revision",
        ));
    }
    Ok(())
}

/// A supplied geometry implementation must preserve failed samples and paths.
/// Moly installs its real carved WalkField adapter separately; this input owner
/// never manufactures success or changes a requested endpoint radius.
pub(crate) trait FixtureNavigationGeometry: Send + Sync {
    fn sample_position(&self, position: Vec3, max_distance: f32) -> Option<Vec3>;
    fn path(
        &self,
        from: Vec3,
        to: Vec3,
    ) -> Result<PlayerFixturePath, PlayerFixturePreparationError>;
    fn constrain_move(&self, from: Vec3, to: Vec3) -> Option<Vec3>;
}

struct BoundNavigation {
    geometry: Arc<dyn FixtureNavigationGeometry>,
    inputs: Arc<FixtureSceneInputs>,
}

impl PlayerFixtureNavWorld for BoundNavigation {
    fn sample_position(&self, position: Vec3, max_distance: f32) -> Option<Vec3> {
        self.geometry.sample_position(position, max_distance)
    }
    fn path(
        &self,
        from: Vec3,
        to: Vec3,
    ) -> Result<PlayerFixturePath, PlayerFixturePreparationError> {
        self.geometry.path(from, to)
    }
    fn constrain_move(&self, from: Vec3, to: Vec3) -> Option<Vec3> {
        self.geometry.constrain_move(from, to)
    }
    fn tile_at(&self, position: Vec3) -> FixtureTileKnowledge {
        self.inputs.floor.tile_at_world(position)
    }
}

/// The actual geometry owner calls this only after its corresponding rebuild
/// has been installed. A late completion cannot overwrite newer input state.
pub(crate) fn publish_navigation(
    world: &mut World,
    source_stamp: FixtureSceneStamp,
    geometry: Arc<dyn FixtureNavigationGeometry>,
    navigation_generation: u64,
    coverage_notes: Vec<String>,
) -> Result<(), &'static str> {
    let inputs = world
        .get_resource::<FixtureSceneSupply>()
        .and_then(FixtureSceneSupply::current)
        .filter(|inputs| inputs.stamp == source_stamp)
        .cloned()
        .ok_or("navigation completed for stale scene inputs")?;
    if world
        .get_resource::<PlayerFixtureNavigation>()
        .is_some_and(|installed| {
            installed.site_generation == source_stamp.site_epoch
                && installed.navigation_generation >= navigation_generation
        })
    {
        return Err("navigation installation generation did not advance");
    }
    let parameters = inputs.agent;
    world.insert_resource(PlayerFixtureNavigation {
        world: Arc::new(BoundNavigation { geometry, inputs }),
        scene_stamp: source_stamp,
        site_generation: source_stamp.site_epoch,
        navigation_generation,
        agent_radius: parameters.radius(),
        agent_speed: parameters.active_speed(),
        default_move_speed: parameters.default_move_speed(),
        coverage_notes,
    });
    Ok(())
}
