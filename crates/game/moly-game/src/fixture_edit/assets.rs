//! Source tables and preview assets, shared by the editor's read-only views.

use bevy::{asset::LoadState, gltf::Gltf, prelude::*};
use moly_assets::json::JsonAsset;
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Clone)]
pub(crate) struct MetaGrid {
    pub rows: usize,
    pub cols: usize,
    pub cells: Vec<bool>,
}

impl MetaGrid {
    pub fn cell(&self, row: usize, col: usize) -> bool {
        self.cells
            .get(row * self.cols + col)
            .copied()
            .unwrap_or(false)
    }

    fn from_json(value: &serde_json::Value) -> Option<Self> {
        let rows = value.get("dims")?.get("rows")?.as_u64()? as usize;
        let cols = value.get("dims")?.get("cols")?.as_u64()? as usize;
        if rows == 0 || cols == 0 {
            return None;
        }
        let mut cells = Vec::with_capacity(rows.checked_mul(cols)?);
        for row in value.get("cells")?.as_array()? {
            if row.as_array()?.len() != cols {
                return None;
            }
            for cell in row.as_array()? {
                cells.push(cell.as_bool()?);
            }
        }
        if cells.len() != rows * cols || !cells.iter().any(|cell| *cell) {
            return None;
        }
        Some(Self { rows, cols, cells })
    }
}

#[derive(Resource, Default)]
pub(super) struct FixtureAreas {
    pub loaded: bool,
    pub motion: HashMap<String, MetaGrid>,
    pub cutscene: HashMap<String, MetaGrid>,
    /// Each bundle meta's `stackEnables` and `AddUsingGrid` (the put rules'
    /// inputs).
    pub stack: Arc<HashMap<String, super::tile_rules::StackMeta>>,
}

#[derive(Resource)]
pub(super) struct AreasAsset(Handle<JsonAsset>);

#[derive(Resource, Default)]
pub(super) struct CandidateAssets {
    index: Option<Handle<JsonAsset>>,
    pub paths: HashMap<String, String>,
    pub glbs: HashMap<String, Handle<Gltf>>,
}

pub(super) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(CandidateAssets {
        index: Some(server.load("moly://fixture-models/index.json")),
        ..Default::default()
    });
    commands.insert_resource(AreasAsset(server.load("moly://fixture-areas/areas.json")));
    commands.init_resource::<FixtureAreas>();
}

pub(super) fn parse_areas(
    mut commands: Commands,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    asset: Option<Res<AreasAsset>>,
    mut areas: ResMut<FixtureAreas>,
    mut floor_areas: ResMut<crate::fixture_scene_inputs::FixtureFloorAreas>,
    mut npc_areas: ResMut<crate::npc_fixture_activity::NpcFixtureAreas>,
) {
    let Some(asset) = asset else {
        return;
    };
    if !matches!(server.load_state(&asset.0), LoadState::Loaded) {
        return;
    }
    let Some(raw) = json.get(&asset.0).map(|asset| asset.0.as_str()) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(raw)
        .unwrap_or_else(|err| panic!("fixture-areas/areas.json invalid: {err}"));
    floor_areas
        .replace_document(&value)
        .unwrap_or_else(|err| panic!("fixture floor area document is invalid: {err}"));
    npc_areas
        .replace_document(&value)
        .unwrap_or_else(|err| panic!("NPC fixture area document is invalid: {err}"));
    let packages = value
        .get("packages")
        .and_then(|packages| packages.as_object())
        .expect("areas.json must contain packages");
    let mut stack = HashMap::with_capacity(packages.len());
    for (name, entry) in packages {
        if let Some(meta) = entry.get("motionArea").and_then(MetaGrid::from_json) {
            areas.motion.insert(name.clone(), meta);
        }
        if let Some(meta) = entry.get("cutsceneArea").and_then(MetaGrid::from_json) {
            areas.cutscene.insert(name.clone(), meta);
        }
        stack.insert(
            name.clone(),
            super::tile_rules::StackMeta {
                stack_enables: entry.get("stackEnables").and_then(MetaGrid::from_json),
                add_using: entry.get("AddUsingGrid").and_then(MetaGrid::from_json),
            },
        );
    }
    areas.stack = Arc::new(stack);
    areas.loaded = true;
    commands.remove_resource::<AreasAsset>();
}

pub(super) fn plan_candidates(
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
    mut candidates: ResMut<CandidateAssets>,
) {
    let Some(index) = candidates.index.clone() else {
        return;
    };
    if !matches!(server.load_state(&index), LoadState::Loaded) {
        return;
    }
    let Some(raw) = json.get(&index).map(|asset| asset.0.as_str()) else {
        return;
    };
    let value: serde_json::Value = serde_json::from_str(raw)
        .unwrap_or_else(|err| panic!("fixture model index invalid: {err}"));
    let packages = value
        .get("packages")
        .and_then(|packages| packages.as_object())
        .expect("fixture model index must contain packages");
    // Parse paths, do not load the whole furniture catalog. A listed or stored
    // fixture requests its own package only when it needs a preview.
    for (name, entry) in packages {
        if entry.get("status").and_then(|v| v.as_str()) == Some("exported")
            && entry.get("hasFixtureView").and_then(|v| v.as_bool()) == Some(true)
        {
            moly_assets::coordinates::validate_document(entry).expect("editor fixture coordinates");
            if let Some(path) = entry.get("glb").and_then(|v| v.as_str()) {
                candidates
                    .paths
                    .insert(name.clone(), format!("moly://fixture-models/{path}"));
            }
        }
    }
    candidates.index = None;
}
