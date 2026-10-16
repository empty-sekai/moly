//! Original thumbnail assets and Sprite metrics, loaded by the ordinary asset source.

use bevy::{asset::LoadState, prelude::*};
use moly_assets::json::JsonAsset;
use serde_json::Value;
use std::collections::BTreeMap;

use crate::ui_layout::{SpriteLayoutMetrics, UiLayouts};

#[derive(Resource, Default)]
pub(crate) struct EditorIcons {
    /// Every registered thumbnail by (fixture id, texture id).
    variants: BTreeMap<(i32, i32), String>,
    handles: Vec<Handle<Image>>,
    parsed: bool,
    ready: bool,
}

impl EditorIcons {
    /// Every registered thumbnail has loaded.
    pub(crate) fn is_ready(&self) -> bool {
        self.ready
    }

    /// The registered thumbnail of one fixture colour (`UserMysekaiFixture`
    /// id and texture id).
    pub(crate) fn variant(&self, fixture_id: i32, texture_id: i32) -> Option<&str> {
        self.variants
            .get(&(fixture_id, texture_id))
            .map(String::as_str)
    }
}

/// The fixture thumbnail catalogue. The runtime Sprite metrics of the editor
/// tabs come with the UI root's other sources (UiLayouts::runtime_sprites).
#[derive(Resource)]
pub(crate) struct EditorAssetRequests {
    thumbnails: Handle<JsonAsset>,
}

pub(crate) fn load(
    mut commands: Commands,
    server: Res<AssetServer>,
    stage: Option<Res<crate::browser_stage::BrowserStage>>,
) {
    commands.init_resource::<EditorIcons>();
    commands.init_resource::<super::EditorUiState>();
    if stage.is_some() { return; }
    commands.insert_resource(EditorAssetRequests {
        thumbnails: server.load("moly://fixture-thumbnails/fixture-thumbnails.json"),
    });
}

pub(crate) fn parse(
    mut commands: Commands,
    request: Option<Res<EditorAssetRequests>>,
    json: Res<Assets<JsonAsset>>,
    server: Res<AssetServer>,
    mut layouts: ResMut<UiLayouts>,
    mut icons: ResMut<EditorIcons>,
) {
    if icons.ready || layouts.document("EditorSource").is_none() {
        return;
    }
    if !icons.parsed {
        let Some(request) = request else {
            return;
        };
        if let LoadState::Failed(error) = server.load_state(&request.thumbnails) {
            panic!("furniture editor source metadata failed: {error:?}");
        }
        if json.get(&request.thumbnails).is_none() {
            return;
        }
        let thumbs: Value = serde_json::from_str(&json.get(&request.thumbnails).unwrap().0)
            .expect("source fixture thumbnail manifest");
        for fixture in thumbs["fixtures"]
            .as_array()
            .expect("fixture thumbnail rows")
        {
            let id = i32::try_from(fixture["fixtureId"].as_i64().expect("thumbnail fixtureId"))
                .expect("thumbnail fixture identity range");
            for variant in fixture["variants"]
                .as_array()
                .expect("fixture thumbnail variants")
            {
                let texture_id = variant["textureId"].as_u64().expect("thumbnail textureId");
                let path = variant["image"].as_str().expect("thumbnail image");
                let alias = format!("editor-fixture-{id}-{texture_id}");
                let handle = layouts.register_runtime_texture(
                    &alias,
                    &format!("moly://fixture-thumbnails/{path}"),
                    &server,
                );
                icons.handles.push(handle);
                if let Ok(texture) = i32::try_from(texture_id) {
                    icons.variants.insert((id, texture), alias);
                }
            }
        }
        let sprites = layouts
            .runtime_sprites()
            .expect("the UI root's runtime Sprite metadata is loaded with its layouts")
            .clone();
        for (name, sprite) in sprites.as_object().expect("runtime Sprite map") {
            if !name.starts_with("editor-tab-") {
                continue;
            }
            let number = |field: &str, index: usize| {
                sprite[field][index]
                    .as_f64()
                    .filter(|v| v.is_finite())
                    .expect("source Sprite metric") as f32
            };
            layouts.set_runtime_sprite_layout(
                name,
                SpriteLayoutMetrics {
                    rect_size: Vec2::new(number("rectSize", 0), number("rectSize", 1)),
                    border: [
                        number("border", 0),
                        number("border", 1),
                        number("border", 2),
                        number("border", 3),
                    ],
                    pixels_per_unit: sprite["pixelsPerUnit"]
                        .as_f64()
                        .expect("Sprite pixelsPerUnit") as f32,
                },
            );
        }
        icons.parsed = true;
        commands.remove_resource::<EditorAssetRequests>();
    }
    icons.ready = icons
        .handles
        .iter()
        .all(|handle| match server.load_state(handle) {
            LoadState::Loaded => true,
            LoadState::Failed(error) => panic!("furniture editor thumbnail failed: {error:?}"),
            _ => false,
        });
}
