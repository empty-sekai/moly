//! The inventory lists' rows and their drawn order: the genre and type
//! tabs, the fixture view type and the sort types of the source's two content
//! lists.
//!
//! `FixtureInventoryContentList.CreateCellDataList` makes one cell per owned
//! fixture row with a quantity and a master row (the canvases it lists in
//! `Single` view are not listed here), then `UpdateCellDataList` filters the
//! cells (`GetFilteredCellDataList`: the fixture filter and the selected
//! genre tab) and sorts them (`SortCellDataList`). `UserResourceContentList`
//! makes one cell per owned material, item or tool row with a quantity above
//! 0 and filters it by the selected material type tab; its sort ties every
//! type but the default one on the master sequence.
//!
//! The sort types each dropdown offers are the arrays its `Initialize`
//! passes to `ContentListSortDropDown.Initialize`: `{Get, Sequence, Count,
//! AlphabeticalOrder}` and the layout-cost value 6 for the fixture list, the
//! first four for the resource lists. `SortOrderBy.Desc` is the ascending
//! `OrderBy` list reversed, not a second sort.
//!
//! `ContentListTypeToMysekaiFixtureTabViewType` maps the inventory's fixture
//! list to `edit_layout_home`, the view type of its tab rows: the tabs are
//! those rows in `seq` order, the first (the all tab) carrying every main
//! genre but the reserved ids of `<CheckAllCategoryFilter>b__46_2`, and each
//! tab's sub tabs are the rows whose parent is its `seq`. The material tabs
//! are one per material type (`GenerateMaterialTabCellDataList`).
//!
//! The resource lists filter by the selected type tab alone (`CheckFilter`,
//! `CheckFilterMaterial`); the fixture list by its genre tab and view type.
//! The fixture search word is the filter's `Word`; no carried master names a
//! searchable fixture text, so a word filters nothing and the screen names
//! that once.

use serde_json::Value;

use super::{ContentListType, Row};

/// `Sekai.Mysekai.ContentList.SortType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortType {
    #[default]
    Default,
    Sequence,
    Get,
    Count,
    AlphabeticalOrder,
    /// The fixture list's layout-cost sort (`SortCellDataList` case 6).
    LayoutCost,
}

impl SortType {
    /// The source's value for this type.
    pub fn value(self) -> i32 {
        match self {
            Self::Default => 0,
            Self::Sequence => 1,
            Self::Get => 2,
            Self::Count => 3,
            Self::AlphabeticalOrder => 4,
            Self::LayoutCost => 6,
        }
    }

    /// `ContentListSortDropDown.SortWordDictionary`: the wording of the
    /// dropdown's row for this type.
    pub fn wording(self) -> &'static str {
        match self {
            Self::Default => "WORD_DEFAULT",
            Self::Sequence => "WORD_CATEGORY_ORDER",
            Self::Get => "WORD_GET_ORDER",
            Self::Count => "WORD_POSSESSION_COUNT_ORDER",
            Self::AlphabeticalOrder => "WORD_ALPHABETICAL_ORDER",
            Self::LayoutCost => "WORD_MYSEKAI_LAYOUT_COST_ORDER",
        }
    }

    /// The type of `value`, when a list offers one.
    pub fn from_value(value: i32) -> Option<Self> {
        [
            Self::Default,
            Self::Sequence,
            Self::Get,
            Self::Count,
            Self::AlphabeticalOrder,
            Self::LayoutCost,
        ]
        .into_iter()
        .find(|sort| sort.value() == value)
    }
}

/// `SortOrderBy`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum SortOrder {
    #[default]
    Asc,
    Desc,
}

impl SortOrder {
    /// `UIPartsSortOrder.OnClickSortOrderBy`.
    pub fn reversed(self) -> Self {
        match self {
            Self::Asc => Self::Desc,
            Self::Desc => Self::Asc,
        }
    }
}

/// `FixtureInventoryContentList.Initialize`'s `specifySortTypes`.
pub const FIXTURE_SORTS: [SortType; 5] = [
    SortType::Get,
    SortType::Sequence,
    SortType::Count,
    SortType::AlphabeticalOrder,
    SortType::LayoutCost,
];

