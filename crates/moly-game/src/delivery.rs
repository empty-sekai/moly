//! The birthday-party delivery on the delivery site (`DeliverySiteController`
//! and its model): walk into the tree, hold the button, the tree blooms, the
//! reward drops scatter, the player gathers them, and the honor reward plays
//! with its own camera.
//!
//! Flow, each piece in its module:
//! - `site`: the delivery site's scene objects (`DeliverySiteView` and the
//!   views it references), the place and board collision circles and their
//!   enter / exit message, the start / end requests the delivery screen
//!   sends, the named keyboard stand-in and the `MOLY_DELIVERY_AUTOPLAY`
//!   instrument.
//! - `server_mock`: `DeliveryServerMock`, the one panel for what the server
//!   decides (user rows, the two master configs not on disk, the API
//!   replies).
//! - `flow`: `OnStartDelivery` / `ExecuteDelivery`: the approach, the start
//!   wait, the hold loop, the release window, the end action and the API.
//! - `drops`: `OnDropItem`, the drop hop, auto-gather above the limit, the
//!   unclaimed drops of an arrival, gathering and the gather API loop.
//! - `honor`: `PlayTotalRewardAnimation` and the honor reward state.
//!
//! The honor reward camera (state 20) is `delivery_camera`.
//!
//! GameState 12: the pre-action of the first press of a visit sets
//! `GameStateManager.ChangeState(Delivery)`, and nothing reachable from the
//! delivery controller or the delivery screen sets another state until the
//! next site move; the collision manager updates only in Normal, Harvest and
//! Sketch, so from that press on no collision edge fires on this visit
//! ([`DeliveryGameState`]).
//!
//! Frames: one site is loaded at a time with its origin at the world origin,
//! and assets import with the source x axis reflected; the laws in
//! `moly_law::delivery` take source-frame positions, converted here at the
//! call.

pub(crate) mod bloom;
pub(crate) mod drops;
pub(crate) mod flow;
pub(crate) mod honor;
pub(crate) mod server_mock;
pub(crate) mod site;

use bevy::prelude::*;
use moly_law::delivery::{DropRange, PartyTally};

/// `DeliveryObjectType` (the view's serialized `_objectType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryObjectType {
    DeliveryPlace = 0,
    Information = 1,
}

/// `CollisionEventType` of an edge.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CollisionEdge {
    Enter,
    Exit,
}

/// Event 68 (`OnCollisionDeliveryObject`): the player entered or left one of
/// the two delivery circles. The delivery screen (the UI lane's) reads it to
/// show the delivery button and the board's icon.
#[derive(Message, Clone, Copy, Debug)]
pub struct DeliveryCollision {
    pub object: DeliveryObjectType,
    pub edge: CollisionEdge,
}

/// What the delivery screen sends: the delivery button's press
/// (`OnStartDelivery(birthdayDeliveryId)`) and release (`OnEndDelivery`).
/// A press without a party id means the first party in session, the
/// keyboard stand-in's choice (the screen names the party it shows).
#[derive(Message, Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeliveryRequest {
    Start(Option<i32>),
    End,
}

/// `DeliveryActionStateType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum DeliveryActionState {
    #[default]
    Idle = 0,
    InDelivery = 1,
    EndDeliveryLoop = 2,
    Pending = 3,
    GetReward = 4,
    InGathering = 5,
    Gather = 6,
}

/// `PublishBirthdayPartyProgressView(state, party, beforePoint, dt)`: what
/// the screen's gauge reads (the UI lane's).
#[derive(Message, Clone, Copy, Debug)]
pub struct DeliveryProgress {
    pub state: DeliveryActionState,
    pub party_id: Option<i32>,
    pub current_points: i32,
    /// `CurrentDeliveryPoint - beforeDeliveryPoint`.
    pub gained_points: i32,
    pub remaining_items: i32,
    pub dt: f32,
}

