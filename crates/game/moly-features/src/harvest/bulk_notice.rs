//! ScreenLayerMysekaiNotice's right-upper one-cell tool notice controller.
use super::bulk_ui::{component, reference, referenced_component};
use crate::collect_notice::slide::{CellCall, NoticeData, SlideNotices};
use crate::ui_layout::{UiLayouts, UiPrefabView};
use bevy::{camera::visibility::RenderLayers, prelude::*, window::PrimaryWindow};
use std::collections::VecDeque;

#[derive(Clone)]
pub struct ToolNotice {
    pub tool: i64,
    pub broken: bool,
}
impl NoticeData for ToolNotice {
    fn notice_duration(&self) -> f32 {
        1.5
    }
}

#[derive(Resource, Default)]
pub struct BulkNotices {
    pub queue: VecDeque<ToolNotice>,
    shown: Option<Shown>,
}
struct Shown {
    entity: Entity,
    root: String,
    text: String,
    image: String,
    atlas: String,
    x: f32,
    y: f32,
    slide: SlideNotices<ToolNotice>,
}
const RUNTIME: &str = "BulkToolNoticeRuntime";

pub fn tick(world: &mut World) {
    let mut state = world
        .remove_resource::<BulkNotices>()
        .expect("bulk notices installed");
    if state.shown.is_none() && state.queue.is_empty() {
        world.insert_resource(state);
        return;
    }
    if state.shown.is_none() {
        let layouts = world.resource::<UiLayouts>();
        let server = world.resource::<AssetServer>().clone();
        if !layouts.ready("Notice", &server) {
            world.insert_resource(state);
            return;
        }
        let template = layouts.document("Notice").expect("notice document").clone();
        let make = (|| -> Result<_, String> {
            let fields = &component(&template, "Sekai.Mysekai.ScreenLayerMysekaiNotice")?.fields;
            let source = reference(&template, &fields["rightUpperMessageCellPrefab"])?;
            let parent = reference(&template, &fields["rightUpperMessageRoot"])?;
            let c = template.nodes[template.find(&source)?]
                .components
                .iter()
                .find(|c| c.class == "Sekai.Mysekai.MySekaiCommonNoticeCell")
                .ok_or("source tool notice cell absent")?;
            let (loader, _) = referenced_component(&template, &c.fields, "iconLoader")?;
            let mut doc = template.clone();
            let instance =
                doc.instantiate_subtree(&parent, &template, &source, "BulkToolNotice")?;
            let clone = |v: &serde_json::Value| -> Result<String, String> {
                let key = reference(&template, v)?;
                instance.selector(key[1..].parse().map_err(|_| "identity")?)
            };
            let root = instance.root_selector();
            let root_path = doc.nodes[doc.find(&root)?].path.clone();
            let mut view = UiPrefabView::new(RUNTIME, crate::balloon::BALLOON_LAYER);
            for node in &doc.nodes {
                if node.path != root_path
                    && !node.path.starts_with(&format!("{root_path}/"))
                    && !root_path.starts_with(&format!("{}/", node.path))
                {
                    view.set_visible(&format!("@{}", node.game_object_id), false);
                }
            }
            view.set_visible(&root, false);
            for field in ["loadingObject", "notFoundObject"] {
                if let Ok(selector) = clone(&loader.fields[field]) {
                    view.set_visible(&selector, false);
                }
            }
            let pos = template.nodes[template.find(&source)?]
                .rect
                .anchored_position;
            let bound = (
                root,
                clone(&c.fields["messageText"])?,
                clone(&loader.fields["targetGraphic"])?,
                clone(&c.fields["iconAtlasImage"])?,
                pos,
            );
            Ok((doc, view, bound))
        })();
        let Ok((doc, view, (root, text, image, atlas, pos))) = make else {
            if let Err(e) = make {
                error!("[bulk-notice] {e}");
            }
            world.insert_resource(state);
            return;
        };
        world
            .resource_mut::<UiLayouts>()
            .replace_runtime_document(RUNTIME, doc, &server)
            .expect("tool notice composition");
        let entity = world
            .spawn((
                view,
                Transform::from_xyz(0., 0., 100.),
                Visibility::Visible,
                RenderLayers::layer(crate::balloon::BALLOON_LAYER),
            ))
            .id();
        state.shown = Some(Shown {
            entity,
            root,
            text,
            image,
            atlas,
            x: pos[0],
            y: 0.,
            slide: SlideNotices::with_shape(pos[0], 1, 64, 600.),
        });
    }
    let shown = state.shown.as_mut().expect("built above");
    let dt = world.resource::<Time>().delta_secs();
    let mut calls = Vec::new();
    while let Some(notice) = state.queue.pop_front() {
        shown.slide.notice(notice, dt, &mut calls);
    }
    shown.slide.step(dt, &mut calls);
    for (_, call) in calls {
        match call {
            CellCall::Setup { slot } => {
                let notice = shown.slide.slot(slot).expect("notice slot").clone();
                let def = world
                    .get_resource::<super::catalog::HarvestCatalog>()
                    .and_then(|c| c.tool(notice.tool))
                    .cloned();
                let Some(def) = def else {
                    continue;
                };
                let kind = match def.tool_type {
                    super::law::ToolType::Axe => "WORD_AXE",
                    super::law::ToolType::Pickaxe => "WORD_PICKAXE",
                };
                let layouts = world.resource::<UiLayouts>();
                let key = if notice.broken {
                    "MSG_BROKEN_TOOL"
                } else {
                    "MSG_WEAR_OUT_TOOL"
                };
                let text = moly_law::text::custom_text_mesh::format_wording(
                    &layouts.wording(key),
                    &[layouts.wording(kind)],
                )
                .expect("tool notice wording");
                let colour = layouts
                    .palette_color(if notice.broken { 17 } else { 14 })
                    .expect("source tool notice colour");
                let bundle = format!("mysekai/thumbnail/tool/{}_t", def.assetbundle);
                let file = format!("{}_t", def.sprite_name);
                let asset = world
                    .resource::<crate::item_icon::ItemIcons>()
                    .texture(&bundle, &file);
                let loaded = asset.is_ok();
                if let Ok(asset) = asset {
                    let server = world.resource::<AssetServer>().clone();
                    world
                        .resource_mut::<UiLayouts>()
                        .register_runtime_texture(&bundle, &asset, &server);
                }
                let mut view = world
                    .get_mut::<UiPrefabView>(shown.entity)
                    .expect("notice view");
                view.set_text(&shown.text, text);
                view.set_graphic_color(shown.text[1..].parse().expect("text identity"), colour);
                view.set_visible(&shown.atlas, false);
                view.set_visible(&shown.image, loaded);
                if loaded {
                    view.set_texture(&shown.image, &bundle);
                }
                view.set_size_delta(&shown.image, Vec2::splat(56.));
                if notice.broken {
                    super::damage::push_se(
                        &mut world.resource_mut::<crate::audio::SeRequests>(),
                        "se_broken_tool",
                        "bulk-tool-notice",
                    );
                }
            }
            CellCall::Wakeup => world
                .get_mut::<UiPrefabView>(shown.entity)
                .expect("notice view")
                .set_visible(&shown.root, true),
            CellCall::Sleep => world
                .get_mut::<UiPrefabView>(shown.entity)
                .expect("notice view")
                .set_visible(&shown.root, false),
            CellCall::X(x) => {
                shown.x = x;
                world
                    .get_mut::<UiPrefabView>(shown.entity)
                    .expect("notice view")
                    .set_anchored_position(&shown.root, Vec2::new(x, shown.y));
            }
        }
    }
    let window = world
        .query_filtered::<&Window, With<PrimaryWindow>>()
        .single(world)
        .ok()
        .cloned();
    if let (Some(window), Some(canvas)) =
        (window, world.get_resource::<crate::canvas::RootCanvas>())
    {
        let scale = canvas.scale(&window);
        if let Some(mut transform) = world.get_mut::<Transform>(shown.entity) {
            transform.scale = Vec3::splat(scale);
        }
    }
    world.insert_resource(state);
}
