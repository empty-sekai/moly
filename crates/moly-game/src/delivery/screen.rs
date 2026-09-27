//! `ScreenLayerMysekaiDelivery` (`MysekaiDelivery`, 656): the delivery
//! screen's view, presenter and model, drawn from the shell layout ([`KEY`]).
//! The layout's menu content (`_menuUIContent`) is the field menu shell's
//! view; this view draws the rest of `ScreenLayerMysekaiDeliveryView`.
//!
//! Source structure:
//! - `OnBoot`: `ScreenLayerMysekaiDeliveryPresenter.Setup(view)`: the model
//!   (kept for the layer's life), `SetupDeliveryScreenLayer` published with
//!   `OnSetup` (the controller answers with its site data list when it has
//!   one), `UpdateModel` and `OnCollisionDeliveryObject` registered,
//!   `SetActiveDeliveryContents(false)` and the information button's
//!   `ShowObject(false)`. `OnExited`: `Dispose` (the view's button, cell
//!   group and gauge disposed, both handlers removed).
//! - `OnSetup(list)`: `ScreenLayerMysekaiDeliveryModel.Setup` (view data per
//!   party: a party gone from the list is removed, a known one takes the
//!   list's quantity, a new one is built; the first selected view data is
//!   selected, index 0 when none is), then `SetupView` of the selected view
//!   data: the view's `Setup` (the buttons' icons), the cell group's setup
//!   and `SetupViewContents` (the gauge's `Setup(fill, AfterRequirePoint,
//!   memberBonus)` and `UpdateEnableButtons`).
//! - The controller's `OnChangeUILayer(MysekaiDelivery)`: the delivery
//!   button's start action (`OnStartDelivery` with the selected party id)
//!   and end action (`OnEndDelivery`) registered, the information listener
//!   replaced, and `ObjectCollisionManager.TriggerOnEnterCollisions`.
//! - `UpdateModel(event 66)`: the model's state; Idle hides the added
//!   points text; `UpdateEnableButtons(state, selected)` (the quantity read
//!   before this event's update); the selected view data's `UpdateViewData`;
//!   a progress update refreshes the cells' counts and runs
//!   `UpdateDeliveryGauge(fill, BeforeRequirePoint, AfterRequirePoint,
//!   (current - before) - added, current - before, animationTime)`, then
//!   resets the animation time.
//! - `OnCollisionDeliveryObject(event 68)`: the place's enter / exit is
//!   `SetActiveDeliveryContents` (the delivery button, the cell group and
//!   the gauge contents), the board's is the information button's
//!   `ShowObject`.
//! - `DeliveryActionButton` (a `CustomButton`): `OnPointerDown` runs the
//!   base, then the start action, and sets `_isTapping`; `OnPointerUp` and
//!   `OnPointerExit` run the base and, while tapping, the end action. Its
//!   view interaction fades `ActiveCover`'s CanvasGroup to 1 (press) or 0
//!   (release) over 0.2 s with the tween default ease.
//! - The gauge (`MysekaiDeliveryGaugeContents`): `UpdateFill` skips an end
//!   value approximately equal to the last target; otherwise a new sequence:
//!   when the fill is past the end value it tweens to 1 over `r * T`
//!   (`r = (1 - p) / ((1 - p) + end)` clamped to [0, 1]), sets the fill to 0
//!   on completion, waits 0.01 s, then tweens to `T - r * T` over `T` (the
//!   source passes the remaining time as the second tween's end value; kept
//!   as it is); otherwise one tween to the end value over `T`, both Linear.
//!   `UpdateRemainingPoint` and `UpdateAddedDeliveryPoint` are int tweens
//!   with the default ease whose updates write `MSG_REST_VALUE` /
//!   `WORD_ADD_FORMAT`; the added one writes its start at once and shows
//!   the text.
//!
//! Product mapping, named:
//! - `OnChangeUILayer` is the frame the screen manager's current screen
//!   becomes this layer (the manager records the change, it calls no
//!   delegate).
//! - The controller answers `SetupDeliveryScreenLayer` once its arrival has
//!   built the site data list: the product's screen change can come before
//!   the arrival, which the source's order does not allow.
//! - Until the site data has set up the view, the delivery contents stay
//!   hidden (the layout's placeholder texts are not the party's).
//! - The G key is a desktop stand-in for the delivery button: its press
//!   and release run the button's pointer handlers without the raycast.
//! - A text is set only when the shared glyph atlas holds every one of its
//!   characters; otherwise its node is hidden and the missing character is
//!   named once (the atlas charset takes the delivery texts from
//!   [`DeliveryCharset`] and [`WORDINGS`]).
//!
//! Named gaps: the dash button (`_dashButton.Setup` and `ShowWithAnimation`)
//! is not drawn; `_menuUIContent.EnableMenuButton` / `EnableSiteMap` are
//! the field menu shell's and are not disabled here; the information
//! button's click (`OnClickInformationButton`: the board's interaction and
//! `PushUIScreen(MysekaiDeliveryInformation)`) is not taken, the root has
//! no prefab for that screen; the press particle is not ported
//! (`CreateParticle` on `Start` instantiates `fx_mysekai_delivery_action_pt`
//! of the festival_garden site bundle under the button and keeps its
//! `UIParticle` inactive; `OnPressed` activates and plays it, `OnReleased`
//! stops and deactivates it): the site document carries that prefab's root
//! (three nodes) and no particle rows for it, so the UI particle host has
//! nothing to play; the cell count's colour
//! (`ColorUtility.FONT_COLOR_BLACK` for a positive quantity, else
//! `FONT_COLOR_PINK2`) needs a text colour the view cannot set, so the
//! serialized colour is drawn; a disabled cell button's look
//! (`disableActionType` 1) is not drawn; the cell icons need the material
//! thumbnails, which only the JP root carries.

use std::collections::{BTreeMap, BTreeSet};
use std::sync::Arc;

use bevy::asset::LoadState;
use bevy::camera::visibility::RenderLayers;
use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use moly_assets::json::JsonAsset;
use moly_assets::ui_layout::{UiComponent, UiPrefab};
use moly_law::ui::custom_button as button_rule;
use moly_law::ui::dotween::{Ease, FloatTween};
use serde_json::Value;

use super::{
    CollisionEdge, DeliveryActionState, DeliveryCollision, DeliveryEnterRetrigger, DeliveryModel,
    DeliveryObjectType, DeliveryProgress, DeliveryRequest, PartySite,
};
use crate::balloon::{BalloonArt, BALLOON_LAYER};
use crate::canvas::RootCanvas;
use crate::gesture::{UiPointerEvent, UiPointerPhase};
use crate::menu_shell::{event_pointer, ShellDialogState, SourceInputManager};
use crate::ui_layers::{MenuScreenType, ScreenHook, ScreenLayerEvent, ScreenManager};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The layout key of the screen layer's prefab.
pub(crate) const KEY: &str = "ShellDelivery";
const SCREEN: MenuScreenType = MenuScreenType::MysekaiDelivery;
const VIEW_CLASS: &str = "Sekai.Mysekai.ScreenLayerMysekaiDeliveryView";
const GAUGE_CONTENTS_CLASS: &str = "Sekai.Mysekai.MysekaiDeliveryGaugeContents";
const CELL_GROUP_CLASS: &str = "Sekai.Mysekai.MysekaiDeliveryCharacterCellGroup";
const CELL_CLASS: &str = "Sekai.Mysekai.MysekaiDeliveryCharacterCell";
const INTERACTION_CLASS: &str = "Sekai.Mysekai.DeliveryActionButtonInteraction";
const BALLOON_CLASS: &str = "Sekai.UIPartsBalloon";
const GAUGE_CLASS: &str = "Sekai.UIPartsGauge";
/// `PlayerActionButtonType.DeliveryInformation`, the information button's
/// serialized `_type`.
const INFORMATION_TYPE: i64 = 25;
const REST_VALUE: &str = "MSG_REST_VALUE";
const ADD_FORMAT: &str = "WORD_ADD_FORMAT";
const MULTIPLY_FORMAT: &str = "WORD_MULTIPLY_FORMAT";
const MEMBER_BONUS: &str = "WORD_MYSEKAI_DELIVERY_MEMBER_BONUS";
/// The wordings this screen formats, for the shared atlas charset.
#[allow(dead_code)] // Read by the atlas charset (the field menu shell's parse) through its seam.
pub(crate) const WORDINGS: [&str; 4] = [REST_VALUE, ADD_FORMAT, MULTIPLY_FORMAT, MEMBER_BONUS];
/// The characters the formatted integers print.
#[allow(dead_code)] // Read by the atlas charset (the field menu shell's parse) through its seam.
pub(crate) const FIXED_TEXTS: &[&str] = &["0123456789-"];
/// `DeliveryActionButtonInteraction`: `DOFade(1 or 0, 0.2)`.
const PRESS_FADE_SECONDS: f32 = 0.2;
/// `MysekaiDeliveryGaugeContents.GAUGE_TWEEN_INTERVAL_SECONDS`.
const GAUGE_TWEEN_INTERVAL_SECONDS: f32 = 0.01;
/// The G key stand-in's pointer id (no finger or mouse uses it).
const KEY_POINTER: i32 = -2;
const MATERIALS: &str = "moly://materials.json";
const THUMBNAILS: &str = "moly://material-thumbnails/material-thumbnails.json";

