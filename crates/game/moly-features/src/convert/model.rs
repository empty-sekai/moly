//! The convert screen's model: the slots as the machine shows them, the items
//! the select list offers, and the values a start or a receive asks for.
//!
//! Every rule is named after the source method it follows:
//!
//! - A slot (`MysekaiConvertFurnitureSlotModel`) is locked, ready, processing
//!   or complete (`MysekaiConvertSlotState`), in that order. A row the server
//!   sent is read through its own status and clock (`SetupSlotState`): the
//!   converts completed are the elapsed time over the slot's per-convert
//!   minutes (`GetObtainResourcesDictionary`), at most the row's count.
//! - An item (`MysekaiConvertItemModelBase` and its three subclasses) is
//!   released when the machine level reaches its `requiredConvertFixtureLevel`
//!   (`IsReleased`), converts at most `onceConvertLimit` a request, and at
//!   most what its `consumeResourceBoxId` consumption box leaves
//!   (`OnceConvertibleCount`); a row with a `convertLimit` is also capped by
//!   the converts since its last refresh (`GetConvertibleCount`).
//! - The pass shortens a convert's minutes (`GetOnceConvertMinutes`), a
//!   weekly or monthly row resets when its refresh has passed
//!   (`HasCrossedWeeklyRefresh`, `HasCrossedMonthlyRefresh`), and a skip costs
//!   the remaining minutes rounded up times the config
//!   (`SetupConsumedJewelQuantityForShorten`).

use std::collections::BTreeMap;

use moly_masters::clock::{next_month_first, CalendarRefresh};
use moly_masters::convert::{
    BoxDetail, ConvertItem, ConvertMasters, ObtainType, RecordCategory, RefreshCycle,
    BOX_CONSUMPTION, BOX_OBTAIN,
};
use moly_protocol::rows::{
    ConvertItemHistory, ConvertSlot, UserMysekaiConvertItem, UserResource,
    CONVERT_SLOT_COMPLETE,
};

/// `MysekaiConvertSlotState`, in the order the source's enum declares.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum SlotState {
    #[default]
    Locked,
    Ready,
    Processing,
    Complete,
}

impl SlotState {
    /// `IsProcessing`: processing or complete.
    pub fn is_processing(self) -> bool {
        matches!(self, Self::Processing | Self::Complete)
    }
}

/// `MysekaiUtility.HasMysekaiColorfulPass` and
/// `GetRemainingTimeValidMysekaiColorfulPass`, from the user's pass row.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PassState {
    /// `UserMysekaiColorfulPass.expiredAt` (epoch milliseconds; 0: no pass).
    pub expired_at: i64,
}

impl PassState {
    /// `HasMysekaiColorfulPass`: a pass whose expiry is after the clock.
    pub fn is_valid(&self, now: i64) -> bool {
        self.expired_at > 0 && now < self.expired_at
    }

    /// `GetRemainingTimeValidMysekaiColorfulPass`, never below zero.
    pub fn remaining_ms(&self, now: i64) -> i64 {
        if self.is_valid(now) {
            (self.expired_at - now).max(0)
        } else {
            0
        }
    }
}

/// The user side the model reads: the slot rows, the items' histories, the
/// item rows the server carries, the pass, the machine's level and the
/// resources the consumption boxes are paid from.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ConvertUserState {
    pub slots: Vec<ConvertSlot>,
    pub histories: Vec<ConvertItemHistory>,
    /// `mysekaiConvertItems` as the server carries them (see
    /// `userdata::convert::ClientConvertSlots`).
    pub items: Vec<UserMysekaiConvertItem>,
    pub pass: PassState,
    /// `UserMysekaiGamedata.mysekaiConvertFixtureLevel`.
    pub level: Option<i32>,
    /// The resources a consumption box is paid from (`GetHaveQuantity`).
    pub holdings: Holdings,
}

impl ConvertUserState {
    pub fn slot(&self, slot_id: i32) -> Option<&ConvertSlot> {
        self.slots.iter().find(|slot| slot.slot_id == slot_id)
    }

    pub fn history(&self, item_id: i32) -> Option<&ConvertItemHistory> {
        self.histories
            .iter()
            .find(|row| row.mysekai_convert_item_id == item_id)
    }

    pub fn server_item(&self, item_id: i32) -> Option<&UserMysekaiConvertItem> {
        self.items
            .iter()
            .find(|row| row.mysekai_convert_item_id == item_id)
    }

    /// The machine's level (0 when the server has not seated one).
    pub fn level(&self) -> i32 {
        self.level.unwrap_or(0)
    }
}

/// The resources the client keeps a count of (the types `GetHaveQuantity`
/// answers from a user table); any other type is not limited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Holdings {
    pub materials: BTreeMap<i32, i32>,
    pub items: BTreeMap<i32, i32>,
    pub fixtures: BTreeMap<i32, i32>,
}

impl Holdings {
    /// `UserDataUtility.GetHaveQuantity(resource)`: the held quantity, or
    /// `i32::MAX` for a type without a table.
    pub fn held(&self, resource_type: &str, resource_id: i32) -> i32 {
        match resource_type {
            "mysekai_material" => self.materials.get(&resource_id).copied().unwrap_or(0),
            "mysekai_item" => self.items.get(&resource_id).copied().unwrap_or(0),
            "mysekai_fixture" => self.fixtures.get(&resource_id).copied().unwrap_or(0),
            _ => i32::MAX,
        }
    }
}

/// What every item rule reads: the masters, the user data and the clock.
#[derive(Clone, Copy)]
pub struct ItemContext<'a> {
    pub masters: &'a ConvertMasters,
    pub user: &'a ConvertUserState,
    /// `TimeUtility.GetCurrentTimestamp` (epoch milliseconds).
    pub now: i64,
}

impl<'a> ItemContext<'a> {
    pub fn machine(&self) -> Machine<'a> {
        Machine::new(self.masters)
    }

    /// The pass runs at the clock.
    pub fn pass_valid(&self) -> bool {
        self.user.pass.is_valid(self.now)
    }

    /// The local offset the calendar rules read the clock in.
    pub fn utc_offset_ms(&self) -> i64 {
        crate::userdata::clock::device_utc_offset_ms(self.now)
    }
}

// ---------------------------------------------------------------------------
// The machine
// ---------------------------------------------------------------------------

/// The config used when the `configs` table is not in (one crystal a minute).
const DEFAULT_JEWEL_PER_MINUTE: i32 = 1;

/// The machine's slot numbers, levels, pass rate and skip price.
#[derive(Clone, Copy)]
pub struct Machine<'a> {
    masters: &'a ConvertMasters,
}

impl<'a> Machine<'a> {
    pub fn new(masters: &'a ConvertMasters) -> Self {
        Self { masters }
    }

    /// `MysekaiConvertUtility.IsExtendedSlotByMysekaiColorfulPass(slotNo)`: a
    /// slot number above the machine's plain count.
    pub fn is_pass_slot(&self, slot_id: i32) -> bool {
        self.masters
            .slots
            .is_some_and(|(plain, _)| slot_id >= plain)
    }

    /// `CreateConvertFurnitureSlotModels`: how many slots the machine has with
    /// or without the pass (the plain number, or the pass-extended one).
    pub fn slot_count(&self, has_pass: bool) -> i32 {
        match self.masters.slots {
            Some((plain, extended)) => {
                if has_pass {
                    extended
                } else {
                    plain
                }
            }
            None => 0,
        }
    }

    /// `GetCurrentConvertFixtureLevel`.
    pub fn level(&self, user: &ConvertUserState) -> i32 {
        user.level()
    }

    /// `IsReleasedShorten`: the level the `convert_time_shorten` release row
    /// names is reached.
    pub fn is_released_shorten(&self, level: i32) -> bool {
        is_released_shorten(self.masters, level)
    }

    /// The level the skip is released at (0 when no row names one).
    pub fn released_shorten_level(&self) -> i32 {
        released_shorten_level(self.masters)
    }

    /// `MysekaiConvertUtility.GetConvertTimeDecreaseRate`
    /// (`colorfulPasses.mysekaiConvertFixtureConvertMinutesDecreaseRate`; 1
    /// when the table is not in).
    pub fn convert_time_decrease_rate(&self) -> f32 {
        self.masters.pass_rate.unwrap_or(1.0)
    }

