//! The inventory preview's description page and the recycle confirmation.
//!
//! ## Description (`InventoryPreview.RefreshDescriptionView`)
//!
//! A fixture's page shows its `flavorText` (`DescriptionTextView`) and its
//! grid size (`MysekaiFixtureUtility.GetGridSizeText`: the master's grid
//! `width` × `depth`); a material's page its flavour text; a tool's
//! (`SetTool` -> `DescriptionView.Setup(flavorText)`) its `description`. The
//! page's site group (`DescriptionCell`'s `SiteNameGroup`, the site names)
//! needs the site master and is left off.
//!
//! ## Recycle (`InventoryPreview.OpenDisassembleDialog`)
//!
//! `SetRecycleButtonState`: the button is off for a list with no rows and
//! for the fixture list; `SetRecycleButtonEnable` enables it for a row whose
//! quantity is above 0. The source shows the item's icon, name and count
//! with `MysekaiInventoryScrapFixtureDialog` (dialog type 328); that prefab is
//! not among the UI documents a runtime root serves here, so the shared
//! two-button dialog stands in with the item's name and count as its message,
//! and the missing item dialog is named once.
//!
//! Its OK calls `MysekaiInventoryUtility.SendDisassembleApi`, which sends
//! `PostUserMysekaiDisassembleFixtureApi` for a fixture (the fixture id, the
//! texture and the count), `PostUserMysekaiRecycleMaterialApi` for a material
//! and `PostUserMysekaiRecycleToolApi` for a tool (the id and the count); its
//! cancel (`onClickCancel`) drops the request. `OnDisassemble` then shows the
//! returned resources with the common one-button dialog. The item list shows
//! no confirmation: the source builds no request for it.

use bevy::prelude::*;
use moly_protocol::api::{
    FixtureDisassembleRequest, MysekaiInventoryDisassemble, RecycleMaterialRequest,
    RecycleToolRequest,
};
use moly_protocol::Reply;

use crate::common_dialog::Buttons;
use crate::ui_layers::{DialogBackKey, DialogId, DialogType, DisplayLayerType, ScreenManager};
use crate::ui_layout::UiPrefabView;

use super::view::Bindings;
use super::{ContentListType, Masters, Row};

/// `DialogType::MysekaiFixtureSearchDialog`.
const SEARCH_DIALOG: DialogType = DialogType(333);
/// `MysekaiInventoryUtility.SelectedNum`: the count this screen recycles.
const SELECTED_NUM: i32 = 1;

/// The page of the selected row (`InventoryPreview`'s `_descriptionView`).
pub(crate) fn show_description(
    view: &mut UiPrefabView,
    bindings: &Bindings,
    kind: ContentListType,
    row: &Row,
    masters: &Masters,
) {
    let page = &bindings.description;
    let Some(master) = masters.row(kind, row) else {
        view.set_visible(&page.root, false);
        view.set_visible(&page.sites, false);
        return;
    };
    let text = match kind {
        ContentListType::Fixture => {
            let grid = grid_size_text(master.grid);
            if master.description.is_empty() {
                grid
            } else {
                format!("{}\n{}", master.description, grid)
            }
        }
        ContentListType::Material | ContentListType::Item | ContentListType::Tool => {
            master.description.clone()
        }
    };
    view.set_text(&page.text, text);
    view.set_visible(&page.root, true);
    // The site page (a material's `mysekaiSiteIds` names) needs the site
    // master: it is not shown.
    view.set_visible(&page.sites, false);
}

/// `MysekaiFixtureUtility.GetGridSizeText`: the master's grid `width` ×
/// `depth`; a fixture without a grid shows no size.
fn grid_size_text(grid: Option<(i32, i32)>) -> String {
    match grid {
        Some((width, depth)) => format!("{width}×{depth}"),
        None => String::new(),
    }
}

/// `SetRecycleButtonState` and `SetRecycleButtonEnable`: the button shows for
/// a row of quantity above 0 in a list that recycles, and only when the root
/// carries the confirmation.
pub(crate) fn set_button(view: &mut UiPrefabView, bindings: &Bindings, row: &Row) {
    let recycle = bindings.dialog.available
        && request_of(bindings.kind, row).is_some()
        && row.quantity > 0;
    view.set_visible(&bindings.recycle_button, recycle);
}

/// The confirmation: the dialog it shows and the request its OK sends.
#[derive(Resource, Default)]
pub(crate) struct Recycle {
    dialog: Option<DialogId>,
    request: Option<MysekaiInventoryDisassemble>,
}

impl Recycle {
    /// The confirmation is shown: the screen takes no tap behind it, and the
    /// back key is the dialog's.
    pub(crate) fn confirming(&self) -> bool {
        self.dialog.is_some()
    }

