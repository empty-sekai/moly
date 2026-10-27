//! Bulk target settings and result dialogs, composed from the source list,
//! group, check-box and reward-cell prefabs. Settings commit only on OK.

use super::{
    bulk::BulkHarvest,
    bulk_ui::{component, reference, referenced_component},
};
use crate::{
    ui_layers::{
        DialogBackKey, DialogBackKeyEvent, DialogId, DialogType, DisplayLayerType, ScreenManager,
    },
    ui_layout::{UiLayouts, UiPrefabView},
};
use bevy::{
    camera::visibility::RenderLayers,
    input::mouse::{MouseScrollUnit, MouseWheel},
    prelude::*,
    window::PrimaryWindow,
};
use moly_assets::ui_layout::{UiInstance, UiPrefab};
use moly_law::bulk_harvest::Plan;
use moly_protocol::{api::UserMysekaiSiteBulkHarvestResponse, rows::UserResource};
use std::collections::{BTreeMap, VecDeque};

const SETTINGS: &str = "BulkHarvestSettings";
const SETTINGS_RUNTIME: &str = "BulkHarvestSettingsRuntime";
const REWARD: &str = "BulkHarvestReward";
const REWARD_RUNTIME: &str = "BulkHarvestRewardRuntime";
const CELL: &str = "BulkHarvestRewardCell";
const CHECK_BOX: &str = "Sekai.Mysekai.BulkHarvestTargetSettingCheckBox";
const LAYER: usize = crate::sitemap::SITEMAP_LAYER;

pub const WORDINGS: &[&str] = &[
    "BODY_BULK_HARVEST_SETTING_DIALOG",
    "WORD_DECIDE",
    "WORD_CANCEL",
    "WORD_ALL",
    "WORD_SELECT_ALL",
    "MSG_API_SERVER_ERROR",
    "MSG_ERROR",
    "WORD_OK",
    "WORD_AXE",
    "WORD_PICKAXE",
    "MSG_BROKEN_TOOL",
    "MSG_WEAR_OUT_TOOL",
    "MSG_RECEIVED_REWARD",
    "WORD_REWARD",
    "MSG_MYSEKAI_BULK_HARVEST_CANNOT_PASS_EXPIRED",
    "MSG_MYSEKAI_BULK_HARVEST_CANNOT_POSSESSION_LIMIT",
    "MSG_MYSEKAI_BULK_HARVEST_CANNOT_STAMINA_SHORTAGE",
    "MSG_MYSEKAI_BULK_HARVEST_CANNOT_TOOL_SHORTAGE",
    "MSG_MYSEKAI_BULK_HARVEST_CANNOT_NO_TARGET",
];

pub struct RewardResult {
    pub plan: Plan,
    pub special_started: bool,
    pub site_id: u32,
    pub before_stamina: super::law::Stamina,
    pub before_tools: Vec<super::server_mock::UserTool>,
    pub reply: UserMysekaiSiteBulkHarvestResponse,
}

#[derive(Resource, Default)]
pub struct BulkDialogs {
    pub rewards: VecDeque<RewardResult>,
    open_settings: bool,
    shown: Option<Shown>,
}

#[derive(Component)]
struct DialogRoot;

struct Check {
    button: String,
    check: String,
    ids: Vec<i32>,
}

struct List {
    site: i32,
    root: String,
    tab: String,
    tab_background: Option<String>,
    tab_text: Option<String>,
    checks: Vec<Check>,
    content: String,
    viewport: String,
    source_position: Vec2,
    scrollbar: Option<String>,
    scroll: f32,
    sensitivity: f32,
}

struct Shown {
    id: DialogId,
    entity: Entity,
    positive: String,
    negative: Option<String>,
    close: String,
    window: String,
    message: String,
    lists: Vec<List>,
    current: usize,
    opened_at: f64,
    draft: Option<BTreeMap<i32, Vec<i32>>>,
    rewards: Option<RewardResult>,
    pressed: Option<(i32, Vec2, bool)>,
}

fn id_of(selector: &str) -> Result<i64, String> {
    selector
        .trim_start_matches('@')
        .parse()
        .map_err(|_| "invalid UI selector".into())
}
fn cloned(instance: &UiInstance, selector: &str) -> Result<String, String> {
    instance.selector(id_of(selector)?)
}
fn at<'a>(
    doc: &'a UiPrefab,
    selector: &str,
    class: &str,
) -> Result<&'a moly_assets::ui_layout::UiComponent, String> {
    doc.nodes[doc.find(selector)?]
        .components
        .iter()
        .find(|c| c.class == class)
        .ok_or_else(|| format!("{}: {selector} does not carry {class}", doc.prefab))
}

