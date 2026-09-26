//! The delivery site's scene objects, their collision circles and the
//! autoplay instrument.
//!
//! Scene objects (`DeliverySiteView`, one instance in the site's scene
//! document): `_deliveryCollisionObjects` references the place view
//! (`DeliveryPlaceObjectView`: `_radius`, `_deliveryAnimationRadius`, the
//! tree's director) and the information board (`DeliveryInformationObjectView`:
//! `_radius`, `_offsetY`); `_itemDropStartPosition` is the transform the
//! drops start from; `_playerJumpOffsetPosition` makes the arrival point.
//! The references are source object ids; the site's scene nodes carry their
//! source identities, so each reference resolves to the node that holds that
//! component (or is that transform) under the site root. The fields come
//! from the one instance of each view class, cross-checked against the
//! node's name.
//!
//! Collision: both views register as collision objects of the collision
//! manager, whose per-object test is the 3D distance from the player to the
//! object's position against the object's own radius (inclusive). While it
//! updates (GameState Normal, Harvest or Sketch: the product's
//! `collision_updates`, which is false once GameState Delivery is set), an
//! enter or exit edge becomes event 68 (`OnCollisionDeliveryObject`), which
//! this module publishes as [`DeliveryCollision`] for the delivery screen.
//!
//! Requests: the delivery screen (`super::screen`) sends
//! [`DeliveryRequest::Start`] on the delivery button's press and
//! [`DeliveryRequest::End`] on its release. The reward dialogs close on the
//! screen manager's hardware back key (Escape). `MOLY_DELIVERY_AUTOPLAY`
//! (off by default, WARN when set) is an instrument: `hold[,close]` in
//! seconds; it walks the player through the joystick's touch stream to each
//! drop on the ground, then into the place circle, touches the drawn
//! delivery button through the window's touch stream (the gesture layer
//! and the button's source handlers take it), holds it `hold` seconds and
//! lifts the finger, and `close` seconds (default 1.5) after an awaited
//! dialog is shown presses the back key (an Escape key press and release
//! through the keyboard input stream).

use bevy::input::keyboard::{Key, KeyboardInput};
use bevy::input::touch::{TouchInput, TouchPhase};
use bevy::input::ButtonState;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::source_navigation::SourceObjectIdentity;
use moly_law::action_button::inside_circle;
use serde_json::Value;

use super::{CollisionEdge, DeliveryCollision, DeliveryModel, DeliveryObjectType};
use crate::fixture_activity_timeline::{
    ReceiverCall, SignalReaction, SignalReceiverBinding, SourceAssetId,
};
use crate::fixture_timeline_particles::SignalCall;
use crate::player::PlayerControlled;

/// A site's package: its bundle under the field prefix.
const SITE_PACKAGE_PREFIX: &str = "mysekai__site__field__";

/// The resolved scene objects of the current delivery site (product frame).
#[derive(Clone, Debug)]
pub(crate) struct DeliveryObjects {
    pub(crate) site_id: u32,
    /// The site's scene directory.
    pub(crate) scene: String,
    /// The site master's assetbundle name (the step items' bundle suffix).
    pub(crate) bundle: String,
    pub(crate) place: Vec3,
    pub(crate) place_radius: f32,
    /// `_deliveryAnimationRadius`.
    pub(crate) animation_radius: f32,
    pub(crate) board: Vec3,
    pub(crate) board_radius: f32,
    pub(crate) board_offset_y: f32,
    /// `ItemDropStartPosition`.
    pub(crate) drop_start: Vec3,
    /// `GetArrivePosition`: the site position plus the jump offset.
    pub(crate) arrive: Vec3,
    /// The place view's director (the bloom timeline's name).
    pub(crate) bloom_timeline: Option<String>,
    /// The place view's `_playableDirector` (its component id).
    pub(crate) place_director: Option<i64>,
    /// `_deliveryAnimationSignalReceiver` with its reaction table, or why it
    /// cannot be followed in the scene document.
    pub(crate) signal_receiver: Result<SignalReceiverBinding, String>,
}

#[derive(Resource, Default)]
pub(crate) struct DeliverySite {
    /// The last settle epoch seen, delivery site or not (kept across site
    /// changes, so a new site seen before its own settle does not resolve on
    /// the previous one's ground).
    seen_epoch: Option<u64>,
    /// The epoch this delivery arrival resolves on.
    active_epoch: Option<u64>,
    doc: Option<Handle<JsonAsset>>,
    pub(crate) objects: Option<DeliveryObjects>,
    failed: bool,
    /// Collision state per object (place, board).
    inside: [bool; 2],
    /// The arrival (model setup, unclaimed drops) ran for this epoch.
    pub(crate) arrived: bool,
    /// The delivery signal receiver's particle targets are prepared.
    signals_prepared: bool,
}

