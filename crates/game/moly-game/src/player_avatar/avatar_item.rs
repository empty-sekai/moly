//! The avatar's hand items (`PlayerAvatarView.UpdateAvatarItem` /
//! `ClearAvatarItem`), driven by the clips' `PublishAvatarItemUseLeft/Right`
//! events.
//!
//! `UpdateAvatarItem(hand, item, "mysekai/player_tool_model/", file)`: for
//! "Left" (else "Right") the hand's current item object is destroyed; unless
//! the item is `None`, the prefab `<file>.prefab` of the package
//! `mysekai/player_tool_model/<file>` is instantiated under the arm (the left
//! arm is the avatar body's `Penlight_L`, the right arm its `Penlight_R`, both
//! found by name below the avatar root when the view is set up) with a zero
//! local position and an identity local rotation. `ClearAvatarItem` is the
//! update of both hands with `None`. The file comes from
//! `GetFileNameForAvatarItem` (a table of the nine items; any other value
//! gives the empty string).
//!
//! The source loads the prefab synchronously; here every listed model is
//! requested when the index arrives, and an item asked for before its model
//! is loaded is created on the first frame it can be (named, with the wait).
//! The items keep their glTF materials (their source shader is not ported).

use std::collections::HashMap;

use bevy::asset::LoadState;
use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::SceneRoot;
use moly_assets::json::JsonAsset;

use crate::player::PlayerControlled;
use crate::player_avatar::AvatarDriver;

/// The player tool model export's index.
pub(crate) const INDEX: &str = "moly://avatar/player-tool-models/index.json";
/// `CreateAvatarItem`'s bundle directory literal.
const BUNDLE_DIRECTORY: &str = "mysekai/player_tool_model/";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hand {
    Left,
    Right,
}

impl Hand {
    /// The arm the view found at setup (`GetArm`).
    fn arm(self) -> &'static str {
        match self {
            Self::Left => "Penlight_L",
            Self::Right => "Penlight_R",
        }
    }

    fn slot(self) -> usize {
        match self {
            Self::Left => 0,
            Self::Right => 1,
        }
    }
}

/// `AvatarItem`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvatarItem {
    None = 0,
    Hammer = 1,
    Saw = 2,
    Kanna = 3,
    Spray = 4,
    Brush = 5,
    Memo = 6,
    Pencil = 7,
    DrawingBoard = 8,
    SketchPencil = 9,
}

impl AvatarItem {
    /// `TextUtility.StringToEnum(name, None)`: the member of that name, else
    /// the default.
    pub(crate) fn from_name(name: &str) -> Self {
        match name {
            "Hammer" => Self::Hammer,
            "Saw" => Self::Saw,
            "Kanna" => Self::Kanna,
            "Spray" => Self::Spray,
            "Brush" => Self::Brush,
            "Memo" => Self::Memo,
            "Pencil" => Self::Pencil,
            "DrawingBoard" => Self::DrawingBoard,
            "SketchPencil" => Self::SketchPencil,
            _ => Self::None,
        }
    }

    /// `GetFileNameForAvatarItem`: the table entry of `item - 1` when it is
    /// below 9, else the empty string.
    pub(crate) fn file_name(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Hammer => "tonkachi",
            Self::Saw => "saw",
            Self::Kanna => "planer",
            Self::Spray => "spray",
            Self::Brush => "brush",
            Self::Memo => "memo",
            Self::Pencil => "pencil",
            Self::DrawingBoard => "mdl_site_tool_drawingboard01",
            Self::SketchPencil => "mdl_site_tool_pencil01",
        }
    }
}

/// A call on the avatar's item view.
#[derive(Message, Debug, Clone, Copy)]
pub(crate) enum AvatarItemRequest {
    /// `UpdateAvatarItem(hand, item, ...)`.
    Update { hand: Hand, item: AvatarItem },
    /// `ClearAvatarItem`.
    Clear,
}

struct ItemFiles {
    glb: String,
    scene: usize,
}

#[derive(Resource, Default)]
pub(crate) struct AvatarItemView {
    index_request: Option<Handle<JsonAsset>>,
    index: Option<HashMap<String, ItemFiles>>,
    absent: bool,
    gltfs: HashMap<String, Handle<Gltf>>,
    /// The item object on each hand (left, right).
    objects: [Option<(AvatarItem, Entity)>; 2],
    /// An item asked for before it could be created, with when it was asked.
    pending: [Option<(AvatarItem, f32)>; 2],
}