/// The screen closed a dialog the flow awaits: the get-resource dialog of a
/// reward or the honor reward chain. The dialogs are the UI lane's; this
/// message is their close.
#[derive(Message, Clone, Copy, Debug)]
pub struct DeliveryDialogClosed;

/// GameState Delivery (12): set by the pre-action of a visit's first press,
/// gone with the site.
#[derive(Resource, Debug)]
pub struct DeliveryGameState;

/// On the player from the press's approach until the end action's Idle, and
/// again through the honor reward state: the player state machine owns the
/// avatar (states AutoMove, 8 and 9 with the intercept gate closed), so the
/// input moves nothing (`player::advance` skips the player).
#[derive(Component)]
pub struct DeliveryHold;

/// One drop's model (`MysekaiDropItemModel`: resource type 41, the party's
/// reward material, its rarity, the site, quantity 1).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct DropModel {
    pub(crate) uid: u64,
    pub(crate) party_id: i32,
    pub(crate) material_id: i64,
    pub(crate) rarity: i32,
    pub(crate) site_id: u32,
}

/// `BirthdayPartySiteData`: one party in session on this site.
#[derive(Clone, Debug)]
pub(crate) struct PartySite {
    pub(crate) id: i32,
    pub(crate) label: String,
    pub(crate) item_material_id: i64,
    pub(crate) reward_material_id: i64,
    pub(crate) tally: PartyTally,
    /// `DropItemModelList` (synchronized).
    pub(crate) drops: Vec<DropModel>,
    /// `UnSynchronizedDropItemModelList`.
    pub(crate) unsynced_drops: Vec<DropModel>,
    /// `UnSynchronizedAutoGatherItemModelList`.
    pub(crate) auto_gathered: Vec<DropModel>,
    pub(crate) range: DropRange,
}

impl PartySite {
    /// `DropCount`: both lists.
    pub(crate) fn drop_count(&self) -> i32 {
        (self.drops.len() + self.unsynced_drops.len()) as i32
    }

    /// `IsWithinDropItemLimit`.
    pub(crate) fn within_limit(&self) -> bool {
        moly_law::delivery::is_within_drop_item_limit(
            self.drop_count(),
            self.tally.max_drop_item_count,
        )
    }

    /// `ChangeDropItemRangeData(baseAngle)`.
    pub(crate) fn change_range(&mut self, base_angle: f32) {
        let (max, synced, unsynced) = self.range_inputs();
        self.range.change(base_angle, max, synced, unsynced);
    }

    /// `UpdateDropItemRangeData`.
    pub(crate) fn update_range(&mut self) {
        let (max, synced, unsynced) = self.range_inputs();
        self.range.update(max, synced, unsynced);
    }

    fn range_inputs(&self) -> (i32, i32, i32) {
        (
            self.tally.max_drop_item_count,
            self.drops.len() as i32,
            self.unsynced_drops.len() as i32,
        )
    }

    pub(crate) fn has_drop(&self, uid: u64) -> bool {
        self.drops
            .iter()
            .chain(&self.unsynced_drops)
            .any(|d| d.uid == uid)
    }

    pub(crate) fn remove_drop(&mut self, uid: u64) {
        self.drops.retain(|d| d.uid != uid);
        self.unsynced_drops.retain(|d| d.uid != uid);
    }

    /// `BirthdayPartySiteData.UpdateSynchronizedData`: the have-quantity and
    /// the delivery points come back from the user data, the unsynchronized
    /// cost and points clear; unsynchronized drops join the synchronized
    /// list; the auto-gather list clears and the range is the full circle.
    pub(crate) fn update_synchronized(&mut self, have_quantity: i32, delivery_total_point: i32) {
        self.tally.synchronized_cost_quantity = have_quantity;
        self.tally.unsynchronized_cost = 0;
        self.tally.synchronized_points = delivery_total_point;
        self.tally.unsynchronized_points = 0;
        if !self.unsynced_drops.is_empty() {
            let moved = std::mem::take(&mut self.unsynced_drops);
            self.drops.extend(moved);
        }
        self.auto_gathered.clear();
        self.range = DropRange::ALL;
    }
}