fn checkbox(doc: &UiPrefab, selector: &str) -> Result<(String, String, String), String> {
    let c = at(doc, selector, CHECK_BOX)?;
    let (checkbox, _) = referenced_component(doc, &c.fields, "_checkBox")?;
    Ok((
        reference(doc, &checkbox.fields["toggle"])?,
        reference(doc, &checkbox.fields["checkObj"])?,
        reference(doc, &c.fields["_text"])?,
    ))
}

fn make_check(
    doc: &UiPrefab,
    instance: &UiInstance,
    source: &str,
    ids: Vec<i32>,
    text: String,
    view: &mut UiPrefabView,
) -> Result<Check, String> {
    let (button, check, label) = checkbox(doc, source)?;
    view.set_text(&cloned(instance, &label)?, text);
    Ok(Check {
        button: cloned(instance, &button)?,
        check: cloned(instance, &check)?,
        ids,
    })
}

fn word(layouts: &UiLayouts, key: &str) -> String {
    layouts.wording(key).to_owned()
}

fn settings_document(
    world: &World,
) -> Result<(UiPrefab, UiPrefabView, Vec<List>, BTreeMap<i32, Vec<i32>>), String> {
    let layouts = world.resource::<UiLayouts>();
    let template = layouts
        .document(SETTINGS)
        .ok_or("bulk settings prefab is not ready")?;
    let state = world.resource::<BulkHarvest>();
    let masters = state.masters.as_ref().ok_or("bulk masters are not ready")?;
    let rank = world
        .get_resource::<crate::userdata::ClientUserData>()
        .ok_or("user data is not ready")?
        .gamedata
        .mysekai_rank
        .ok_or("user rank is not ready")?;
    let sites = world
        .get_resource::<crate::sitemap::SitemapData>()
        .ok_or("site metadata is not ready")?
        .released_harvest_sites(rank);
    let fields = &component(template, "Sekai.Mysekai.BulkHarvestTargetSettingDialog")?.fields;
    let (list_source, list_selector) =
        referenced_component(template, fields, "_checkBoxListViewPrefab")?;
    let list_parent = reference(template, &fields["_checkBoxListViewParent"])?;
    let (_, tabs_key) = referenced_component(template, fields, "_siteTabGroup")?;
    let tab_group = at(template, &tabs_key, "Sekai.UIPartsDialogTabGroup")?;
    let (toggles, _) = referenced_component(template, &tab_group.fields, "toggleGroup")?;
    let tab_refs = toggles.fields["indexToggles"]
        .as_array()
        .ok_or("bulk site tabs are absent")?;
    if sites.len() > tab_refs.len() {
        return Err("bulk site tabs exceed the source prefab's tab capacity".into());
    }
    let scroll = at(template, &list_selector, "UnityEngine.UI.ScrollRect")?;
    let content = reference(template, &scroll.fields["m_Content"])?;
    let viewport = reference(template, &scroll.fields["m_Viewport"])?;
    let source_position = Vec2::from_array(
        template.nodes[template.find(&content)?]
            .rect
            .anchored_position,
    );
    let sensitivity = scroll.fields["m_ScrollSensitivity"]
        .as_f64()
        .ok_or("bulk scroll sensitivity is absent")? as f32;
    let (group_source, group_selector) =
        referenced_component(template, &list_source.fields, "_groupViewBase")?;
    let group_parent = reference(template, &list_source.fields["_groupViewRoot"])?;
    let target_base = reference(template, &group_source.fields["_checkBoxBase"])?;
    let target_parent = reference(template, &group_source.fields["_checkBoxRoot"])?;
    let mut doc = template.clone();
    let mut view = UiPrefabView::new(SETTINGS_RUNTIME, LAYER);
    view.set_visible(&list_selector, false);
    view.set_text(
        &reference(template, &fields["positiveButtonLabelMesh"])?,
        word(layouts, "WORD_DECIDE"),
    );
    view.set_text(
        &reference(template, &fields["negativeButtonLabelMesh"])?,
        word(layouts, "WORD_CANCEL"),
    );
    view.set_text(
        &reference(template, &fields["messageBodyTextMesh"])?,
        word(layouts, "BODY_BULK_HARVEST_SETTING_DIALOG"),
    );
    let mut lists = Vec::new();
    let mut draft = state.settings.clone();
    for (index, tab_ref) in tab_refs.iter().enumerate() {
        let tab = reference(template, tab_ref)?;
        let Some((site, name)) = sites.get(index) else {
            view.set_visible(&tab, false);
            continue;
        };
        let tab_component = at(template, &tab, "Sekai.UIPartsDialogTab")?;
        view.set_text(
            &reference(template, &tab_component.fields["text"])?,
            name.clone(),
        );
        view.set_visible(&reference(template, &tab_component.fields["badge"])?, false);
        let targets = masters.targets(*site);
        let ids: Vec<_> = targets.iter().map(|t| t.id).collect();
        draft.entry(*site).or_insert_with(|| ids.clone());
        let list_instance = doc.instantiate_subtree(
            &list_parent,
            template,
            &list_selector,
            &format!("Site{site}"),
        )?;
        view.set_visible(&list_instance.root_selector(), true);
        view.set_visible(&cloned(&list_instance, &group_selector)?, false);
        let all = reference(template, &list_source.fields["_allCheckBox"])?;
        let mut checks = vec![make_check(
            template,
            &list_instance,
            &all,
            ids,
            word(layouts, "WORD_SELECT_ALL"),
            &mut view,
        )?];
        let mut groups = Vec::new();
        for target in targets {
            if !groups.contains(&target.group_id) {
                groups.push(target.group_id);
            }
        }
        for group in groups {
            let rows: Vec<_> = targets
                .iter()
                .filter(|target| target.group_id == group)
                .collect();
            let group_instance = doc.instantiate_subtree(
                &cloned(&list_instance, &group_parent)?,
                template,
                &group_selector,
                &format!("Group{group}"),
            )?;
            view.set_visible(&group_instance.root_selector(), true);
            view.set_visible(&cloned(&group_instance, &target_base)?, false);
            view.set_text(
                &cloned(
                    &group_instance,
                    &reference(template, &group_source.fields["_groupNameText"])?,
                )?,
                rows[0].group_name.clone(),
            );
            let all = reference(template, &group_source.fields["_allCheckBox"])?;
            checks.push(make_check(
                template,
                &group_instance,
                &all,
                rows.iter().map(|t| t.id).collect(),
                word(layouts, "WORD_ALL"),
                &mut view,
            )?);
            for row in rows {
                let instance = doc.instantiate_subtree(
                    &cloned(&group_instance, &target_parent)?,
                    template,
                    &target_base,
                    &format!("Target{}", row.id),
                )?;
                view.set_visible(&instance.root_selector(), true);
                checks.push(make_check(
                    template,
                    &instance,
                    &target_base,
                    vec![row.id],
                    row.name.clone(),
                    &mut view,
                )?);
            }
        }
        lists.push(List {
            site: *site,
            root: list_instance.root_selector(),
            tab,
            tab_background: Some(reference(
                template,
                &tab_component.fields["backgroundImage"],
            )?),
            tab_text: Some(reference(template, &tab_component.fields["text"])?),
            checks,
            content: cloned(&list_instance, &content)?,
            viewport: cloned(&list_instance, &viewport)?,
            source_position,
            scrollbar: reference(template, &scroll.fields["m_VerticalScrollbar"])
                .ok()
                .map(|key| cloned(&list_instance, &key))
                .transpose()?,
            scroll: 0.0,
            sensitivity,
        });
    }
    Ok((doc, view, lists, draft))
}