impl AvatarItemView {
    /// The item on each hand, for the evidence lines.
    pub(crate) fn describe(&self) -> String {
        let hand = |slot: usize, hand: Hand| match (self.objects[slot], self.pending[slot]) {
            (Some((item, entity)), _) => format!("{hand:?} {item:?} {entity:?} under {}", hand.arm()),
            (None, Some((item, _))) => format!("{hand:?} {item:?} waiting for its model"),
            (None, None) => format!("{hand:?} none"),
        };
        format!("{}; {}", hand(0, Hand::Left), hand(1, Hand::Right))
    }
}

pub(crate) fn load(mut view: ResMut<AvatarItemView>, server: Res<AssetServer>) {
    view.index_request = Some(server.load(bevy::asset::AssetPath::from(INDEX.to_owned())));
}

/// Update: read the index and request every listed model.
pub(crate) fn parse(
    mut view: ResMut<AvatarItemView>,
    server: Res<AssetServer>,
    json: Res<Assets<JsonAsset>>,
) {
    if view.index.is_some() || view.absent {
        return;
    }
    let Some(handle) = view.index_request.clone() else {
        return;
    };
    match server.load_state(&handle) {
        LoadState::Failed(error) => {
            warn!("[avatar-item] input absent, no item in the avatar's hands: {INDEX}: {error}");
            view.absent = true;
            return;
        }
        LoadState::Loaded => {}
        _ => return,
    }
    let Some(asset) = json.get(&handle) else {
        return;
    };
    let value: serde_json::Value =
        serde_json::from_str(&asset.0).unwrap_or_else(|error| panic!("{INDEX}: not JSON: {error}"));
    let tools = value["tools"]
        .as_object()
        .unwrap_or_else(|| panic!("{INDEX}: no tools object"));
    let mut index = HashMap::new();
    for (leaf, row) in tools {
        let glb = row["glb"]
            .as_str()
            .unwrap_or_else(|| panic!("{INDEX}: item {leaf} without glb"));
        let scene = row["sourcePrefab"]["scene"]
            .as_u64()
            .unwrap_or_else(|| panic!("{INDEX}: item {leaf} without a prefab scene"))
            as usize;
        let request = server.load::<Gltf>(bevy::asset::AssetPath::from(format!(
            "moly://avatar/player-tool-models/{glb}"
        )));
        view.gltfs.insert(leaf.clone(), request);
        index.insert(
            leaf.clone(),
            ItemFiles {
                glb: glb.to_owned(),
                scene,
            },
        );
    }
    let mut leaves: Vec<&String> = index.keys().collect();
    leaves.sort();
    info!(
        "[avatar-item] player tool model index: {} models {leaves:?} (all requested)",
        index.len()
    );
    view.index = Some(index);
}

/// The arm below the avatar's model root.
fn find_arm(
    hand: Hand,
    driver: &AvatarDriver,
    names: &Query<(Entity, &Name)>,
    parents: &Query<&ChildOf>,
) -> Option<Entity> {
    names.iter().find_map(|(entity, name)| {
        if name.as_str() != hand.arm() {
            return None;
        }
        let mut cursor = entity;
        while let Ok(parent) = parents.get(cursor) {
            cursor = parent.parent();
            if cursor == driver.visual_root || cursor == driver.player {
                return Some(entity);
            }
        }
        None
    })
}