/// `UserResourceContentList.Initialize`'s `specifySortTypes`.
pub const RESOURCE_SORTS: [SortType; 4] = [
    SortType::Get,
    SortType::Sequence,
    SortType::Count,
    SortType::AlphabeticalOrder,
];

/// The sort types the list of a content type offers.
pub fn sorts(kind: ContentListType) -> &'static [SortType] {
    match kind {
        ContentListType::Fixture => &FIXTURE_SORTS,
        ContentListType::Material | ContentListType::Item | ContentListType::Tool => &RESOURCE_SORTS,
    }
}

// ---------------------------------------------------------------------------
// The keys
// ---------------------------------------------------------------------------

/// One row's sort key; rows of one list always carry one shape.
#[derive(Clone, PartialEq, Eq)]
enum Key {
    /// `Nullable<int>`, where a missing value sorts below every value.
    Optional(i32),
    Long(i64),
    Int(i32),
    Text(String),
}

/// `OrderBy`'s comparison of two keys of one shape.
fn compare(left: &Key, right: &Key) -> std::cmp::Ordering {
    match (left, right) {
        (Key::Optional(left), Key::Optional(right)) => left.cmp(right),
        (Key::Long(left), Key::Long(right)) => left.cmp(right),
        (Key::Int(left), Key::Int(right)) => left.cmp(right),
        (Key::Text(left), Key::Text(right)) => left.cmp(right),
        _ => std::cmp::Ordering::Equal,
    }
}

/// `OrderBy<TKey>`'s stable order.
fn ordered(mut rows: Vec<Row>, key: impl Fn(&Row) -> Key) -> Vec<Row> {
    rows.sort_by(|left, right| compare(&key(left), &key(right)));
    rows
}

/// The fixture list's sort key (`<SortCellDataList>b__47_0`,
/// `<SortCellDataList>b__47_1`, `GetSortTypeGetOrderValue`,
/// `GetSortTypeCountOrderValue`, `GetSortTypeLayoutCostOrderValue`).
fn fixture_key(row: &Row, sort: SortType) -> Key {
    let master = row.master.as_ref();
    match sort {
        // The fixture master's `seq` (`Nullable<int>`; a row without one is
        // below every row that has one).
        SortType::Default | SortType::Sequence => {
            Key::Optional(master.and_then(|m| m.seq).unwrap_or(i32::MIN))
        }
        // The owned row's `lastObtainedAt`, `long.MaxValue` without one.
        SortType::Get => Key::Long(row.last_obtained_at.unwrap_or(i64::MAX)),
        // The owned row's quantity, `int.MaxValue` without one.
        SortType::Count => Key::Int(row.quantity.max(0)),
        // The fixture master's `pronunciation`.
        SortType::AlphabeticalOrder => {
            Key::Text(master.map(|m| m.pronunciation.clone()).unwrap_or_default())
        }
        // The IPC layout's `PutCost` (0 without one).
        SortType::LayoutCost => Key::Int(master.map_or(0, |m| m.layout_cost)),
    }
}

/// The default order value of the resource lists
/// (`UserResourceContentList.GetSortTypeDefaultOrderValue`): the row's master
/// `seq`, `int.MaxValue` without one.
fn resource_sequence(row: &Row) -> i32 {
    row.master.as_ref().and_then(|m| m.seq).unwrap_or(i32::MAX)
}

/// The resource list's sort key (`GetSortTypeGetOrderValue`,
/// `GetSortTypeCountOrderValue`, `GetSortTypeAlphabeticalOrderOrderValue`).
fn resource_key(row: &Row, sort: SortType) -> Key {
    match sort {
        // The owned row's `lastObtainedAt`, `long.MaxValue` without one.
        SortType::Get => Key::Long(row.last_obtained_at.unwrap_or(i64::MAX)),
        SortType::Count => Key::Int(row.quantity.max(0)),
        SortType::AlphabeticalOrder => Key::Text(
            row.master
                .as_ref()
                .map(|m| m.pronunciation.clone())
                .unwrap_or_default(),
        ),
        SortType::Default | SortType::Sequence | SortType::LayoutCost => {
            Key::Int(resource_sequence(row))
        }
    }
}

