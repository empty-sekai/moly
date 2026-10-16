//! Explicit embedded-stage initialization: no site/editor chrome, no offline
//! fixture preset and no demonstration NPC roster. Gameplay, temporary cast,
//! layout installation and restoration retain their ordinary runtime owners.
use bevy::prelude::*;

#[derive(Resource)]
pub(crate) struct BrowserStage;

/// Called before Startup; this never changes an already-running world.
pub fn configure_browser_stage(app: &mut App) {
    app.insert_resource(BrowserStage);
    #[cfg(target_arch = "wasm32")]
    crate::audio_startup::configure(app);
}

pub(crate) fn standalone(stage: Option<Res<BrowserStage>>) -> bool {
    stage.is_none()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stage_is_an_explicit_initialization_option() {
        let mut app = App::new();
        assert!(!app.world().contains_resource::<BrowserStage>());
        configure_browser_stage(&mut app);
        assert!(app.world().contains_resource::<BrowserStage>());
    }
}

/// The stage displays the original dialogue panel, not the original Auto/Skip
/// menu. Resolve its authored anchor chain with the shared geometry law only
/// when no automatic-layout controller can influence it. Unrelated hidden
/// controls must not require fonts or localized wordings just to position it.
pub(crate) fn dialogue_panel(
    doc: &moly_assets::ui_layout::UiPrefab,
    path: &str,
    canvas: Vec2,
) -> Result<moly_assets::ui_layout::UiRect, String> {
    use std::collections::HashMap;
    let target = doc.find(path)?;
    let mut current = Some(target);
    while let Some(index) = current {
        let node = &doc.nodes[index];
        if node.components.iter().any(|component| {
            component.enabled
                && matches!(
                    component.class.rsplit('.').next(),
                    Some(
                        "HorizontalLayoutGroup"
                            | "VerticalLayoutGroup"
                            | "GridLayoutGroup"
                            | "ContentSizeFitter"
                            | "AspectRatioFitter"
                    )
                )
        }) {
            return Err(format!(
                "Stage dialogue anchor depends on automatic layout: {}",
                node.path
            ));
        }
        current = if node.parent_transform_id == 0 {
            None
        } else {
            Some(
                doc.nodes
                    .iter()
                    .position(|parent| parent.transform_id == node.parent_transform_id)
                    .ok_or_else(|| {
                        format!("Stage dialogue anchor parent missing: {}", node.path)
                    })?,
            )
        };
    }
    Ok(doc.resolve(canvas, &HashMap::new())[target].clone())
}