/// `DeliverySiteModel` and the controller fields the flow shares.
#[derive(Resource)]
pub(crate) struct DeliveryModel {
    /// Set up for this site id (`RefreshBirthdayParty` on the arrival).
    pub(crate) site_id: Option<u32>,
    pub(crate) parties: Vec<PartySite>,
    pub(crate) rate: moly_law::delivery::DeliveryRate,
    /// `BirthdayDeliveryStartWaitTime`.
    pub(crate) start_wait: f32,
    /// `IsEnableDelivery`: the button is held.
    pub(crate) enabled: bool,
    /// `IsExecuteDelivery`.
    pub(crate) executing: bool,
    pub(crate) state: DeliveryActionState,
    /// `CollisionDropItemUidList`.
    pub(crate) collision_drops: Vec<u64>,
    /// `_gatherDropItemStackList`.
    pub(crate) gather_stack: Vec<DropModel>,
    /// `DeliveryPlaceObjectView._isFlowered`.
    pub(crate) flowered: bool,
    pub(crate) next_uid: u64,
}

impl Default for DeliveryModel {
    fn default() -> Self {
        Self {
            site_id: None,
            parties: Vec::new(),
            rate: moly_law::delivery::DeliveryRate::new(0.0, 0.0, 0.0),
            start_wait: 0.0,
            enabled: false,
            executing: false,
            state: DeliveryActionState::Idle,
            collision_drops: Vec::new(),
            gather_stack: Vec::new(),
            flowered: false,
            next_uid: 0,
        }
    }
}

impl DeliveryModel {
    pub(crate) fn party(&self, id: i32) -> Option<&PartySite> {
        self.parties.iter().find(|p| p.id == id)
    }

    pub(crate) fn party_mut(&mut self, id: i32) -> Option<&mut PartySite> {
        self.parties.iter_mut().find(|p| p.id == id)
    }

    /// `DeliverySiteModel.GetDropItem(uid)`.
    pub(crate) fn drop_model(&self, uid: u64) -> Option<DropModel> {
        self.parties
            .iter()
            .flat_map(|p| p.drops.iter().chain(&p.unsynced_drops))
            .find(|d| d.uid == uid)
            .cloned()
    }

    /// `DeliverySiteModel.RemoveDropItem(uid)`: the first party holding it.
    pub(crate) fn remove_drop(&mut self, uid: u64) {
        if let Some(party) = self.parties.iter_mut().find(|p| p.has_drop(uid)) {
            party.remove_drop(uid);
        }
    }

    /// `AddGatherStack`: once per uid.
    pub(crate) fn add_gather_stack(&mut self, model: DropModel) {
        if !self.gather_stack.iter().any(|d| d.uid == model.uid) {
            self.gather_stack.push(model);
        }
    }

    /// `DeliverySiteModel.UpdateSynchronizedData`: every party, from the
    /// user rows the panel last replied.
    pub(crate) fn update_synchronized(&mut self, mock: &server_mock::DeliveryServerMock) {
        for party in &mut self.parties {
            let (have, points) = mock.user_rows(party.id, party.item_material_id);
            party.update_synchronized(have, points);
        }
    }

    pub(crate) fn next_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }
}

/// `PublishBirthdayPartyProgressView`.
pub(crate) fn publish(
    progress: &mut MessageWriter<DeliveryProgress>,
    state: DeliveryActionState,
    party: Option<&PartySite>,
    before_points: i32,
    dt: f32,
) {
    let (party_id, current, remaining) = match party {
        Some(p) => (Some(p.id), p.tally.current_points(), p.tally.remaining()),
        None => (None, 0, 0),
    };
    progress.write(DeliveryProgress {
        state,
        party_id,
        current_points: current,
        gained_points: if party.is_some() {
            current - before_points
        } else {
            0
        },
        remaining_items: remaining,
        dt,
    });
}

