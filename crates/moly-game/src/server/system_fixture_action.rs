//! The system fixture action: the user's actioned system
//! fixtures, the request that adds one, and the party cakes the home layout
//! serves.
//!
//! - `userMysekaiSystemFixtureActions` (`UserMysekaiSystemFixtureAction`
//!   {userId, mysekaiSystemFixtureId}): the system fixtures the user has
//!   actioned. It is user data: the document holds it and the join carries
//!   it to the client's copy
//!   ([`super::client::system_fixture_action::ClientSystemFixtureActions`]),
//!   which `IsActionedSystemFixture` reads. **Named default**: no row.
//! - `PostUserMysekaiSystemFixtureActionApi` (`userId`,
//!   `mysekaiSystemFixtureId`; an empty body), sent by
//!   `MysekaiSystemFixtureActionService.ExecuteAPI`, which the birthday
//!   party's `ExecuteAsync` calls after its confirm dialog. The reply
//!   (`UserMysekaiSystemFixtureActionResponse`) carries `updatedResources`
//!   (`SuiteUser`) and `obtainedResources`. The client sends it through
//!   [`super::client::system_fixture_action::post`]; [`install`] puts the
//!   endpoint in place.
//!
//! **Named policies** (the server's rules are not in the client):
//! - *System fixture action reply* (`policies.systemFixtureActionReply`):
//!   `success` (the product default) adds the fixture's row (a repeat adds
//!   none) and replies with the section and the pending ones; `failure`
//!   refuses every request (the client's reply is null: the API error, and
//!   the party stops before its cut-scene). The system fixture id is taken
//!   as stated (the server model does not read the system fixture master).
//!   `obtainedResources` is empty: which rewards an action grants is the
//!   server's, and no master table names one.
//! - *Birthday party fixtures* (`policies.birthdayPartyFixtures`): `placed`
//!   (the product default) serves the two party cakes of the system fixture
//!   master (rows 58 and 59, type `birthday`, parties 1 and 2) in the home
//!   site's housing layout: fixture 844 `mdl_bir1106_fixture_cake1` at cells
//!   x 2..5, z 2..5 and fixture 849 `mdl_bir1103_fixture_cake1` at x 8..11,
//!   z 2..5, both 4 x 3 x 4 cells (the fixture master's grid size), floor
//!   layout, rotation 0, texture 1. Where the server places a party's cake,
//!   and in which days, is the server's: the cells are this mock's choice and
//!   both cakes stand at every time (the client's own windows decide whether
//!   the button shows and whether the party opens). Each cake is checked when
//!   the layout is served, against the layout's floor rows and the starter
//!   house ([`crate::entry::house::STARTER_HOUSE`]), with the XZ overlap rule
//!   the house append uses; a cake that overlaps one, or whose fixture a row
//!   already places, is not served, loudly. `none` serves no cake.

use bevy::prelude::*;
use moly_law::fixture::{Direction, GridPosition};
use serde_json::{json, Map, Value};

use super::client::system_fixture_action::{
    rows_value, ClientSystemFixtureActions, SystemFixtureActionEndpoint, SystemFixtureActionReply,
    UserMysekaiSystemFixtureAction, API,
};
use super::document::{int32, object, only};
use super::{ResponseKind, ServerModel};

pub(crate) const SECTION: &str = "userMysekaiSystemFixtureActions";
pub(crate) const REPLY_POLICY: &str = "systemFixtureActionReply";
pub(crate) const PARTY_FIXTURES_POLICY: &str = "birthdayPartyFixtures";
const REPLY_SUCCESS: &str = "success";
const REPLY_FAILURE: &str = "failure";
const PARTY_PLACED: &str = "placed";
const PARTY_NONE: &str = "none";

/// The document keys this part reads at the top level.
pub(crate) const DOCUMENT_KEYS: [&str; 1] = [SECTION];
/// The keys this part reads under `policies`.
pub(crate) const POLICY_KEYS: [&str; 2] = [REPLY_POLICY, PARTY_FIXTURES_POLICY];