impl DeliverySite {
    pub(crate) fn clear(&mut self) {
        let seen = self.seen_epoch;
        *self = Self::default();
        self.seen_epoch = seen;
    }

    /// The place circle holds the player (the last edge was an enter).
    pub(crate) fn in_place(&self) -> bool {
        self.inside[0]
    }
}

fn reference_id(value: &Value, what: &str) -> Result<i64, String> {
    value["pathId"]
        .as_i64()
        .ok_or_else(|| format!("DeliverySiteView.{what} has no pathId"))
}

fn one_instance<'a>(doc: &'a Value, class: &str) -> Result<&'a Value, String> {
    let instances = doc["components"][class]["instances"]
        .as_array()
        .ok_or_else(|| format!("the delivery scene has no {class}"))?;
    match instances.as_slice() {
        [one] => Ok(one),
        many => Err(format!(
            "the delivery scene has {} {class} instances; the site view references one",
            many.len()
        )),
    }
}

fn float(fields: &Value, class: &str, name: &str) -> Result<f32, String> {
    fields[name]
        .as_f64()
        .map(|v| v as f32)
        .ok_or_else(|| format!("{class}.{name} is missing"))
}

enum Resolve {
    Pending,
    Failed(String),
}

/// Update (exclusive): the delivery signal receiver's particle targets are
/// prepared on the particle host as soon as the site's objects resolve, so
/// the step item that binds the receiver does not wait for them.
pub(crate) fn prepare_signal_targets(world: &mut World) {
    let Some(state) = world.get_resource::<DeliverySite>() else {
        return;
    };
    if state.signals_prepared {
        return;
    }
    let Some((receiver, bundle)) = state.objects.as_ref().and_then(|objects| {
        objects
            .signal_receiver
            .as_ref()
            .ok()
            .map(|receiver| (receiver.clone(), objects.bundle.clone()))
    }) else {
        return;
    };
    let package = format!("{SITE_PACKAGE_PREFIX}{bundle}");
    let started = crate::fixture_timeline_particles::realtime(world);
    match crate::fixture_activity_timeline::prepare_receiver_targets(world, &receiver, &package) {
        Ok(false) => {}
        Ok(true) => {
            world.resource_mut::<DeliverySite>().signals_prepared = true;
            info!(
                "[delivery] {}: its particle targets are prepared on the particle host ahead of the step item (real time {started:.3})",
                receiver.receiver
            );
        }
        Err(error) => {
            world.resource_mut::<DeliverySite>().signals_prepared = true;
            error!(
                "[delivery] {}: its particle targets are not prepared ahead: {}",
                receiver.receiver, error.message
            );
        }
    }
}

/// Update: resolve the scene objects on each delivery-site arrival.
#[allow(clippy::too_many_arguments)]
pub(crate) fn resolve(
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    site: Option<Res<crate::site::SiteActive>>,
    epoch: Option<Res<crate::site::GroundEpoch>>,
    mut state: ResMut<DeliverySite>,
    identities: Query<(
        Entity,
        &SourceObjectIdentity,
        &GlobalTransform,
        Option<&Name>,
    )>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<crate::site::SiteRoot>>,
) {
    let (Some(site), Some(epoch)) = (site, epoch) else {
        return;
    };
    if state.seen_epoch != Some(epoch.0) {
        state.clear();
        state.seen_epoch = Some(epoch.0);
        if site.category == "delivery" {
            state.active_epoch = Some(epoch.0);
            state.doc = Some(server.load(moly_assets::site_scene_json(&site.scene)));
        }
        return;
    }
    if state.active_epoch != Some(epoch.0) || state.objects.is_some() || state.failed {
        return;
    }
    let Some(handle) = state.doc.clone() else {
        return;
    };
    if server.load_state(&handle).is_failed() {
        error!("[delivery] the delivery site's scene document failed to load: no delivery on this visit");
        state.failed = true;
        return;
    }
    let Some(doc) = jsons.get(&handle) else {
        return;
    };
    let under_site = |entity: Entity| -> bool {
        let mut current = entity;
        loop {
            if roots.contains(current) {
                return true;
            }
            match parents.get(current) {
                Ok(parent) => current = parent.parent(),
                Err(_) => return false,
            }
        }
    };
    let find = |matches: &dyn Fn(&SourceObjectIdentity) -> bool| -> Result<(Vec3, Option<String>), Resolve> {
        let hits: Vec<_> = identities
            .iter()
            .filter(|(entity, identity, _, _)| matches(identity) && under_site(*entity))
            .collect();
        match hits.as_slice() {
            [] => Err(Resolve::Pending),
            [(_, _, global, name)] => Ok((global.translation(), name.map(|n| n.as_str().to_owned()))),
            many => Err(Resolve::Failed(format!(
                "{} site nodes carry the same source reference",
                many.len()
            ))),
        }
    };
    match read_objects(&doc.0, &site, &find) {
        Ok(objects) => {
            info!(
                "[delivery] DeliverySiteView on {} (site {}): place {:.3} radius {} animation radius {}, board {:.3} radius {} offsetY {}, ItemDropStartPosition {:.3}, arrive {:.3}, place director {:?}",
                site.scene,
                site.site_id,
                objects.place,
                objects.place_radius,
                objects.animation_radius,
                objects.board,
                objects.board_radius,
                objects.board_offset_y,
                objects.drop_start,
                objects.arrive,
                objects.bloom_timeline
            );
            state.objects = Some(objects);
        }
        Err(Resolve::Pending) => {}
        Err(Resolve::Failed(reason)) => {
            error!("[delivery] the delivery site's objects cannot be read: {reason}; no delivery on this visit");
            state.failed = true;
        }
    }
}

