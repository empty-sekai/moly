//! The birthday-party delivery on the delivery site (`DeliverySiteController`
//! and its model): walk into the tree, hold the button, the tree blooms, the
//! reward drops scatter, the player gathers them, and the honor reward plays
//! with its own camera.
//!
//! Flow, each piece in its module:
//! - `site`: the delivery site's scene objects (`DeliverySiteView` and the
//!   views it references), the place and board collision circles and their
//!   enter / exit message, and the `MOLY_DELIVERY_AUTOPLAY` instrument.
//! - `screen`: `ScreenLayerMysekaiDelivery`'s view, presenter and model
//!   (the gauge, the party cells, the delivery button and the information
//!   button), following the screen manager's layer, and the controller's
//!   `OnChangeUILayer`: the delivery button's press and release are the
//!   start / end requests.
//! - The server's side is the server model's delivery section
//!   (`crate::userdata::client::birthday_party`): the user rows and the two master configs
//!   the client reads as its copies (`ClientBirthdayPartyData`), the two API
//!   replies, and the delivery master tables (`DeliveryTables`).
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

pub mod bloom;
pub mod drops;
pub mod flow;
pub mod honor;
pub mod screen;
pub mod site;

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

/// Event 66 (`UpdateBirthdayPartyProgressView`,
/// `UpdateBirthdayPartyProgressViewEventData`) as
/// `PublishBirthdayPartyProgressView(state, siteData, beforeDeliveryPoint,
/// deltaTime)` builds it (see [`publish`]); the delivery screen reads it.
#[derive(Message, Clone, Copy, Debug)]
pub struct DeliveryProgress {
    pub state: DeliveryActionState,
    pub party_id: Option<i32>,
    /// `IsProgressUpdate`: state InDelivery and some point added.
    pub is_progress_update: bool,
    /// `CurrentMaterialQuantity`: the have-quantity minus the spent count.
    pub current_material_quantity: i32,
    /// `BeforeBirthdayDeliveryPoint`: the synchronized points.
    pub before_point: i32,
    /// `CurrentBirthdayDeliveryPoint`: synchronized plus unsynchronized.
    pub current_point: i32,
    /// `AddBirthdayDeliveryPoint`: `CurrentDeliveryPoint - beforeDeliveryPoint`.
    pub add_point: i32,
    /// `AnimationTime`: the frame time, or the first item's animation time.
    pub animation_time: f32,
}

pub use crate::field_modes::DeliveryHold;

pub use crate::field_modes::DeliveryGameState;

/// One drop's model (`MysekaiDropItemModel`: resource type 41, the party's
/// reward material, its rarity, the site, quantity 1).
#[derive(Clone, Debug, PartialEq)]
pub struct DropModel {
    pub uid: u64,
    pub party_id: i32,
    pub material_id: i64,
    pub rarity: i32,
    pub site_id: u32,
}

/// `BirthdayPartySiteData`: one party in session on this site.
#[derive(Clone, Debug)]
pub struct PartySite {
    pub id: i32,
    pub label: String,
    pub item_material_id: i64,
    pub reward_material_id: i64,
    pub tally: PartyTally,
    /// `DropItemModelList` (synchronized).
    pub drops: Vec<DropModel>,
    /// `UnSynchronizedDropItemModelList`.
    pub unsynced_drops: Vec<DropModel>,
    /// `UnSynchronizedAutoGatherItemModelList`.
    pub auto_gathered: Vec<DropModel>,
    pub range: DropRange,
}

impl PartySite {
    /// `DropCount`: both lists.
    pub fn drop_count(&self) -> i32 {
        (self.drops.len() + self.unsynced_drops.len()) as i32
    }

    /// `IsWithinDropItemLimit`.
    pub fn within_limit(&self) -> bool {
        moly_law::delivery::is_within_drop_item_limit(
            self.drop_count(),
            self.tally.max_drop_item_count,
        )
    }

    /// `ChangeDropItemRangeData(baseAngle)`.
    pub fn change_range(&mut self, base_angle: f32) {
        let (max, synced, unsynced) = self.range_inputs();
        self.range.change(base_angle, max, synced, unsynced);
    }