fn special_group(world: &World, row: &UserResource) -> Option<usize> {
    match row.resource_type.as_str() {
        "mysekai_blueprint" => Some(0),
        "mysekai_music_record" => Some(1),
        "mysekai_item" => world
            .resource::<moly_masters::load::MasterTables>()
            .rows
            .get("mysekaiItems")?
            .iter()
            .find(|item| item["id"].as_i64() == Some(i64::from(row.resource_id)))
            .and_then(|item| match item["mysekaiItemType"].as_str() {
                Some("surplus_blueprint") => Some(2),
                Some("surplus_music_record") => Some(3),
                _ => None,
            }),
        _ => None,
    }
}

fn reward_rows(result: &RewardResult) -> Vec<UserResource> {
    result.reply.obtained_drop_resources.clone()
}

/// Item names and thumbnail routes come from the same master rows as the
/// inventory. Missing resource types are errors, never fabricated labels.
fn resource_display(
    world: &World,
    row: &UserResource,
) -> Result<(String, Option<(String, String)>), String> {
    let tables = &world.resource::<moly_masters::load::MasterTables>().rows;
    let (table, directory, icon_field) = match row.resource_type.as_str() {
        "mysekai_material" => ("mysekaiMaterials", "material", "iconAssetbundleName"),
        "mysekai_item" => ("mysekaiItems", "item", "iconAssetbundleName"),
        "mysekai_fixture" => ("mysekaiFixtures", "fixture", "assetbundleName"),
        "mysekai_tool" => ("mysekaiTools", "tool", "assetbundleName"),
        "mysekai_blueprint" | "mysekai_music_record" => return Ok((String::new(), None)),
        other => {
            return Err(format!(
                "bulk reward resource type {other} has no thumbnail adapter"
            ))
        }
    };
    let master = tables
        .get(table)
        .and_then(|rows| {
            rows.iter()
                .find(|m| m["id"].as_i64() == Some(i64::from(row.resource_id)))
        })
        .ok_or_else(|| format!("bulk reward {table} {} has no master", row.resource_id))?;
    let name = master["name"]
        .as_str()
        .ok_or("bulk reward master name is absent")?
        .to_owned();
    let resource = master[icon_field]
        .as_str()
        .ok_or("bulk reward master icon is absent")?
        .to_owned();
    Ok((
        name,
        Some((
            format!("mysekai/thumbnail/{directory}/{resource}"),
            resource,
        )),
    ))
}