    /// `GetOnceConvertMinutes`: the master minutes, times the pass's rate
    /// rounded down while the pass runs.
    pub fn once_convert_minutes(&self, master_minutes: i32, pass_valid: bool) -> i32 {
        if !pass_valid {
            return master_minutes;
        }
        let rate = self.convert_time_decrease_rate();
        if rate.is_finite() && (0.0..1.0).contains(&rate) {
            (master_minutes as f32 * rate).floor() as i32
        } else {
            master_minutes
        }
    }

    /// `SetupConsumedJewelQuantityForShorten`: the remaining minutes, rounded
    /// up, times `mysekai_convert_time_shorten_consume_jewel_per_minute`.
    pub fn skip_price(&self, remaining_ms: i64) -> i32 {
        let per_minute = self
            .masters
            .shorten_jewel_per_minute
            .unwrap_or(DEFAULT_JEWEL_PER_MINUTE);
        let minutes = (remaining_ms.max(0) as f64 / 60_000.0).ceil() as i64;
        minutes
            .saturating_mul(i64::from(per_minute))
            .clamp(0, i64::from(i32::MAX)) as i32
    }

    /// The `mysekai_convert_consumption` box an item spends, every detail
    /// (`ResourceBoxUtility.GetResources(consumeBox, 1)`).
    pub fn consumption<'b>(&self, item: &ConvertItem) -> Vec<&'b BoxDetail>
    where
        'a: 'b,
    {
        self.masters
            .box_of(BOX_CONSUMPTION, item.consume_box_id)
            .map(|box_| box_.details.iter().collect())
            .unwrap_or_default()
    }

    /// The `mysekai_convert_obtain` box a `resource_box` item draws, every
    /// detail.
    pub fn obtain<'b>(&self, item: &ConvertItem) -> Vec<&'b BoxDetail>
    where
        'a: 'b,
    {
        if item.obtain_type != ObtainType::ResourceBox {
            return Vec::new();
        }
        self.masters
            .box_of(BOX_OBTAIN, item.external_id)
            .map(|box_| box_.details.iter().collect())
            .unwrap_or_default()
    }
}

/// The level the `convert_time_shorten` release row names (0 when the table
/// has no such row).
pub fn released_shorten_level(masters: &ConvertMasters) -> i32 {
    masters
        .level_releases
        .as_ref()
        .and_then(|rows| {
            rows.iter()
                .find(|row| row.kind == "convert_time_shorten")
                .map(|row| row.level)
        })
        .unwrap_or(0)
}

/// `IsReleasedShorten` at a level.
pub fn is_released_shorten(masters: &ConvertMasters, level: i32) -> bool {
    masters.level_releases.as_ref().is_some_and(|rows| {
        rows.iter().any(|row| row.kind == "convert_time_shorten" && row.level <= level)
    })
}

// ---------------------------------------------------------------------------
// The item models
// ---------------------------------------------------------------------------

/// `IMysekaiConvertItemModel`: one item the select list offers, its master
/// row and the server's item row.
#[derive(Clone, Copy, Debug)]
pub enum ItemModel<'a> {
    /// `MysekaiConvertNormalItemModel`.
    Normal {
        master: &'a ConvertItem,
        server: &'a UserMysekaiConvertItem,
    },
    /// `MysekaiConvertBlueprintItemModel`.
    Blueprint {
        master: &'a ConvertItem,
        server: &'a UserMysekaiConvertItem,
        obtain: BlueprintKind,
        /// `ChoiceId`: the choice row this model was made for (0 for the
        /// one model of a `random` or `white_blueprint` obtain row).
        choice: i32,
    },
    /// `MysekaiConvertRecordItemModel`.
    Record {
        master: &'a ConvertItem,
        server: &'a UserMysekaiConvertItem,
        obtain: RecordKind,
        /// `ChoiceId`: the record category's id (0 for the one model of a
        /// `random` obtain row).
        choice: i32,
    },
}

/// `MysekaiConvertItemObtainMysekaiBlueprintType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum BlueprintKind {
    /// `random`.
    Random,
    /// `mysekai_fixture_main_genre_choice`.
    FixtureMainGenreChoice,
    /// `white_blueprint`.
    White,
}

impl BlueprintKind {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "random" => Some(Self::Random),
            "mysekai_fixture_main_genre_choice" => Some(Self::FixtureMainGenreChoice),
            "white_blueprint" => Some(Self::White),
            _ => None,
        }
    }

    /// `IsRandomBox`.
    pub fn is_random_box(self) -> bool {
        matches!(self, Self::Random | Self::FixtureMainGenreChoice)
    }

    /// `IsWhiteBlueprint`.
    pub fn is_white(self) -> bool {
        self == Self::White
    }

    /// `GetItemName`'s wording key (the white blueprint is an item name).
    pub fn name_key(self) -> Option<&'static str> {
        match self {
            Self::Random => Some("WORD_CONVERT_ITEM_SURPLUS_BLUEPRINT_RANDOM"),
            Self::FixtureMainGenreChoice => {
                Some("WORD_CONVERT_ITEM_SURPLUS_BLUEPRINT_FIXTURE_MAIN_GENRE_CHOICE")
            }
            Self::White => None,
        }
    }

    /// `GetObtainedItemDescription`'s wording key (the white blueprint is an
    /// item description).
    pub fn description_key(self) -> Option<&'static str> {
        match self {
            Self::Random => Some("MSG_CONVERT_BLUEPRINT_RANDOM_BOX_DESCRIPTION"),
            Self::FixtureMainGenreChoice => {
                Some("MSG_CONVERT_BLUEPRINT_RANDOM_BOX_CHOICE_DESCRIPTION")
            }
            Self::White => None,
        }
    }
}

/// `MysekaiConvertItemObtainMysekaiMusicRecordType`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum RecordKind {
    /// `random`.
    Random,
    /// `mysekai_music_record_category_choice`.
    CategoryChoice,
}

impl RecordKind {
    pub fn parse(word: &str) -> Option<Self> {
        match word {
            "random" => Some(Self::Random),
            "mysekai_music_record_category_choice" => Some(Self::CategoryChoice),
            _ => None,
        }
    }

    /// `GetItemName`'s wording key.
    pub fn name_key(self) -> &'static str {
        match self {
            Self::Random => "WORD_CONVERT_ITEM_SURPLUS_RECORD_RANDOM",
            Self::CategoryChoice => "WORD_CONVERT_ITEM_SURPLUS_RECORD_CATEGORY_CHOICE",
        }
    }

    /// `GetObtainedItemDescription`'s wording key.
    pub fn description_key(self) -> &'static str {
        match self {
            Self::Random => "MSG_CONVERT_RECORD_RANDOM_BOX_DESCRIPTION",
            Self::CategoryChoice => "MSG_CONVERT_RECORD_RANDOM_BOX_CHOICE_DESCRIPTION",
        }
    }
}

/// The list [`ItemModel::all`] reads, and the refusal of a row whose choice
/// table the convert master reader does not load (named once by the caller).
#[derive(Clone, Debug, Default)]
pub struct ItemModels<'a> {
    models: Vec<ItemModel<'a>>,
    skipped: Option<&'static str>,
}

impl<'a> ItemModels<'a> {
    /// Why a row of the list has no model, when one has.
    pub fn skipped(&self) -> Option<&'static str> {
        self.skipped
    }

    /// The models, in `CompareTo` order.
    pub fn list(&self) -> &[ItemModel<'a>] {
        &self.models
    }

    /// The models, in `CompareTo` order (the list's own order).
    pub fn iter(&self) -> std::slice::Iter<'_, ItemModel<'a>> {
        self.models.iter()
    }

    /// The models, in `CompareTo` order.
    pub fn into_models(self) -> Vec<ItemModel<'a>> {
        self.models
    }
}

impl<'a> IntoIterator for ItemModels<'a> {
    type Item = ItemModel<'a>;
    type IntoIter = std::vec::IntoIter<ItemModel<'a>>;

    fn into_iter(self) -> Self::IntoIter {
        self.models.into_iter()
    }
}