type Finder<'a> =
    dyn Fn(&dyn Fn(&SourceObjectIdentity) -> bool) -> Result<(Vec3, Option<String>), Resolve> + 'a;

fn read_objects(
    text: &str,
    site: &crate::site::SiteActive,
    find: &Finder<'_>,
) -> Result<DeliveryObjects, Resolve> {
    let fail = Resolve::Failed;
    let doc: Value = serde_json::from_str(text).map_err(|e| fail(format!("unreadable: {e}")))?;
    let view = crate::site_move::arrival::delivery_site_view(&doc).map_err(fail)?;
    let fields = &view["fields"];
    let refs = fields["_deliveryCollisionObjects"]
        .as_array()
        .ok_or_else(|| fail("DeliverySiteView._deliveryCollisionObjects is missing".into()))?;
    let mut place = None;
    let mut place_director = None;
    let mut board = None;
    for reference in refs {
        let class = reference["class"].as_str().unwrap_or("");
        let id = reference_id(reference, "_deliveryCollisionObjects").map_err(fail)?;
        let instance = one_instance(&doc, class).map_err(fail)?;
        let fields = &instance["fields"];
        let node_leaf = instance["node"]
            .as_str()
            .and_then(|path| path.rsplit('/').next())
            .unwrap_or("")
            .to_owned();
        let (position, name) =
            find(&|identity: &SourceObjectIdentity| identity.components.contains(&id))?;
        if name.as_deref() != Some(node_leaf.as_str()) {
            return Err(fail(format!(
                "{class} (component {id}) resolves to node {name:?}, the document names {node_leaf:?}"
            )));
        }
        let object_type = fields["_objectType"].as_i64();
        match (class, object_type) {
            ("DeliveryPlaceObjectView", Some(0)) => {
                place_director = fields["_playableDirector"]["pathId"].as_i64();
                place = Some((
                    position,
                    float(fields, class, "_radius").map_err(fail)?,
                    float(fields, class, "_deliveryAnimationRadius").map_err(fail)?,
                ));
            }
            ("DeliveryInformationObjectView", Some(1)) => {
                board = Some((
                    position,
                    float(fields, class, "_radius").map_err(fail)?,
                    float(fields, class, "_offsetY").map_err(fail)?,
                ));
            }
            _ => {
                return Err(fail(format!(
                    "a delivery collision object of class {class:?} with object type {object_type:?} is outside the two views"
                )))
            }
        }
    }
    let (place, place_radius, animation_radius) = place
        .ok_or_else(|| fail("no DeliveryPlaceObjectView among the collision objects".into()))?;
    let (board, board_radius, board_offset_y) = board.ok_or_else(|| {
        fail("no DeliveryInformationObjectView among the collision objects".into())
    })?;
    let start_id =
        reference_id(&fields["_itemDropStartPosition"], "_itemDropStartPosition").map_err(fail)?;
    let (drop_start, _) = find(&|identity: &SourceObjectIdentity| identity.transform == start_id)?;
    let offset = crate::site_move::arrival::jump_offset_of(view).map_err(fail)?;
    let bloom_timeline = doc["timelineSockets"].as_array().and_then(|sockets| {
        sockets
            .iter()
            .find(|socket| {
                socket["node"]
                    .as_str()
                    .is_some_and(|node| node.starts_with("decoration/DeliveryPlace/"))
            })
            .and_then(|socket| socket["playableAsset"].as_str())
            .map(str::to_owned)
    });
    Ok(DeliveryObjects {
        site_id: site.site_id,
        scene: site.scene.clone(),
        bundle: site.env_site.clone(),
        place,
        place_radius,
        animation_radius,
        board,
        board_radius,
        board_offset_y,
        drop_start,
        // One site at a time, its origin at the world origin.
        arrive: offset,
        bloom_timeline,
        place_director,
        signal_receiver: signal_receiver(&doc, view),
    })
}