fn reward_document(
    world: &mut World,
    result: &RewardResult,
) -> Result<(UiPrefab, UiPrefabView, List), String> {
    let layouts = world.resource::<UiLayouts>();
    let template = layouts
        .document(REWARD)
        .ok_or("bulk reward prefab is not ready")?
        .clone();
    let cell = layouts
        .document(CELL)
        .ok_or("source UIPartsThumbnailRewardListCell is not ready")?
        .clone();
    let thumb_template = layouts
        .document("InventoryItemCell")
        .ok_or("item thumbnail prefab is not ready")?
        .clone();
    let fields = &component(&template, "Sekai.Mysekai.MysekaiBulkHarvestRewardDialog")?.fields;
    let (list, _) = referenced_component(&template, fields, "_listView")?;
    let (scroll, _) = referenced_component(&template, &list.fields, "scrollRect")?;
    let content = reference(&template, &scroll.fields["m_Content"])?;
    let viewport = reference(&template, &scroll.fields["m_Viewport"])?;
    let cell_c = component(&cell, "Sekai.UIPartsThumbnailRewardListCell")?;
    let cell_name = reference(&cell, &cell_c.fields["nameText"])?;
    let roots = cell_c.fields["thumbnailRoots"]
        .as_array()
        .ok_or("reward thumbnail roots absent")?;
    let cell_mount = reference(&cell, roots.first().ok_or("reward thumbnail mount absent")?)?;
    let thumb = component(&thumb_template, "Sekai.UIPartsItemThumbnail")?;
    let thumb_root = format!("@{}", thumb.path_id);
    let thumb_image = reference(&thumb_template, &thumb.fields["thumbnailImage"])?;
    let thumb_quantity = reference(&thumb_template, &thumb.fields["innerText"])?;
    let mut doc = template.clone();
    let mut view = UiPrefabView::new(REWARD_RUNTIME, LAYER);
    view.set_text(
        &reference(&template, &fields["messageBodyTextMesh"])?,
        word(layouts, "MSG_RECEIVED_REWARD"),
    );
    view.set_text(
        &reference(&template, &fields["positiveButtonLabelText"])?,
        word(layouts, "WORD_OK"),
    );
    view.set_visible(
        &reference(&template, &fields["_shortageTextRoot"])?,
        result.plan.exists_stamina_shortage_excluded_target
            || result.plan.exists_tool_shortage_excluded_target,
    );
    view.set_visible(
        &reference(&template, &fields["_staminaShortageText"])?,
        result.plan.exists_stamina_shortage_excluded_target,
    );
    view.set_visible(
        &reference(&template, &fields["_toolShortageText"])?,
        result.plan.exists_tool_shortage_excluded_target,
    );
    if let Ok(tabs) = doc.find("WindowRoot/Tabs") {
        view.set_visible(&format!("@{}", doc.nodes[tabs].transform_id), false);
    }
    let spacing = list.fields["verticalSpacing"]
        .as_f64()
        .ok_or("reward list spacing absent")? as f32;
    let height = cell_c.fields["sizeY"]
        .as_f64()
        .ok_or("reward cell height absent")? as f32;
    let mut texture_loads = Vec::new();
    let mut count = 0;
    for resource in reward_rows(result) {
        if special_group(world, &resource).is_some() {
            continue;
        }
        let (name, icon) = resource_display(world, &resource)?;
        let Some((bundle, filename)) = icon else {
            continue;
        };
        let instance =
            doc.instantiate_subtree(&content, &cell, &cell.prefab, &format!("Reward{count}"))?;
        view.set_text(&cloned(&instance, &cell_name)?, name);
        view.set_anchored_position(
            &instance.root_selector(),
            Vec2::new(0.0, -(height + spacing) * count as f32),
        );
        let thumbnail = doc.instantiate_subtree(
            &cloned(&instance, &cell_mount)?,
            &thumb_template,
            &thumb_root,
            "Thumbnail",
        )?;
        view.set_text(
            &cloned(&thumbnail, &thumb_quantity)?,
            resource.quantity.to_string(),
        );
        for field in ["disableCover", "labelImage", "frameImage"] {
            if let Ok(selector) = reference(&thumb_template, &thumb.fields[field]) {
                view.set_visible(&cloned(&thumbnail, &selector)?, false);
            }
        }
        let path = world
            .resource::<crate::item_icon::ItemIcons>()
            .texture(&bundle, &filename)?;
        let alias = format!("bulk-reward/{bundle}");
        texture_loads.push((alias.clone(), path));
        view.set_texture(&cloned(&thumbnail, &thumb_image)?, &alias);
        count += 1;
    }
    view.set_size_delta(
        &content,
        Vec2::new(0.0, ((height + spacing) * count as f32 - spacing).max(0.0)),
    );
    let server = world.resource::<AssetServer>().clone();
    for (alias, path) in texture_loads {
        world
            .resource_mut::<UiLayouts>()
            .register_runtime_texture(&alias, &path, &server);
    }
    let list = List {
        site: 0,
        root: template.prefab.clone(),
        tab: String::new(),
        tab_background: None,
        tab_text: None,
        checks: Vec::new(),
        source_position: Vec2::from_array(
            template.nodes[template.find(&content)?]
                .rect
                .anchored_position,
        ),
        sensitivity: scroll.fields["m_ScrollSensitivity"]
            .as_f64()
            .ok_or("reward scroll sensitivity absent")? as f32,
        content,
        viewport,
        scrollbar: reference(&template, &scroll.fields["m_VerticalScrollbar"]).ok(),
        scroll: 0.0,
    };
    Ok((doc, view, list))
}