impl<'a> ItemModel<'a> {
    /// The three kinds of the screen's list (`CreateMysekaiConvertItemModels`):
    /// one model per item row the server carries, matched to its master row,
    /// with the `ChoiceId` its obtain row makes it for (a
    /// `mysekai_music_record_category_choice` record one per valid category, a
    /// `mysekai_fixture_main_genre_choice` blueprint one per choice row, which
    /// this reader does not load: none of those). A row whose master row,
    /// obtain row or type this reader cannot read is left out, and the list is
    /// ordered (`CompareTo`: normal, blueprint, record, then id and choice).
    pub fn all(ctx: ItemContext<'a>) -> ItemModels<'a> {
        let masters = ctx.masters;
        let Some(items) = masters.items.as_ref() else {
            return ItemModels::default();
        };
        let mut models = Vec::new();
        // The record obtain row asks for the categories: without them its
        // `..._choice` row has no model.
        let mut missing_categories = false;
        // A `mysekai_fixture_main_genre_choice` obtain row's choices.
        let mut missing_blueprint_choices = false;
        for server in &ctx.user.items {
            let Some(master) = items.iter().find(|item| item.id == server.mysekai_convert_item_id) else {
                continue;
            };
            match master.obtain_type {
                ObtainType::ResourceBox => models.push(ItemModel::Normal { master, server }),
                ObtainType::Blueprint => {
                    let Some(obtain) = masters
                        .blueprint_obtain(server.external_id)
                        .and_then(|row| BlueprintKind::parse(&row.kind))
                    else {
                        continue;
                    };
                    // `random` and `white_blueprint` make one model with the
                    // default choice.
                    if obtain != BlueprintKind::FixtureMainGenreChoice {
                        models.push(ItemModel::Blueprint {
                            master,
                            server,
                            obtain,
                            choice: 0,
                        });
                        continue;
                    }
                    // The choice obtain's rows are not read by the convert
                    // master reader, so the choice models have no row to
                    // name: the list carries none of them (named once).
                    missing_blueprint_choices = true;
                }
                ObtainType::Record => {
                    let Some(obtain) = masters
                        .record_obtain(server.external_id)
                        .and_then(|row| RecordKind::parse(&row.kind))
                    else {
                        continue;
                    };
                    if obtain == RecordKind::Random {
                        models.push(ItemModel::Record {
                            master,
                            server,
                            obtain,
                            choice: 0,
                        });
                        continue;
                    }
                    // `GetValidMusicRecordCategories`: the `music` track
                    // categories in `seq` order, one model each, its id the
                    // `ChoiceId`.
                    let mut categories: Vec<&RecordCategory> = masters
                        .record_categories
                        .as_ref()
                        .map(|rows| {
                            rows.iter()
                                .filter(|row| row.track_type == "music")
                                .collect()
                        })
                        .unwrap_or_default();
                    categories.sort_by_key(|row| row.seq);
                    if categories.is_empty() {
                        missing_categories = true;
                    }
                    for category in categories {
                        models.push(ItemModel::Record {
                            master,
                            server,
                            obtain,
                            choice: category.id,
                        });
                    }
                }
            }
        }
        let skipped = match (missing_blueprint_choices, missing_categories) {
            (true, _) => Some(
                "a mysekai_fixture_main_genre_choice blueprint lists no model: the convert master reader does not load mysekaiConvertItemObtainMysekaiBlueprintChoices",
            ),
            (false, true) => Some(
                "a mysekai_music_record_category_choice record lists no model: mysekaiMusicRecordCategories has no music row",
            ),
            (false, false) => None,
        };
        // `list.OrderBy((IMysekaiConvertItemModel x) => x)`
        // (`MysekaiConvertFurnitureItemSelectView.GetValidConvertItemModels`):
        // LINQ's `OrderBy` sorts stably, so models `CompareTo` calls equal
        // (one item's category-choice models) keep the order they were made
        // in, here the obtain row's `orderby x.seq`.
        models.sort_by(|a, b| a.compare(b, ctx));
        ItemModels { models, skipped }
    }

    pub fn master(&self) -> &'a ConvertItem {
        match self {
            Self::Normal { master, .. } | Self::Blueprint { master, .. } => master,
            Self::Record { master, .. } => master,
        }
    }