// ---------------------------------------------------------------------------
// Bindings
// ---------------------------------------------------------------------------

/// A `CustomButton` of the layout: its node, its component and its
/// serialized click fields.
struct SourceButton {
    node: usize,
    id: i64,
    config: button_rule::CustomButtonConfig,
}

struct CellBinding {
    root: String,
    button: SourceButton,
    count: String,
    selected: String,
    icon: String,
}

/// The layout nodes the view's serialized references point to, as `@id`
/// paths.
struct Bindings {
    menu_content: String,
    dash_button: String,
    gauge_contents: String,
    remaining_text: String,
    added_text: String,
    fill: String,
    fill_initial: f32,
    fill_note: &'static str,
    balloon: String,
    balloon_text: String,
    cell_group: String,
    item_name: String,
    cells: Vec<CellBinding>,
    delivery: SourceButton,
    delivery_root: String,
    delivery_icon: String,
    press_image: String,
    press_image_alpha: f32,
    info_root: String,
    info_icon: String,
    /// The layout is the JP root's (the root that carries the material
    /// thumbnails).
    region_jp: bool,
}

impl Bindings {
    fn button(&self, index: usize) -> &SourceButton {
        if index == 0 {
            &self.delivery
        } else {
            &self.cells[index - 1].button
        }
    }

    fn button_count(&self) -> usize {
        1 + self.cells.len()
    }
}

fn pointer(value: &Value, what: &str) -> i64 {
    let pair = value
        .as_array()
        .filter(|pair| pair.len() == 2)
        .unwrap_or_else(|| panic!("{KEY}: {what} is not a serialized reference"));
    assert_eq!(
        pair[0].as_i64(),
        Some(0),
        "{KEY}: {what} points outside the prefab"
    );
    pair[1]
        .as_i64()
        .filter(|id| *id != 0)
        .unwrap_or_else(|| panic!("{KEY}: {what} is null"))
}

fn reference(fields: &Value, name: &str) -> i64 {
    pointer(
        fields
            .get(name)
            .unwrap_or_else(|| panic!("{KEY}: {name} is not serialized")),
        name,
    )
}

fn component(doc: &UiPrefab, id: i64) -> (usize, &UiComponent) {
    let node = doc
        .find(&format!("@{id}"))
        .unwrap_or_else(|e| panic!("{e}"));
    let component = doc.nodes[node]
        .components
        .iter()
        .find(|c| c.path_id == id)
        .unwrap_or_else(|| panic!("{KEY}: {} has no component {id}", doc.nodes[node].path));
    (node, component)
}

fn classed<'a>(doc: &'a UiPrefab, id: i64, class: &str) -> (usize, &'a UiComponent) {
    let (node, found) = component(doc, id);
    assert_eq!(found.class, class, "{KEY}: component {id} is not a {class}");
    (node, found)
}

fn node_path(doc: &UiPrefab, index: usize) -> String {
    format!("@{}", doc.nodes[index].game_object_id)
}

/// A `UITextureLoader`'s target graphic node.
fn loader_target(doc: &UiPrefab, loader: i64) -> String {
    let (_, loader) = component(doc, loader);
    node_path(
        doc,
        component(doc, reference(&loader.fields, "targetGraphic")).0,
    )
}

/// The CustomButton's serialized click fields; the JP class serializes
/// `enableHoldRepeat`, the shared root's CN class does not.
fn button_config(doc: &UiPrefab, found: &UiComponent) -> button_rule::CustomButtonConfig {
    let fields = &found.fields;
    let jp = match doc.source.region.as_deref() {
        None => false,
        Some("jp") => true,
        Some(region) => panic!("{KEY}: no CustomButton field set for region {region}"),
    };
    let flag = |name: &str| {
        fields[name]
            .as_bool()
            .unwrap_or_else(|| panic!("{KEY}: button {} has no {name}", found.path_id))
    };
    let number = |name: &str| {
        fields[name]
            .as_i64()
            .unwrap_or_else(|| panic!("{KEY}: button {} has no {name}", found.path_id))
    };
    let enable_hold_repeat = if jp {
        flag("enableHoldRepeat")
    } else {
        assert!(
            fields.get("enableHoldRepeat").is_none(),
            "{KEY}: button {} serializes enableHoldRepeat, which its region's class does not declare",
            found.path_id
        );
        false
    };
    button_rule::CustomButtonConfig {
        se: button_rule::SeType::from_serialized(number("se"))
            .unwrap_or_else(|| panic!("{KEY}: button {} se", found.path_id)),
        other_se_name: fields["otherSeName"]
            .as_str()
            .unwrap_or_else(|| panic!("{KEY}: button {} otherSeName", found.path_id))
            .to_owned(),
        interval: button_rule::IntervalUseType::from_serialized(number("interval"))
            .unwrap_or_else(|| panic!("{KEY}: button {} interval", found.path_id)),
        absolutely_press: flag("absolutelyPress"),
        enable_long_press: flag("enableLongPress"),
        enable_hold_repeat,
    }
}

fn source_button(doc: &UiPrefab, id: i64) -> SourceButton {
    let (node, found) = component(doc, id);
    SourceButton {
        node,
        id,
        config: button_config(doc, found),
    }
}

fn bind(doc: &UiPrefab) -> Bindings {
    let view = doc
        .nodes
        .iter()
        .flat_map(|node| &node.components)
        .find(|c| c.class == VIEW_CLASS)
        .unwrap_or_else(|| panic!("{KEY}: no {VIEW_CLASS}"));
    let vf = &view.fields;
    let menu_content = node_path(doc, component(doc, reference(vf, "_menuUIContent")).0);
    let dash_button = node_path(doc, component(doc, reference(vf, "_dashButton")).0);
    // The gauge contents.
    let (gauge_node, gauge) = classed(
        doc,
        reference(vf, "_deliveryGaugeContents"),
        GAUGE_CONTENTS_CLASS,
    );
    let gf = &gauge.fields;
    let remaining_text = node_path(doc, component(doc, reference(gf, "_remainingCountText")).0);
    let added_text = node_path(doc, component(doc, reference(gf, "_addedDeliveryPoint")).0);
    let (part_node, part) = classed(doc, reference(gf, "_deliveryGauge"), GAUGE_CLASS);
    let (fill_node, fill_note) = if part.fields.get("fillImage").is_some() {
        (
            component(doc, reference(&part.fields, "fillImage")).0,
            "UIPartsGauge.fillImage",
        )
    } else {
        // This root's UIPartsGauge is not decoded: its GaugeBase/Mask is the
        // node the JP root's decoded fillImage names.
        let path = format!("{}/GaugeBase/Mask", doc.nodes[part_node].path);
        let index = doc
            .nodes
            .iter()
            .position(|node| node.path == path)
            .unwrap_or_else(|| panic!("{KEY}: {path} missing"));
        (
            index,
            "GaugeBase/Mask (this root's UIPartsGauge is not decoded; the JP root's fillImage)",
        )
    };
    let fill_initial = doc.nodes[fill_node]
        .components
        .iter()
        .find_map(|c| c.fields.get("m_FillAmount").and_then(Value::as_f64))
        .unwrap_or_else(|| panic!("{KEY}: the gauge fill image has no m_FillAmount"))
        as f32;
    let (balloon_node, balloon) = classed(doc, reference(gf, "_memberBonusBalloon"), BALLOON_CLASS);
    let balloon_text = node_path(doc, component(doc, reference(&balloon.fields, "message")).0);
    assert_eq!(
        balloon.fields["enableHideOnAwake"].as_bool(),
        Some(false),
        "{KEY}: the member bonus balloon hides on Awake (its canvas alpha rule is not ported)"
    );
    // The cell group.
    let (group_node, group) = classed(doc, reference(vf, "_characterCellGroup"), CELL_GROUP_CLASS);
    let item_name = node_path(
        doc,
        component(doc, reference(&group.fields, "_itemNameText")).0,
    );
    let cells = group.fields["_cellList"]
        .as_array()
        .unwrap_or_else(|| panic!("{KEY}: _cellList is not a list"))
        .iter()
        .map(|entry| {
            let (cell_node, cell) = classed(doc, pointer(entry, "_cellList entry"), CELL_CLASS);
            let cf = &cell.fields;
            CellBinding {
                root: node_path(doc, cell_node),
                button: source_button(doc, reference(cf, "_cellButton")),
                count: node_path(doc, component(doc, reference(cf, "_materialCount")).0),
                selected: node_path(doc, component(doc, reference(cf, "_selectedImage")).0),
                icon: loader_target(doc, reference(cf, "_textureLoader")),
            }
        })
        .collect();
    // The delivery button and its view interaction.
    let delivery = source_button(doc, reference(vf, "_deliveryActionButton"));
    let (_, button) = component(doc, delivery.id);
    let (_, interaction) = classed(
        doc,
        reference(&button.fields, "buttonViewInteraction"),
        INTERACTION_CLASS,
    );
    let (press_node, press_group) = component(doc, reference(&interaction.fields, "_onPressImage"));
    let press_image_alpha = press_group.fields["m_Alpha"]
        .as_f64()
        .unwrap_or_else(|| panic!("{KEY}: _onPressImage has no m_Alpha"))
        as f32;
    let delivery_icon = loader_target(doc, reference(&button.fields, "_iconLoader"));
    // The information button (`MysekaiActionButtonBase`).
    let (info_node, info) = component(doc, reference(vf, "_informationButton"));
    assert_eq!(
        info.fields["_type"].as_i64(),
        Some(INFORMATION_TYPE),
        "{KEY}: the information button is not of type DeliveryInformation"
    );
    let info_icon = loader_target(doc, reference(&info.fields, "_iconLoader"));
    Bindings {
        menu_content,
        dash_button,
        gauge_contents: node_path(doc, gauge_node),
        remaining_text,
        added_text,
        fill: node_path(doc, fill_node),
        fill_initial,
        fill_note,
        balloon: node_path(doc, balloon_node),
        balloon_text,
        cell_group: node_path(doc, group_node),
        item_name,
        cells,
        delivery_root: node_path(doc, delivery.node),
        delivery,
        delivery_icon,
        press_image: node_path(doc, press_node),
        press_image_alpha,
        info_root: node_path(doc, info_node),
        info_icon,
        region_jp: doc.source.region.as_deref() == Some("jp"),
    }
}