fn open(world: &mut World, state: &mut BulkDialogs) {
    // The icon index is asynchronous. Leave rewards and special playback in
    // their current order until it settles, before consuming any result.
    if !state.open_settings && !world.contains_resource::<crate::item_icon::ItemIcons>() {
        return;
    }
    if state.shown.is_some() || world.resource::<ScreenManager>().exists_dialog() {
        return;
    }
    let is_settings = state.open_settings;
    if !is_settings && state.rewards.is_empty() {
        return;
    }
    let key = if is_settings { SETTINGS } else { REWARD };
    let server = world.resource::<AssetServer>().clone();
    if !world.resource::<UiLayouts>().ready(key, &server) {
        return;
    }
    let (doc, view, lists, draft, result) = if is_settings {
        match settings_document(world) {
            Ok((doc, view, lists, draft)) => (doc, view, lists, Some(draft), None),
            Err(reason) => {
                error!("[bulk-settings] {reason}");
                state.open_settings = false;
                return;
            }
        }
    } else {
        if !world.resource::<UiLayouts>().ready(CELL, &server) {
            return;
        }
        let Some(mut result) = state.rewards.pop_front() else {
            return;
        };
        if world
            .get_resource::<crate::site::SiteActive>()
            .is_none_or(|site| site.site_id != result.site_id)
        {
            return;
        }
        if !result.special_started {
            let mut special: Vec<_> = reward_rows(&result)
                .into_iter()
                .filter_map(|row| special_group(world, &row).map(|group| (group, row)))
                .collect();
            special.sort_by_key(|(group, _)| *group);
            let mut openers = world.resource_mut::<crate::get_resource::GetResourceOpeners>();
            for (_, row) in special {
                openers.acquired(row);
            }
            result.special_started = true;
        }
        if world
            .resource::<crate::get_resource::GetResourceOpeners>()
            .acquisition_pending(world.resource::<crate::get_resource::GetResourcePlayer>())
        {
            state.rewards.push_front(result);
            return;
        }
        if reward_rows(&result)
            .iter()
            .all(|row| special_group(world, row).is_some())
        {
            finish_feedback(world, &result);
            return;
        }
        match reward_document(world, &result) {
            Ok((doc, view, list)) => (doc, view, vec![list], None, Some(result)),
            Err(reason) => {
                error!("[bulk-reward] {reason}");
                state.rewards.push_front(result);
                return;
            }
        }
    };
    let class = if is_settings {
        "Sekai.Mysekai.BulkHarvestTargetSettingDialog"
    } else {
        "Sekai.Mysekai.MysekaiBulkHarvestRewardDialog"
    };
    let fields = &component(&doc, class)
        .expect("composed dialog component")
        .fields;
    let positive = reference(&doc, &fields["positiveButton"]).expect("source positive button");
    let negative = if is_settings {
        Some(reference(&doc, &fields["negativeButton"]).expect("source negative button"))
    } else {
        None
    };
    let close = reference(&doc, &fields["closeButton"]).expect("source close button");
    let window = reference(&doc, &fields["windowObject"]).expect("source window object");
    let message = reference(&doc, &fields["messageBodyTextMesh"]).expect("source message");
    let runtime = if is_settings {
        SETTINGS_RUNTIME
    } else {
        REWARD_RUNTIME
    };
    world
        .resource_mut::<UiLayouts>()
        .replace_runtime_document(runtime, doc, &server)
        .expect("bulk composed UI");
    let mut screens = world.resource_mut::<ScreenManager>();
    let Ok(id) = screens.show_dialog(
        DialogType(if is_settings { 468 } else { 473 }),
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        "BulkHarvest",
    ) else {
        return;
    };
    screens.open_dialog(id);
    screens.dialog_open_finished(id);
    let current_site = world
        .get_resource::<crate::site::SiteActive>()
        .map(|site| site.site_id as i32);
    let current = lists
        .iter()
        .position(|list| Some(list.site) == current_site)
        .unwrap_or(0);
    let entity = world
        .spawn((
            DialogRoot,
            view,
            Transform::from_xyz(0.0, 0.0, 100.0),
            Visibility::Visible,
            RenderLayers::layer(LAYER),
        ))
        .id();
    state.shown = Some(Shown {
        id,
        entity,
        positive,
        negative,
        close,
        window,
        message,
        lists,
        current,
        opened_at: world.resource::<Time>().elapsed_secs_f64(),
        draft,
        rewards: result,
        pressed: None,
    });
    state.open_settings = false;
}