/// Queued by the site transition: the drops go with the site, GameState 12
/// ends, the flow and the hold are cancelled; the next arrival rebuilds.
pub(crate) fn clear_for_site_change(world: &mut World) {
    let drops: Vec<Entity> = world
        .query_filtered::<Entity, With<drops::DeliveryDropItem>>()
        .iter(world)
        .collect();
    let count = drops.len();
    for entity in drops {
        if let Ok(entity) = world.get_entity_mut(entity) {
            entity.despawn();
        }
    }
    let held: Vec<Entity> = world
        .query_filtered::<Entity, With<DeliveryHold>>()
        .iter(world)
        .collect();
    for entity in held {
        world.entity_mut(entity).remove::<DeliveryHold>();
    }
    let had_state = world.remove_resource::<DeliveryGameState>().is_some();
    let was_executing = world.resource::<DeliveryModel>().executing;
    *world.resource_mut::<DeliveryModel>() = DeliveryModel::default();
    world.resource_mut::<flow::DeliveryFlow>().cancel();
    world.resource_mut::<flow::DeliveryFace>().0 = None;
    world.resource_mut::<drops::DeliveryGatherLoop>().cancel();
    world.resource_mut::<drops::DeliveryDropSpawns>().clear();
    world.resource_mut::<honor::RewardRuns>().cancel();
    world.resource_mut::<honor::DialogAwait>().open = false;
    world.resource_mut::<site::DeliverySite>().clear();
    if count > 0 || had_state || was_executing {
        info!(
            "[delivery] site change: {count} drops removed with the site; GameState Delivery {}; flow {}",
            if had_state { "ends" } else { "was not set" },
            if was_executing { "cancelled" } else { "idle" }
        );
    }
}

/// Delivery plugin. Order inside a frame: the site objects and their
/// circles, the requests, the flow (between the input read and the player
/// state writers, like the harvest action), the drops and the gather loop,
/// then the honor camera after the field camera's follow.
pub struct DeliveryPlugin;

impl Plugin for DeliveryPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<DeliveryCollision>()
            .add_message::<DeliveryRequest>()
            .add_message::<DeliveryProgress>()
            .add_message::<DeliveryDialogClosed>()
            .init_resource::<DeliveryModel>()
            .init_resource::<site::DeliverySite>()
            .init_resource::<site::DeliveryAutoplay>()
            .init_resource::<flow::DeliveryFlow>()
            .init_resource::<drops::DeliveryDropSpawns>()
            .init_resource::<drops::DeliveryGatherLoop>()
            .init_resource::<flow::DeliveryFace>()
            .init_resource::<honor::RewardRuns>()
            .init_resource::<honor::DialogAwait>()
            .init_resource::<crate::delivery_camera::DeliveryHonorCamera>()
            .init_resource::<bloom::DeliveryBloom>()
            .add_systems(Startup, server_mock::load)
            .add_systems(
                Update,
                (
                    server_mock::parse,
                    server_mock::open_panel,
                    site::resolve,
                    site::arrive,
                    site::scan,
                    drops::scan,
                    site::keyboard,
                    site::autoplay,
                    flow::advance,
                    drops::spawn,
                    drops::advance,
                    drops::gather,
                    drops::gather_loop,
                    honor::advance,
                    flow::apply_face,
                )
                    .chain()
                    .after(crate::player::read_input)
                    .before(crate::player_state::drive_from_input)
                    .before(crate::player::advance)
                    .before(crate::audio::SeDrainSet::Drain),
            )
            .add_systems(
                Update,
                bloom::advance
                    .after(crate::player_avatar::item_timeline::advance)
                    .before(crate::fixture_activity_timeline::advance),
            )
            .add_systems(
                PostUpdate,
                crate::delivery_camera::advance.after(crate::camera::follow_avatar),
            );
    }
}