// ---------------------------------------------------------------------------
// Model
// ---------------------------------------------------------------------------

/// `MysekaiBirthdayDeliveryViewData` with its cell's `ViewData`.
#[derive(Clone, Debug)]
struct ViewData {
    party_id: i32,
    /// `deliveryItemMaterialId` (the cell's `MaterialId`).
    material_id: i64,
    /// `_memberBonus`: the owned point bonus rows' rates, summed as ints.
    member_bonus: i32,
    /// `_maxDeliveryPoint`: the last reward row's requirement.
    max_point: i32,
    /// `_currentMaterialQuantity` (the cell's `Quantity`).
    quantity: i32,
    current_point: i32,
    before_point: i32,
    add_point_in_loop: i32,
    before_require_point: i32,
    after_require_point: i32,
    animation_time: f32,
    /// The cell's `Selected`.
    selected: bool,
}

impl ViewData {
    /// The constructor from a party's site data: `CurrentPoint` is the
    /// delivery point modulo the maximum, `AfterRequirePoint` what is left
    /// to it. A party without reward rows has a maximum of 0, and the
    /// source's integer division throws: refused.
    fn new(party: &PartySite) -> Option<Self> {
        let max = party.tally.reward_loop_requirement;
        if max <= 0 {
            return None;
        }
        let total = party.tally.current_points();
        let current = total - total / max * max;
        Some(Self {
            party_id: party.id,
            material_id: party.item_material_id,
            // The site data keeps the bonus as the rates' sum over 100 in
            // single precision; times 100 and rounded it is the int sum
            // exactly for any sum the table can reach.
            member_bonus: (party.tally.member_bonus * 100.0).round() as i32,
            max_point: max,
            quantity: party.tally.remaining(),
            current_point: current,
            before_point: 0,
            add_point_in_loop: 0,
            before_require_point: 0,
            after_require_point: max - current,
            animation_time: 0.0,
            selected: false,
        })
    }

    /// `UpdateViewData(event)`.
    fn update(&mut self, event: &DeliveryProgress) {
        self.animation_time = if event.state == DeliveryActionState::InDelivery {
            self.animation_time + event.animation_time
        } else {
            0.0
        };
        if event.is_progress_update {
            self.quantity = event.current_material_quantity;
            self.add_point_in_loop = event.add_point;
            let current = event.current_point;
            let before = current.wrapping_sub(event.add_point);
            self.after_require_point = self.max_point - current % self.max_point;
            self.before_require_point = self.max_point - before % self.max_point;
            self.current_point = current;
            self.before_point = event.before_point;
        }
    }

    /// The gauge fill: `fmodf(CurrentPoint, max) / max`, 0 below 0, at most 1.
    fn gauge_fill(&self) -> f32 {
        let value = (self.current_point as f32 % self.max_point as f32) / self.max_point as f32;
        if value < 0.0 {
            0.0
        } else {
            value.min(1.0)
        }
    }
}

/// `ScreenLayerMysekaiDeliveryModel`.
#[derive(Default)]
struct ScreenModel {
    views: Vec<ViewData>,
    selected: usize,
    state: DeliveryActionState,
}

impl ScreenModel {
    fn setup(&mut self, parties: &[PartySite]) {
        if !self.views.is_empty() {
            self.views
                .retain(|view| parties.iter().any(|party| party.id == view.party_id));
        }
        for party in parties {
            match self.views.iter_mut().find(|view| view.party_id == party.id) {
                Some(view) => view.quantity = party.tally.remaining(),
                None => match ViewData::new(party) {
                    Some(view) => self.views.push(view),
                    None => error!(
                        "[delivery-screen] party {}: no reward rows, the view data's maximum is 0 and the source's division throws; the party has no view data",
                        party.id
                    ),
                },
            }
        }
        match self.views.iter().position(|view| view.selected) {
            None => self.change_selected(0),
            Some(index) if index != self.selected => self.change_selected(index),
            Some(_) => {}
        }
    }

    fn change_selected(&mut self, index: usize) {
        self.selected = index;
        for (i, view) in self.views.iter_mut().enumerate() {
            view.selected = i == index;
        }
    }

    fn selected_view(&self) -> Option<&ViewData> {
        self.views.get(self.selected)
    }
}

// ---------------------------------------------------------------------------
// Tweens
// ---------------------------------------------------------------------------

/// One Linear float tween of the gauge sequence.
struct SequenceTween {
    at: f32,
    end: f32,
    duration: f32,
    /// `OnComplete(() => _deliveryGauge.Setup(0))`.
    zero_on_complete: bool,
    started: Option<(f32, f32)>,
    done: bool,
}

impl SequenceTween {
    fn new(at: f32, end: f32, duration: f32, zero_on_complete: bool) -> Self {
        Self {
            at,
            end,
            duration,
            zero_on_complete,
            started: None,
            done: false,
        }
    }
}

/// `_gaugeSequence`: stepped as its position (the sum of the frame deltas);
/// each nested tween reads the fill on its first step and writes through
/// `UIPartsGauge.Setup(value)`.
struct GaugeSequence {
    position: f32,
    duration: f32,
    tweens: Vec<SequenceTween>,
}

impl GaugeSequence {
    fn new(tweens: Vec<SequenceTween>) -> Self {
        let duration = tweens
            .iter()
            .map(|tween| tween.at + tween.duration)
            .fold(0.0_f32, f32::max);
        Self {
            position: 0.0,
            duration,
            tweens,
        }
    }

    /// One step; returns whether the sequence completed.
    fn step(&mut self, dt: f32, fill: &mut f32) -> bool {
        self.position += dt;
        let complete = self.duration <= self.position;
        let position = if complete {
            self.duration
        } else {
            self.position
        };
        for tween in &mut self.tweens {
            if tween.done || position < tween.at {
                continue;
            }
            let local = (position - tween.at).min(tween.duration);
            let end = tween.end;
            let current = *fill;
            let (start, change) = *tween.started.get_or_insert((current, end - current));
            let eased = if tween.duration > 0.0 {
                Ease::Linear.evaluate(local, tween.duration)
            } else {
                1.0
            };
            *fill = gauge_setup(start + change * eased);
            if tween.duration <= local {
                tween.done = true;
                if tween.zero_on_complete {
                    *fill = 0.0;
                }
            }
        }
        complete
    }
}

/// `UIPartsGauge.Setup(progress)`: 0 below 0, at most 1.
fn gauge_setup(progress: f32) -> f32 {
    if progress < 0.0 {
        0.0
    } else {
        progress.min(1.0)
    }
}

/// `Mathf.Approximately(a, b)`.
fn approximately(a: f32, b: f32) -> bool {
    let tolerance = (1e-6_f32 * a.abs().max(b.abs())).max(f32::from_bits(1) * 8.0);
    (b - a).abs() < tolerance
}

/// `DOTween.To` of an int: the start read on the first update, then
/// `(int)Math.Round(start + change * ease)` (ties to even) per update.
struct IntTween {
    from: i32,
    end: i32,
    duration: f32,
    ease: Ease,
    started: Option<(i32, i32)>,
    position: f32,
    complete: bool,
}

impl IntTween {
    fn new(from: i32, end: i32, duration: f32, ease: Ease) -> Self {
        Self {
            from,
            end,
            duration,
            ease,
            started: None,
            position: 0.0,
            complete: false,
        }
    }

    fn step(&mut self, dt: f32) -> Option<i32> {
        if self.complete {
            return None;
        }
        let (start, change) = *self
            .started
            .get_or_insert((self.from, self.end.wrapping_sub(self.from)));
        self.position += dt;
        self.complete = self.duration <= self.position;
        let position = if self.complete {
            self.duration
        } else {
            self.position
        };
        let eased = if self.duration > 0.0 {
            self.ease.evaluate(position, self.duration)
        } else {
            1.0
        };
        let value = start as f32 + change as f32 * eased;
        Some((value as f64).round_ties_even() as i32)
    }
}

