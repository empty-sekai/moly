//! `Common2ButtonDialog`, the confirmation `ScreenManager.Show2ButtonDialog`
//! shows. Every screen that opens it resolves its buttons here.

use moly_assets::ui_layout::UiPrefab;

/// The dialog's buttons and the two footer labels, as `@` addresses of the
/// layout document.
pub(crate) struct TwoButtonDialog {
    pub positive: String,
    pub positive_label: String,
    pub negative: String,
    pub negative_label: String,
    pub close: String,
}

/// The dialog component itself is not decoded in the layout data, so its
/// `positiveButton` and `negativeButton` are found as the two footer
/// buttons whose serialized labels are the prefab's own OK and cancel texts.
/// Component and object ids differ between regions' prefabs; the labels and
/// the hierarchy do not.
pub(crate) fn resolve(doc: &UiPrefab) -> Result<TwoButtonDialog, String> {
    let footer = doc.find("WindowRoot/FooterButtons")?;
    let button_path = format!("{}/UIPartsCommonButton", doc.nodes[footer].path);
    let mut buttons = Vec::new();
    for node in &doc.nodes {
        if node.path != button_path {
            continue;
        }
        let button = node
            .components
            .iter()
            .find(|c| c.class.ends_with(".CustomButton"))
            .ok_or("a footer button has no CustomButton")?;
        buttons.push((node.transform_id, button.path_id));
    }
    let label_of = |transform: i64| -> Result<(i64, String), String> {
        let text = doc
            .nodes
            .iter()
            .find(|node| node.parent_transform_id == transform && node.path.ends_with("/Text"))
            .ok_or("a footer button has no Text child")?;
        let mesh = text
            .components
            .iter()
            .find(|c| c.class.ends_with(".CustomTextMesh"))
            .ok_or("a footer button label has no CustomTextMesh")?;
        let serialized = mesh.fields["m_text"]
            .as_str()
            .unwrap_or_default()
            .to_owned();
        Ok((mesh.path_id, serialized))
    };
    let (mut positive, mut negative) = (None, None);
    for (transform, button) in &buttons {
        let (label, serialized) = label_of(*transform)?;
        match serialized.as_str() {
            "OK" => positive = Some((*button, label)),
            "キャンセル" => negative = Some((*button, label)),
            other => {
                return Err(format!(
                    "a footer button's label {other:?} is neither OK nor cancel"
                ))
            }
        }
    }
    let (Some((positive, positive_label)), Some((negative, negative_label))) = (positive, negative)
    else {
        return Err(format!(
            "the dialog footer has {} buttons, not one OK and one cancel",
            buttons.len()
        ));
    };
    let close = doc.find("WindowRoot/UIPartsCloseButton")?;
    let close = doc.nodes[close]
        .components
        .iter()
        .find(|c| c.class.ends_with(".CustomButton"))
        .ok_or("the close button has no CustomButton")?;
    Ok(TwoButtonDialog {
        positive: format!("@{positive}"),
        positive_label: format!("@{positive_label}"),
        negative: format!("@{negative}"),
        negative_label: format!("@{negative_label}"),
        close: format!("@{}", close.path_id),
    })
}