    /// `UpdateDropItemRangeData`.
    pub fn update_range(&mut self) {
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

    pub fn has_drop(&self, uid: u64) -> bool {
        self.drops
            .iter()
            .chain(&self.unsynced_drops)
            .any(|d| d.uid == uid)
    }

    pub fn remove_drop(&mut self, uid: u64) {
        self.drops.retain(|d| d.uid != uid);
        self.unsynced_drops.retain(|d| d.uid != uid);
    }

    /// `BirthdayPartySiteData.UpdateSynchronizedData`: the have-quantity and
    /// the delivery points come back from the user data, the unsynchronized
    /// cost and points clear; unsynchronized drops join the synchronized
    /// list; the auto-gather list clears and the range is the full circle.
    /// The user rows `UpdateSynchronizedData` reads: the delivery item's
    /// have-quantity and the party's `deliveryTotalPoint` (0 without a row).
    pub fn user_rows(
        &self,
        client: &crate::userdata::client::birthday_party::ClientBirthdayPartyData,
    ) -> (i32, i32) {
        (
            i32::try_from(self.item_material_id).map_or(0, |id| client.have_quantity(id)),
            client
                .user_birthday_party(self.id)
                .map_or(0, |row| row.delivery_total_point),
        )
    }

    pub fn update_synchronized(&mut self, have_quantity: i32, delivery_total_point: i32) {
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
pub struct DeliveryModel {
    /// Set up for this site id (`RefreshBirthdayParty` on the arrival).
    pub site_id: Option<u32>,
    pub parties: Vec<PartySite>,
    pub rate: moly_law::delivery::DeliveryRate,
    /// `BirthdayDeliveryStartWaitTime`.
    pub start_wait: f32,
    /// `IsEnableDelivery`: the button is held.
    pub enabled: bool,
    /// `IsExecuteDelivery`.
    pub executing: bool,
    pub state: DeliveryActionState,
    /// `CollisionDropItemUidList`.
    pub collision_drops: Vec<u64>,
    /// `_gatherDropItemStackList`.
    pub gather_stack: Vec<DropModel>,
    /// `DeliveryPlaceObjectView._isFlowered`.
    pub flowered: bool,
    pub next_uid: u64,
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
    pub fn party(&self, id: i32) -> Option<&PartySite> {
        self.parties.iter().find(|p| p.id == id)
    }

    pub fn party_mut(&mut self, id: i32) -> Option<&mut PartySite> {
        self.parties.iter_mut().find(|p| p.id == id)
    }

    /// `DeliverySiteModel.GetDropItem(uid)`.
    pub fn drop_model(&self, uid: u64) -> Option<DropModel> {
        self.parties
            .iter()
            .flat_map(|p| p.drops.iter().chain(&p.unsynced_drops))
            .find(|d| d.uid == uid)
            .cloned()
    }

    /// `DeliverySiteModel.RemoveDropItem(uid)`: the first party holding it.
    pub fn remove_drop(&mut self, uid: u64) {
        if let Some(party) = self.parties.iter_mut().find(|p| p.has_drop(uid)) {
            party.remove_drop(uid);
        }
    }

    /// `AddGatherStack`: once per uid.
    pub fn add_gather_stack(&mut self, model: DropModel) {
        if !self.gather_stack.iter().any(|d| d.uid == model.uid) {
            self.gather_stack.push(model);
        }
    }

    /// `DeliverySiteModel.UpdateSynchronizedData`: every party, from the
    /// client's copies of the user data (`GetHaveQuantity` of the delivery
    /// item, `GetUserBirthdayParty(id).deliveryTotalPoint`, 0 without a row).
    pub fn update_synchronized(
        &mut self,
        client: &crate::userdata::client::birthday_party::ClientBirthdayPartyData,
    ) {
        for party in &mut self.parties {
            let (have, points) = party.user_rows(client);
            party.update_synchronized(have, points);
        }
    }

    pub fn next_uid(&mut self) -> u64 {
        self.next_uid += 1;
        self.next_uid
    }
}

/// `DeliverySiteModel.GetFirstAnimationTime`: `(sqrt(v * v + a * 4) - v) /
/// a` with `v` the minimum rate and `a` the acceleration, in single
/// precision.
pub fn first_animation_time(min: f32, acceleration: f32) -> f32 {
    ((min * min + acceleration * 4.0).sqrt() - min) / acceleration
}

/// `PublishBirthdayPartyProgressView(state, siteData, beforeDeliveryPoint,
/// deltaTime)`: with site data the added points are `CurrentDeliveryPoint -
/// beforeDeliveryPoint`, the event is a progress update when the state is
/// InDelivery and the added points are above 0, and the first item of a
/// delivery (the spent count is 1) of a progress update carries
/// `GetFirstAnimationTime` instead of the frame time. The event's quantity
/// and points are filled only for a progress update with site data.
pub fn publish(
    progress: &mut MessageWriter<DeliveryProgress>,
    state: DeliveryActionState,
    party: Option<&PartySite>,
    before_points: i32,
    dt: f32,
    rate: moly_law::delivery::DeliveryRate,
) {
    let (add, is_progress_update, first) = match party {
        Some(p) => {
            let add = p.tally.current_points() - before_points;
            (
                add,
                state == DeliveryActionState::InDelivery && add > 0,
                p.tally.unsynchronized_cost == 1,
            )
        }
        None => (0, false, false),
    };
    let animation_time = if first && is_progress_update {
        first_animation_time(rate.min, rate.acceleration)
    } else {
        dt
    };
    let (quantity, before, current) = match (is_progress_update, party) {
        (true, Some(p)) => (
            p.tally.remaining(),
            p.tally.synchronized_points,
            p.tally.current_points(),
        ),
        _ => (0, 0, 0),
    };
    progress.write(DeliveryProgress {
        state,
        party_id: party.map(|p| p.id),
        is_progress_update,
        current_material_quantity: quantity,
        before_point: before,
        current_point: current,
        add_point: if is_progress_update { add } else { 0 },
        animation_time,
    });
}

/// `ObjectCollisionManager.TriggerOnEnterCollisions` (the controller's
/// `OnChangeUILayer` of the delivery screen): every object the player
/// collides with runs its enter callback again, the place and the board in
/// `site::scan`, the drops in `drops::scan`, on the next scan.
#[derive(Resource, Default)]
pub struct DeliveryEnterRetrigger {
    pub site: bool,
    pub drops: bool,
}

/// Queued by the site transition: the drops go with the site, GameState 12
/// ends, the flow and the hold are cancelled; the next arrival rebuilds.
pub fn clear_for_site_change(world: &mut World) {
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
    flow::release_auto_move(world);
    world.resource_mut::<flow::DeliveryFlow>().cancel();
    world.resource_mut::<flow::DeliveryFace>().0 = None;
    world.resource_mut::<drops::DeliveryGatherLoop>().cancel();
    world.resource_mut::<drops::DeliveryDropSpawns>().clear();
    let shown = world.resource_mut::<honor::RewardRuns>().cancel();
    if !shown.is_empty() {
        let mut screens = world.resource_mut::<crate::ui_layers::ScreenManager>();
        for id in &shown {
            screens.close_dialog(*id);
            screens.dialog_destroyed(*id);
        }
        info!("[delivery] site change: the reward dialogs {shown:?} close with their run");
    }
    let refresh = std::mem::take(&mut *world.resource_mut::<honor::DialogAwait>()).refresh;
    if let Some(id) = refresh {
        // The site's enable token cancels the refresh's `WaitUntil`; the
        // dialog goes with the site's UI.
        let mut screens = world.resource_mut::<crate::ui_layers::ScreenManager>();
        screens.close_dialog(id);
        screens.dialog_destroyed(id);
        info!("[delivery] site change: the refresh dialog {id:?} closes with the end action");
    }
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
            .init_resource::<screen::DeliveryScreen>()
            .init_resource::<DeliveryEnterRetrigger>()
            .add_systems(Startup, screen::load)
            .add_systems(Update, (screen::load_names, screen::spawn_when_ready))
            .add_systems(
                Update,
                (
                    site::resolve,
                    site::arrive,
                    site::scan,
                    drops::scan,
                    screen::follow_screen,
                    screen::input.run_if(crate::game_settings::scene_input_enabled),
                    site::autoplay,
                    flow::advance,
                    drops::spawn,
                    drops::advance,
                    drops::gather,
                    drops::gather_loop,
                    honor::advance,
                    flow::apply_face,
                    screen::update_model,
                    screen::advance,
                )
                    .chain()
                    .after(crate::player_input::read_input)
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
                Update,
                site::prepare_signal_targets
                    .after(site::resolve)
                    .before(crate::player_avatar::item_timeline::advance),
            )
            .add_systems(
                PostUpdate,
                crate::delivery_camera::advance.after(crate::camera::follow_avatar),
            );
    }
}