// ---------------------------------------------------------------------------
// View state
// ---------------------------------------------------------------------------

#[derive(Default, Clone)]
struct CellView {
    active: bool,
    selected: bool,
    material_id: Option<i64>,
    count_text: Option<String>,
}

/// What the view shows.
#[derive(Default)]
struct ViewState {
    contents_active: bool,
    info_shown: bool,
    /// `_deliveryActionButton.enabled`.
    delivery_enabled: bool,
    /// The cells' `_cellButton.enabled`.
    cells_enabled: bool,
    /// The information button's `_button.enabled`.
    info_enabled: bool,
    /// `SetupView` ran (the buttons' icons are loaded, the cells and the
    /// gauge hold the party's values).
    view_setup: bool,
    cells: Vec<CellView>,
    /// The cell group's `_selectedCellIndex`.
    group_selected: usize,
    /// The material whose name the item name text shows.
    item_material: Option<i64>,
    fill: f32,
    before_target: f32,
    sequence: Option<GaugeSequence>,
    remaining_tween: Option<IntTween>,
    added_tween: Option<IntTween>,
    remaining_text: Option<String>,
    added_text: Option<String>,
    added_shown: bool,
    /// The member bonus balloon's message; None while it is inactive.
    balloon_text: Option<String>,
    cover_alpha: f32,
    cover_tweens: Vec<FloatTween>,
    /// Progress updates seen (the log prints the first and every 30th).
    progress_updates: u32,
}

/// The pointer's press on the view's buttons (index 0: the delivery
/// button; 1 and on: the cells).
#[derive(Default)]
struct Press {
    states: Vec<button_rule::CustomButtonState>,
    /// The button the pointer's press went to (the event system's
    /// `pointerPress`).
    pressed: Option<usize>,
    held: Option<(i32, Vec2)>,
    /// The pressed button is in the held pointer's hover chain.
    entered: bool,
    /// `DeliveryActionButton._isTapping`.
    tapping: bool,
    /// The G key stand-in holds the delivery button.
    key: bool,
}

#[derive(Default)]
struct Presenter {
    /// `Setup` ran and `Dispose` did not.
    set_up: bool,
    /// `SetupDeliveryScreenLayer` is waiting for the controller's answer.
    awaiting_site_data: bool,
    /// The delivery button's start and end actions are registered.
    registered: bool,
    was_current: bool,
}

/// The material names (`materials.json`) and thumbnails.
#[derive(Default)]
struct Materials {
    names_handle: Option<Handle<JsonAsset>>,
    names: Option<BTreeMap<i64, String>>,
    thumbs_handle: Option<Handle<JsonAsset>>,
    thumbs: Option<BTreeMap<i64, String>>,
    registered: BTreeSet<i64>,
}

/// The characters of the delivery item names, for the shared atlas
/// charset (inserted once `materials.json` is read or has failed).
#[derive(Resource, Debug, Default)]
pub(crate) struct DeliveryCharset {
    #[allow(dead_code)]
    // Read by the atlas charset (the field menu shell's parse) through its seam.
    pub(crate) chars: Vec<char>,
}

/// The delivery screen's state.
#[derive(Resource, Default)]
pub(crate) struct DeliveryScreen {
    bindings: Option<Arc<Bindings>>,
    presenter: Presenter,
    model: ScreenModel,
    view: ViewState,
    press: Press,
    materials: Materials,
    icons_registered: bool,
    /// The view is drawn this frame.
    shown: bool,
    warned: BTreeSet<String>,
    /// The delivery button's raycast graphic centre in window coordinates
    /// while the button can take a press.
    button_point: Option<Vec2>,
}

impl DeliveryScreen {
    /// Where a tap presses the delivery button, while it can take one.
    pub(crate) fn button_point(&self) -> Option<Vec2> {
        self.button_point
    }
}

#[derive(Component)]
pub(crate) struct DeliveryScreenRoot;

const DELIVERY_ICON_ALIAS: &str = "delivery-screen/delivery-action-icon";
const INFO_ICON_ALIAS: &str = "delivery-screen/information-icon";

fn material_alias(id: i64) -> String {
    format!("delivery-screen/material{id}")
}

/// The tween settings' default ease: OutQuad on the JP root's settings;
/// the shared (CN) root carries no tween settings and takes the JP root's
/// value (named).
fn tween_ease(layouts: &UiLayouts) -> Ease {
    let _ = layouts.tween_defaults();
    Ease::OutQuad
}

fn wording(layouts: &UiLayouts, key: &str, args: &[String]) -> String {
    let format = layouts.wording(key);
    moly_law::text::custom_text_mesh::format_wording(&format, args)
        .unwrap_or_else(|e| panic!("UI wording {key}: {e}"))
}

// ---------------------------------------------------------------------------
// Presenter steps
// ---------------------------------------------------------------------------

/// `MysekaiDeliveryCharacterCell.SetMaterialCount(quantity)`.
fn set_material_count(cell: &mut CellView, quantity: i32, layouts: &UiLayouts) {
    cell.count_text = Some(wording(layouts, MULTIPLY_FORMAT, &[quantity.to_string()]));
}

/// `MysekaiDeliveryCharacterCellGroup.Setup(list)`.
fn cells_setup(view: &mut ViewState, model: &ScreenModel, layouts: &UiLayouts) {
    if model.views.len() >= 3 {
        error!(
            "[delivery-screen] MysekaiDeliveryCharacterCellGroup: {} parties in session, more than two",
            model.views.len()
        );
    }
    view.group_selected = model.views.iter().position(|v| v.selected).unwrap_or(0);
    for cell in &mut view.cells {
        cell.active = false;
    }
    let count = model.views.len().min(view.cells.len());
    for index in 0..count {
        let data = &model.views[index];
        let cell = &mut view.cells[index];
        cell.active = true;
        cell.material_id = Some(data.material_id);
        set_material_count(cell, data.quantity, layouts);
        cell.selected = data.selected;
    }
    set_item_name(view);
}

/// `SetItemNameText`: the selected cell's material name (`SetText`).
fn set_item_name(view: &mut ViewState) {
    view.item_material = view
        .cells
        .get(view.group_selected)
        .and_then(|cell| cell.material_id);
}

/// `UpdateEnableButtons(state, selectedData)`.
fn update_enable_buttons(view: &mut ViewState, state: DeliveryActionState, quantity: i32) {
    let idle = state == DeliveryActionState::Idle;
    view.cells_enabled = idle;
    view.delivery_enabled =
        quantity > 0 && (state as i32) < (DeliveryActionState::EndDeliveryLoop as i32);
    view.info_enabled = idle;
}

/// `MysekaiDeliveryGaugeContents.Setup(fill, remainingPoint, memberBonus)`.
fn gauge_setup_contents(
    view: &mut ViewState,
    fill: f32,
    remaining: i32,
    bonus: i32,
    bindings: &Bindings,
    layouts: &UiLayouts,
) {
    view.sequence = None;
    view.remaining_tween = None;
    view.added_tween = None;
    view.before_target = fill;
    view.fill = gauge_setup(fill);
    view.balloon_text = (bonus > 0).then(|| wording(layouts, MEMBER_BONUS, &[bonus.to_string()]));
    if let Some(text) = layouts.set_wording_text(
        KEY,
        &bindings.remaining_text,
        REST_VALUE,
        Some(&[remaining.to_string()][..]),
    ) {
        view.remaining_text = Some(text);
    }
    view.added_shown = false;
}

/// `SetupViewContents(viewData)`.
fn setup_view_contents(
    view: &mut ViewState,
    model: &ScreenModel,
    bindings: &Bindings,
    layouts: &UiLayouts,
) {
    let Some(data) = model.selected_view() else {
        return;
    };
    let fill = data.gauge_fill();
    gauge_setup_contents(
        view,
        fill,
        data.after_require_point,
        data.member_bonus,
        bindings,
        layouts,
    );
    update_enable_buttons(view, model.state, data.quantity);
    info!(
        "[delivery-screen] SetupViewContents party {}: gauge Setup(fill {fill:.4}, AfterRequirePoint {}, memberBonus {}); UpdateEnableButtons(state {:?}, quantity {}): delivery button {}, cells {}, information button {} (the menu and site map buttons are the field menu shell's)",
        data.party_id,
        data.after_require_point,
        data.member_bonus,
        model.state,
        data.quantity,
        view.delivery_enabled,
        view.cells_enabled,
        view.info_enabled
    );
}

/// `UpdateFill(endValue, animationTime)`.
fn update_fill(view: &mut ViewState, end: f32, time: f32) {
    if approximately(view.before_target, end) {
        return;
    }
    view.sequence = None;
    view.before_target = end;
    let progress = view.fill;
    let tweens = if progress > end {
        let rest = 1.0 - progress as f64;
        let sum = rest + end as f64;
        let ratio = rest as f32 / sum as f32;
        let ratio = if ratio < 0.0 { 0.0 } else { ratio.min(1.0) };
        let first = ratio * time;
        vec![
            SequenceTween::new(0.0, 1.0, first, true),
            SequenceTween::new(
                first + GAUGE_TWEEN_INTERVAL_SECONDS,
                time - first,
                time,
                false,
            ),
        ]
    } else {
        vec![SequenceTween::new(0.0, end, time, false)]
    };
    view.sequence = Some(GaugeSequence::new(tweens));
}

