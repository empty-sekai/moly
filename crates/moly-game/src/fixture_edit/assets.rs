//! Source tables and preview assets, shared by the editor's read-only views.

use bevy::{asset::LoadState, gltf::Gltf, prelude::*};
use moly_assets::json::JsonAsset;
use moly_law::fixture::Vector3Int;
use std::collections::HashMap;

pub(super) struct CatalogRow {
    pub package: &'static str,
    pub fixture_id: i32,
    pub grid_size: Vector3Int,
}

/// Product-owned unlimited inventory mock; source master identities/dimensions.
pub(super) const CANDIDATES: [CatalogRow; 4] = [
    CatalogRow {
        package: "mysekai__fixture__mdl_bir1103_fixture_chair1",
        fixture_id: 853,
        grid_size: Vector3Int { x: 2, y: 4, z: 2 },
    },
    CatalogRow {
        package: "mysekai__fixture__mdl_bir1103_fixture_balloon1",
        fixture_id: 850,
        grid_size: Vector3Int { x: 2, y: 5, z: 2 },
    },
    CatalogRow {
        package: "mysekai__fixture__mdl_bir1103_fixture_cake1",
        fixture_id: 849,
        grid_size: Vector3Int { x: 4, y: 3, z: 4 },
    },
    CatalogRow {
        package: "mysekai__fixture__mdl_env0002_fixture_byoubu1",
        fixture_id: 455,
        grid_size: Vector3Int { x: 6, y: 5, z: 1 },
    },
];

#[derive(Clone)]
pub(super) struct MetaGrid {
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
    for (name, entry) in packages {
        if let Some(meta) = entry.get("motionArea").and_then(MetaGrid::from_json) {
            areas.motion.insert(name.clone(), meta);
        }
        if let Some(meta) = entry.get("cutsceneArea").and_then(MetaGrid::from_json) {
            areas.cutscene.insert(name.clone(), meta);
        }
    }
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
    // Parse paths, do not load the whole furniture catalog. An inventory item
    // from another map requests its own package only when it needs a preview.
    for (name, entry) in packages {
        if entry.get("status").and_then(|v| v.as_str()) == Some("exported")
            && entry.get("hasFixtureView").and_then(|v| v.as_bool()) == Some(true)
        {
            if let Some(path) = entry.get("glb").and_then(|v| v.as_str()) {
                candidates
                    .paths
                    .insert(name.clone(), format!("moly://fixture-models/{path}"));
            }
        }
    }
    for row in &CANDIDATES {
        let path = candidates
            .paths
            .get(row.package)
            .unwrap_or_else(|| {
                panic!(
                    "offline editor package is not a source fixture: {}",
                    row.package
                )
            })
            .clone();
        candidates
            .glbs
            .insert(row.package.into(), server.load(path));
    }
    candidates.index = None;
}