/// A pointer field of an instance, followed to (file, pathId): `fileId` 0 is
/// the referrer's own file; another file cannot be followed here, the scene
/// document carries no external table.
fn follow(referrer: &Value, pointer: &Value, what: &str) -> Result<Option<SourceAssetId>, String> {
    let path_id = pointer["pathId"]
        .as_i64()
        .ok_or_else(|| format!("{what} has no pathId"))?;
    if path_id == 0 {
        return Ok(None);
    }
    match pointer["fileId"].as_i64() {
        Some(0) => {}
        other => {
            return Err(format!(
                "{what} points into file {other:?}, outside the referrer's"
            ))
        }
    }
    let file = referrer["file"].as_str().ok_or_else(|| {
        "the scene document's component instances carry no identity (file, pathId); the site's scene was exported before they did".to_owned()
    })?;
    Ok(Some(SourceAssetId {
        file: file.to_owned(),
        path_id: path_id.to_string(),
    }))
}

/// The node of the component `id` in the scene document: a particle system
/// or any exported component instance.
fn component_node(doc: &Value, id: &SourceAssetId) -> Option<String> {
    let path_id: i64 = id.path_id.parse().ok()?;
    let particle = doc["particles"].as_array().and_then(|rows| {
        rows.iter()
            .find(|row| row["pathId"].as_i64() == Some(path_id))
            .and_then(|row| row["node"].as_str())
    });
    particle.map(str::to_owned).or_else(|| {
        doc["components"].as_object()?.values().find_map(|entry| {
            entry["instances"].as_array()?.iter().find_map(|instance| {
                (instance["pathId"].as_i64() == Some(path_id)
                    && instance["file"].as_str() == Some(id.file.as_str()))
                .then(|| instance["node"].as_str().unwrap_or("").to_owned())
            })
        })
    })
}

/// `PersistentListenerMode`: the argument a persistent call passes.
fn call_argument(arguments: &Value, mode: i64) -> String {
    match mode {
        0 => "the event's argument".to_owned(),
        1 => String::new(),
        2 => format!("{}", arguments["m_ObjectArgument"]),
        3 => format!("{}", arguments["m_IntArgument"]),
        4 => format!("{}", arguments["m_FloatArgument"]),
        5 => format!("{}", arguments["m_StringArgument"]),
        6 => format!("{}", arguments["m_BoolArgument"].as_i64() == Some(1)),
        other => format!("mode {other}"),
    }
}

