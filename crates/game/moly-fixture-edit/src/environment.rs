//! The environment tab of the layout editor: the environment list, the
//! preview a choice shows and the content switch between ground, road and
//! environment tabs.
//!
//! Input: the player's phenomena, `ServerPanel` and `PhenomenaMaster`
//! resources, the editor icons and wordings, and the tab the player picks.
//! Output: the tab's `EditEnvironmentView` rows (default cell first, then the
//! player's phenomena in master order), the draft's environment choice with
//! `LayoutEnvironmentPreview`, and the grid and camera for the chosen content.
//!
//! Invariants: choosing an environment changes the draft only; today's
//! scheduled weather stays authoritative for the default cell (thumbnail id 0
//! is master id 0, not a separate sequence). The ordinary weather publisher
//! applies the preview, and after a save or discard the committed home
//! layout. A choice whose icons or resources are not ready is refused, as is
//! any choice while a save is out. Boundary: only the home site offers the
//! outdoor, road and environment tabs; other sites offer the floor alone.

use super::{EditContentType, EditEnvironmentView, EditSession};
use bevy::prelude::*;

pub(super) fn choices(world: &World) -> Vec<EditEnvironmentView> {
    let mut rows = vec![EditEnvironmentView {
        phenomena_id: None,
        name: world
            .get_resource::<moly_features::ui_layout::UiLayouts>()
            .map(|layouts| layouts.wording("WORD_PHENOMENA_DEFAULT").to_owned())
            .unwrap_or_default(),
    }];
    if world
        .get_resource::<moly_features::server_panel::ServerPanel>()
        .is_some_and(|panel| panel.is_tutorial())
    {
        return rows;
    }
    let (Some(user), Some(masters)) = (
        world.get_resource::<moly_features::userdata::phenomena::ClientMysekaiPhenomena>(),
        world.get_resource::<moly_features::sitemap_phenomena::PhenomenaMaster>(),
    ) else {
        return rows;
    };
    for row in user.rows() {
        if let Some(master) = masters.phenomena(row.mysekai_phenomena_id) {
            rows.push(EditEnvironmentView {
                phenomena_id: Some(master.id),
                name: master.name.clone(),
            });
        }
    }
    // ThumbnailData.Seq is MasterData.id (0 for default), not a separate master sequence column.
    rows.sort_by_key(|row| row.phenomena_id.unwrap_or(0));
    rows
}

pub(super) fn preview(world: &mut World, session: &EditSession) {
    if let Some(base) = session
        .baseline
        .as_ref()
        .filter(|base| base.site_type() == "home_site")
    {
        world.insert_resource(moly_features::field_modes::LayoutEnvironmentPreview {
            site_id: base.site_id(),
            phenomena_id: session.environment,
        });
    }
}

pub(super) fn select(world: &mut World, session: &mut EditSession, phenomena_id: Option<i32>) {
    if session.content != EditContentType::Environment || session.saving.is_some() {
        return;
    }
    if !choices(world)
        .iter()
        .any(|row| row.phenomena_id == phenomena_id)
    {
        session.say("该天气尚未解锁，未改变当前布置。");
        return;
    }
    let ready = world
        .get_resource::<crate::ui::EditorIcons>()
        .is_some_and(|icons| icons.environment(phenomena_id).is_some());
    let target = phenomena_id.or_else(|| {
        world
            .get_resource::<moly_features::server_panel::TodayPhenomena>()
            .and_then(|today| today.phenomena_id())
    });
    let available = target.is_some_and(|id| {
        world
            .get_resource::<moly_features::weather::PhenomenonCatalogue>()
            .is_some_and(|catalogue| catalogue.0.iter().any(|option| option.id == id))
    });
    if !ready || !available {
        session.say("天气资源仍在加载，请稍后重试。");
        return;
    }
    if session.environment != phenomena_id {
        session.environment = phenomena_id;
        session.changed();
        preview(world, session);
        info!(
            "[edit-environment] preview fixed {:?}; default resolves to today's weather; not saved",
            phenomena_id
        );
    }
}

pub(super) fn select_content(
    world: &mut World,
    session: &mut EditSession,
    content: EditContentType,
) {
    let Some(base) = session.baseline.as_ref() else {
        return;
    };
    let allowed = if base.site_type() == "home_site" {
        matches!(
            content,
            EditContentType::Outdoor | EditContentType::Road | EditContentType::Environment
        )
    } else {
        content == EditContentType::Floor
    };
    if !allowed || session.content == content || session.saving.is_some() {
        return;
    }
    if content == EditContentType::Environment {
        let ready = world
            .get_resource::<crate::ui::EditorIcons>()
            .is_some_and(|icons| icons.environment(None).is_some())
            && world
                .get_resource::<moly_features::ui_layout::UiLayouts>()
                .zip(world.get_resource::<AssetServer>())
                .is_some_and(|(layouts, server)| layouts.ready("EditorEnvironmentCell", server));
        if !ready {
            session.say("天气列表仍在加载或当前资源版本未提供，请稍后重试。");
            return;
        }
    }
    session.cancel_selection();
    session.road = super::road::RoadSession::default();
    session.content = content;
    session.changed();
    if content == EditContentType::Environment {
        super::edit_grid::hide(world);
        if let Err(reason) = crate::floor_camera::enter_environment(world) {
            warn!("[edit-environment] {reason}");
        }
    } else {
        super::edit_grid::show(
            world,
            if content == EditContentType::Road {
                8
            } else {
                2
            },
        );
        if let Err(reason) = crate::floor_camera::enter(world) {
            warn!("[edit-environment] {reason}");
        }
    }
    info!("[edit-selector] selected {content:?}");
}
