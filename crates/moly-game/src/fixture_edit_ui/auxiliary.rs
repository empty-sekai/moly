//! Original shared BackUI and the actual three-button save confirmation.

use crate::ui_layout::{UiLayouts, UiPrefabView};
use moly_assets::ui_layout::UiPrefab;

use super::compose::{component, enabled, field};

pub(super) const HEADER: &str = "EditorHeader";
pub(super) const EXIT: &str = "EditorExit";
/// `Common2ButtonDialog`, the remove-all confirmation.
pub(super) const CLEAN_UP: &str = "Common2";

pub(super) struct ExitBindings {
    pub window: String,
    pub close: String,
    pub cancel: String,
    pub discard: String,
    pub save: String,
}

/// The remove-all confirmation: `ScreenManager.Show2ButtonDialog` with the
/// message `MSG_MYSEKAI_SITE_LAYOUT_EDIT_CLEAN_UP_CONFIRM`, the positive
/// label `WORD_CLEAN_UP` and the negative `WORD_CANCEL`
/// (`ScreenLayerSiteEditMode.OnRemoveFixtureAll`); the close button and a
/// tap outside the window (`allowCloseExternal`) cancel.
pub(crate) struct CleanUpBindings {
    pub window: String,
    pub close: String,
    pub negative: String,
    pub positive: String,
}

pub(super) fn clean_up(
    doc: &UiPrefab,
    layouts: &UiLayouts,
    view: &mut UiPrefabView,
) -> Result<CleanUpBindings, String> {
    let dialog = crate::two_button_dialog::resolve(doc)?;
    view.set_visible("WindowRoot/Tabs", false);
    view.set_text(
        &dialog.positive_label,
        layouts.wording("WORD_CLEAN_UP"),
    );
    view.set_text(
        &dialog.negative_label,
        layouts.wording("WORD_CANCEL"),
    );
    view.set_text(
        "Content/MessageBody",
        layouts.wording("MSG_MYSEKAI_SITE_LAYOUT_EDIT_CLEAN_UP_CONFIRM"),
    );
    for button in [&dialog.positive, &dialog.negative, &dialog.close] {
        enabled(view, doc, button, true);
    }
    Ok(CleanUpBindings {
        window: "WindowRoot".to_owned(),
        close: dialog.close,
        negative: dialog.negative,
        positive: dialog.positive,
    })
}

pub(super) fn header(doc: &UiPrefab, view: &mut UiPrefabView) -> Result<String, String> {
    // Header's unrelated root business component is intentionally opaque.
    // Resolve the fully decoded persistent BackUI callback, not guessed fields.
    let mut buttons = doc
        .nodes
        .iter()
        .flat_map(|node| node.components.iter())
        .filter(|c| {
            c.class.ends_with(".CustomButton")
                && c.fields["m_OnClick"]["m_Calls"]
                    .as_array()
                    .is_some_and(|calls| {
                        calls.iter().any(|call| {
                            call["m_MethodName"].as_str() == Some("OnClickBackUIScreen")
                        })
                    })
        });
    let button = buttons
        .next()
        .ok_or("shared source BackUI callback missing")?;
    if buttons.next().is_some() {
        return Err("shared source BackUI callback is ambiguous".into());
    }
    let selector = format!("@{}", button.path_id);
    let back = &doc.nodes[doc.find(&selector)?].path;
    for node in &doc.nodes {
        let is_ancestor = back == &node.path || back.starts_with(&format!("{}/", node.path));
        let is_descendant = node.path.starts_with(&format!("{back}/"));
        if !is_ancestor && !is_descendant {
            view.set_visible(&format!("@{}", node.transform_id), false);
        }
    }
    enabled(view, doc, &selector, true);
    Ok(selector)
}

pub(super) fn exit(
    doc: &UiPrefab,
    layouts: &UiLayouts,
    view: &mut UiPrefabView,
) -> Result<ExitBindings, String> {
    let fields = &component(doc, &doc.prefab, ".MysekaiEditSaveConfirmationDialog")?.fields;
    let rows = fields["dialogButtonObjectList"]
        .as_array()
        .ok_or("source exit button rows")?;
    let mut bind = |key: &str, word: &str| -> Result<String, String> {
        let row = rows
            .iter()
            .find(|row| row["key"].as_str() == Some(key))
            .ok_or_else(|| format!("source exit button {key} is missing"))?;
        let button = field(row, "dialogButton")?;
        view.set_visible(&button, true);
        view.set_text(
            &field(row, "dialogButtonLabelMesh")?,
            layouts.wording(word),
        );
        enabled(view, doc, &button, true);
        Ok(button)
    };
    let cancel = bind("Cancel", "WORD_CANCEL")?;
    let discard = bind("Discard", "WORD_NOT_SAVE_RETURN")?;
    let save = bind("Save", "WORD_SAVE_RETURN")?;
    view.set_visible("WindowRoot/Tabs", false);
    view.set_text(
        &field(fields, "messageBodyTextMesh")?,
        layouts.wording("WORD_EDIT_SAVE_CONFIRMATION"),
    );
    Ok(ExitBindings {
        window: field(fields, "windowObject")?,
        close: field(fields, "closeButton")?,
        cancel,
        discard,
        save,
    })
}