/// One party cake of the birthday party fixtures policy.
struct PartyCake {
    fixture_id: i32,
    package: &'static str,
    min: [i8; 3],
    max: [i8; 3],
}

/// The party cakes (system fixture rows 58 and 59).
const PARTY_CAKES: [PartyCake; 2] = [
    PartyCake {
        fixture_id: 844,
        package: "mysekai__fixture__mdl_bir1106_fixture_cake1",
        min: [2, 0, 2],
        max: [5, 2, 5],
    },
    PartyCake {
        fixture_id: 849,
        package: "mysekai__fixture__mdl_bir1103_fixture_cake1",
        min: [8, 0, 2],
        max: [11, 2, 5],
    },
];
/// `MysekaiFixtureLayoutType` floor.
const FLOOR_LAYOUT: u8 = moly_law::fixture::position::layout_type::FLOOR;
const HOME_SITE: &str = "home_site";

/// `policies.systemFixtureActionReply`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum ReplyPolicy {
    #[default]
    Success,
    Failure,
}

/// `policies.birthdayPartyFixtures`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub(crate) enum PartyFixturesPolicy {
    #[default]
    Placed,
    None,
}

/// This part of the server document.
#[derive(Clone, Debug, PartialEq, Default)]
pub(crate) struct SystemFixtureActionDoc {
    pub(crate) rows: Vec<UserMysekaiSystemFixtureAction>,
    pub(crate) reply: ReplyPolicy,
    pub(crate) party_fixtures: PartyFixturesPolicy,
}

// ---------------------------------------------------------------------------
// Reading and writing
// ---------------------------------------------------------------------------

fn parse_rows(value: &Value) -> Result<Vec<UserMysekaiSystemFixtureAction>, String> {
    let rows = value
        .as_array()
        .ok_or_else(|| format!("{SECTION} is not an array"))?;
    let mut seen = std::collections::BTreeSet::new();
    rows.iter()
        .enumerate()
        .map(|(i, row)| {
            let at = format!("{SECTION}[{i}]");
            let row = object(row, &at)?;
            only(row, &["mysekaiSystemFixtureId"], &at)?;
            let id = int32(row, "mysekaiSystemFixtureId", &at)?;
            if id < 1 {
                return Err(format!("{at}.mysekaiSystemFixtureId = {id} is below 1"));
            }
            if !seen.insert(id) {
                return Err(format!("{at} repeats system fixture {id}"));
            }
            Ok(UserMysekaiSystemFixtureAction {
                mysekai_system_fixture_id: id,
            })
        })
        .collect()
}

fn parse_reply(value: &Value) -> Result<ReplyPolicy, String> {
    match value.as_str() {
        Some(REPLY_SUCCESS) => Ok(ReplyPolicy::Success),
        Some(REPLY_FAILURE) => Ok(ReplyPolicy::Failure),
        _ => Err(format!(
            "policies.{REPLY_POLICY} is neither \"{REPLY_SUCCESS}\" nor \"{REPLY_FAILURE}\""
        )),
    }
}

fn parse_party_fixtures(value: &Value) -> Result<PartyFixturesPolicy, String> {
    match value.as_str() {
        Some(PARTY_PLACED) => Ok(PartyFixturesPolicy::Placed),
        Some(PARTY_NONE) => Ok(PartyFixturesPolicy::None),
        _ => Err(format!(
            "policies.{PARTY_FIXTURES_POLICY} is neither \"{PARTY_PLACED}\" nor \"{PARTY_NONE}\""
        )),
    }
}

impl SystemFixtureActionDoc {
    /// This part from a document (the named defaults when absent).
    pub(crate) fn parse(
        doc: &Map<String, Value>,
        policies: &Map<String, Value>,
    ) -> Result<Self, String> {
        Ok(Self {
            rows: doc.get(SECTION).map_or(Ok(Vec::new()), parse_rows)?,
            reply: policies
                .get(REPLY_POLICY)
                .map_or(Ok(ReplyPolicy::default()), parse_reply)?,
            party_fixtures: policies
                .get(PARTY_FIXTURES_POLICY)
                .map_or(Ok(PartyFixturesPolicy::default()), parse_party_fixtures)?,
        })
    }