    pub fn server(&self) -> &'a UserMysekaiConvertItem {
        match self {
            Self::Normal { server, .. } | Self::Blueprint { server, .. } => server,
            Self::Record { server, .. } => server,
        }
    }

    pub fn id(&self) -> i32 {
        self.master().id
    }

    /// `ChoiceId`: the obtain row's choice this model was made for (`IsEqual`
    /// and `CompareTo` read it); 0 for a normal item and for a blueprint or
    /// record row that has one model.
    pub fn choice(&self) -> i32 {
        match self {
            Self::Normal { .. } => 0,
            Self::Blueprint { choice, .. } | Self::Record { choice, .. } => *choice,
        }
    }

    /// `EnoughConsumedResources`: `ConsumedUserResourceModelList?.All(x =>
    /// x.IsEnoughResource) ?? false`, and `IsEnoughResource` is
    /// `quantity <= PossessionQuantity` (`GetHaveQuantity`) per detail at one
    /// convert. An item whose consumption box is not read has no list and so
    /// reads false, as the source's `?? false` does.
    pub fn enough_consumed_resources(&self, ctx: ItemContext<'a>) -> bool {
        let details = ctx.machine().consumption(self.master());
        !details.is_empty()
            && details.iter().all(|detail| {
                ctx.user.holdings.held(&detail.resource_type, detail.resource_id) >= detail.quantity
            })
    }

    /// `IComparable<IMysekaiConvertItemModel>.CompareTo`
    /// (`MysekaiConvertItemModelBase`) for the list's `OrderBy(x => x)`
    /// (`MysekaiConvertFurnitureItemSelectView.GetValidConvertItemModels`):
    /// four steps, and `Id` is the last. `ChoiceId` is not one of them (only
    /// `Equals` reads it, and the sort never calls it).
    ///
    /// Written out of the decompiled method's labels:
    /// - `IL_00bd` (`return 1`) is "I come after";
    /// - `IL_010b` (`return -1`) is "I come before";
    /// - `IL_000f` is the step from `ConvertibleCount` to `IsReleased`;
    /// - `IL_005b` is the step from `IsReleased` to `EnoughConsumedResources`.
    ///
    /// Step by step, where `HasConvertibleCount` is `ConvertibleCount >= 1`
    /// (`IL_00bd` from a model whose `ConvertibleCount <= 0` when `other` has
    /// one, and `IL_010b` from one that has it when `other`'s is `<= 0`),
    /// then `IsReleased` and then `EnoughConsumedResources`, each by the same
    /// "I lack it after, I have it before"; equal by all of them, `Id`.
    /// A model's own half of a step is decided before `other`'s is read, as
    /// the `goto`s do.
    pub fn compare(&self, other: &Self, ctx: ItemContext<'a>) -> std::cmp::Ordering {
        use std::cmp::Ordering;
        // `ConvertibleCount` then `HasConvertibleCount`, then `IsReleased`,
        // then `EnoughConsumedResources`: each is "true comes first", so each
        // compares the two flags reversed.
        let steps = [
            (
                self.has_convertible_count(ctx),
                other.has_convertible_count(ctx),
            ),
            (
                self.is_released(ctx.machine().level(ctx.user)),
                other.is_released(ctx.machine().level(ctx.user)),
            ),
            (
                self.enough_consumed_resources(ctx),
                other.enough_consumed_resources(ctx),
            ),
        ];
        for (mine, theirs) in steps {
            if mine != theirs {
                return if mine {
                    Ordering::Less
                } else {
                    Ordering::Greater
                };
            }
        }
        self.id().cmp(&other.id())
    }

    /// `IsReleased`: the machine level reaches the item's requirement.
    pub fn is_released(&self, level: i32) -> bool {
        self.master().required_level <= level
    }

    /// `IsLimited`: a stated window.
    pub fn is_limited(&self) -> bool {
        let master = self.master();
        master.start_at > 0 || master.end_at != 0
    }

    /// `IsResetItem`: a weekly or monthly row.
    pub fn is_reset_item(&self) -> bool {
        self.master().refresh != RefreshCycle::None
    }

    /// `IsDisplayable`: within its window at the clock (no window: always).
    pub fn is_displayable(&self, now: i64) -> bool {
        let master = self.master();
        if master.start_at == 0 && master.end_at == 0 {
            return true;
        }
        if master.end_at == 0 {
            master.start_at <= now
        } else {
            master.start_at <= now && now < master.end_at
        }
    }

    /// `IsMatch(tabId)`: the item's tab row.
    pub fn is_match(&self, tab_id: i32) -> bool {
        self.master().tab_id == tab_id
    }

    /// `ConvertMinutes` for `convert_count` converts: the pass-shortened
    /// minutes of one convert times the count.
    pub fn convert_minutes(&self, ctx: ItemContext<'a>, convert_count: i32) -> i32 {
        let each = ctx
            .machine()
            .once_convert_minutes(self.master().convert_minutes, ctx.pass_valid());
        each.saturating_mul(convert_count.max(0))
    }

    /// `ConvertibleCount`: the limit less the converts since the row's last
    /// refresh (`i32::MAX` when the row has no limit).
    pub fn convertible_count(&self, ctx: ItemContext<'a>) -> i32 {
        let master = self.master();
        if master.convert_limit <= 0 {
            return i32::MAX;
        }
        let used = history_converts(
            master.refresh,
            ctx.masters.calendar.as_ref(),
            ctx.user.history(master.id),
            ctx.now,
            ctx.utc_offset_ms(),
        );
        (master.convert_limit - used).max(0)
    }

    /// `HasConvertibleCount`.
    pub fn has_convertible_count(&self, ctx: ItemContext<'a>) -> bool {
        self.convertible_count(ctx) >= 1
    }

    /// How many converts the consumption box pays for (the fewest whole
    /// converts any detail allows; a detail type without a user table does
    /// not limit).
    pub fn affordable_converts(&self, ctx: ItemContext<'a>) -> i32 {
        let mut fewest = i32::MAX;
        for detail in ctx.machine().consumption(self.master()) {
            if detail.quantity <= 0 {
                continue;
            }
            let held = ctx.user.holdings.held(&detail.resource_type, detail.resource_id);
            fewest = fewest.min(held / detail.quantity);
        }
        fewest.max(0)
    }

    /// `OnceConvertibleCount`: at least 1, at most the master's
    /// `onceConvertLimit` and what the consumption box pays for; a limited
    /// row is also at most its remaining converts.
    pub fn once_convertible_count(&self, ctx: ItemContext<'a>) -> i32 {
        let master = self.master();
        let limited = master.convert_limit > 0;
        let left = self.convertible_count(ctx);
        let affordable = self.affordable_converts(ctx);
        let by_resources = if limited { left.min(affordable) } else { affordable };
        let once = if limited {
            master.once_convert_limit.min(by_resources).max(1)
        } else {
            master.once_convert_limit.max(1)
        };
        once
    }

    /// `GetOnceConvertibleCount(slotId)`: a pass slot needs the pass and is
    /// also capped by the pass's remaining minutes.
    pub fn once_convertible_count_in_slot(&self, ctx: ItemContext<'a>, slot_id: i32) -> i32 {
        let machine = ctx.machine();
        let once = self.once_convertible_count(ctx);
        if !machine.is_pass_slot(slot_id) {
            return once;
        }
        if !ctx.pass_valid() {
            return 0;
        }
        let remaining_minutes = ctx.user.pass.remaining_ms(ctx.now) / 60_000;
        let each = i64::from(
            ctx.machine()
                .once_convert_minutes(self.master().convert_minutes, true)
                .max(1),
        );
        let by_pass = (remaining_minutes / each).clamp(0, i64::from(i32::MAX)) as i32;
        by_pass.clamp(1, once)
    }

    /// `IsAllowedToConvertStart`: released, with a convert left, and its
    /// consumption box held.
    pub fn is_allowed_to_convert_start(&self, ctx: ItemContext<'a>) -> bool {
        self.is_released(ctx.machine().level(ctx.user))
            && self.has_convertible_count(ctx)
            && self.affordable_converts(ctx) >= 1
    }

    /// `GetResetAt` (epoch milliseconds; 0 when the row does not reset, or
    /// a weekly row on masters without a calendar).
    pub fn reset_at(&self, ctx: ItemContext<'a>) -> i64 {
        match self.master().refresh {
            RefreshCycle::None => 0,
            RefreshCycle::Weekly => ctx
                .masters
                .calendar
                .map_or(0, |calendar| calendar.next_week_first(ctx.now, ctx.utc_offset_ms())),
            RefreshCycle::Monthly => next_month_first(ctx.now, ctx.utc_offset_ms()),
        }
    }

    /// `GetEndAt` (epoch milliseconds; 0 when unlimited).
    pub fn end_at(&self) -> i64 {
        self.master().end_at.max(0)
    }

    /// The items of one tab the list shows (`GetValidConvertItemModels`):
    /// displayable, of the tab, ordered.
    pub fn of_tab(ctx: ItemContext<'a>, tab_id: i32) -> Vec<ItemModel<'a>> {
        Self::all(ctx)
            .into_iter()
            .filter(|model| model.is_displayable(ctx.now) && model.is_match(tab_id))
            .collect()
    }
}

/// `SetupConvertibleCount`'s history read: the converts a row has had since
/// its refresh (0 after the refresh).
pub fn history_converts(
    refresh: RefreshCycle,
    calendar: Option<&CalendarRefresh>,
    history: Option<&ConvertItemHistory>,
    now: i64,
    offset_ms: i64,
) -> i32 {
    let Some(history) = history else {
        return 0;
    };
    if history.last_convert_start_at <= 0 {
        return 0;
    }
    if has_crossed(refresh, calendar, history.last_convert_start_at, now, offset_ms) {
        return 0;
    }
    history.convert_count.max(0)
}

/// `MysekaiConvertItemModelBase.HasCrossedRefresh`: `HasCrossedWeeklyRefresh`
/// or `HasCrossedMonthlyRefresh` on the master calendar. Masters without
/// `configs` have no calendar, and nothing crosses on it.
pub fn has_crossed(
    refresh: RefreshCycle,
    calendar: Option<&CalendarRefresh>,
    last_start_at: i64,
    now: i64,
    offset_ms: i64,
) -> bool {
    let Some(calendar) = calendar else {
        return false;
    };
    match refresh {
        RefreshCycle::None => false,
        RefreshCycle::Weekly => calendar.has_crossed_weekly(last_start_at, now, offset_ms),
        RefreshCycle::Monthly => calendar.has_crossed_monthly(last_start_at, now, offset_ms),
    }
}

// ---------------------------------------------------------------------------
// The slot model
// ---------------------------------------------------------------------------

/// `MysekaiConvertFurnitureSlotModel`: one machine slot as the screen shows
/// it. The row is the server's; a slot without one is empty.
#[derive(Clone, Debug)]
pub struct SlotModel<'a> {
    row: Option<&'a ConvertSlot>,
    slot_id: i32,
    pass_slot: bool,
    state: SlotState,
    now: i64,
}

impl<'a> SlotModel<'a> {
    /// A slot's model, its state read at the clock (`new` then
    /// `SetupSlotState`).
    pub fn new(ctx: ItemContext<'a>, slot_id: i32, pass_slot: bool) -> Self {
        let mut model = Self {
            row: ctx.user.slot(slot_id),
            slot_id,
            pass_slot,
            state: SlotState::Ready,
            now: ctx.now,
        };
        model.refresh(ctx);
        model
    }

    /// The models of the slot rows the server sent, by slot id
    /// (`SetupConvertState` over the rows).
    pub fn all(ctx: ItemContext<'a>, pass_slot: impl Fn(i32) -> bool) -> Vec<SlotModel<'a>> {
        let mut rows: Vec<&ConvertSlot> = ctx.user.slots.iter().collect();
        rows.sort_by_key(|row| row.slot_id);
        rows.into_iter()
            .map(|row| {
                let mut model = Self {
                    row: Some(row),
                    slot_id: row.slot_id,
                    pass_slot: pass_slot(row.slot_id),
                    state: SlotState::Ready,
                    now: ctx.now,
                };
                model.refresh(ctx);
                model
            })
            .collect()
    }

    pub fn row(&self) -> Option<&'a ConvertSlot> {
        self.row
    }