fn finish_feedback(world: &mut World, result: &RewardResult) {
    if world
        .get_resource::<crate::site::SiteActive>()
        .is_some_and(|site| site.site_id == result.site_id)
    {
        let after = world
            .resource::<super::action::HarvestPlayerModel>()
            .stamina;
        world
            .resource_mut::<super::stamina_hud::HarvestStaminaHud>()
            .show();
        world
            .resource_mut::<super::stamina_hud::HarvestStaminaHud>()
            .hit(result.before_stamina, after);
        let alert = world
            .resource::<crate::client_config::ClientConfigs>()
            .float(88);
        let model = world.resource::<super::action::HarvestPlayerModel>();
        let catalog = world.resource::<super::catalog::HarvestCatalog>();
        let mut notices = Vec::new();
        for before in &result.before_tools {
            let Some(def) = catalog.tool(before.tool_id) else {
                continue;
            };
            let after = model
                .tools
                .iter()
                .find(|tool| tool.tool_id == before.tool_id);
            let broken = after.map_or(0, |tool| tool.quantity) < before.quantity;
            let worn = before.durability as f32 / def.max_durability as f32 >= alert
                && after.map_or(0, |tool| tool.durability) as f32 / (def.max_durability as f32)
                    < alert;
            if broken || worn {
                notices.push(super::bulk_notice::ToolNotice {
                    tool: before.tool_id,
                    broken,
                });
            }
        }
        world
            .resource_mut::<super::bulk_notice::BulkNotices>()
            .queue
            .extend(notices);
    }
}

fn close(world: &mut World, state: &mut BulkDialogs, accepted: bool) {
    let Some(shown) = state.shown.as_mut() else {
        return;
    };
    if accepted {
        if let Some(draft) = shown.draft.as_ref() {
            match world
                .resource_mut::<BulkHarvest>()
                .save_settings(draft.clone())
            {
                Ok(changed) => info!("[bulk-settings] ChangeBulkHarvestTargetSetting {changed:?}"),
                Err(reason) => {
                    error!("[bulk-settings] save refused: {reason}");
                    world.resource_mut::<BulkHarvest>().settings_error = Some(reason);
                    let text = world.resource::<UiLayouts>().wording("MSG_ERROR");
                    world
                        .get_mut::<UiPrefabView>(shown.entity)
                        .expect("bulk settings view")
                        .set_text(&shown.message, text);
                    return;
                }
            }
        }
    }
    let shown = state.shown.take().expect("shown above");
    if let Some(result) = shown.rewards {
        finish_feedback(world, &result);
    }
    world.despawn(shown.entity);
    let mut screens = world.resource_mut::<ScreenManager>();
    screens.close_dialog(shown.id);
    screens.dialog_destroyed(shown.id);
}

pub fn tick(world: &mut World) {
    let mut state = world
        .remove_resource::<BulkDialogs>()
        .expect("bulk dialogs installed");
    open(world, &mut state);
    let Some(shown) = state.shown.as_mut() else {
        world.insert_resource(state);
        return;
    };
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .cloned();
    let Some(window) = window else {
        world.insert_resource(state);
        return;
    };
    let Some(canvas) = world.get_resource::<crate::canvas::RootCanvas>() else {
        world.insert_resource(state);
        return;
    };
    let size = canvas.size(&window);
    let scale = canvas.scale(&window);
    if let Some(mut transform) = world.get_mut::<Transform>(shown.entity) {
        transform.scale = Vec3::splat(scale);
    }
    let since_open = world.resource::<Time>().elapsed_secs_f64() - shown.opened_at;
    world.resource_scope(|world, layouts: Mut<UiLayouts>| {
        let Some(mut view) = world.get_mut::<UiPrefabView>(shown.entity) else {
            return;
        };
        for (index, list) in shown.lists.iter_mut().enumerate() {
            if shown.draft.is_none() {
                view.set_visible(&list.content, since_open >= 0.01);
            }
            if shown.draft.is_some() {
                view.set_visible(&list.root, index == shown.current);
                if let Some(background) = &list.tab_background {
                    view.set_behaviour_enabled(
                        id_of(background).expect("tab background"),
                        index == shown.current,
                    );
                }
                if let Some(text) = &list.tab_text {
                    view.set_graphic_color(
                        id_of(text).expect("tab text"),
                        layouts
                            .palette_color(if index == shown.current { 0 } else { 2 })
                            .expect("source tab color"),
                    );
                }
                let ids = shown.draft.as_ref().and_then(|draft| draft.get(&list.site));
                for check in &list.checks {
                    view.set_visible(
                        &check.check,
                        ids.is_some_and(|ids| check.ids.iter().all(|id| ids.contains(id))),
                    );
                }
            }
            if index != shown.current {
                continue;
            }
            if let (Some(content), Some(viewport)) = (
                view.rect(&layouts, &list.content, size),
                view.rect(&layouts, &list.viewport, size),
            ) {
                list.scroll = list
                    .scroll
                    .clamp(0.0, (content.size.y - viewport.size.y).max(0.0));
                if let Some(bar) = &list.scrollbar {
                    let range = (content.size.y - viewport.size.y).max(0.0);
                    view.set_scrollbar(
                        bar,
                        if range > 0.0 {
                            1.0 - list.scroll / range
                        } else {
                            1.0
                        },
                        (viewport.size.y / content.size.y.max(1.0)).clamp(0.0, 1.0),
                    );
                    view.set_visible(bar, range > 0.0);
                }
            }
            view.set_anchored_position(&list.content, list.source_position + Vec2::Y * list.scroll);
        }
    });
    world.insert_resource(state);
}