/// `UpdateRemainingPoint(start, end, animationTime)`: both clamped to 0.
fn update_remaining(view: &mut ViewState, start: i32, end: i32, time: f32, ease: Ease) {
    view.remaining_tween = Some(IntTween::new(start.max(0), end.max(0), time, ease));
}

/// `UpdateAddedDeliveryPoint(start, end, animationTime)`: the start clamped
/// to 0 and written at once, the text shown.
#[allow(clippy::too_many_arguments)]
fn update_added(
    view: &mut ViewState,
    start: i32,
    end: i32,
    time: f32,
    ease: Ease,
    bindings: &Bindings,
    layouts: &UiLayouts,
) {
    let start = start.max(0);
    if let Some(text) = layouts.set_wording_text(
        KEY,
        &bindings.added_text,
        ADD_FORMAT,
        Some(&[start.to_string()][..]),
    ) {
        view.added_text = Some(text);
    }
    view.added_shown = true;
    view.added_tween = Some(IntTween::new(start, end, time, ease));
}

impl DeliveryScreen {
    /// `ScreenLayerMysekaiDeliveryPresenter.Setup(view)`.
    fn presenter_setup(&mut self) {
        self.presenter.set_up = true;
        self.presenter.awaiting_site_data = true;
        self.view.contents_active = false;
        self.view.info_shown = false;
        info!("[delivery-screen] OnBoot: presenter Setup: SetupDeliveryScreenLayer published (answered once the controller holds its site data), UpdateModel and OnCollisionDeliveryObject registered, SetActiveDeliveryContents(false), information button ShowObject(false)");
    }

    /// `ScreenLayerMysekaiDeliveryPresenter.Dispose()`.
    fn dispose(&mut self) {
        self.presenter.set_up = false;
        self.presenter.awaiting_site_data = false;
        self.presenter.registered = false;
        self.view.sequence = None;
        self.view.remaining_tween = None;
        self.view.added_tween = None;
        info!("[delivery-screen] OnExited: presenter Dispose: the delivery button's actions cleared, the cell group and the gauge disposed, both handlers removed");
    }

    /// `OnSetup(siteDataList)`.
    fn on_setup(&mut self, parties: &[PartySite], layouts: &UiLayouts) {
        let Some(bindings) = self.bindings.clone() else {
            return;
        };
        self.model.setup(parties);
        if self.model.selected_view().is_none() {
            error!("[delivery-screen] OnSetup: no view data for the selected cell");
            return;
        }
        // SetupView: the view's Setup (the icons), the cell group, the
        // contents.
        self.view.info_enabled = true;
        cells_setup(&mut self.view, &self.model, layouts);
        setup_view_contents(&mut self.view, &self.model, &bindings, layouts);
        self.view.view_setup = true;
        info!(
            "[delivery-screen] OnSetup: {} view data {:?}, selected {}; SetupView: button icons loaded, the dash button's Setup and ShowWithAnimation not drawn (named), cells set up",
            self.model.views.len(),
            self.model
                .views
                .iter()
                .map(|v| (v.party_id, v.quantity, v.current_point, v.max_point, v.member_bonus))
                .collect::<Vec<_>>(),
            self.model.selected
        );
    }

    /// `UpdateModel(event 66)`.
    fn update_model(&mut self, event: &DeliveryProgress, layouts: &UiLayouts) {
        let Some(bindings) = self.bindings.clone() else {
            return;
        };
        if self.model.views.is_empty() {
            return;
        }
        let ease = tween_ease(layouts);
        let view = &mut self.view;
        let model = &mut self.model;
        model.state = event.state;
        if event.state == DeliveryActionState::Idle {
            view.added_shown = false;
        }
        let selected = model.selected.min(model.views.len() - 1);
        update_enable_buttons(view, model.state, model.views[selected].quantity);
        model.views[selected].update(event);
        if !event.is_progress_update {
            return;
        }
        let count = model.views.len().min(view.cells.len());
        for index in 0..count {
            let quantity = model.views[index].quantity;
            set_material_count(&mut view.cells[index], quantity, layouts);
        }
        let data = model.views[selected].clone();
        let fill = data.gauge_fill();
        let after_delivery = data.current_point - data.before_point;
        let before_delivery = after_delivery - data.add_point_in_loop;
        update_fill(view, fill, data.animation_time);
        update_remaining(
            view,
            data.before_require_point,
            data.after_require_point,
            data.animation_time,
            ease,
        );
        update_added(
            view,
            before_delivery,
            after_delivery,
            data.animation_time,
            ease,
            &bindings,
            layouts,
        );
        model.views[selected].animation_time = 0.0;
        view.progress_updates += 1;
        if view.progress_updates % 30 != 1 {
            return;
        }
        info!(
            "[delivery-screen] UpdateModel (progress update {}, every 30th logged): state {:?}, quantity {}, points {} (before {}, added {}); UpdateDeliveryGauge(fill {fill:.4}, require {} -> {}, added {before_delivery} -> {after_delivery}, time {:.4})",
            view.progress_updates,
            event.state,
            data.quantity,
            data.current_point,
            data.before_point,
            data.add_point_in_loop,
            data.before_require_point,
            data.after_require_point,
            data.animation_time
        );
    }

    /// `OnCollisionDeliveryObject(event 68)`.
    fn on_collision(&mut self, collision: &DeliveryCollision) {
        let enter = collision.edge == CollisionEdge::Enter;
        match collision.object {
            DeliveryObjectType::DeliveryPlace => {
                self.view.contents_active = enter;
                info!("[delivery-screen] OnCollisionDeliveryObject {:?} place: SetActiveDeliveryContents({enter})", collision.edge);
            }
            DeliveryObjectType::Information => {
                self.view.info_shown = enter;
                info!("[delivery-screen] OnCollisionDeliveryObject {:?} board: information button ShowObject({enter})", collision.edge);
            }
        }
    }