    pub fn slot_id(&self) -> i32 {
        self.slot_id
    }

    pub fn is_pass_slot(&self) -> bool {
        self.pass_slot
    }

    pub fn state(&self) -> SlotState {
        self.state
    }

    /// `SetupSlotState`: locked (a pass slot without the pass), ready (no row,
    /// or a row with nothing started), processing or complete.
    pub fn refresh(&mut self, ctx: ItemContext<'a>) {
        self.now = ctx.now;
        let Some(row) = self.row else {
            self.state = if self.pass_slot && !ctx.pass_valid() {
                SlotState::Locked
            } else {
                SlotState::Ready
            };
            return;
        };
        self.state = if row.mysekai_convert_slot_status == CONVERT_SLOT_COMPLETE {
            SlotState::Complete
        } else if row.convert_start_at <= 0 || row.convert_count <= 0 {
            if self.pass_slot && !ctx.pass_valid() {
                SlotState::Locked
            } else {
                SlotState::Ready
            }
        } else if converted(row, ctx.now) >= row.convert_count {
            SlotState::Complete
        } else {
            SlotState::Processing
        };
    }

    /// `IsProcessing`.
    pub fn is_processing(&self) -> bool {
        self.state.is_processing()
    }

    /// `ConvertItemModel`: the item the slot converts.
    pub fn item_id(&self) -> Option<i32> {
        let row = self.row?;
        (row.mysekai_convert_item_id != 0).then_some(row.mysekai_convert_item_id)
    }

    /// The draw variant the row holds.
    pub fn choice_id(&self) -> i32 {
        self.row.map_or(0, |row| row.choice_id)
    }

    /// `TotalConvertCount`.
    pub fn total_convert_count(&self) -> i32 {
        self.row.map_or(0, |row| row.convert_count.max(0))
    }

    /// `ReceivedCount` (`receivedProgress`).
    pub fn received_count(&self) -> i32 {
        self.row.map_or(0, |row| row.received_progress.max(0))
    }

    /// `RemainingConvertCount`: the converts not received.
    pub fn remaining_convert_count(&self) -> i32 {
        (self.total_convert_count() - self.received_count()).max(0)
    }

    /// `TotalCompletedConvertCount`: the converts the clock has completed,
    /// at most the row's count.
    pub fn total_completed_convert_count(&self) -> i32 {
        self.row.map_or(0, |row| {
            if self.state == SlotState::Complete {
                row.convert_count.max(0)
            } else {
                converted(row, self.now)
            }
        })
    }

    /// `CurrentCompletedConvertCount`: the completed converts not received.
    pub fn current_completed_convert_count(&self) -> i32 {
        (self.total_completed_convert_count() - self.received_count()).max(0)
    }

    /// `IsReceivable`.
    pub fn is_receivable(&self) -> bool {
        self.current_completed_convert_count() > 0
    }

    /// `RemainingTimeToCompleteConvert`: the time to the row's last convert
    /// (0 when complete or not started).
    pub fn remaining_time_to_complete_convert(&self) -> i64 {
        let Some(row) = self.row else {
            return 0;
        };
        if self.state != SlotState::Processing {
            return 0;
        }
        row.end_at().saturating_sub(self.now).max(0)
    }

    /// `NormalizedProgressTime`: the completed part of the converts not
    /// received yet.
    pub fn normalized_progress_time(&self) -> f32 {
        let Some(row) = self.row else {
            return 0.0;
        };
        if self.state == SlotState::Complete {
            return 1.0;
        }
        let total = self.total_convert_count();
        let received = self.received_count().min(total);
        let segments = total - received;
        if segments <= 0 {
            return 0.0;
        }
        let done = (self.total_completed_convert_count() - received).clamp(0, segments);
        done as f32 / segments as f32
    }

    /// `IsReleasedShorten` at the user's level.
    pub fn is_released_shorten(&self, machine: &Machine<'_>, user: &ConvertUserState) -> bool {
        machine.is_released_shorten(machine.level(user))
    }

    /// `SetupConsumedJewelQuantityForShorten`: the skip's price now.
    pub fn consumed_jewel_quantity_for_shorten(&self, machine: &Machine<'_>) -> i32 {
        machine.skip_price(self.remaining_time_to_complete_convert())
    }

    /// The step a receive of `receive_convert_count` converts would land on,
    /// with the rows it hands over (`UserMysekaiConvertSlot.GetObtainResources`
    /// per step above the received progress). `None` when the row has no rows
    /// for a step it would hand out.
    pub fn obtain_rows(&self, receive_convert_count: i32) -> Option<Vec<UserResource>> {
        let row = self.row?;
        if receive_convert_count < 1 {
            return None;
        }
        let mut rows = Vec::new();
        for progress in (self.received_count() + 1)..=(self.received_count() + receive_convert_count) {
            let step = row.obtain_resources_at(progress);
            if step.is_empty() {
                return None;
            }
            rows.extend(step);
        }
        Some(rows)
    }
}

/// A slot the model reads without borrowing the server's row: the values of
/// [`SlotModel`] at the clock it was read (the screen keeps one per cell to
/// tell a refresh of its numbers from a rebuild of its views).
#[derive(Clone, Debug, PartialEq)]
pub struct OwnedSlotModel {
    slot_id: i32,
    pass_slot: bool,
    state: SlotState,
    total: i32,
    received: i32,
    completed: i32,
    remaining_ms: i64,
    progress: f32,
}

/// The detector (`MysekaiConvertSlotStateDetector`): whether the slot's own
/// numbers changed since the last frame, in which case the cell refreshes
/// them without rebuilding its views (`UpdateSlotView` -> `RefreshView`), or
/// its state changed, in which case the cell rebuilds them (`SetupView`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotChange {
    /// Nothing changed.
    Same,
    /// The counts changed, the state did not (`RefreshView`).
    Numbers,
    /// The state changed (`SetupView`).
    State,
}

impl SlotModel<'_> {
    /// The change between this model and the same slot read before.
    pub fn change_since(&self, before: &OwnedSlotModel) -> SlotChange {
        if self.state != before.state {
            return SlotChange::State;
        }
        if self.total_convert_count() != before.total
            || self.received_count() != before.received
            || self.total_completed_convert_count() != before.completed
            || self.remaining_time_to_complete_convert() != before.remaining_ms
            || (self.normalized_progress_time() - before.progress).abs() > f32::EPSILON
        {
            return SlotChange::Numbers;
        }
        SlotChange::Same
    }

    /// The model's values as the next frame's `before` (no row borrowed).
    pub fn owned(&self) -> OwnedSlotModel {
        OwnedSlotModel {
            slot_id: self.slot_id(),
            pass_slot: self.is_pass_slot(),
            state: self.state(),
            total: self.total_convert_count(),
            received: self.received_count(),
            completed: self.total_completed_convert_count(),
            remaining_ms: self.remaining_time_to_complete_convert(),
            progress: self.normalized_progress_time(),
        }
    }
}

impl OwnedSlotModel {
    pub fn slot_id(&self) -> i32 {
        self.slot_id
    }

    pub fn is_pass_slot(&self) -> bool {
        self.pass_slot
    }

    pub fn state(&self) -> SlotState {
        self.state
    }
}

/// The converts a row's clock has completed (`Math.Min` of the elapsed
/// converts and the row's count).
fn converted(row: &ConvertSlot, now: i64) -> i32 {
    let total = row.convert_count.max(0);
    if row.convert_start_at <= 0 || row.once_convert_minutes < 1 {
        return 0;
    }
    let each = i64::from(row.once_convert_minutes) * 60_000;
    let elapsed = now.saturating_sub(row.convert_start_at);
    (elapsed / each.max(1)).clamp(0, i64::from(total)) as i32
}

// ---------------------------------------------------------------------------
// The select list
// ---------------------------------------------------------------------------