/// `DeliverySiteView._deliveryAnimationSignalReceiver`: the receiver's
/// `SignalReceiver.m_Events` pairs `m_Signals[i]` with `m_Events[i]`, a
/// UnityEvent whose persistent calls are its reaction.
fn signal_receiver(doc: &Value, view: &Value) -> Result<SignalReceiverBinding, String> {
    let field = "DeliverySiteView._deliveryAnimationSignalReceiver";
    let id = follow(
        view,
        &view["fields"]["_deliveryAnimationSignalReceiver"],
        field,
    )?
    .ok_or_else(|| format!("{field} is null"))?;
    let instances = doc["components"]["SignalReceiver"]["instances"]
        .as_array()
        .ok_or("the delivery scene has no SignalReceiver")?;
    let hits: Vec<_> = instances
        .iter()
        .filter(|instance| {
            instance["file"].as_str() == Some(id.file.as_str())
                && instance["pathId"].as_i64().map(|n| n.to_string()) == Some(id.path_id.clone())
        })
        .collect();
    let [receiver] = hits.as_slice() else {
        return Err(format!(
            "{field} ({}/{}) matches {} SignalReceiver instances",
            id.file,
            id.path_id,
            hits.len()
        ));
    };
    let events = &receiver["fields"]["m_Events"];
    let signals = events["m_Signals"]
        .as_array()
        .ok_or("SignalReceiver.m_Events.m_Signals is missing")?;
    let reactions = events["m_Events"]
        .as_array()
        .ok_or("SignalReceiver.m_Events.m_Events is missing")?;
    if signals.len() != reactions.len() {
        return Err(format!(
            "SignalReceiver.m_Events has {} signals and {} events",
            signals.len(),
            reactions.len()
        ));
    }
    let mut rows = Vec::with_capacity(signals.len());
    for (signal, event) in signals.iter().zip(reactions) {
        let signal_id = follow(receiver, signal, "SignalReceiver.m_Events.m_Signals")?
            .ok_or("SignalReceiver.m_Events.m_Signals holds a null signal")?;
        let calls = event["m_PersistentCalls"]["m_Calls"]
            .as_array()
            .ok_or("a SignalReceiver event has no m_PersistentCalls.m_Calls")?;
        let calls = calls
            .iter()
            .map(|call| {
                let full_type = call["m_TargetAssemblyTypeName"]
                    .as_str()
                    .and_then(|name| name.split(',').next());
                let type_name = full_type
                    .and_then(|name| name.rsplit('.').next())
                    .unwrap_or("?");
                let target = follow(receiver, &call["m_Target"], "a persistent call's m_Target")
                    .ok()
                    .flatten();
                let on = target.as_ref().map_or("no target".to_owned(), |target| {
                    match component_node(doc, target) {
                        Some(node) => format!("{node} ({})", target.path_id),
                        None => format!("component {}", target.path_id),
                    }
                });
                // `UnityEventCallState`: Off 0, EditorAndRuntime 1,
                // RuntimeOnly 2; an Off call gives no runtime call.
                let call_state = call["m_CallState"].as_i64();
                let state = match call_state {
                    Some(0) => ", off",
                    Some(1) => ", editor and runtime",
                    _ => "",
                };
                let method = call["m_MethodName"].as_str().unwrap_or("?");
                let mode = call["m_Mode"].as_i64().unwrap_or(-1);
                let runtime = matches!(call_state, Some(1) | Some(2));
                // `PersistentListenerMode.Void` (1) finds the method with no
                // parameter: `ParticleSystem.Play()` plays with its
                // children, `Stop()` stops emitting with its children.
                let particle = match (full_type, method, mode, &target) {
                    (Some("UnityEngine.ParticleSystem"), "Play", 1, Some(target)) if runtime => {
                        Some((SignalCall::Play, target.clone()))
                    }
                    (Some("UnityEngine.ParticleSystem"), "Stop", 1, Some(target)) if runtime => {
                        Some((SignalCall::Stop, target.clone()))
                    }
                    _ => None,
                };
                ReceiverCall {
                    text: format!(
                        "{type_name}.{method}({}) on {on}{state}",
                        call_argument(&call["m_Arguments"], mode)
                    ),
                    runtime,
                    particle,
                }
            })
            .collect();
        rows.push(SignalReaction {
            signal: signal_id,
            signal_name: signal["name"].as_str().unwrap_or("?").to_owned(),
            calls,
        });
    }
    Ok(SignalReceiverBinding {
        receiver: format!(
            "SignalReceiver on {} ({})",
            receiver["node"].as_str().unwrap_or(""),
            id.path_id
        ),
        reactions: rows,
    })
}