    /// `HandleClick(index)` of the cell group, then the presenter's
    /// `ChangeSelectCharacter(index)`.
    fn handle_click(&mut self, next: usize, bindings: &Bindings, layouts: &UiLayouts) {
        let view = &mut self.view;
        if view.group_selected == next || view.cells.len() <= next {
            return;
        }
        let old = view.group_selected;
        view.cells[old].selected = false;
        if let Some(data) = self.model.views.get_mut(old) {
            data.selected = false;
        }
        view.group_selected = next;
        view.cells[next].selected = true;
        if let Some(data) = self.model.views.get_mut(next) {
            data.selected = true;
        }
        set_item_name(view);
        self.model.change_selected(next);
        setup_view_contents(view, &self.model, bindings, layouts);
        info!("[delivery-screen] cell {next} clicked: HandleClick, ChangeSelectCharacter({next})");
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Startup: the material names (and, on the JP root, the thumbnails once
/// the layout says which root this is).
pub(crate) fn load(server: Res<AssetServer>, mut screen: ResMut<DeliveryScreen>) {
    screen.materials.names_handle = Some(server.load::<JsonAsset>(MATERIALS));
}

/// Update: read the material names; the atlas charset takes the delivery
/// item names.
pub(crate) fn load_names(
    mut commands: Commands,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    mut screen: ResMut<DeliveryScreen>,
) {
    if screen.materials.names.is_some() {
        return;
    }
    let Some(handle) = screen.materials.names_handle.clone() else {
        return;
    };
    if let LoadState::Failed(error) = server.load_state(&handle) {
        error!("[delivery-screen] materials.json failed to load ({error:?}): the item name keeps the layout's text (named missing input)");
        screen.materials.names = Some(BTreeMap::new());
        commands.insert_resource(DeliveryCharset::default());
        return;
    }
    let Some(json) = jsons.get(&handle) else {
        return;
    };
    let doc: Value =
        serde_json::from_str(&json.0).unwrap_or_else(|e| panic!("materials.json is not JSON: {e}"));
    let entries = doc["entries"]
        .as_object()
        .unwrap_or_else(|| panic!("materials.json has no entries"));
    let mut names = BTreeMap::new();
    let mut chars = Vec::new();
    for (id, row) in entries {
        let id: i64 = id
            .parse()
            .unwrap_or_else(|_| panic!("materials.json entry id {id}"));
        let name = row["name"]
            .as_str()
            .unwrap_or_else(|| panic!("materials.json entry {id} has no name"));
        if row["materialType"].as_str() == Some("birthday_party_delivery") {
            chars.extend(name.chars());
        }
        names.insert(id, name.to_owned());
    }
    chars.sort_unstable();
    chars.dedup();
    info!(
        "[delivery-screen] materials.json: {} rows; {} characters of the delivery item names for the atlas charset",
        names.len(),
        chars.len()
    );
    screen.materials.names = Some(names);
    commands.insert_resource(DeliveryCharset { chars });
}

/// Update: the view, once the layout is ready.
pub(crate) fn spawn_when_ready(
    mut commands: Commands,
    layouts: Res<UiLayouts>,
    server: Res<AssetServer>,
    mut screen: ResMut<DeliveryScreen>,
) {
    if screen.bindings.is_some() || !layouts.ready(KEY, &server) {
        return;
    }
    let doc = layouts.document(KEY).expect("ready delivery layout");
    let bindings = bind(doc);
    let mut view = UiPrefabView::new(KEY, BALLOON_LAYER);
    // The menu content is the field menu shell's view; the dash button is
    // not drawn (named).
    view.set_visible(&bindings.menu_content, false);
    view.set_visible(&bindings.dash_button, false);
    info!(
        "[delivery-screen] {KEY} view: {} cells, gauge fill {} (serialized {}), delivery button @{} (se {:?}, absolutelyPress {}), press image {} alpha {}; the menu content is the field menu shell's, the dash button is not drawn",
        bindings.cells.len(),
        bindings.fill_note,
        bindings.fill_initial,
        bindings.delivery.id,
        bindings.delivery.config.se,
        bindings.delivery.config.absolutely_press,
        bindings.press_image,
        bindings.press_image_alpha
    );
    if bindings.region_jp {
        screen.materials.thumbs_handle = Some(server.load::<JsonAsset>(THUMBNAILS));
    } else {
        info!("[delivery-screen] this root carries no material thumbnails (only the JP root does): the cell icons are not drawn (named missing input)");
    }
    screen.view.cells = vec![CellView::default(); bindings.cells.len()];
    screen.press.states = vec![Default::default(); bindings.button_count()];
    screen.view.cover_alpha = bindings.press_image_alpha;
    screen.view.fill = bindings.fill_initial;
    commands.spawn((
        DeliveryScreenRoot,
        Visibility::Hidden,
        Transform::default(),
        RenderLayers::layer(BALLOON_LAYER),
        view,
    ));
    screen.bindings = Some(Arc::new(bindings));
}

/// Update: the layer's hooks, `OnChangeUILayer` and the controller's answer
/// to `SetupDeliveryScreenLayer`.
pub(crate) fn follow_screen(
    mut events: MessageReader<ScreenLayerEvent>,
    screens: Res<ScreenManager>,
    model: Res<DeliveryModel>,
    layouts: Res<UiLayouts>,
    mut screen: ResMut<DeliveryScreen>,
    mut retrigger: ResMut<DeliveryEnterRetrigger>,
) {
    for event in events.read() {
        if event.screen != SCREEN {
            continue;
        }
        match event.hook {
            ScreenHook::OnBoot => screen.presenter_setup(),
            ScreenHook::OnExited => screen.dispose(),
            _ => {}
        }
    }
    let current = screens.current_screen() == Some(SCREEN);
    if current && !screen.presenter.was_current {
        if screen.presenter.set_up {
            screen.presenter.registered = true;
            retrigger.site = true;
            retrigger.drops = true;
            info!("[delivery-screen] OnChangeUILayer(MysekaiDelivery): RegisterDeliveryStartAction(OnStartDelivery), RegisterDeliveryEndAction(OnEndDelivery), RemoveAllAndAddInformationListener (the information screen is not taken, named), TriggerOnEnterCollisions");
        } else {
            info!("[delivery-screen] OnChangeUILayer(MysekaiDelivery) before the layer's boot: nothing to register");
        }
    }
    screen.presenter.was_current = current;
    if screen.presenter.awaiting_site_data
        && screen.bindings.is_some()
        && model.site_id.is_some()
        && !model.parties.is_empty()
    {
        screen.presenter.awaiting_site_data = false;
        screen.on_setup(&model.parties, &layouts);
    }
}

/// Update, after the flow: event 68 and event 66, in the frame's order
/// (the collision circles publish before the flow and the drops).
pub(crate) fn update_model(
    mut progress: MessageReader<DeliveryProgress>,
    mut collisions: MessageReader<DeliveryCollision>,
    layouts: Res<UiLayouts>,
    mut screen: ResMut<DeliveryScreen>,
) {
    let collisions: Vec<DeliveryCollision> = collisions.read().copied().collect();
    let events: Vec<DeliveryProgress> = progress.read().copied().collect();
    if !screen.presenter.set_up {
        return;
    }
    for collision in &collisions {
        screen.on_collision(collision);
    }
    for event in &events {
        screen.update_model(event, &layouts);
    }
}

/// A button's `IsActive()` (with the runtime `enabled`) and
/// `IsInteractable()`.
fn button_lives(
    screen: &DeliveryScreen,
    bindings: &Bindings,
    view: &UiPrefabView,
    layouts: &UiLayouts,
    canvas: Vec2,
) -> Vec<button_rule::ButtonLive> {
    (0..bindings.button_count())
        .map(|index| {
            let button = bindings.button(index);
            let enabled = if index == 0 {
                screen.view.delivery_enabled
            } else {
                screen.view.cells_enabled
            };
            match view.selectable_live(layouts, canvas, button.node, button.id) {
                Some((active, interactable)) => button_rule::ButtonLive {
                    active: screen.shown && active && enabled,
                    interactable,
                },
                None => button_rule::ButtonLive {
                    active: false,
                    interactable: false,
                },
            }
        })
        .collect()
}

/// What a button's handlers asked for, applied after the event.
enum ButtonAction {
    Se(button_rule::SeType, String),
    Pressed(usize),
    Released(usize),
    Click(usize),
    FinishedOf(u64),
    TapStart,
    TapEnd,
}

fn effect_actions(index: usize, effects: Vec<button_rule::ButtonEffect>) -> Vec<ButtonAction> {
    effects
        .into_iter()
        .filter_map(|effect| match effect {
            button_rule::ButtonEffect::PlaySe { se, other_se_name } => {
                Some(ButtonAction::Se(se, other_se_name))
            }
            button_rule::ButtonEffect::PressEffect => Some(ButtonAction::Pressed(index)),
            button_rule::ButtonEffect::ReleaseEffect => Some(ButtonAction::Released(index)),
            button_rule::ButtonEffect::Click => Some(ButtonAction::Click(index)),
            button_rule::ButtonEffect::FinishedControlOf(owner) => {
                Some(ButtonAction::FinishedOf(owner))
            }
            // Long press and hold repeat are off on these buttons.
            button_rule::ButtonEffect::StartLongPressCheck
            | button_rule::ButtonEffect::CancelLongPressCheck
            | button_rule::ButtonEffect::StartHoldRepeat
            | button_rule::ButtonEffect::StopHoldRepeat => None,
        })
        .collect()
}

/// Apply the handlers' requests in order.
#[allow(clippy::too_many_arguments)]
fn apply_actions(
    actions: Vec<ButtonAction>,
    screen: &mut DeliveryScreen,
    bindings: &Bindings,
    layouts: &UiLayouts,
    requests: &mut MessageWriter<DeliveryRequest>,
    sounds: &mut crate::audio::SeRequests,
) {
    let ease = tween_ease(layouts);
    for action in actions {
        match action {
            ButtonAction::Se(se, other) => sounds.button(se, other),
            ButtonAction::Pressed(0) => {
                screen
                    .view
                    .cover_tweens
                    .push(FloatTween::new(1.0, PRESS_FADE_SECONDS, ease));
                info!("[delivery-screen] DeliveryActionButtonInteraction.OnPressed: ActiveCover DOFade(1, {PRESS_FADE_SECONDS}); the press particle is not ported (no particle rows in the site document)");
            }
            ButtonAction::Released(0) => {
                screen
                    .view
                    .cover_tweens
                    .push(FloatTween::new(0.0, PRESS_FADE_SECONDS, ease));
                info!("[delivery-screen] DeliveryActionButtonInteraction.OnReleased: ActiveCover DOFade(0, {PRESS_FADE_SECONDS})");
            }
            // The cells' buttons carry no view interaction.
            ButtonAction::Pressed(_) | ButtonAction::Released(_) => {}
            // The delivery button has no click listener.
            ButtonAction::Click(0) => {}
            ButtonAction::Click(index) => screen.handle_click(index - 1, bindings, layouts),
            ButtonAction::FinishedOf(owner) => {
                let found =
                    (0..bindings.button_count()).find(|i| bindings.button(*i).id as u64 == owner);
                match found {
                    Some(index) => {
                        let mut effects = Vec::new();
                        button_rule::on_finished_control_selectable(
                            &mut screen.press.states[index],
                            &mut effects,
                        );
                        let more = effect_actions(index, effects);
                        apply_actions(more, screen, bindings, layouts, requests, sounds);
                    }
                    None => error!("[delivery-screen] the input manager's finish callback belongs to source button {owner}, which this view does not drive; its release is not played"),
                }
            }
            ButtonAction::TapStart => {
                if !screen.presenter.registered {
                    info!("[delivery-screen] DeliveryActionButton.OnPointerDown: no start action registered");
                    continue;
                }
                match screen.model.selected_view() {
                    Some(data) => {
                        let party = data.party_id;
                        requests.write(DeliveryRequest::Start(Some(party)));
                        info!("[delivery-screen] DeliveryActionButton.OnPointerDown: the start action, OnStartDelivery({party})");
                    }
                    None => error!("[delivery-screen] DeliveryActionButton.OnPointerDown: the start action has no selected view data"),
                }
            }
            ButtonAction::TapEnd => {
                if screen.presenter.registered {
                    requests.write(DeliveryRequest::End);
                    info!("[delivery-screen] DeliveryActionButton: the end action, OnEndDelivery");
                }
            }
        }
    }
}

/// Update: the pointer's press, release and hover on the view's buttons
/// (the delivery button and the cells), by the source raycast over this
/// view, through the CustomButton handlers; the G key stand-in. The event
/// system delivers a handler only to a component active and enabled at that
/// moment; a release goes to the pressed button, clicks when it is over the
/// same button (a cancelled touch never clicks), then exits.
#[allow(clippy::too_many_arguments)]
pub(crate) fn input(
    mut pointers: MessageReader<UiPointerEvent>,
    keys: Res<ButtonInput<KeyCode>>,
    windows: Query<&Window, With<PrimaryWindow>>,
    layouts: Res<UiLayouts>,
    real: Res<Time<Real>>,
    (stack, dialog): (Res<ScreenManager>, Res<ShellDialogState>),
    roots: Query<&UiPrefabView, With<DeliveryScreenRoot>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut manager: ResMut<SourceInputManager>,
    mut screen: ResMut<DeliveryScreen>,
    mut requests: MessageWriter<DeliveryRequest>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    eligibility: crate::interaction::InteractionEligibility,
) {
    let events: Vec<UiPointerEvent> = pointers.read().copied().collect();
    let key_down = keys.just_pressed(KeyCode::KeyG);
    let key_up = keys.just_released(KeyCode::KeyG);
    if events.is_empty() && screen.press.held.is_none() && !key_down && !key_up {
        return;
    }
    let (Ok(window), Some(root_canvas), Ok(view)) =
        (windows.single(), root_canvas.as_deref(), roots.single())
    else {
        return;
    };
    let screen = &mut *screen;
    let Some(bindings) = screen.bindings.clone() else {
        return;
    };
    let canvas = root_canvas.size(window);
    let blocked = !screen.shown || dialog.blocks_field_input() || !stack.on_field();
    let startup = real.startup();
    let mut clock = move || startup.elapsed().as_secs_f32();
    for event in events {
        let rule_pointer = button_rule::Pointer {
            pointer_id: event.pointer_id,
            left_button: true,
        };
        let lives = button_lives(screen, &bindings, view, &layouts, canvas);
        let mut actions = Vec::new();
        let press = &mut screen.press;
        match event.phase {
            UiPointerPhase::Move => {
                if let Some(held) = press
                    .held
                    .as_mut()
                    .filter(|(id, _)| *id == event.pointer_id)
                {
                    held.1 = event.position;
                }
            }
            UiPointerPhase::Down => {
                press.held = Some((event.pointer_id, event.position));
                press.pressed = None;
                press.entered = false;
                if !blocked {
                    let pointer = event_pointer(window, root_canvas, event.position);
                    let target = view.press_target(&layouts, pointer, canvas);
                    let hit = (0..bindings.button_count()).find(|&i| {
                        let button = bindings.button(i);
                        target == Some((button.node, button.id)) && lives[i].active
                    });
                    if let Some(index) = hit {
                        let button = bindings.button(index);
                        press.pressed = Some(index);
                        press.entered = view.hovers(&layouts, pointer, canvas, button.node);
                        let effects = button_rule::on_pointer_down(
                            &mut manager.0,
                            &mut press.states[index],
                            &button.config,
                            button.id as u64,
                            rule_pointer,
                            event.touch_count,
                            lives[index],
                        );
                        actions.extend(effect_actions(index, effects));
                        if index == 0 {
                            actions.push(ButtonAction::TapStart);
                            press.tapping = true;
                        }
                    }
                }
            }
            UiPointerPhase::Up | UiPointerPhase::Cancel => {
                if let Some(index) = press.pressed {
                    let button = bindings.button(index);
                    let live = lives[index];
                    if live.active {
                        let mut effects = button_rule::on_pointer_up(
                            &mut manager.0,
                            &mut press.states[index],
                            &button.config,
                            button.id as u64,
                            rule_pointer,
                        );
                        if event.phase == UiPointerPhase::Up && !blocked {
                            let pointer = event_pointer(window, root_canvas, event.position);
                            if view.press_target(&layouts, pointer, canvas)
                                == Some((button.node, button.id))
                            {
                                effects.extend(button_rule::on_pointer_click(
                                    &mut manager.0,
                                    &mut press.states[index],
                                    &button.config,
                                    button.id as u64,
                                    rule_pointer,
                                    &mut clock,
                                    live,
                                ));
                            }
                        }
                        actions.extend(effect_actions(index, effects));
                        if index == 0 && press.tapping {
                            press.tapping = false;
                            actions.push(ButtonAction::TapEnd);
                        }
                    }
                    if press.entered && live.active {
                        let effects = button_rule::on_pointer_exit(
                            &mut manager.0,
                            &mut press.states[index],
                            &button.config,
                            button.id as u64,
                            rule_pointer,
                        );
                        actions.extend(effect_actions(index, effects));
                        if index == 0 && press.tapping {
                            press.tapping = false;
                            actions.push(ButtonAction::TapEnd);
                        }
                    }
                }
                press.pressed = None;
                press.held = None;
                press.entered = false;
            }
        }
        apply_actions(
            actions,
            screen,
            &bindings,
            &layouts,
            &mut requests,
            &mut sounds,
        );
    }
    let lives = button_lives(screen, &bindings, view, &layouts, canvas);
    let mut actions = Vec::new();
    // The input module's per-frame move of a held pointer: leaving the
    // pressed button's hover chain sends it the exit.
    if let (Some((pointer_id, position)), Some(index)) = (screen.press.held, screen.press.pressed) {
        let button = bindings.button(index);
        let pointer = event_pointer(window, root_canvas, position);
        let hovered = !blocked && view.hovers(&layouts, pointer, canvas, button.node);
        if screen.press.entered && !hovered && lives[index].active {
            let effects = button_rule::on_pointer_exit(
                &mut manager.0,
                &mut screen.press.states[index],
                &button.config,
                button.id as u64,
                button_rule::Pointer {
                    pointer_id,
                    left_button: true,
                },
            );
            actions.extend(effect_actions(index, effects));
            if index == 0 && screen.press.tapping {
                screen.press.tapping = false;
                actions.push(ButtonAction::TapEnd);
            }
        }
        screen.press.entered = hovered;
    }
    // The G key stand-in: the delivery button's down and up handlers.
    let key_pointer = button_rule::Pointer {
        pointer_id: KEY_POINTER,
        left_button: true,
    };
    if key_down
        && !screen.press.key
        && screen.press.pressed.is_none()
        && !blocked
        && eligibility.field_input_open()
        && lives[0].active
    {
        screen.press.key = true;
        let button = &bindings.delivery;
        let effects = button_rule::on_pointer_down(
            &mut manager.0,
            &mut screen.press.states[0],
            &button.config,
            button.id as u64,
            key_pointer,
            1,
            lives[0],
        );
        actions.extend(effect_actions(0, effects));
        actions.push(ButtonAction::TapStart);
        screen.press.tapping = true;
        info!("[delivery-screen] G down (the desktop stand-in for the delivery button's press)");
    }
    if key_up && screen.press.key {
        screen.press.key = false;
        if lives[0].active {
            let button = &bindings.delivery;
            let effects = button_rule::on_pointer_up(
                &mut manager.0,
                &mut screen.press.states[0],
                &button.config,
                button.id as u64,
                key_pointer,
            );
            actions.extend(effect_actions(0, effects));
            if screen.press.tapping {
                screen.press.tapping = false;
                actions.push(ButtonAction::TapEnd);
            }
        }
        info!("[delivery-screen] G up (the desktop stand-in for the delivery button's release)");
    }
    apply_actions(
        actions,
        screen,
        &bindings,
        &layouts,
        &mut requests,
        &mut sounds,
    );
}

/// A text a node can draw: every non-blank character is in the shared
/// glyph atlas. Sets the text and answers whether the node may show; with
/// no text to set the node keeps its own.
fn put_text(
    view: &mut UiPrefabView,
    path: &str,
    text: Option<&String>,
    art: Option<&BalloonArt>,
    warned: &mut BTreeSet<String>,
) -> bool {
    let Some(text) = text else {
        return true;
    };
    let Some(art) = art else {
        return false;
    };
    if let Some(missing) = text
        .chars()
        .find(|ch| !ch.is_whitespace() && art.glyph_cell(*ch).is_none())
    {
        if warned.insert(format!("{path} {missing}")) {
            warn!("[delivery-screen] the text {text:?} at {path} has the character {missing:?}, which the shared glyph atlas lacks: the node is hidden (the atlas charset seam adds the delivery texts)");
        }
        return false;
    }
    view.set_text(path, text.clone());
    true
}

/// Update, last in the delivery chain: the tweens' step (the tween manager
/// after the event handlers), a pressed button that is no longer active and
/// enabled (`CustomButton.OnDisable`), the textures and the view.
#[allow(clippy::too_many_arguments)]
pub(crate) fn advance(
    time: Res<Time>,
    windows: Query<&Window, With<PrimaryWindow>>,
    root_canvas: Option<Res<RootCanvas>>,
    mut layouts: ResMut<UiLayouts>,
    server: Res<AssetServer>,
    jsons: Res<Assets<JsonAsset>>,
    art: Option<Res<BalloonArt>>,
    (screens, entry, site): (
        Res<ScreenManager>,
        Option<Res<crate::entry::EntrySequence>>,
        Option<Res<crate::site::SiteActive>>,
    ),
    mut manager: ResMut<SourceInputManager>,
    mut screen: ResMut<DeliveryScreen>,
    mut requests: MessageWriter<DeliveryRequest>,
    mut sounds: ResMut<crate::audio::SeRequests>,
    mut roots: Query<
        (&mut Transform, &mut UiPrefabView, &mut Visibility),
        With<DeliveryScreenRoot>,
    >,
) {
    let screen = &mut *screen;
    let Some(bindings) = screen.bindings.clone() else {
        return;
    };
    let Ok((mut transform, mut view, mut visibility)) = roots.single_mut() else {
        return;
    };
    let (Ok(window), Some(root_canvas)) = (windows.single(), root_canvas.as_deref()) else {
        return;
    };
    let scale = root_canvas.scale(window);
    let canvas = root_canvas.size(window);
    let dt = time.delta_secs();
    // The material thumbnails (the JP root).
    if screen.materials.thumbs.is_none() {
        if let Some(handle) = screen.materials.thumbs_handle.clone() {
            if let LoadState::Failed(error) = server.load_state(&handle) {
                error!("[delivery-screen] material-thumbnails.json failed to load ({error:?}): the cell icons are not drawn (named missing input)");
                screen.materials.thumbs = Some(BTreeMap::new());
            } else if let Some(json) = jsons.get(&handle) {
                let doc: Value = serde_json::from_str(&json.0)
                    .unwrap_or_else(|e| panic!("material-thumbnails.json is not JSON: {e}"));
                let rows = doc["materials"]
                    .as_array()
                    .unwrap_or_else(|| panic!("material-thumbnails.json has no materials"));
                let thumbs: BTreeMap<i64, String> = rows
                    .iter()
                    .map(|row| {
                        (
                            row["materialId"].as_i64().expect("thumbnail materialId"),
                            row["image"].as_str().expect("thumbnail image").to_owned(),
                        )
                    })
                    .collect();
                info!(
                    "[delivery-screen] material thumbnails: {} rows",
                    thumbs.len()
                );
                screen.materials.thumbs = Some(thumbs);
            }
        }
    }
    // The buttons' icons (`_iconLoader`: the action icon of each type).
    if !screen.icons_registered {
        screen.icons_registered = true;
        for (alias, button) in [
            (
                DELIVERY_ICON_ALIAS,
                moly_law::action_button::ButtonType::Delivery,
            ),
            (
                INFO_ICON_ALIAS,
                moly_law::action_button::ButtonType::DeliveryInformation,
            ),
        ] {
            let name = button
                .icon_file_name()
                .unwrap_or_else(|| panic!("{button:?} has no action icon"));
            layouts.register_runtime_texture(
                alias,
                &format!("moly://ui/action-icon/{name}.png"),
                &server,
            );
        }
    }
    // Tweens: the gauge sequence, the two int tweens, the press fades.
    {
        let state = &mut screen.view;
        if let Some(sequence) = state.sequence.as_mut() {
            if sequence.step(dt, &mut state.fill) {
                state.sequence = None;
            }
        }
        if let Some(value) = state.remaining_tween.as_mut().and_then(|t| t.step(dt)) {
            if let Some(text) = layouts.set_wording_text(
                KEY,
                &bindings.remaining_text,
                REST_VALUE,
                Some(&[value.to_string()][..]),
            ) {
                state.remaining_text = Some(text);
            }
        }
        if state.remaining_tween.as_ref().is_some_and(|t| t.complete) {
            state.remaining_tween = None;
        }
        if let Some(value) = state.added_tween.as_mut().and_then(|t| t.step(dt)) {
            if let Some(text) = layouts.set_wording_text(
                KEY,
                &bindings.added_text,
                ADD_FORMAT,
                Some(&[value.to_string()][..]),
            ) {
                state.added_text = Some(text);
            }
        }
        if state.added_tween.as_ref().is_some_and(|t| t.complete) {
            state.added_tween = None;
        }
        let mut alpha = state.cover_alpha;
        for tween in &mut state.cover_tweens {
            if let Some(value) = tween.update(alpha, dt) {
                alpha = value;
            }
        }
        state.cover_alpha = alpha;
        state.cover_tweens.retain(|tween| !tween.is_complete());
    }
    // The root: drawn while the layer is active on the delivery site.
    screen.shown = screens.is_active(SCREEN)
        && crate::entry::hud_open(entry.as_deref())
        && site
            .as_deref()
            .is_some_and(|site| site.category == "delivery");
    let wanted = if screen.shown {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };
    if *visibility != wanted {
        *visibility = wanted;
    }
    transform.scale = Vec3::splat(scale);
    // CustomButton.OnDisable of a pressed button.
    let lives = button_lives(screen, &bindings, &view, &layouts, canvas);
    let mut actions = Vec::new();
    for (index, live) in lives.iter().enumerate() {
        if screen.press.states[index].control == button_rule::ControlState::Press && !live.active {
            let effects = button_rule::on_disable(
                &mut manager.0,
                &mut screen.press.states[index],
                bindings.button(index).id as u64,
            );
            actions.extend(effect_actions(index, effects));
        }
    }
    if !actions.is_empty() {
        apply_actions(
            actions,
            screen,
            &bindings,
            &layouts,
            &mut requests,
            &mut sounds,
        );
    }
    // The view.
    let art = art.as_deref();
    let state = &screen.view;
    let warned = &mut screen.warned;
    let contents = state.contents_active && state.view_setup;
    view.set_visible(&bindings.delivery_root, contents);
    view.set_visible(&bindings.cell_group, contents);
    view.set_visible(&bindings.gauge_contents, contents);
    view.set_visible(&bindings.info_root, state.info_shown);
    view.set_visible(&bindings.delivery_icon, state.view_setup);
    view.set_visible(&bindings.info_icon, state.view_setup);
    if state.view_setup {
        view.set_texture(&bindings.delivery_icon, DELIVERY_ICON_ALIAS);
        view.set_texture(&bindings.info_icon, INFO_ICON_ALIAS);
    }
    for (cell, binding) in state.cells.iter().zip(&bindings.cells) {
        view.set_visible(&binding.root, cell.active);
        view.set_visible(&binding.selected, cell.selected);
        let count = put_text(
            &mut view,
            &binding.count,
            cell.count_text.as_ref(),
            art,
            warned,
        );
        view.set_visible(&binding.count, count);
        let icon = cell.material_id.and_then(|id| {
            screen
                .materials
                .thumbs
                .as_ref()
                .and_then(|thumbs| thumbs.get(&id))
                .map(|image| (id, image.clone()))
        });
        match icon {
            Some((id, image)) => {
                let alias = material_alias(id);
                if screen.materials.registered.insert(id) {
                    layouts.register_runtime_texture(
                        &alias,
                        &format!("moly://material-thumbnails/{image}"),
                        &server,
                    );
                }
                view.set_texture(&binding.icon, &alias);
                view.set_visible(&binding.icon, true);
            }
            None => view.set_visible(&binding.icon, false),
        }
    }
    let item_name = state.item_material.and_then(|id| {
        screen
            .materials
            .names
            .as_ref()
            .and_then(|names| names.get(&id).cloned())
    });
    let shows = put_text(
        &mut view,
        &bindings.item_name,
        item_name.as_ref(),
        art,
        warned,
    );
    view.set_visible(&bindings.item_name, shows);
    view.set_fill(&bindings.fill, state.fill);
    let shows = put_text(
        &mut view,
        &bindings.remaining_text,
        state.remaining_text.as_ref(),
        art,
        warned,
    );
    view.set_visible(&bindings.remaining_text, shows);
    let shows = put_text(
        &mut view,
        &bindings.added_text,
        state.added_text.as_ref(),
        art,
        warned,
    );
    view.set_visible(&bindings.added_text, state.added_shown && shows);
    let shows = state.balloon_text.is_some()
        && put_text(
            &mut view,
            &bindings.balloon_text,
            state.balloon_text.as_ref(),
            art,
            warned,
        );
    view.set_visible(&bindings.balloon, shows);
    view.set_alpha(&bindings.press_image, state.cover_alpha);
    // The instrument's tap point: the delivery button's raycast graphic.
    screen.button_point = if lives[0].active && contents {
        view.rect(&layouts, &bindings.delivery_icon, canvas)
            .filter(|rect| rect.active)
            .map(|rect| {
                let centre = rect.center() * scale;
                Vec2::new(
                    window.width() * 0.5 + centre.x,
                    window.height() * 0.5 - centre.y,
                )
            })
    } else {
        None
    };
}