/// The tabs the select list shows (`CreateItemSelectTabListCellDataList`):
/// (id, seq) in `seq` order.
pub fn tabs(masters: &ConvertMasters) -> Vec<(i32, i32)> {
    let mut tabs = masters.tabs.clone().unwrap_or_default();
    tabs.sort_by_key(|(_, seq)| *seq);
    tabs
}

/// The tab the list selects when it opens (the first in `seq` order; 0 when
/// the table is not in).
pub fn first_tab(masters: &ConvertMasters) -> i32 {
    tabs(masters).first().map_or(0, |(id, _)| *id)
}

/// `MysekaiConvertFurnitureItemSelectDetailViewModel`: the detail pane's
/// values for one item.
#[derive(Clone, Debug)]
pub struct DetailModel<'a> {
    pub item: ItemModel<'a>,
    /// `ConvertSlotId`.
    pub slot_id: i32,
    /// `ConvertCount`: the count selector's value.
    pub convert_count: i32,
    /// `ConvertTimeString`'s milliseconds.
    pub convert_ms: i64,
    pub is_allowed_to_convert_start: bool,
}

impl<'a> DetailModel<'a> {
    /// `new …DetailViewModel(slotId, item, onConvertStart)` then
    /// `Setup(convertCount)`: the time is the item's `ConvertMinutes` at the
    /// count, and the start button's enable is `IsAllowedToConvertStart`.
    pub fn new(ctx: ItemContext<'a>, item: ItemModel<'a>, slot_id: i32, convert_count: i32) -> Self {
        let convert_count = convert_count.max(1);
        let convert_ms = i64::from(item.convert_minutes(ctx, convert_count)) * 60_000;
        Self {
            item,
            slot_id,
            convert_count,
            convert_ms,
            is_allowed_to_convert_start: item.is_allowed_to_convert_start(ctx),
        }
    }

    /// `ConsumedResourceCellDataList`: the consumption box's details.
    pub fn consumed_resources<'b>(&self, ctx: ItemContext<'a>) -> Vec<&'b BoxDetail>
    where
        'a: 'b,
    {
        ctx.machine().consumption(self.item.master())
    }

    /// `GetOnceConvertibleCount` at the slot: the count selector's bound.
    pub fn max_convert_count(&self, ctx: ItemContext<'a>) -> i32 {
        self.item.once_convertible_count_in_slot(ctx, self.slot_id)
    }

    /// `ShowsNotEnoughResourceBalloon`: released, a convert left, and the
    /// consumption box not held.
    pub fn shows_not_enough_resource_balloon(&self, ctx: ItemContext<'a>) -> bool {
        self.item.is_released(ctx.machine().level(ctx.user))
            && self.item.has_convertible_count(ctx)
            && self.item.affordable_converts(ctx) < 1
    }

    /// `ShowsShortenByJoinedPassBalloon`: the pass runs.
    pub fn shows_shorten_by_joined_pass_balloon(&self, ctx: ItemContext<'a>) -> bool {
        ctx.pass_valid()
    }

    /// `GetShortenByJoinedPassMessage`'s argument: the rate as a percentage.
    pub fn shorten_by_joined_pass_percent(&self, ctx: ItemContext<'a>) -> f32 {
        ctx.machine().convert_time_decrease_rate() * 100.0
    }
}

