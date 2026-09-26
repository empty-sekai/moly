//! The change-site controller: which NPCs move between the home site and the
//! floors, and the loop that sends them.
//!
//! The source's shape:
//!
//! - **Lottery on entering a site.** Home and every floor call
//!   `LotteryChangeSiteCharacters(site type)` when the player enters them.
//!   It does nothing unless the player owns the room and no birthday-party
//!   context is running. Otherwise it first cancels the running change (every
//!   NPC of the change in flight has its objective cancelled) and then builds
//!   the new change list: entering home (0) orders every NPC that is not on
//!   home back to it; entering a floor (1 to 3) draws a cast for that floor
//!   ([`lottery_characters`]) and orders each drawn NPC that is not already
//!   there; any other site type logs an error and stores no list.
//! - **The cast of a floor.** The cap is the entry maximum of the level the
//!   current site stands at for the player's rank (no level: nobody). The
//!   floor's placed fixtures select the talk-list talks whose first fixture
//!   condition names one of them. Among the unread ones whose cast fits the
//!   cap, one is picked with one engine draw; when none fits, the first talk
//!   (read or not) with the smallest cast within the cap is taken; when there
//!   is none either, nobody is drawn. The NPC list is then sorted with the
//!   picked talk's cast **last** (the sort key is "is in the cast", ascending)
//!   and a random order inside each part, and the first `min(NPC count, cap)`
//!   are the cast.
//! - **The loop.** Every `CharacterChangeSiteIntervalSecond` seconds
//!   (truncated to whole seconds, on the scaled clock) the loop looks at the
//!   change list: with no list or an empty one it waits again; otherwise one
//!   engine draw `d = Range(0, 100)` and, unless `CharacterChangeSiteRate >
//!   d` (a float compare; NaN proceeds), it runs the change: the NPCs in list
//!   order, one at a time, each skipped when already on its target site,
//!   otherwise ordered to move and followed until its objective is no longer
//!   the change-site objective or it stands on the target site. When every
//!   NPC of the list stands on its target, the list is dropped.
//! - **One order.** A move order cancels the NPC's current objective; when
//!   the cancel reports false nothing else happens; otherwise the NPC gets
//!   the change-site objective with its target site, the flag that runs the
//!   next objective without a rest, and the talk interrupt of the change-site
//!   objective type.

/// `NPCAvatarChangeSiteData.type` of a move order.
pub const MOVE: i32 = 1;

/// One row of the change list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangeSiteData {
    pub move_type: i32,
    pub target_site: i32,
    pub unit: i32,
}

/// One NPC of the avatar store's list, in list order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NpcOnSite {
    pub unit: i32,
    /// The NPC's own current site type.
    pub site: i32,
}

/// What `LotteryChangeSiteCharacters` did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LotteryOutcome {
    /// Not the room owner, or a birthday-party context: nothing ran.
    NotRun,
    /// The running change was cancelled and the list replaced.
    Replaced(Vec<ChangeSiteData>),
    /// The running change was cancelled; the site type is not home or a
    /// floor, an error is logged and the stored list is left as it was.
    RefusedSiteType(i32),
}

/// `AllCharacterMoveOutChangeSiteDataList(target)`: every NPC not on the
/// target, in list order.
pub fn all_character_move_out(npcs: &[NpcOnSite], target: i32) -> Vec<ChangeSiteData> {
    npcs.iter()
        .filter(|npc| npc.site != target)
        .map(|npc| ChangeSiteData {
            move_type: MOVE,
            target_site: target,
            unit: npc.unit,
        })
        .collect()
}

/// `LotteryChangeSiteDataList(target)`: the drawn cast minus the NPCs already
/// on the target, in cast order.
pub fn lottery_change_site_data_list(
    cast: &[i32],
    npcs: &[NpcOnSite],
    target: i32,
) -> Vec<ChangeSiteData> {
    cast.iter()
        .filter(|unit| {
            npcs.iter()
                .find(|npc| npc.unit == **unit)
                .is_some_and(|npc| npc.site != target)
        })
        .map(|unit| ChangeSiteData {
            move_type: MOVE,
            target_site: target,
            unit: *unit,
        })
        .collect()
}

/// `LotteryChangeSiteCharacters(target)`. `owner_and_no_birthday` is the
/// gate; `draw_cast` runs [`lottery_characters`] for a floor.
pub fn lottery_change_site_characters(
    target: i32,
    owner_and_no_birthday: bool,
    npcs: &[NpcOnSite],
    draw_cast: &mut dyn FnMut(i32) -> Vec<i32>,
) -> LotteryOutcome {
    if !owner_and_no_birthday {
        return LotteryOutcome::NotRun;
    }
    if target == 0 {
        return LotteryOutcome::Replaced(all_character_move_out(npcs, 0));
    }
    // `(target - 1) as u32 > 2`: outside 1..=3.
    if (target.wrapping_sub(1) as u32) > 2 {
        return LotteryOutcome::RefusedSiteType(target);
    }
    let cast = draw_cast(target);
    LotteryOutcome::Replaced(lottery_change_site_data_list(&cast, npcs, target))
}

/// One talk of the talk list as the floor lottery reads it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TalkForLottery {
    pub talk_id: i32,
    pub is_read: bool,
    /// The value of the talk's first condition of the fixture-id type, if
    /// the talk has one.
    pub fixture_condition: Option<i32>,
    /// `TalkCharacterCount`: the non-zero units of the talk's unit group.
    pub character_count: i32,
    /// The unit ids of the talk's character group.
    pub cast: Vec<i32>,
}

