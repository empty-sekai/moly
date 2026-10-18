//! The site map's tap targets for `moly.ui.views` / `moly.ui.tap`.
//!
//! The map itself stays the custom mesh hierarchy in [`super`]; this module
//! registers one runtime `UiPrefabView` whose nodes are the same icon rects
//! (site type, canvas position, button size). It draws nothing: it exists so
//! `moly.ui.views` lists the icons as controls and `moly.ui.tap` resolves a
//! path to the window point. The point goes through the ordinary gesture
//! stream, and the map's own `click` hands it to the same `tap` function a
//! finger uses (`SiteMapIcon.OnClickButtonFeedback` → `OnPushSiteMapIcon`);
//! the released/locked branch, the release balloon and the site change stay
//! the map's, unchanged.
//!
//! Paths: `Sitemap/Site/<mysekaiSiteType>` (`home_site`, `grassland`, ...).
//! Open the map through the ordinary menu view/key, then tap one of these.

use bevy::prelude::*;
use moly_assets::ui_layout::UiPrefab;
use serde_json::json;

use super::{SitemapData, SITEMAP_LAYER};
use crate::ui_layout::{UiLayouts, UiPrefabView};

/// The runtime document's key and the listed view name.
pub(super) const KEY: &str = "Sitemap";

/// The component that marks the tap view and lets the map's visibility pass
/// hide it with the screen.
#[derive(Component)]
pub(crate) struct SitemapTapView;

/// Register the document and spawn the hidden view. `data` is the parsed map
/// layout; the icons are the on-map sites in master order.
pub(super) fn spawn(
    commands: &mut Commands,
    data: &SitemapData,
    layouts: &mut UiLayouts,
    server: &AssetServer,
) {
    if layouts.document(KEY).is_some() {
        return;
    }
    let mut nodes = Vec::new();
    let root_id = 1_i64;
    nodes.push(json!({
        "path": KEY,
        "name": KEY,
        "gameObjectId": root_id,
        "transformId": root_id,
        "parentTransformId": 0,
        "active": true,
        // Centred: the spots are canvas positions with a centre origin.
        "rect": {
            "anchorsMin": [0.5, 0.5],
            "anchorsMax": [0.5, 0.5],
            "anchoredPosition": [0.0, 0.0],
            "sizeDelta": [0.0, 0.0],
            "pivot": [0.5, 0.5],
            "localScale": [1.0, 1.0, 1.0],
            "localRotation": [0.0, 0.0, 0.0, 1.0],
        },
        "components": [],
    }));
    let mut id = 10_i64;
    for (i, site) in data.sites.iter().enumerate() {
        if !site.on_map {
            continue;
        }
        let spot = data.spot(i);
        let path = format!("{KEY}/Site/{}", spot.site_type);
        let scale = spot.root_scale;
        nodes.push(json!({
            "path": path,
            "name": spot.site_type,
            "gameObjectId": id,
            "transformId": id,
            "parentTransformId": root_id,
            "active": true,
            "rect": {
                "anchorsMin": [0.5, 0.5],
                "anchorsMax": [0.5, 0.5],
                "anchoredPosition": [spot.position.x, spot.position.y],
                "sizeDelta": [spot.button.x * scale, spot.button.y * scale],
                "pivot": [0.5, 0.5],
                "localScale": [1.0, 1.0, 1.0],
                "localRotation": [0.0, 0.0, 0.0, 1.0],
            },
            "components": [
                {
                    "pathId": id * 10 + 1,
                    "class": "Sekai.UI.CustomImage",
                    "enabled": true,
                    "fields": {
                        "m_GameObject": { "m_FileID": 0, "m_PathID": id },
                        // Transparent: a hit target only, the map draws the icon.
                        "m_Color": [1.0, 1.0, 1.0, 0.0],
                        "m_RaycastTarget": true,
                        "m_RaycastPadding": [0.0, 0.0, 0.0, 0.0],
                    },
                },
                {
                    "pathId": id * 10 + 2,
                    "class": "Sekai.UI.CustomButton",
                    "enabled": true,
                    "fields": {
                        "m_Interactable": true,
                        "m_Transition": 0,
                        "m_TargetGraphic": [0, id * 10 + 1],
                    },
                },
            ],
        }));
        id += 1;
    }
    let text = json!({
        "version": 2,
        "prefab": KEY,
        "nodes": nodes,
    })
    .to_string();
    let document = match UiPrefab::parse(&text) {
        Ok(document) => document,
        Err(reason) => {
            error!("[sitemap] the tap-target document is not readable ({reason}); the map keeps no view path");
            return;
        }
    };
    if let Err(reason) = layouts.replace_runtime_document(KEY, document, server) {
        error!("[sitemap] the tap-target document is not installed ({reason}); the map keeps no view path");
        return;
    }
    commands.spawn((
        SitemapTapView,
        Visibility::Hidden,
        Transform::default(),
        UiPrefabView::new(KEY, SITEMAP_LAYER),
    ));
    info!(
        "[sitemap] tap-target view {KEY} registered: {} on-map sites as {KEY}/Site/<siteType>",
        data.sites.iter().filter(|site| site.on_map).count()
    );
}