/// The rows in the order the list draws them: the sort's ascending order
/// (the resource lists tie on the master sequence), reversed for
/// `SortOrder.Desc`.
pub fn sorted(
    kind: ContentListType,
    rows: &[Row],
    sort: SortType,
    order: SortOrder,
) -> Vec<Row> {
    let mut out = match kind {
        ContentListType::Fixture => ordered(rows.to_vec(), |row| fixture_key(row, sort)),
        ContentListType::Material | ContentListType::Item | ContentListType::Tool => {
            let sequence = ordered(rows.to_vec(), resource_sequence_key);
            ordered(sequence, |row| resource_key(row, sort))
        }
    };
    if order == SortOrder::Desc {
        out.reverse();
    }
    out
}

fn resource_sequence_key(row: &Row) -> Key {
    Key::Int(resource_sequence(row))
}

// ---------------------------------------------------------------------------
// The tabs
// ---------------------------------------------------------------------------

/// One tab of a list: its cell ids (genre ids, or material type ids), its
/// label and its sub tabs.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Tab {
    /// A parent tab's main genre ids, or a sub tab's one sub genre id.
    pub ids: Vec<i32>,
    /// For a material type tab, the row's `mysekaiMaterialType`.
    pub material_type: Option<String>,
    /// The tab's label: a genre's or a material type's name.
    pub name: String,
    /// The sub genre tabs of a parent tab (`GenerateFixtureSubTabCellDataList`).
    pub subs: Vec<Tab>,
}

impl Tab {
    /// `CheckCategoryFilter` and `CheckParentCategoryFilter`: whether a
    /// fixture row of the main genre `main` and the sub genre `sub` passes
    /// the list's tab `index`.
    ///
    /// The all tab (index 0) passes every row (`CheckAllCategoryFilter`);
    /// any other tab passes the rows whose sub genre one of its sub tabs
    /// names (`<CheckCategoryFilter>b__43_0`: ids below 2 are left out).
    pub fn passes(tabs: &[Tab], index: usize, main: i32, sub: i32) -> bool {
        let _ = main;
        let Some(tab) = tabs.get(index) else {
            return true;
        };
        if index == 0 {
            return true;
        }
        tab.subs
            .iter()
            .any(|sub_tab| sub_tab.ids.iter().any(|id| *id >= 2 && *id == sub))
    }
}

/// `MysekaiFixtureTabViewType.edit_layout_home`: the view type of the
/// inventory's fixture tab rows.
const INVENTORY_FIXTURE_VIEW: &str = "edit_layout_home";

/// A master row's id, or 0.
fn id_of(row: &Value) -> i32 {
    row["id"]
        .as_i64()
        .and_then(|id| i32::try_from(id).ok())
        .unwrap_or_default()
}

/// A master row's name, or its id.
fn name_of(row: &Value) -> String {
    row["name"]
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| id_of(row).to_string())
}

/// `<CheckAllCategoryFilter>b__46_2`: the all tab's ids are every main genre
/// but the reserved 1 and 11.
pub fn all_genre_ids(main_genres: &[Value]) -> Vec<i32> {
    main_genres
        .iter()
        .map(id_of)
        .filter(|id| *id != 1 && *id != 11)
        .collect()
}