/// Update: the arrival (`DeliverySiteController.Initialize`): the party data
/// (`RefreshBirthdayParty`: state Idle, `SetupDeliveryData`,
/// `UpdateSynchronizedData`), the place view's `ResetView`, the unclaimed
/// drops (`GenerateUnclaimedDropItemsIfNeededAsync`) and the gather loop.
pub(crate) fn arrive(
    mut state: ResMut<DeliverySite>,
    mut model: ResMut<DeliveryModel>,
    client: Res<crate::server::delivery::ClientBirthdayPartyData>,
    parties: Option<Res<crate::birthday::BirthdayParties>>,
    tables: Option<Res<crate::server::delivery::DeliveryTables>>,
    configs: Option<Res<crate::client_config::ClientConfigs>>,
    mut spawns: ResMut<super::drops::DeliveryDropSpawns>,
    mut gather_loop: ResMut<super::drops::DeliveryGatherLoop>,
    catalog: Option<Res<crate::harvest::catalog::HarvestCatalog>>,
    mut glbs: ResMut<crate::harvest::HarvestGltfs>,
    mut docs: ResMut<crate::harvest::HarvestDocs>,
    server: Res<AssetServer>,
    frames: Res<bevy::diagnostic::FrameCount>,
) {
    if state.arrived {
        return;
    }
    let Some(objects) = state.objects.clone() else {
        return;
    };
    let (Some(configs), Some(catalog), Some(parties)) = (configs, catalog, parties) else {
        return;
    };
    // The login's user data: the join delivers the delivery sections.
    if client.revision == 0 {
        return;
    }
    state.arrived = true;
    // GetMasterBirthdayPartiesInSession.
    let in_session = parties.in_session(crate::birthday::now_ms());
    if in_session.is_empty() {
        info!("[delivery] arrival on {}: no party in session; RefreshBirthdayParty returns, no delivery", objects.scene);
        return;
    }
    let Some(tables) = tables else {
        error!("[delivery] arrival on {}: the delivery tables (birthday-party-delivery.json) are absent (the server model names them among its missing masters); no delivery", objects.scene);
        return;
    };
    use crate::server::delivery::{CONFIG_BASE_POINT, CONFIG_DROP_UPPER_LIMIT};
    let (Some(base_point), Some(drop_upper_limit)) = (
        client.master_config_int(CONFIG_BASE_POINT),
        client.master_config_int(CONFIG_DROP_UPPER_LIMIT),
    ) else {
        error!("[delivery] arrival on {}: the master configs {CONFIG_BASE_POINT} / {CONFIG_DROP_UPPER_LIMIT} were not delivered; no delivery", objects.scene);
        return;
    };
    let next_uid = model.next_uid;
    *model = DeliveryModel {
        site_id: Some(objects.site_id),
        next_uid,
        ..DeliveryModel::default()
    };
    // SetupDeliveryData: the party rows, then the four server configs.
    for party in &in_session {
        let id = party.id as i32;
        // BirthdayPartySiteData: the member bonus over the owned cards, the
        // reward loop requirement of the last reward row.
        let member_bonus = tables.member_bonus(party.id, client.cards());
        let requirement = tables.reward_loop_requirement(party.id);
        model.parties.push(super::PartySite {
            id,
            label: party.label.clone(),
            item_material_id: party.delivery_item_material_id,
            reward_material_id: party.delivery_reward_material_id,
            tally: moly_law::delivery::PartyTally {
                synchronized_cost_quantity: 0,
                unsynchronized_cost: 0,
                synchronized_points: 0,
                unsynchronized_points: 0,
                base_point,
                member_bonus,
                reward_loop_requirement: requirement,
                max_drop_item_count: drop_upper_limit,
            },
            drops: Vec::new(),
            unsynced_drops: Vec::new(),
            auto_gathered: Vec::new(),
            range: moly_law::delivery::DropRange::ALL,
        });
        // The reward drop's model is requested now (the source preloads the
        // site's bundles before the reveal).
        if let Some(material) = catalog.materials.get(&party.delivery_reward_material_id) {
            super::drops::request(&server, &catalog, &material.package, &mut glbs, &mut docs);
        }
    }
    model.rate = moly_law::delivery::DeliveryRate::new(
        configs.float(super::flow::KEY_MIN_COST_PER_SECOND),
        configs.float(super::flow::KEY_MAX_COST_PER_SECOND),
        configs.float(super::flow::KEY_COST_ACCELERATION),
    );
    model.start_wait =
        moly_law::delivery::start_wait_time(configs.int(super::flow::KEY_START_WAIT_FRAME));
    model.update_synchronized(&client);
    model.flowered = false;
    info!(
        "[delivery] DeliverySiteController.Initialize on {}: RefreshBirthdayParty {} parties {:?}; rate min {} max {} acceleration {}, start wait {} s; tallies {:?}; place view ResetView (not flowered)",
        objects.scene,
        model.parties.len(),
        model.parties.iter().map(|p| (p.id, p.label.clone())).collect::<Vec<_>>(),
        model.rate.min,
        model.rate.max,
        model.rate.acceleration,
        model.start_wait,
        model.parties.iter().map(|p| p.tally).collect::<Vec<_>>()
    );
    super::drops::generate_unclaimed(&mut model, &client, &mut spawns, &objects, "arrival");
    gather_loop.start(
        frames.0 as u64,
        configs.float(super::drops::KEY_GATHER_API_INTERVAL),
    );
}

/// Update: the collision circles of the place and the board, and the
/// delivery screen's `TriggerOnEnterCollisions` (the enter callback of each
/// object the player collides with again, whatever the game state).
pub(crate) fn scan(
    mut state: ResMut<DeliverySite>,
    eligibility: crate::interaction::InteractionEligibility,
    players: Query<&Transform, With<PlayerControlled>>,
    mut collisions: MessageWriter<DeliveryCollision>,
    mut retrigger: ResMut<super::DeliveryEnterRetrigger>,
) {
    let again = std::mem::take(&mut retrigger.site);
    let Some(objects) = state.objects.clone() else {
        return;
    };
    if again {
        for (index, object) in [
            DeliveryObjectType::DeliveryPlace,
            DeliveryObjectType::Information,
        ]
        .into_iter()
        .enumerate()
        {
            if state.inside[index] {
                collisions.write(DeliveryCollision {
                    object,
                    edge: CollisionEdge::Enter,
                });
                info!("[delivery] TriggerOnEnterCollisions: OnCollisionDeliveryObject Enter {object:?} again");
            }
        }
    }
    if !eligibility.collision_updates() {
        return;
    }
    let Ok(player) = players.single() else {
        return;
    };
    let p = player.translation.to_array();
    let circles = [
        (
            DeliveryObjectType::DeliveryPlace,
            objects.place,
            objects.place_radius,
        ),
        (
            DeliveryObjectType::Information,
            objects.board,
            objects.board_radius,
        ),
    ];
    for (index, (object, position, radius)) in circles.into_iter().enumerate() {
        let inside = inside_circle(p, position.to_array(), radius);
        if inside == state.inside[index] {
            continue;
        }
        state.inside[index] = inside;
        let edge = if inside {
            CollisionEdge::Enter
        } else {
            CollisionEdge::Exit
        };
        collisions.write(DeliveryCollision { object, edge });
        info!(
            "[delivery] OnCollisionDeliveryObject {edge:?} {object:?}: player ({:.3}, {:.3}, {:.3}), {:.3} m from the object, radius {radius}",
            p[0],
            p[1],
            p[2],
            player.translation.distance(position)
        );
    }
}