    pub(crate) fn write(&self, top: &mut Map<String, Value>, policies: &mut Map<String, Value>) {
        top.insert(SECTION.into(), rows_value(&self.rows));
        policies.insert(
            REPLY_POLICY.into(),
            json!(match self.reply {
                ReplyPolicy::Success => REPLY_SUCCESS,
                ReplyPolicy::Failure => REPLY_FAILURE,
            }),
        );
        policies.insert(
            PARTY_FIXTURES_POLICY.into(),
            json!(match self.party_fixtures {
                PartyFixturesPolicy::Placed => PARTY_PLACED,
                PartyFixturesPolicy::None => PARTY_NONE,
            }),
        );
    }

    /// A `server.edit` of the section (next response) or of the reply
    /// policy (live); `None` for another path. The party fixtures policy is
    /// read when the home layout is served, so it is the document's only.
    pub(crate) fn edit_path(
        &mut self,
        parts: &[&str],
        value: &Value,
    ) -> Option<Result<bool, String>> {
        match parts {
            [SECTION] => Some(parse_rows(value).map(|rows| {
                self.rows = rows;
                true
            })),
            ["policies", REPLY_POLICY] => Some(parse_reply(value).map(|reply| {
                self.reply = reply;
                false
            })),
            _ => None,
        }
    }