enum Created {
    Done(Entity),
    Wait(&'static str),
    Refused(String),
}

#[allow(clippy::too_many_arguments)]
fn create(
    view: &AvatarItemView,
    hand: Hand,
    item: AvatarItem,
    commands: &mut Commands,
    server: &AssetServer,
    gltfs: &Assets<Gltf>,
    driver: Option<&AvatarDriver>,
    names: &Query<(Entity, &Name)>,
    parents: &Query<&ChildOf>,
) -> Created {
    let file = item.file_name();
    if view.absent {
        return Created::Refused(format!("{INDEX} is absent"));
    }
    let Some(index) = view.index.as_ref() else {
        return Created::Wait("the model index");
    };
    let Some(files) = index.get(file) else {
        return Created::Refused(format!(
            "no model {BUNDLE_DIRECTORY}{file} in the player tool model index"
        ));
    };
    let Some(handle) = view.gltfs.get(file) else {
        return Created::Wait("the model request");
    };
    if let LoadState::Failed(error) = server.load_state(handle) {
        return Created::Refused(format!("model {} failed to load: {error}", files.glb));
    }
    let Some(gltf) = gltfs.get(handle) else {
        return Created::Wait("the model file");
    };
    let Some(driver) = driver else {
        return Created::Wait("the avatar body");
    };
    let Some(arm) = find_arm(hand, driver, names, parents) else {
        return Created::Wait("the arm node");
    };
    let Some(scene) = gltf.scenes.get(files.scene).cloned() else {
        return Created::Refused(format!(
            "model {}: prefab scene {} out of range",
            files.glb, files.scene
        ));
    };
    let entity = commands
        .spawn((
            SceneRoot(scene),
            Transform::IDENTITY,
            Visibility::Inherited,
            ChildOf(arm),
            Name::new(format!("avatar item {file}")),
        ))
        .id();
    info!(
        "[avatar-item] UpdateAvatarItem({hand:?}, {item:?}, {BUNDLE_DIRECTORY}, {file}): {entity:?} created under {} ({arm:?}) at local position 0, identity rotation",
        hand.arm()
    );
    Created::Done(entity)
}

/// Update: apply the item calls; create a waiting item once it can be.
#[allow(clippy::too_many_arguments)]
pub(crate) fn apply(
    mut commands: Commands,
    time: Res<Time>,
    server: Res<AssetServer>,
    gltfs: Res<Assets<Gltf>>,
    mut requests: MessageReader<AvatarItemRequest>,
    mut view: ResMut<AvatarItemView>,
    players: Query<&AvatarDriver, With<PlayerControlled>>,
    names: Query<(Entity, &Name)>,
    parents: Query<&ChildOf>,
) {
    let now = time.elapsed_secs();
    let driver = players.single().ok();
    let mut calls: Vec<(Hand, AvatarItem)> = Vec::new();
    for request in requests.read() {
        match *request {
            AvatarItemRequest::Update { hand, item } => calls.push((hand, item)),
            AvatarItemRequest::Clear => {
                calls.push((Hand::Left, AvatarItem::None));
                calls.push((Hand::Right, AvatarItem::None));
            }
        }
    }
    for (hand, item) in calls {
        let slot = hand.slot();
        if let Some((old, entity)) = view.objects[slot].take() {
            if let Ok(mut entity_commands) = commands.get_entity(entity) {
                entity_commands.despawn();
            }
            info!(
                "[avatar-item] UpdateAvatarItem({hand:?}, {item:?}): {old:?} {entity:?} destroyed"
            );
        }
        if let Some((waiting, _)) = view.pending[slot].take() {
            info!("[avatar-item] UpdateAvatarItem({hand:?}, {item:?}): the waiting {waiting:?} is dropped");
        }
        if item == AvatarItem::None {
            continue;
        }
        match create(
            &view, hand, item, &mut commands, &server, &gltfs, driver, &names, &parents,
        ) {
            Created::Done(entity) => view.objects[slot] = Some((item, entity)),
            Created::Wait(what) => {
                info!("[avatar-item] UpdateAvatarItem({hand:?}, {item:?}): waiting for {what}");
                view.pending[slot] = Some((item, now));
            }
            Created::Refused(reason) => {
                warn!("[avatar-item] UpdateAvatarItem({hand:?}, {item:?}) not drawn: {reason}");
            }
        }
    }
    for hand in [Hand::Left, Hand::Right] {
        let slot = hand.slot();
        let Some((item, since)) = view.pending[slot] else {
            continue;
        };
        match create(
            &view, hand, item, &mut commands, &server, &gltfs, driver, &names, &parents,
        ) {
            Created::Done(entity) => {
                info!(
                    "[avatar-item] {hand:?} {item:?} created {:.3}s after it was asked for (its model was not loaded yet)",
                    now - since
                );
                view.pending[slot] = None;
                view.objects[slot] = Some((item, entity));
            }
            Created::Wait(_) => {}
            Created::Refused(reason) => {
                warn!("[avatar-item] {hand:?} {item:?} not drawn: {reason}");
                view.pending[slot] = None;
            }
        }
    }
}