    /// `InventoryPreview.OpenDisassembleDialog`: shows the two-button dialog
    /// for the row; `None` when it cannot show (named by the caller).
    pub(crate) fn open(
        &mut self,
        world: &mut World,
        kind: ContentListType,
        row: &Row,
        name: &str,
    ) -> Option<DialogId> {
        if self.dialog.is_some() {
            return None;
        }
        let request = request_of(kind, row)?;
        let dialog = crate::common_dialog::open(
            world,
            "InventoryPreview.OpenDisassembleDialog",
            format!("{name} ×{SELECTED_NUM}"),
            Buttons::Two {
                ok: "WORD_OK",
                cancel: "WORD_CANCEL",
            },
            Some("MOLY_INVENTORY_RECYCLE_CONFIRM_SECS"),
        );
        let Some(dialog) = dialog else {
            return None;
        };
        info!(
            "[inventory] OpenDisassembleDialog: Show2ButtonDialog(MysekaiInventoryScrapFixtureDialog, WORD_OK, WORD_CANCEL) as {dialog:?}; Setup({name:?} ×{SELECTED_NUM}); OK sends {}",
            request.api_name()
        );
        self.dialog = Some(dialog);
        self.request = Some(request);
        Some(dialog)
    }

    /// The confirmation's answer, once the shown dialog has one (`None` while
    /// it is shown). OK keeps the request; cancel, the close button and an
    /// outside tap drop it (`onClickCancel`).
    pub(crate) fn answer(&mut self, world: &mut World) -> Option<bool> {
        let dialog = self.dialog?;
        let answer = crate::common_dialog::answer(world, dialog)?;
        self.dialog = None;
        if !answer {
            self.request = None;
            info!("[inventory] recycle confirmation {dialog:?}: cancelled");
        }
        Some(answer)
    }

    /// `OnClickOKMysekaiInventoryScrapFixtureDialog`: the request the OK
    /// sends, once.
    pub(crate) fn take_request(&mut self) -> Option<MysekaiInventoryDisassemble> {
        self.request.take()
    }

    /// `SetEnableRecycleButton`: the screen left the confirmation open, so it
    /// closes and its request is dropped.
    pub(crate) fn dismiss(&mut self) {
        if let Some(dialog) = self.dialog.take() {
            info!("[inventory] recycle confirmation {dialog:?}: closed with the screen");
        }
        self.request = None;
    }
}

/// `SendDisassembleApi`'s request for the row of a list: a fixture, a material
/// or a tool; the item list has none.
fn request_of(kind: ContentListType, row: &Row) -> Option<MysekaiInventoryDisassemble> {
    match kind {
        ContentListType::Fixture => Some(MysekaiInventoryDisassemble::Fixture(
            FixtureDisassembleRequest {
                mysekai_fixture_id: row.id,
                texture_id: row.texture,
                quantity: SELECTED_NUM,
            },
        )),
        ContentListType::Material => Some(MysekaiInventoryDisassemble::Material(
            RecycleMaterialRequest {
                mysekai_material_id: row.id,
                quantity: SELECTED_NUM,
            },
        )),
        ContentListType::Tool => Some(MysekaiInventoryDisassemble::Tool(RecycleToolRequest {
            mysekai_tool_id: row.id,
            quantity: SELECTED_NUM,
        })),
        ContentListType::Item => None,
    }
}

/// `OnDisassemble`: the resources the reply returns, with the common
/// one-button dialog.
pub(crate) fn reward(world: &mut World, outcome: &Result<Reply, moly_protocol::Refusal>) {
    match outcome {
        Ok(Reply::InventoryRecycle(reply)) => {
            let rows: Vec<(String, i32, i32)> = reply
                .obtained_resources
                .iter()
                .map(|row| (row.resource_type.clone(), row.resource_id, row.quantity))
                .collect();
            info!(
                "[inventory] OnDisassemble: {} obtained resource row(s) {rows:?}; the list selects its first item again",
                rows.len()
            );
            // The rows are named by their resource type and id: the resource
            // name master is not read here.
            let text = rows
                .iter()
                .map(|(kind, id, quantity)| format!("{kind} {id} ×{quantity}"))
                .collect::<Vec<_>>()
                .join("\n");
            if !text.is_empty() {
                let _ = crate::common_dialog::open_detached(
                    world,
                    "InventoryPreview.OnDisassemble",
                    text,
                    Buttons::One { ok: "WORD_OK" },
                );
            }
        }
        Ok(other) => warn!("[inventory] OnDisassemble: not the recycle reply: {other:?}"),
        Err(refusal) => {
            error!(
                "[inventory] OnDisassemble: the recycle was refused ({refusal}); the API error shows"
            );
            let _ = crate::common_dialog::open_detached(
                world,
                "InventoryPreview.OnDisassemble",
                refusal.reason.clone(),
                Buttons::One { ok: "WORD_OK" },
            );
        }
    }
}

/// `ScreenLayerMysekaiInventory.OnSearch`: the search dialog. Its word field
/// (`CustomInputFieldTextMesh`) is not built, so it closes again at once.
pub(crate) fn open_search(screens: &mut ScreenManager) {
    match screens.show_dialog(
        SEARCH_DIALOG,
        DisplayLayerType::LayerDialog,
        DialogBackKey::Close,
        "ScreenLayerMysekaiInventory.OnSearch",
    ) {
        Ok(id) => {
            screens.open_dialog(id);
            screens.dialog_open_finished(id);
            info!(
                "[inventory] OnSearch: ShowDialog({SEARCH_DIALOG:?}) {id:?}; the search field is not built, so no word reaches the fixture filter"
            );
            screens.close_dialog(id);
            screens.dialog_destroyed(id);
        }
        Err(reason) => warn!("[inventory] OnSearch: {reason}"),
    }
}