/// `ContentListBase.GenerateFixtureTabCellDataList` and
/// `GenerateFixtureSubTabCellDataList`: the fixture tabs in `seq` order.
pub fn fixture_tabs(
    tab_views: &[Value],
    main_genres: &[Value],
    sub_genres: &[Value],
) -> Vec<Tab> {
    let name = |rows: &[Value], id: i32| {
        rows.iter()
            .find(|row| id_of(row) == id)
            .map_or_else(|| id.to_string(), name_of)
    };
    let int = |row: &Value, key: &str| {
        row[key]
            .as_i64()
            .and_then(|value| i32::try_from(value).ok())
            .unwrap_or_default()
    };
    let mut parents: Vec<&Value> = tab_views
        .iter()
        .filter(|row| row["mysekaiFixtureTabViewType"].as_str() == Some(INVENTORY_FIXTURE_VIEW))
        .collect();
    parents.sort_by_key(|row| int(row, "seq"));
    let mut out = Vec::with_capacity(parents.len());
    for (index, parent) in parents.iter().enumerate() {
        let seq = int(parent, "seq");
        let main = int(parent, "mysekaiFixtureMainGenreId");
        let ids = if index == 0 {
            // The first tab of the list is its all tab.
            all_genre_ids(main_genres)
        } else {
            vec![main]
        };
        let mut subs: Vec<Tab> = tab_views
            .iter()
            .filter(|row| {
                row["mysekaiFixtureTabViewType"].as_str() == Some(INVENTORY_FIXTURE_VIEW)
                    && int(row, "mysekaiFixtureTabParentSeq") == seq
            })
            .map(|row| {
                let sub = int(row, "mysekaiFixtureSubGenreId");
                Tab {
                    ids: vec![sub],
                    material_type: None,
                    name: name(sub_genres, sub),
                    subs: Vec::new(),
                }
            })
            .collect();
        if index == 0 {
            subs.sort_by_key(|tab| tab.ids.first().copied().unwrap_or_default());
        }
        out.push(Tab {
            ids,
            material_type: None,
            name: if index == 0 {
                // The all tab has no genre row of its own.
                String::new()
            } else {
                name(main_genres, main)
            },
            subs,
        });
    }
    out
}

/// `ContentListBase.GenerateMaterialTabCellDataList`: one tab per material
/// type (`MasterMysekaiMaterialTab`), in master order.
pub fn material_tabs(material_tabs: &[Value]) -> Vec<Tab> {
    material_tabs
        .iter()
        .map(|row| Tab {
            ids: vec![id_of(row)],
            material_type: row["mysekaiMaterialType"].as_str().map(str::to_owned),
            name: name_of(row),
            subs: Vec::new(),
        })
        .collect()
}

/// `CheckCategoryFilter`: whether a fixture row passes the list's tab
/// `tab` (an index into its tabs).
pub fn passes_genre(tabs: &[Tab], tab: usize, row: &Row) -> bool {
    let Some(master) = row.master.as_ref() else {
        return false;
    };
    Tab::passes(tabs, tab, master.main_genre, master.sub_genre)
}

/// `UserResourceContentList.CheckFilterMaterial`: the selected material type
/// tab passes the materials of its own `mysekaiMaterialType`; the first tab
/// also passes the types no tab lists.
pub fn passes_material_type(tab: usize, selected: Option<&str>, row_type: Option<&str>) -> bool {
    match (selected, row_type) {
        (Some(selected), Some(row_type)) => selected == row_type,
        (None, _) => tab == 0,
        (Some(_), None) => false,
    }
}

// ---------------------------------------------------------------------------
// The filter document
// ---------------------------------------------------------------------------

/// `MysekaiFixtureFilteredData`, as the client stores it: the search word,
/// the tag ids, the "not created" filter and the series order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FixtureFilter {
    pub word: String,
    pub tag_ids: Vec<i32>,
    pub is_not_created_filter: bool,
    pub series_sort_order: SortOrder,
}

impl FixtureFilter {
    /// `MysekaiFixtureFilteredData.IsFilteredTag`.
    pub fn is_filtered_tag(&self) -> bool {
        !self.tag_ids.is_empty()
    }

    /// `MysekaiFixtureFilteredData.IsFiltered`.
    pub fn is_filtered(&self) -> bool {
        self.is_filtered_tag() || !self.word.is_empty() || self.is_not_created_filter
    }

    /// One stored row (`word`, `tagIds`, `isNotCreatedFilter`,
    /// `seriesSortOrderBy`).
    pub fn read(value: &Value) -> Self {
        let order = value["seriesSortOrderBy"]
            .as_i64()
            .filter(|order| *order == 1)
            .map_or(SortOrder::Asc, |_| SortOrder::Desc);
        Self {
            word: value["word"].as_str().unwrap_or_default().to_owned(),
            tag_ids: value["tagIds"]
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(Value::as_i64)
                        .filter_map(|id| i32::try_from(id).ok())
                        .collect()
                })
                .unwrap_or_default(),
            is_not_created_filter: value["isNotCreatedFilter"].as_bool().unwrap_or(false),
            series_sort_order: order,
        }
    }
}

/// The search target of the inventory's filter
/// (`MysekaiFixtureFilteredData.SearchTargetType.Inventory`).
pub const SEARCH_INVENTORY: usize = 1;