    /// The party fixtures policy on the served layouts: each cake joins the
    /// home site's rows unless a row places its fixture or it overlaps a
    /// floor row or the starter house.
    pub(crate) fn serve_party_fixtures(&self, layouts: &mut Value) {
        if self.party_fixtures == PartyFixturesPolicy::None {
            info!(
                "[server] policies.{PARTY_FIXTURES_POLICY} = {PARTY_NONE}: no party cake is served"
            );
            return;
        }
        let Some(rows) = layouts.as_array_mut().and_then(|layouts| {
            layouts
                .iter_mut()
                .find(|layout| layout["siteType"].as_str() == Some(HOME_SITE))
                .and_then(|layout| layout["mysekaiFixtures"].as_array_mut())
        }) else {
            error!("[server] policies.{PARTY_FIXTURES_POLICY}: the served layouts have no {HOME_SITE} row with mysekaiFixtures; no party cake is served");
            return;
        };
        for cake in &PARTY_CAKES {
            let row = cake.row();
            match check_cake(cake, rows) {
                Ok(()) => {
                    info!(
                        "[server] policies.{PARTY_FIXTURES_POLICY}: party cake {} (fixture {}) served at cells x {}..{}, z {}..{}; overlaps none of {} rows nor the starter house",
                        cake.package, cake.fixture_id, cake.min[0], cake.max[0], cake.min[2], cake.max[2], rows.len()
                    );
                    rows.push(row);
                }
                Err(reason) => error!(
                    "[server] policies.{PARTY_FIXTURES_POLICY}: party cake {} (fixture {}) is not served: {reason}",
                    cake.package, cake.fixture_id
                ),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The party cakes
// ---------------------------------------------------------------------------

impl PartyCake {
    fn row(&self) -> Value {
        json!({
            "mysekaiFixtureId": self.fixture_id,
            "package": self.package,
            "textureId": 1,
            "minimum": {"x": self.min[0], "y": self.min[1], "z": self.min[2]},
            "maximum": {"x": self.max[0], "y": self.max[1], "z": self.max[2]},
            "centerY": 0,
            "layoutType": FLOOR_LAYOUT,
            "rotation": 0,
            "mock": format!("A birthday party cake (policies.{PARTY_FIXTURES_POLICY}): the system fixture master's party fixture; its cells are the mock's choice, clear of the starter rows and the house."),
        })
    }
}

fn grid(value: &Value, at: &str) -> Result<GridPosition, String> {
    let axis = |key: &str| {
        value[key]
            .as_i64()
            .and_then(|v| i8::try_from(v).ok())
            .ok_or_else(|| format!("{at}.{key} is not a grid cell"))
    };
    Ok(GridPosition {
        x: axis("x")?,
        y: axis("y")?,
        z: axis("z")?,
    })
}

/// The placed XZ footprint of a floor row, `None` for a wall row.
fn footprint(
    min: GridPosition,
    max: GridPosition,
    center_y: i8,
    direction: Direction,
    layout: u8,
) -> Result<Option<(GridPosition, GridPosition)>, String> {
    if layout & moly_law::fixture::position::WALL_LAYOUT_MASK != 0 {
        return Ok(None);
    }
    let (mut center, size) = moly_law::fixture::position::footprint_to_center_size(min, max)?;
    center.y = center_y;
    moly_law::fixture::position::layout_footprint(center, size, direction, layout).map(Some)
}

fn row_footprint(row: &Value, at: &str) -> Result<Option<(GridPosition, GridPosition)>, String> {
    let byte = |key: &str| {
        row[key]
            .as_u64()
            .and_then(|v| u8::try_from(v).ok())
            .ok_or_else(|| format!("{at}.{key} is not a byte"))
    };
    let center_y = row["centerY"]
        .as_i64()
        .and_then(|v| i8::try_from(v).ok())
        .ok_or_else(|| format!("{at}.centerY is not a grid cell"))?;
    let direction = Direction::from_u8(byte("rotation")?)
        .ok_or_else(|| format!("{at}.rotation is not 0..3"))?;
    footprint(
        grid(&row["minimum"], &format!("{at}.minimum"))?,
        grid(&row["maximum"], &format!("{at}.maximum"))?,
        center_y,
        direction,
        byte("layoutType")?,
    )
}

fn overlaps(a: (GridPosition, GridPosition), b: (GridPosition, GridPosition)) -> bool {
    a.0.x <= b.1.x && a.1.x >= b.0.x && a.0.z <= b.1.z && a.1.z >= b.0.z
}

fn check_cake(cake: &PartyCake, rows: &[Value]) -> Result<(), String> {
    let cell = |[x, y, z]: [i8; 3]| GridPosition { x, y, z };
    let placed = footprint(
        cell(cake.min),
        cell(cake.max),
        0,
        Direction::Front,
        FLOOR_LAYOUT,
    )?
    .ok_or("a floor row has no floor footprint")?;
    let house = &crate::entry::house::STARTER_HOUSE;
    let house_placed = footprint(house.min, house.max, 0, house.direction, FLOOR_LAYOUT)?
        .ok_or("the starter house has no floor footprint")?;
    if overlaps(placed, house_placed) {
        return Err(format!(
            "its cells overlap the starter house {} ({},{})..({},{})",
            house.package, house_placed.0.x, house_placed.0.z, house_placed.1.x, house_placed.1.z
        ));
    }
    for (i, row) in rows.iter().enumerate() {
        let at = format!("home row {i}");
        if row["mysekaiFixtureId"].as_i64() == Some(i64::from(cake.fixture_id)) {
            return Err(format!("{at} already places fixture {}", cake.fixture_id));
        }
        if let Some(other) = row_footprint(row, &at)? {
            if overlaps(placed, other) {
                return Err(format!(
                    "its cells overlap {at} ({}) at ({},{})..({},{})",
                    row["package"].as_str().unwrap_or("?"),
                    other.0.x,
                    other.0.z,
                    other.1.x,
                    other.1.z
                ));
            }
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// The endpoint
// ---------------------------------------------------------------------------

impl ServerModel {
    /// The reply policy on one request: the reply's `obtainedResources`.
    fn system_fixture_action(
        &mut self,
        mysekai_system_fixture_id: i32,
    ) -> Result<Vec<super::client::talk_read::UserResource>, String> {
        if self.doc.system_fixture_actions.reply == ReplyPolicy::Failure {
            return Err(format!("policies.{REPLY_POLICY} is {REPLY_FAILURE}"));
        }
        if mysekai_system_fixture_id < 1 {
            return Err(format!(
                "mysekaiSystemFixtureId {mysekai_system_fixture_id} is not a system fixture id"
            ));
        }
        let rows = &mut self.doc.system_fixture_actions.rows;
        let added = !rows
            .iter()
            .any(|row| row.mysekai_system_fixture_id == mysekai_system_fixture_id);
        if added {
            rows.push(UserMysekaiSystemFixtureAction {
                mysekai_system_fixture_id,
            });
            self.commit();
        }
        info!(
            "[server] {API}: system fixture {mysekai_system_fixture_id} {} (policies.{REPLY_POLICY} {REPLY_SUCCESS}); obtainedResources none",
            if added { "added to userMysekaiSystemFixtureActions" } else { "already actioned; no row added" }
        );
        self.respond(ResponseKind::SystemFixtureAction, false, &[SECTION]);
        Ok(Vec::new())
    }
}

/// The endpoint ([`super::client::system_fixture_action::post`]): the reply's
/// `updatedResources` are merged into the client's copy before it returns.
pub(super) fn handle(
    In(mysekai_system_fixture_id): In<i32>,
    mut copy: ResMut<ClientSystemFixtureActions>,
) -> SystemFixtureActionReply {
    let answer = super::with_model(|model| {
        if !model.joined {
            return Err("the server has not joined the client yet".to_owned());
        }
        let obtained = model.system_fixture_action(mysekai_system_fixture_id)?;
        let response = model.responses.last().expect("the response just recorded");
        Ok((obtained, response.system_fixture_actions.clone()))
    })
    .unwrap_or_else(|| Err("the server model is not installed".to_owned()));
    match answer {
        Ok((obtained_resources, rows)) => {
            if let Some(rows) = rows {
                copy.apply(rows);
            }
            SystemFixtureActionReply {
                success: true,
                refusal: None,
                obtained_resources,
            }
        }
        Err(reason) => SystemFixtureActionReply::refused(reason),
    }
}

/// The endpoint and the client's copy.
pub(crate) fn install(app: &mut App) {
    app.init_resource::<ClientSystemFixtureActions>();
    let endpoint = app.world_mut().register_system(handle);
    app.insert_resource(SystemFixtureActionEndpoint(endpoint));
}

// ---------------------------------------------------------------------------
// The panel's schema
// ---------------------------------------------------------------------------

pub(crate) fn schema_sections() -> Value {
    json!([{
        "key": SECTION,
        "title": "Actioned system fixtures",
        "delivery": "next-response",
        "fields": [{
            "path": SECTION,
            "type": "rows",
            "row": {"mysekaiSystemFixtureId": "int"},
            "note": "the system fixtures the user has actioned (IsActionedSystemFixture); the birthday party's confirm dialog reads it (named default: none)",
        }],
    }, {
        "key": "systemFixtureAction",
        "title": "System fixture action replies",
        "delivery": "live",
        "fields": [{
            "path": format!("policies.{REPLY_POLICY}"),
            "type": "enum",
            "values": [
                {"value": REPLY_SUCCESS, "label": "the fixture's row is added; the reply carries the section and the pending ones"},
                {"value": REPLY_FAILURE, "label": "every request is refused (the party stops before its cut-scene)"},
            ],
            "note": "the system fixture id is taken as stated; obtainedResources is empty",
        }],
    }])
}

pub(crate) fn schema_policies() -> Value {
    json!([{
        "name": "system fixture action reply",
        "rule": "success: the request's system fixture joins userMysekaiSystemFixtureActions (a repeat adds none), obtainedResources empty; failure refuses every request",
    }, {
        "name": "birthday party fixtures",
        "rule": "placed: the party cakes 844 (x 2..5, z 2..5) and 849 (x 8..11, z 2..5), 4x3x4 floor rows, rotation 0, join the served home layout unless a row places the fixture or they overlap a floor row or the starter house (then not served, loudly); none: no cake",
    }])
}