/// `GetMasterTalksByFixtures(fixtures, only_unread)`: talk-list order.
pub fn talks_by_fixtures<'a>(
    talks: &'a [TalkForLottery],
    fixture_ids: &[i32],
    only_unread: bool,
) -> Vec<&'a TalkForLottery> {
    talks
        .iter()
        .filter(|talk| !(only_unread && talk.is_read))
        .filter(|talk| {
            talk.fixture_condition
                .is_some_and(|value| fixture_ids.contains(&value))
        })
        .collect()
}

/// Which talk the floor lottery keyed its cast on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CastKey {
    /// No site level for the current site at the player's rank.
    NoLevel,
    /// An unread talk within the cap, picked with one draw over `pool` rows.
    Picked {
        talk_id: i32,
        pool: usize,
        draw: usize,
    },
    /// No unread talk fits: the first talk with the smallest cast within the
    /// cap.
    Smallest { talk_id: i32 },
    /// No talk fits at all.
    NoTalk,
}

/// `LotteryCharacters(site type)`. `cap` is the entry maximum of the current
/// site's level (`None`: no level); `fixture_ids` the ids of the fixtures
/// placed on the site type; `npcs` the NPC list's units in order; `draw(n)`
/// the engine's `Range(0, n)`; `order_key()` one random ordering key per NPC,
/// drawn in list order.
pub fn lottery_characters(
    cap: Option<i32>,
    fixture_ids: &[i32],
    talks: &[TalkForLottery],
    npcs: &[i32],
    draw: &mut dyn FnMut(usize) -> usize,
    order_key: &mut dyn FnMut() -> u128,
) -> (CastKey, Vec<i32>) {
    let Some(cap) = cap else {
        return (CastKey::NoLevel, Vec::new());
    };
    let npc_count = npcs.len() as i32;
    let unread = talks_by_fixtures(talks, fixture_ids, true);
    let within: Vec<&TalkForLottery> = unread
        .iter()
        .copied()
        .filter(|talk| talk.character_count <= cap)
        .collect();
    let (key, cast): (CastKey, &[i32]) = if !within.is_empty() {
        let max = within
            .iter()
            .map(|talk| talk.character_count)
            .max()
            .expect("non-empty");
        let pool: Vec<&TalkForLottery> = unread
            .iter()
            .copied()
            .filter(|talk| talk.character_count <= max)
            .collect();
        let index = draw(pool.len());
        assert!(
            index < pool.len(),
            "engine range draw {index} outside [0, {})",
            pool.len()
        );
        let talk = pool[index];
        (
            CastKey::Picked {
                talk_id: talk.talk_id,
                pool: pool.len(),
                draw: index,
            },
            &talk.cast,
        )
    } else {
        let all = talks_by_fixtures(talks, fixture_ids, false);
        // OrderBy is stable: the first of the smallest casts in list order.
        let smallest = all
            .iter()
            .copied()
            .filter(|talk| talk.character_count <= cap)
            .fold(None::<&TalkForLottery>, |best, talk| match best {
                Some(best) if best.character_count <= talk.character_count => Some(best),
                _ => Some(talk),
            });
        match smallest {
            Some(talk) => (
                CastKey::Smallest {
                    talk_id: talk.talk_id,
                },
                &talk.cast,
            ),
            None => return (CastKey::NoTalk, Vec::new()),
        }
    };
    let mut ordered: Vec<(bool, u128, i32)> = npcs
        .iter()
        .map(|unit| (cast.contains(unit), 0, *unit))
        .collect();
    for row in ordered.iter_mut() {
        row.1 = order_key();
    }
    // Stable sort: "in the cast" ascending (false first), then the key.
    ordered.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let take = npc_count.min(cap).max(0) as usize;
    (
        key,
        ordered
            .into_iter()
            .take(take)
            .map(|(_, _, unit)| unit)
            .collect(),
    )
}

/// The loop's wait: `CharacterChangeSiteIntervalSecond` truncated to an int
/// (a positive infinity converts to `i32::MIN`, NaN to 0, other out-of-range
/// values saturate), times 1000 with wrap-around.
pub fn interval_milliseconds(interval_seconds: f32) -> i32 {
    let whole = if interval_seconds == f32::INFINITY {
        i32::MIN
    } else {
        interval_seconds as i32
    };
    whole.wrapping_mul(1000)
}

/// The loop's rate test on draw `d` (`Range(0, 100)`): the change runs unless
/// `rate > d` as floats.
pub fn change_proceeds(rate: f32, draw: i32) -> bool {
    !(rate > draw as f32)
}

/// `HasCharactersMovedAsExpected`: the NPC's current objective is the
/// change-site objective (type 5).
pub fn moving_as_expected(objective_type: i32) -> bool {
    objective_type == 5
}

/// The follow of one order ends when this is false: the NPC is still on the
/// change-site objective and not yet on its target site.
pub fn still_following(objective_type: i32, site: i32, target: i32) -> bool {
    moving_as_expected(objective_type) && site != target
}

/// `AllCharacterArrived`: every row's NPC stands on its target.
pub fn all_arrived(rows: &[ChangeSiteData], site_of: &dyn Fn(i32) -> i32) -> bool {
    rows.iter().all(|row| site_of(row.unit) == row.target_site)
}