/// The instrument's progress on the current visit.
#[derive(Resource, Default)]
pub(crate) struct DeliveryAutoplay {
    warned: bool,
    visit: Option<u64>,
    walking: bool,
    pressed_at: Option<f32>,
    released: bool,
    /// The delivery button's window point the finger went down on.
    tap_point: Vec2,
    /// The wait for a drawn delivery button is logged.
    waiting_logged: bool,
    /// The awaited dialog and when the instrument first saw it.
    dialog_since: Option<(crate::ui_layers::DialogId, f32)>,
    /// The back key is down (released on the next frame).
    back_down: bool,
    /// When the walk toward the drops began; after 30 s the instrument
    /// leaves the remaining drops and walks to the place.
    drops_since: Option<f32>,
}

fn autoplay_config() -> Option<(f32, f32)> {
    let raw = std::env::var("MOLY_DELIVERY_AUTOPLAY").ok()?;
    let mut parts = raw.split(',').map(|part| part.trim().parse::<f32>());
    let hold = match parts.next() {
        Some(Ok(hold)) if hold > 0.0 => hold,
        _ => panic!("MOLY_DELIVERY_AUTOPLAY is not `hold[,close]` in seconds: {raw:?}"),
    };
    let close = match parts.next() {
        None => 1.5,
        Some(Ok(close)) if close >= 0.0 => close,
        Some(_) => panic!("MOLY_DELIVERY_AUTOPLAY is not `hold[,close]` in seconds: {raw:?}"),
    };
    Some((hold, close))
}