#[derive(Resource, Default)]
struct DialogInput {
    pointers: Vec<crate::gesture::UiPointerEvent>,
    wheel: Vec<MouseWheel>,
    back: Vec<DialogBackKeyEvent>,
}

fn collect_input(
    mut state: ResMut<DialogInput>,
    mut pointers: MessageReader<crate::gesture::UiPointerEvent>,
    mut wheel: MessageReader<MouseWheel>,
    mut back: MessageReader<DialogBackKeyEvent>,
) {
    state.pointers = pointers.read().copied().collect();
    state.wheel = wheel.read().copied().collect();
    state.back = back.read().copied().collect();
}

fn input(world: &mut World) {
    let mut state = world
        .remove_resource::<BulkDialogs>()
        .expect("bulk dialogs installed");
    let inputs = world
        .remove_resource::<DialogInput>()
        .expect("bulk input installed");
    let window = world
        .query_filtered::<(Entity, &Window), With<PrimaryWindow>>()
        .single(world)
        .ok()
        .map(|(e, w)| (e, w.clone()));
    let Some((window_entity, window)) = window else {
        world.insert_resource(state);
        world.insert_resource(inputs);
        return;
    };
    let Some(canvas) = world.get_resource::<crate::canvas::RootCanvas>() else {
        world.insert_resource(state);
        world.insert_resource(inputs);
        return;
    };
    let size = canvas.size(&window);
    let scale = canvas.scale(&window);
    let point =
        |p: Vec2| Vec2::new(p.x - window.width() * 0.5, window.height() * 0.5 - p.y) / scale;
    if let Some(shown) = state.shown.as_mut() {
        let view = world
            .get::<UiPrefabView>(shown.entity)
            .expect("bulk dialog view");
        let layouts = world.resource::<UiLayouts>();
        let hit = |selector: &str, p: Vec2| {
            view.rect(layouts, selector, size)
                .is_some_and(|r| r.active && r.contains(point(p)))
        };
        let mut answer = inputs
            .back
            .iter()
            .any(|back| back.id == shown.id)
            .then_some(false);
        if let Some(list) = shown.lists.get_mut(shown.current) {
            if window
                .cursor_position()
                .is_some_and(|p| hit(&list.viewport, p))
            {
                for scroll in inputs
                    .wheel
                    .iter()
                    .filter(|scroll| scroll.window == window_entity)
                {
                    let dy = match scroll.unit {
                        MouseScrollUnit::Line => scroll.y,
                        MouseScrollUnit::Pixel => scroll.y / scale,
                    };
                    list.scroll -= dy * list.sensitivity;
                }
            }
        }
        for event in &inputs.pointers {
            use crate::gesture::UiPointerPhase::*;
            match event.phase {
                Down => shown.pressed = Some((event.pointer_id, event.position, false)),
                Move => {
                    if let Some((id, last, drag)) = shown
                        .pressed
                        .as_mut()
                        .filter(|(id, _, _)| *id == event.pointer_id)
                    {
                        let _ = id;
                        if let Some(list) = shown.lists.get_mut(shown.current) {
                            if *drag || hit(&list.viewport, *last) {
                                let delta = event.position.y - last.y;
                                if delta.abs() > 1.0 {
                                    *drag = true;
                                }
                                if *drag {
                                    list.scroll -= delta / scale;
                                }
                            }
                        }
                        *last = event.position;
                    }
                }
                Cancel => {
                    shown.pressed = None;
                }
                Up => {
                    let Some((id, _, drag)) = shown.pressed.take() else {
                        continue;
                    };
                    if id != event.pointer_id || drag {
                        continue;
                    }
                    if hit(&shown.positive, event.position) {
                        answer = Some(true);
                        continue;
                    }
                    if hit(&shown.close, event.position)
                        || shown
                            .negative
                            .as_ref()
                            .is_some_and(|s| hit(s, event.position))
                        || !hit(&shown.window, event.position)
                    {
                        answer = Some(false);
                        continue;
                    }
                    if shown.draft.is_some() {
                        if let Some(index) = shown
                            .lists
                            .iter()
                            .position(|list| hit(&list.tab, event.position))
                        {
                            shown.current = index;
                            continue;
                        }
                        if let Some(list) = shown.lists.get(shown.current) {
                            if let Some(check) = list
                                .checks
                                .iter()
                                .find(|check| hit(&check.button, event.position))
                            {
                                let ids = shown
                                    .draft
                                    .as_mut()
                                    .expect("settings draft")
                                    .entry(list.site)
                                    .or_default();
                                let checked = check.ids.iter().all(|id| ids.contains(id));
                                for id in &check.ids {
                                    if checked {
                                        ids.retain(|i| i != id);
                                    } else if !ids.contains(id) {
                                        ids.push(*id);
                                    }
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
        if !inputs.pointers.is_empty() {
            world
                .resource_mut::<crate::action_button::ActionTapConsumed>()
                .0 = true;
        }
        if let Some(accepted) = answer {
            close(world, &mut state, accepted);
        }
    } else if world.resource::<ScreenManager>().current() == crate::ui_layers::LayerId::MysekaiInfo
        && !world.resource::<ScreenManager>().exists_dialog()
    {
        let mut query = world.query_filtered::<&UiPrefabView, With<crate::info::InfoRoot>>();
        let layouts = world.resource::<UiLayouts>();
        if let Some(doc) = layouts.document("Info") {
            if let Ok(option) = component(doc, "Sekai.Mysekai.MysekaiInfoOption1Page") {
                if let Ok(button) =
                    reference(doc, &option.fields["_bulkHarvestTargetSettingButton"])
                {
                    let tapped = query.iter(world).any(|view| {
                        inputs.pointers.iter().any(|event| {
                            event.phase == crate::gesture::UiPointerPhase::Up
                                && view
                                    .rect(layouts, &button, size)
                                    .is_some_and(|r| r.active && r.contains(point(event.position)))
                        })
                    });
                    let enabled = !world
                        .get_resource::<crate::server_panel::ServerPanel>()
                        .is_some_and(|p| p.is_tutorial());
                    if tapped && enabled {
                        state.open_settings = true;
                    }
                }
            }
        }
    }
    world.insert_resource(state);
    world.insert_resource(inputs);
}

fn info_visibility(
    layouts: Res<UiLayouts>,
    panel: Option<Res<crate::server_panel::ServerPanel>>,
    mut views: Query<&mut UiPrefabView, With<crate::info::InfoRoot>>,
) {
    let Some(doc) = layouts.document("Info") else {
        return;
    };
    let Ok(option) = component(doc, "Sekai.Mysekai.MysekaiInfoOption1Page") else {
        return;
    };
    let Ok(root) = reference(doc, &option.fields["_bulkHarvestTargetSettingRoot"]) else {
        return;
    };
    let visible = !panel.is_some_and(|p| p.is_tutorial());
    for mut view in &mut views {
        view.set_visible(&root, visible);
    }
}

pub fn install(app: &mut App) {
    app.init_resource::<BulkDialogs>()
        .init_resource::<DialogInput>()
        .add_systems(Update, info_visibility.after(crate::info::place))
        .add_systems(
            Update,
            (collect_input, input, tick)
                .chain()
                .after(super::bulk::tick),
        );
}

#[cfg(test)]
mod icon_readiness_tests {
    use super::*;

    #[test]
    fn pending_icons_do_not_consume_or_restart_rewards() {
        let mut world = World::new();
        let mut state = BulkDialogs::default();
        for (site_id, special_started) in [(5, true), (6, false)] {
            state.rewards.push_back(RewardResult {
                plan: Plan::default(),
                special_started,
                site_id,
                before_stamina: super::super::law::Stamina {
                    normal: 10,
                    enhance: 2,
                    boost: 3,
                },
                before_tools: Vec::new(),
                reply: UserMysekaiSiteBulkHarvestResponse {
                    updated_resources: Default::default(),
                    obtained_drop_resources: Vec::new(),
                    player_rank_rewards: Vec::new(),
                    is_in_break_time: false,
                },
            });
        }
        open(&mut world, &mut state);
        assert_eq!(
            state
                .rewards
                .iter()
                .map(|r| (r.site_id, r.special_started))
                .collect::<Vec<_>>(),
            [(5, true), (6, false)]
        );
        assert!(state.shown.is_none());
    }
}