/// `CalculateSameResourcesQuantity`: the resources of the same (type, id)
/// added up, in first-seen order.
pub fn same_resources(resources: &[UserResource]) -> Vec<UserResource> {
    let mut out: Vec<UserResource> = Vec::new();
    for resource in resources {
        match out.iter_mut().find(|row| {
            row.resource_type == resource.resource_type && row.resource_id == resource.resource_id
        }) {
            Some(row) => row.quantity = row.quantity.saturating_add(resource.quantity),
            None => out.push(resource.clone()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use moly_masters::convert::{
        BlueprintObtain, LevelRelease, RecordObtain, ResourceBox,
    };
    use moly_protocol::rows::{ConvertObtainResource, CONVERT_SLOT_PROCESSING};

    const NOW: i64 = 1_790_300_000_000;
    const HOUR: i64 = 3_600_000;

    /// 2026-01-31 23:00 UTC and 2026-02-01 00:00 UTC.
    const JANUARY: i64 = 1_769_900_400_000;
    const FEBRUARY: i64 = 1_769_904_000_000;

    fn detail(kind: &str, id: i32, quantity: i32) -> BoxDetail {
        BoxDetail {
            resource_type: kind.to_owned(),
            resource_id: id,
            quantity,
        }
    }

    fn master(
        id: i32,
        tab: i32,
        kind: ObtainType,
        external: i32,
        minutes: i32,
        limit: i32,
        once: i32,
        refresh: RefreshCycle,
    ) -> ConvertItem {
        ConvertItem {
            id,
            tab_id: tab,
            consume_box_id: 10001,
            obtain_type: kind,
            external_id: external,
            required_level: 1,
            convert_minutes: minutes,
            convert_limit: limit,
            once_convert_limit: once,
            refresh,
            start_at: 0,
            end_at: 0,
        }
    }

    fn server(master: &ConvertItem) -> UserMysekaiConvertItem {
        UserMysekaiConvertItem {
            mysekai_convert_item_id: master.id,
            obtain_type: "resource_box".to_owned(),
            external_id: master.external_id,
            required_convert_fixture_level: master.required_level,
            convert_minutes: master.convert_minutes,
            convert_limit: master.convert_limit,
            once_convert_limit: master.once_convert_limit,
            refresh_cycle: "none".to_owned(),
            start_at: 0,
            end_at: 0,
        }
    }

    /// The tables the model reads: three slots (slot 1 is the pass slot), the
    /// shorten released at level 1, an item row per kind, one consumption box
    /// (material 5 ×2 and a coin 1000 a convert), one obtain box of two
    /// details, a pass at 50% and the skip price of 1 crystal a minute.
    fn masters() -> ConvertMasters {
        let items = vec![
            master(3, 1, ObtainType::ResourceBox, 30003, 12, 0, 10, RefreshCycle::None),
            master(14, 1, ObtainType::ResourceBox, 30014, 2, 3, 1, RefreshCycle::Monthly),
            master(22, 3, ObtainType::Blueprint, 1, 120, 0, 10, RefreshCycle::None),
            master(26, 4, ObtainType::Record, 2, 480, 0, 5, RefreshCycle::None),
            master(27, 4, ObtainType::Record, 3, 480, 0, 5, RefreshCycle::None),
        ];
        ConvertMasters {
            items: Some(items),
            tabs: Some(vec![(3, 30), (1, 10), (4, 40)]),
            slots: Some((1, 3)),
            levels: Some(vec![1, 2]),
            level_releases: Some(vec![LevelRelease {
                id: 1,
                level: 1,
                kind: "convert_time_shorten".to_owned(),
            }]),
            boxes: Some(vec![
                ResourceBox {
                    id: 10001,
                    purpose: BOX_CONSUMPTION.to_owned(),
                    name: None,
                    description: None,
                    details: vec![detail("mysekai_material", 5, 2), detail("coin", 0, 1000)],
                },
                ResourceBox {
                    id: 30003,
                    purpose: BOX_OBTAIN.to_owned(),
                    name: Some("box".to_owned()),
                    description: Some("box description".to_owned()),
                    details: vec![detail("mysekai_material", 7, 1), detail("mysekai_item", 9, 2)],
                },
            ]),
            blueprint_obtains: Some(vec![BlueprintObtain {
                id: 1,
                kind: "random".to_owned(),
                quantity: 1,
            }]),
            record_obtains: Some(vec![
                // Item 26 reads the `random` row, item 27 the category choice.
                RecordObtain {
                    id: 2,
                    kind: "random".to_owned(),
                    quantity: 1,
                },
                RecordObtain {
                    id: 3,
                    kind: "mysekai_music_record_category_choice".to_owned(),
                    quantity: 1,
                },
            ]),
            // `GetValidMusicRecordCategories`: the `music` categories in `seq`
            // order, one record model each; a `music_sound_track` category
            // is not one of them.
            record_categories: Some(vec![
                RecordCategory {
                    id: 40,
                    track_type: "music_sound_track".to_owned(),
                    seq: 0,
                },
                RecordCategory {
                    id: 41,
                    track_type: "music".to_owned(),
                    seq: 2,
                },
                RecordCategory {
                    id: 42,
                    track_type: "music".to_owned(),
                    seq: 1,
                },
            ]),
            music_records: Some(vec![]),
            pass_rate: Some(0.5),
            shorten_jewel_per_minute: Some(1),
            calendar: Some(CalendarRefresh { hour: 5, weekday: 1 }),
        }
    }

    fn user(masters: &ConvertMasters) -> ConvertUserState {
        let mut user = ConvertUserState {
            items: masters.items.as_ref().unwrap().iter().map(server).collect(),
            level: Some(1),
            pass: PassState {
                expired_at: NOW + 30 * 24 * HOUR,
            },
            ..ConvertUserState::default()
        };
        user.holdings.materials.insert(5, 20);
        user
    }

    fn context<'a>(masters: &'a ConvertMasters, user: &'a ConvertUserState) -> ItemContext<'a> {
        ItemContext {
            masters,
            user,
            now: NOW,
        }
    }

    fn slot_row(id: i32, status: &str, start: i64, minutes: i32, count: i32, received: i32) -> ConvertSlot {
        ConvertSlot {
            slot_id: id,
            mysekai_convert_item_id: 3,
            choice_id: 0,
            convert_start_at: start,
            once_convert_minutes: minutes,
            convert_count: count,
            received_progress: received,
            mysekai_convert_slot_status: status.to_owned(),
            obtain_resources: Vec::new(),
        }
    }

    /// The list's order is `MysekaiConvertItemModelBase.CompareTo`'s steps:
    /// a convertible count first, then released, then the consumption box
    /// held, then the id; LINQ's stable `OrderBy` keeps models equal there in
    /// the order they were made.
    ///
    /// In the fixture every item spends consumption box 10001 (material 5 ×2,
    /// 20 held; coin 1000) and has a count, so with item 22 needing level 2
    /// at the user's level 1, the rows of (`HasConvertibleCount`,
    /// `IsReleased`) are: 3, 14, 26 and 27 (true, true); 22 (true, false).
    /// The released step puts 22 after the rest; the others are ordered by
    /// id, and 27's two models tie on every step, so the stable sort keeps
    /// the order `CreateMysekaiConvertRecordItemModels` made them in: its
    /// categories by `seq` (42 at seq 1, then 41 at seq 2).
    #[test]
    fn the_items_order_by_convertible_count_release_and_resources_then_id() {
        let mut masters = masters();
        for item in masters.items.as_mut().unwrap() {
            if item.id == 22 {
                item.required_level = 2;
            }
        }
        let user = user(&masters);
        let ctx = context(&masters, &user);
        let models = ItemModel::all(ctx);
        let list = models.list();
        let ids: Vec<i32> = list.iter().map(ItemModel::id).collect();
        assert_eq!(ids, vec![3, 14, 26, 27, 27, 22]);
        let choices: Vec<i32> = list
            .iter()
            .filter(|model| model.id() == 27)
            .map(ItemModel::choice)
            .collect();
        assert_eq!(choices, vec![42, 41]);
    }

    #[test]
    fn a_box_item_spends_two_materials_a_convert_and_the_pass_halves_it() {
        let masters = masters();
        let user = user(&masters);
        let ctx = context(&masters, &user);
        let normal = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 3)
            .unwrap();
        assert!(matches!(normal, ItemModel::Normal { .. }));
        assert!(normal.is_released(1));
        assert!(!normal.is_released(0));
        assert_eq!(normal.convertible_count(ctx), i32::MAX);
        // Material 5 ×2 a convert from 20 held.
        assert_eq!(normal.affordable_converts(ctx), 10);
        assert!(normal.is_allowed_to_convert_start(ctx));
        // 12 minutes with a 50% pass is 6 a convert (GetOnceConvertMinutes).
        assert_eq!(normal.convert_minutes(ctx, 3), 18);
    }

    #[test]
    fn a_limited_row_counts_its_history_and_a_month_change_resets_it() {
        let masters = masters();
        let mut user = user(&masters);
        user.histories.push(ConvertItemHistory {
            mysekai_convert_item_id: 14,
            convert_count: 2,
            total_convert_count: 2,
            last_convert_start_at: NOW - HOUR,
        });
        let ctx = context(&masters, &user);
        let limited = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 14)
            .unwrap();
        assert_eq!(limited.convertible_count(ctx), 1);
        assert_eq!(limited.once_convertible_count(ctx), 1);
        assert!(limited.has_convertible_count(ctx));
        // The same row started the month before reads its limit whole (mid
        // month on both sides, so the local offset does not move either
        // instant across a month).
        let mid_month = 14 * 86_400_000;
        user.histories[0].last_convert_start_at = JANUARY - mid_month;
        let ctx = ItemContext {
            masters: &masters,
            user: &user,
            now: FEBRUARY + mid_month,
        };
        let limited = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 14)
            .unwrap();
        assert_eq!(limited.convertible_count(ctx), 3);
    }

    #[test]
    fn the_refresh_reads_the_master_calendar() {
        // 2026-10-19 is a Monday; the calendar turns Mondays at 05:00.
        let monday = 20_745 * 86_400_000;
        let calendar = CalendarRefresh { hour: 5, weekday: 1 };
        let start = monday + 2 * 86_400_000;
        let turn = monday + 7 * 86_400_000 + 5 * HOUR;
        assert!(!has_crossed(RefreshCycle::Weekly, Some(&calendar), start, turn - 1, 0));
        assert!(has_crossed(RefreshCycle::Weekly, Some(&calendar), start, turn, 0));
        assert!(!has_crossed(RefreshCycle::None, Some(&calendar), JANUARY, FEBRUARY, 0));
        assert!(has_crossed(RefreshCycle::Monthly, Some(&calendar), JANUARY, FEBRUARY, 0));
        assert!(!has_crossed(RefreshCycle::Monthly, None, JANUARY, FEBRUARY, 0));
        assert_eq!(history_converts(RefreshCycle::None, None, None, NOW, 0), 0);
    }

    #[test]
    fn a_blueprint_and_a_record_read_their_obtain_rows() {
        let masters = masters();
        let user = user(&masters);
        let models = ItemModel::all(context(&masters, &user));
        let blueprint = models.iter().find(|model| model.id() == 22).unwrap();
        match blueprint {
            ItemModel::Blueprint { obtain, .. } => {
                assert_eq!(*obtain, BlueprintKind::Random);
                assert!(obtain.is_random_box());
                assert!(!obtain.is_white());
                assert_eq!(
                    obtain.name_key(),
                    Some("WORD_CONVERT_ITEM_SURPLUS_BLUEPRINT_RANDOM")
                );
            }
            other => panic!("not a blueprint: {other:?}"),
        }
        let record = models.iter().find(|model| model.id() == 26).unwrap();
        match record {
            ItemModel::Record { obtain, choice, .. } => {
                // Item 26 reads obtain row 2, whose kind is `random`: one
                // model, made with the default choice.
                assert_eq!(*obtain, RecordKind::Random);
                assert_eq!(*choice, 0);
                assert_eq!(
                    obtain.description_key(),
                    "MSG_CONVERT_RECORD_RANDOM_BOX_DESCRIPTION"
                );
            }
            other => panic!("not a record: {other:?}"),
        }
        // Item 27 reads obtain row 3, a category choice: one model per
        // `music` category in `seq` order (42 at seq 1, then 41 at seq 2),
        // each with the category's id as its choice.
        let choices: Vec<(RecordKind, i32)> = models
            .iter()
            .filter(|model| model.id() == 27)
            .map(|model| match model {
                ItemModel::Record { obtain, choice, .. } => (*obtain, *choice),
                other => panic!("not a record: {other:?}"),
            })
            .collect();
        assert_eq!(
            choices,
            [(RecordKind::CategoryChoice, 42), (RecordKind::CategoryChoice, 41)]
        );
        assert_eq!(
            RecordKind::CategoryChoice.description_key(),
            "MSG_CONVERT_RECORD_RANDOM_BOX_CHOICE_DESCRIPTION"
        );
    }

    #[test]
    fn a_pass_slot_needs_the_pass_and_its_remaining_minutes() {
        let masters = masters();
        let machine = Machine::new(&masters);
        assert!(!machine.is_pass_slot(0));
        assert!(machine.is_pass_slot(1));
        assert_eq!(machine.slot_count(true), 3);
        assert_eq!(machine.slot_count(false), 1);
        let mut user = user(&masters);
        user.pass = PassState { expired_at: 0 };
        let ctx = context(&masters, &user);
        let normal = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 3)
            .unwrap();
        assert_eq!(normal.once_convertible_count_in_slot(ctx, 0), 10);
        assert_eq!(normal.once_convertible_count_in_slot(ctx, 1), 0);
        // A pass with two hours left holds ten 12-minute converts (12 × 50%
        // = 6 minutes); the master's once limit is 10.
        user.pass = PassState {
            expired_at: NOW + 2 * HOUR,
        };
        let ctx = context(&masters, &user);
        let normal = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 3)
            .unwrap();
        assert_eq!(normal.once_convertible_count_in_slot(ctx, 1), 10);
    }

    #[test]
    fn a_slot_reads_its_state_and_counts_off_the_clock() {
        let masters = masters();
        let machine = Machine::new(&masters);
        let mut user = user(&masters);
        user.slots
            .push(slot_row(0, CONVERT_SLOT_PROCESSING, NOW - 30 * 60_000, 12, 4, 1));
        let ctx = context(&masters, &user);
        let model = SlotModel::new(ctx, 0, machine.is_pass_slot(0));
        assert_eq!(model.state(), SlotState::Processing);
        assert!(model.is_processing());
        assert_eq!(model.item_id(), Some(3));
        assert_eq!(model.total_convert_count(), 4);
        assert_eq!(model.received_count(), 1);
        assert_eq!(model.remaining_convert_count(), 3);
        // 30 minutes over 12-minute converts: two complete, one received.
        assert_eq!(model.total_completed_convert_count(), 2);
        assert_eq!(model.current_completed_convert_count(), 1);
        assert!(model.is_receivable());
        assert_eq!(model.remaining_time_to_complete_convert(), 18 * 60_000);
        assert!((model.normalized_progress_time() - 1.0 / 3.0).abs() < 1e-6);
        assert_eq!(model.consumed_jewel_quantity_for_shorten(&machine), 18);
        assert!(model.is_released_shorten(&machine, &user));
    }

    #[test]
    fn a_complete_slot_has_every_convert_done() {
        let masters = masters();
        let mut user = user(&masters);
        user.slots.push(slot_row(0, CONVERT_SLOT_COMPLETE, NOW, 12, 4, 4));
        let model = SlotModel::new(context(&masters, &user), 0, false);
        assert_eq!(model.state(), SlotState::Complete);
        assert_eq!(model.total_completed_convert_count(), 4);
        assert_eq!(model.normalized_progress_time(), 1.0);
        assert_eq!(model.remaining_time_to_complete_convert(), 0);
        assert!(!model.is_receivable());
    }

    #[test]
    fn a_slot_without_a_row_is_ready_or_locked() {
        let masters = masters();
        let mut user = user(&masters);
        user.pass = PassState { expired_at: 0 };
        let ctx = context(&masters, &user);
        assert_eq!(SlotModel::new(ctx, 0, false).state(), SlotState::Ready);
        assert_eq!(SlotModel::new(ctx, 1, true).state(), SlotState::Locked);
    }

    #[test]
    fn a_skip_costs_the_minutes_rounded_up() {
        let masters = masters();
        let machine = Machine::new(&masters);
        assert_eq!(machine.skip_price(0), 0);
        assert_eq!(machine.skip_price(1), 1);
        assert_eq!(machine.skip_price(60_000), 1);
        assert_eq!(machine.skip_price(60_001), 2);
        assert_eq!(machine.skip_price(125 * 60_000), 125);
        let mut bare = masters.clone();
        bare.shorten_jewel_per_minute = None;
        assert_eq!(Machine::new(&bare).skip_price(60_000), 1);
    }

    #[test]
    fn the_detail_offers_the_slots_bound_and_its_balloons() {
        let masters = masters();
        let mut user = user(&masters);
        user.holdings.materials.clear();
        let ctx = context(&masters, &user);
        let normal = ItemModel::all(ctx)
            .into_iter()
            .find(|model| model.id() == 3)
            .unwrap();
        let detail = DetailModel::new(ctx, normal, 0, 3);
        assert_eq!(detail.convert_count, 3);
        assert_eq!(detail.convert_ms, 6 * 3 * 60_000);
        assert!(!detail.is_allowed_to_convert_start);
        assert!(detail.shows_not_enough_resource_balloon(ctx));
        assert!(detail.shows_shorten_by_joined_pass_balloon(ctx));
        assert_eq!(detail.shorten_by_joined_pass_percent(ctx), 50.0);
        assert_eq!(detail.consumed_resources(ctx).len(), 2);
        assert_eq!(detail.max_convert_count(ctx), 10);
    }

    #[test]
    fn the_tabs_read_in_seq_order() {
        let masters = masters();
        assert_eq!(tabs(&masters), vec![(1, 10), (3, 30), (4, 40)]);
        assert_eq!(first_tab(&masters), 1);
    }

    #[test]
    fn the_list_shows_the_displayable_items_of_the_tab() {
        let masters = masters();
        let user = user(&masters);
        let ctx = context(&masters, &user);
        let ids: Vec<i32> = ItemModel::of_tab(ctx, 1).iter().map(ItemModel::id).collect();
        assert_eq!(ids, vec![3, 14]);
        assert!(ItemModel::of_tab(ctx, 2).is_empty());
    }

    #[test]
    fn a_receive_hands_out_every_step_above_the_received_progress() {
        // GetObtainResources(progress): the rows of that step, in the order
        // the start wrote them (seq).
        let mut row = slot_row(0, CONVERT_SLOT_COMPLETE, NOW, 12, 2, 0);
        row.obtain_resources = vec![
            ConvertObtainResource {
                slot_id: 0,
                progress: 1,
                seq: 0,
                resource_type: "mysekai_material".to_owned(),
                resource_id: 7,
                quantity: 1,
            },
            ConvertObtainResource {
                slot_id: 0,
                progress: 1,
                seq: 1,
                resource_type: "mysekai_item".to_owned(),
                resource_id: 9,
                quantity: 2,
            },
            ConvertObtainResource {
                slot_id: 0,
                progress: 2,
                seq: 0,
                resource_type: "mysekai_material".to_owned(),
                resource_id: 7,
                quantity: 1,
            },
        ];
        let masters = masters();
        let mut user = user(&masters);
        user.slots.push(row);
        let model = SlotModel::new(context(&masters, &user), 0, false);
        let rows = model.obtain_rows(2).expect("two steps have rows");
        assert_eq!(
            rows.iter()
                .map(|row| (row.resource_id, row.quantity))
                .collect::<Vec<_>>(),
            vec![(7, 1), (9, 2), (7, 1)]
        );
        assert!(model.obtain_rows(0).is_none());
    }

    #[test]
    fn received_resources_add_up_by_type_and_id() {
        let material = |id: i32, quantity: i32| UserResource {
            resource_type: "mysekai_material".to_owned(),
            resource_id: id,
            resource_level: 1,
            quantity,
        };
        let added = same_resources(&[material(7, 2), material(9, 1), material(7, 3)]);
        assert_eq!(added.len(), 2);
        assert_eq!(added[0].quantity, 5);
        assert_eq!(added[1].resource_id, 9);
    }

    #[test]
    fn the_holdings_read_the_types_with_a_user_table() {
        let holdings = Holdings {
            materials: [(5, 20)].into_iter().collect(),
            items: [(9, 2)].into_iter().collect(),
            fixtures: BTreeMap::new(),
        };
        assert_eq!(holdings.held("mysekai_material", 5), 20);
        assert_eq!(holdings.held("mysekai_item", 9), 2);
        assert_eq!(holdings.held("mysekai_fixture", 1), 0);
        // A type without a user table never limits a convert (a coin).
        assert_eq!(holdings.held("coin", 0), i32::MAX);
    }
}