/// Update: `MOLY_DELIVERY_AUTOPLAY` (see the module comment).
#[allow(clippy::too_many_arguments)]
pub(crate) fn autoplay(
    time: Res<Time>,
    mut run: ResMut<DeliveryAutoplay>,
    state: Res<DeliverySite>,
    model: Res<DeliveryModel>,
    awaiting: Res<super::honor::DialogAwait>,
    joystick: Res<crate::joystick::JoystickState>,
    root_canvas: Option<Res<crate::canvas::RootCanvas>>,
    windows: Query<(Entity, &Window), With<PrimaryWindow>>,
    cameras: Query<&GlobalTransform, With<Camera3d>>,
    players: Query<&Transform, With<PlayerControlled>>,
    drops: Query<(&Transform, &super::drops::DeliveryDropItem), Without<PlayerControlled>>,
    mut touches: MessageWriter<TouchInput>,
    mut keys: MessageWriter<KeyboardInput>,
    (screen, mut window_events): (
        Res<super::screen::DeliveryScreen>,
        MessageWriter<bevy::window::WindowEvent>,
    ),
    navigation: Option<Res<crate::player_fixture_action::PlayerFixtureNavigation>>,
) {
    let Some((hold, close)) = autoplay_config() else {
        return;
    };
    if !run.warned {
        run.warned = true;
        warn!("[delivery-autoplay] MOLY_DELIVERY_AUTOPLAY is set: an instrument walks the player to the drops and into the place circle, holds the delivery button {hold} s and closes each awaited dialog after {close} s");
    }
    let now = time.elapsed_secs();
    let Ok((window_entity, window)) = windows.single() else {
        return;
    };
    // The back key, whenever a dialog is awaited: pressed, then released on
    // the next frame, through the keyboard input stream the screen manager's
    // back key reads.
    let escape = |keys: &mut MessageWriter<KeyboardInput>, state: ButtonState| {
        keys.write(KeyboardInput {
            key_code: KeyCode::Escape,
            logical_key: Key::Escape,
            state,
            text: None,
            repeat: false,
            window: window_entity,
        });
    };
    if run.back_down {
        run.back_down = false;
        escape(&mut keys, ButtonState::Released);
    }
    match awaiting.open {
        Some(dialog) => {
            let since = match run.dialog_since {
                Some((seen, since)) if seen == dialog => since,
                _ => {
                    run.dialog_since = Some((dialog, now));
                    now
                }
            };
            if now - since >= close && !run.back_down {
                // Again after another `close` seconds while it stays
                // shown (the input manager's interval gate can block a
                // press).
                run.dialog_since = Some((dialog, now));
                run.back_down = true;
                escape(&mut keys, ButtonState::Pressed);
                info!(
                    "[delivery-autoplay] presses the back key for the awaited dialog {dialog:?} ({:.2} s after it was shown)",
                    now - since
                );
            }
        }
        None => run.dialog_since = None,
    }
    let Some(objects) = state.objects.as_ref() else {
        return;
    };
    let visit = state.seen_epoch;
    if run.visit != visit {
        // A pressed back key keeps its release.
        let back_down = run.back_down;
        *run = DeliveryAutoplay {
            warned: true,
            visit,
            back_down,
            ..default()
        };
    }
    if !state.arrived || model.site_id.is_none() {
        return;
    }
    let (Some(root_canvas), Ok(camera), Ok(player)) =
        (root_canvas.as_deref(), cameras.single(), players.single())
    else {
        return;
    };
    const FINGER: u64 = 99031;
    const TAP_FINGER: u64 = 99032;
    // The joystick reads the touch messages; the gesture layer reads the
    // window's event stream, so the button's finger goes there.
    let mut tap = |phase: TouchPhase, position: Vec2| {
        window_events.write(bevy::window::WindowEvent::TouchInput(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: TAP_FINGER,
        }));
    };
    let base = Vec2::new(window.width() * 0.15, window.height() * 0.75);
    let write = |touches: &mut MessageWriter<TouchInput>, phase: TouchPhase, position: Vec2| {
        touches.write(TouchInput {
            phase,
            position,
            window: window_entity,
            force: None,
            id: FINGER,
        });
    };
    if let Some(pressed_at) = run.pressed_at {
        if !run.released && now - pressed_at >= hold {
            run.released = true;
            tap(TouchPhase::Ended, run.tap_point);
            info!(
                "[delivery-autoplay] lifts the finger from the delivery button at ({:.1}, {:.1}) after {:.3} s",
                run.tap_point.x,
                run.tap_point.y,
                now - pressed_at
            );
        }
        return;
    }
    // Target: the nearest landed drop, else the place circle.
    let drops_open = now - *run.drops_since.get_or_insert(now) < 30.0;
    let target = drops
        .iter()
        .filter(|(_, drop)| drops_open && drop.radius > 0.0 && !drop.gathering())
        .map(|(transform, _)| transform.translation)
        .min_by(|a, b| {
            a.distance(player.translation)
                .total_cmp(&b.distance(player.translation))
        });
    let to_place = target.is_none();
    if to_place && state.in_place() {
        if run.walking {
            write(&mut touches, TouchPhase::Ended, base);
            run.walking = false;
            info!("[delivery-autoplay] inside the place circle at ({:.3}, {:.3}, {:.3}): walk finger up", player.translation.x, player.translation.y, player.translation.z);
            return;
        }
        let Some(point) = screen.button_point() else {
            if !run.waiting_logged {
                run.waiting_logged = true;
                info!("[delivery-autoplay] inside the place circle: waits for the drawn delivery button to take a press");
            }
            return;
        };
        run.pressed_at = Some(now);
        run.tap_point = point;
        tap(TouchPhase::Started, point);
        info!(
            "[delivery-autoplay] touches the delivery button at ({:.1}, {:.1})",
            point.x, point.y
        );
        return;
    }
    if !joystick.enabled {
        return;
    }
    let goal = target.unwrap_or(objects.place);
    // Steer along the walk field's path (the first corner not yet reached),
    // so the trunk does not hold the walk.
    let steer = navigation
        .as_deref()
        .and_then(|navigation| navigation.world.path(player.translation, goal).ok())
        .and_then(|path| {
            path.corners
                .into_iter()
                .find(|corner| corner.xz().distance(player.translation.xz()) > 0.1)
        })
        .unwrap_or(goal);
    let delta = steer - player.translation;
    let mut world = Vec2::new(delta.x, delta.z);
    if world.length_squared() < f32::EPSILON {
        world = Vec2::X;
    }
    let world = world.normalize();
    let forward = camera.forward();
    let flat = (forward.x * forward.x + forward.z * forward.z).sqrt();
    let forward_flat = if flat <= 0.00001 {
        Vec2::ZERO
    } else {
        Vec2::new(forward.x / flat, forward.z / flat)
    };
    let right = camera.right();
    let joy = Vec2::new(
        world.dot(Vec2::new(right.x, right.z)),
        world.dot(forward_flat),
    );
    let radius = crate::joystick::HANDLE_SIZE * root_canvas.scale(window);
    if !run.walking {
        run.walking = true;
        write(&mut touches, TouchPhase::Started, base);
        info!(
            "[delivery-autoplay] walk finger down toward {} ({:.3}, {:.3}, {:.3})",
            if to_place { "the place" } else { "a drop" },
            goal.x,
            goal.y,
            goal.z
        );
        return;
    }
    write(
        &mut touches,
        TouchPhase::Moved,
        base + Vec2::new(joy.x, -joy.y) * radius,
    );
}
