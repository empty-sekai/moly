//! `server.edit`: the panel's edits and actions, validated against the
//! document's structure and the masters, refused by name, then applied; and
//! `game_server_schema()`, the description of what the panel may edit.
//!
//! Wire forms (JSON objects):
//! - `{"type": "server.edit", "path": "<field path>", "value": <JSON>}`
//! - `{"type": "server.edit", "action": "gate.reserve", "mysekaiGameCharacterUnitGroupId": <id>}`
//! - `{"type": "server.edit", "action": "gate.change", "mysekaiGateId": <id>,
//!   "mysekaiGateSkinId": <id>, "mysekaiGameCharacterUnitGroupIds": [<id>, ...]}`
//! - `{"type": "server.edit", "action": "sync"}`
//!
//! Delivery: `clock`, `policies` and the schedule apply live; the game data,
//! the stamina, the pass and the birthday-party sections apply as the next
//! server response; the gate actions are server replies at once. Visitors and layouts are not edited
//! here (they apply on re-entry, a later milestone).

use bevy::log::info;
use serde_json::{json, Map, Value};

use super::document::{self, Clock, GateCharacter, SchedulePolicy, ServerDocument, Stamina};
use super::{
    GateReply, Masters, ReplyTalkList, ResponseKind, ServerModel, SECTION_GAMEDATA, SECTION_PASS,
    SECTION_STAMINA,
};

/// Sections a response carries (the `pending` names).
pub(crate) const PENDING_SECTIONS: [&str; 15] = [
    super::music::SECTION,
    super::avatar::SECTION,
    SECTION_GAMEDATA,
    SECTION_STAMINA,
    SECTION_PASS,
    super::delivery::SECTION_BIRTHDAY_PARTIES,
    super::delivery::SECTION_MATERIALS,
    super::delivery::SECTION_MYSEKAI_MATERIALS,
    super::delivery::SECTION_CARDS,
    super::delivery::SECTION_HONORS,
    super::delivery::SECTION_MASTER_CONFIGS,
    super::inventory::SECTION_FIXTURES,
    super::inventory::SECTION_CANVASES,
    super::inventory::SECTION_BLUEPRINTS,
    super::inventory::SECTION_ITEMS,
];

/// How an accepted edit reaches the client.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Delivery {
    Live,
    NextResponse(&'static str),
}

fn int(value: &Value, what: &str) -> Result<i64, String> {
    value
        .as_i64()
        .ok_or_else(|| format!("{what} must be an integer"))
}

fn int32(value: &Value, what: &str) -> Result<i32, String> {
    let raw = int(value, what)?;
    i32::try_from(raw).map_err(|_| format!("{what} = {raw} does not fit a 32-bit integer"))
}

/// The checks against the masters. A master the root lacks skips its check;
/// the schema names it as missing.
pub(crate) fn check_masters(doc: &ServerDocument, masters: &Masters) -> Result<(), String> {
    if let (Some(stamina), Some(max)) = (doc.stamina, masters.stamina_max) {
        for (name, value, limit) in [
            ("normalStamina", stamina.normal, max.normal),
            ("enhanceStamina", stamina.enhance, max.enhance),
            ("boostStamina", stamina.boost, max.boost),
        ] {
            if value > limit {
                return Err(format!(
                    "userMysekaiStamina.{name} = {value} is above the master maxStamina {limit}"
                ));
            }
        }
    }
    if let (Some(rank), Some(ranks)) = (doc.gamedata.mysekai_rank, &masters.ranks) {
        if !ranks.iter().any(|(row, _)| *row == rank) {
            return Err(format!(
                "userMysekaiGamedata.mysekaiRank = {rank} is not a master rank (1 to {})",
                ranks.iter().map(|(row, _)| *row).max().unwrap_or(0)
            ));
        }
        if masters.rank_of(doc.gamedata.total_exp) != Some(rank) {
            return Err(format!(
                "userMysekaiGamedata.mysekaiRank = {rank} is not the master rank of totalExp {}",
                doc.gamedata.total_exp
            ));
        }
    }
    if let Some(periods) = &masters.periods {
        let known = |id: i32| periods.iter().any(|period| period.id == id);
        for (i, row) in doc.schedules.iter().enumerate() {
            if !known(row.refresh_time_period_id) {
                return Err(format!(
                    "mysekaiPhenomenaSchedules[{i}] names refresh window {}, which the master does not hold",
                    row.refresh_time_period_id
                ));
            }
        }
        if let SchedulePolicy::Daily(map) = &doc.schedule_policy {
            if let Some(period) = map.keys().find(|id| !known(**id)) {
                return Err(format!(
                    "phenomenaSchedulePolicy.byRefreshTimePeriod names refresh window {period}, which the master does not hold"
                ));
            }
        }
    }
    if let Some(phenomena) = &masters.phenomena {
        let known = |id: i32| phenomena.iter().any(|(known, _)| *known == id);
        for (i, row) in doc.schedules.iter().enumerate() {
            if !known(row.phenomena_id) {
                return Err(format!(
                    "mysekaiPhenomenaSchedules[{i}].mysekaiPhenomenaId = {} is not a master phenomenon",
                    row.phenomena_id
                ));
            }
        }
        if let SchedulePolicy::Daily(map) = &doc.schedule_policy {
            if let Some((period, id)) = map.iter().find(|(_, id)| !known(**id)) {
                return Err(format!(
                    "phenomenaSchedulePolicy.byRefreshTimePeriod.{period} = {id} is not a master phenomenon"
                ));
            }
        }
    }
    super::music::check_records(&doc.music_settings, masters.music_records.as_deref())?;
    super::avatar::check(&masters.avatar, &doc.avatar)?;
    super::inventory::check_masters(&doc.inventory, masters)?;
    if let Some((gates, skins)) = &masters.gates {
        if !gates.contains(&doc.gate.gate_id) {
            return Err(format!(
                "userMysekaiGate.mysekaiGateId = {} is not a master gate",
                doc.gate.gate_id
            ));
        }
        if doc.gate.skin_id > 0 && !skins.contains(&doc.gate.skin_id) {
            return Err(format!(
                "userMysekaiGate.mysekaiGateSkinId = {} is not a master gate skin",
                doc.gate.skin_id
            ));
        }
    }
    Ok(())
}

/// One field edit on a copy of the document.
fn edit_path(
    doc: &mut ServerDocument,
    masters: &Masters,
    now: i64,
    path: &str,
    value: &Value,
) -> Result<Delivery, String> {
    let parts: Vec<&str> = path.split('.').collect();
    if let Some(result) = super::delivery::edit_path(&mut doc.delivery, &parts, path, value) {
        return result.map(|section| match section {
            Some(section) => Delivery::NextResponse(section),
            None => Delivery::Live,
        });
    }
    if let Some(result) = super::home_action::edit_path(&mut doc.home_action_reply, &parts, value) {
        return result.map(|()| Delivery::Live);
    }
    if let Some(result) = super::music::edit_path(&mut doc.music_settings, &parts, value) {
        return result.map(|()| Delivery::NextResponse(super::music::SECTION));
    }
    if let Some(result) = super::avatar::edit_path(&mut doc.avatar, &parts, path, value) {
        return result.map(|()| Delivery::NextResponse(super::avatar::SECTION));
    }
    if let Some(result) = super::inventory::edit_path(&mut doc.inventory, &parts, path, value) {
        return result.map(|section| match section {
            Some(section) => Delivery::NextResponse(section),
            None => Delivery::Live,
        });
    }
    match parts.as_slice() {
        ["clock"] => {
            doc.clock = document::parse_clock(value)?;
            Ok(Delivery::Live)
        }
        ["clock", "mode"] => {
            doc.clock = match (value.as_str(), doc.clock) {
                (Some("device"), Clock::Device { .. }) | (Some("fixed"), Clock::Fixed { .. }) => {
                    doc.clock
                }
                // Fixed at the server's current time, so the switch itself
                // moves nothing.
                (Some("fixed"), Clock::Device { .. }) => Clock::Fixed { at_ms: now },
                (Some("device"), Clock::Fixed { at_ms }) => Clock::Device {
                    offset_ms: at_ms - super::clock::device_now_ms(),
                },
                _ => return Err("clock.mode must be \"device\" or \"fixed\"".into()),
            };
            Ok(Delivery::Live)
        }
        ["clock", "offsetMs"] => match doc.clock {
            Clock::Device { .. } => {
                doc.clock = Clock::Device {
                    offset_ms: int(value, "clock.offsetMs")?,
                };
                Ok(Delivery::Live)
            }
            Clock::Fixed { .. } => {
                Err("clock.offsetMs applies to the device clock; the clock is fixed".into())
            }
        },
        ["clock", "at"] => match doc.clock {
            Clock::Fixed { .. } => {
                let at_ms = int(value, "clock.at")?;
                if at_ms <= 0 {
                    return Err(format!(
                        "clock.at = {at_ms} is not a positive epoch millisecond"
                    ));
                }
                doc.clock = Clock::Fixed { at_ms };
                Ok(Delivery::Live)
            }
            Clock::Device { .. } => {
                Err("clock.at applies to the fixed clock; the clock runs on device time".into())
            }
        },
        ["policies", "staminaRefresh"] => {
            doc.stamina_refresh = document::parse_stamina_refresh(value)?;
            Ok(Delivery::Live)
        }
        ["userMysekaiGamedata", "mysekaiRank"] => {
            let rank = int32(value, "userMysekaiGamedata.mysekaiRank")?;
            let exp = masters.exp_of(rank).ok_or_else(|| match masters.ranks {
                None => "userMysekaiGamedata.mysekaiRank needs the master rank table, which this root lacks".to_owned(),
                Some(_) => format!("userMysekaiGamedata.mysekaiRank = {rank} is not a master rank"),
            })?;
            doc.gamedata.mysekai_rank = Some(rank);
            doc.gamedata.total_exp = exp;
            Ok(Delivery::NextResponse(SECTION_GAMEDATA))
        }
        ["userMysekaiGamedata", "totalExp"] => {
            let exp = int32(value, "userMysekaiGamedata.totalExp")?;
            if masters.ranks.is_none() {
                return Err("userMysekaiGamedata.totalExp needs the master rank table (the rank follows it), which this root lacks".into());
            }
            let rank = masters.rank_of(exp).ok_or_else(|| {
                format!("userMysekaiGamedata.totalExp = {exp} is below every master rank")
            })?;
            doc.gamedata.total_exp = exp;
            doc.gamedata.mysekai_rank = Some(rank);
            Ok(Delivery::NextResponse(SECTION_GAMEDATA))
        }
        ["userMysekaiStamina"] => {
            doc.stamina = Some(
                document::parse_stamina(value)?
                    .ok_or("userMysekaiStamina cannot be removed; set each pool")?,
            );
            Ok(Delivery::NextResponse(SECTION_STAMINA))
        }
        ["userMysekaiStamina", pool] => {
            let amount = int32(value, path)?;
            let stamina = doc.stamina.get_or_insert(Stamina {
                normal: 0,
                enhance: 0,
                boost: 0,
            });
            match *pool {
                "normalStamina" => stamina.normal = amount,
                "enhanceStamina" => stamina.enhance = amount,
                "boostStamina" => stamina.boost = amount,
                _ => return Err(format!("{path} is not a stamina pool")),
            }
            Ok(Delivery::NextResponse(SECTION_STAMINA))
        }
        ["userMysekaiColorfulPass"] => {
            doc.colorful_pass = document::parse_colorful_pass(value)?;
            Ok(Delivery::NextResponse(SECTION_PASS))
        }
        ["userMysekaiColorfulPass", key] => {
            let pass = doc.colorful_pass.as_mut().ok_or(
                "the user has no colorful pass; set userMysekaiColorfulPass to an object first",
            )?;
            match *key {
                "expiredAt" => pass.expired_at = int(value, path)?,
                "mysekaiColorfulPassId" => pass.id = int32(value, path)?,
                _ => return Err(format!("{path} is not a colorful pass field")),
            }
            Ok(Delivery::NextResponse(SECTION_PASS))
        }
        ["phenomenaSchedulePolicy"] => {
            doc.schedule_policy = document::parse_schedule_policy(value)?;
            Ok(Delivery::Live)
        }
        ["phenomenaSchedulePolicy", "kind"] => {
            doc.schedule_policy = match (value.as_str(), &doc.schedule_policy) {
                (Some("stated"), SchedulePolicy::Daily(_)) => SchedulePolicy::Stated,
                (Some("stated"), SchedulePolicy::Stated) => SchedulePolicy::Stated,
                (Some("daily"), SchedulePolicy::Daily(map)) => SchedulePolicy::Daily(map.clone()),
                (Some("daily"), SchedulePolicy::Stated) => {
                    // Each window takes the phenomenon of its first stated row.
                    let mut map = std::collections::BTreeMap::new();
                    for row in &doc.schedules {
                        map.entry(row.refresh_time_period_id)
                            .or_insert(row.phenomena_id);
                    }
                    if map.is_empty() {
                        return Err("phenomenaSchedulePolicy.kind daily needs byRefreshTimePeriod; set phenomenaSchedulePolicy as a whole".into());
                    }
                    SchedulePolicy::Daily(map)
                }
                _ => {
                    return Err(
                        "phenomenaSchedulePolicy.kind must be \"stated\" or \"daily\"".into(),
                    )
                }
            };
            Ok(Delivery::Live)
        }
        ["phenomenaSchedulePolicy", "byRefreshTimePeriod", period] => {
            let SchedulePolicy::Daily(map) = &mut doc.schedule_policy else {
                return Err("phenomenaSchedulePolicy.byRefreshTimePeriod applies to the daily policy; the schedule is stated".into());
            };
            let period = period
                .parse::<i32>()
                .map_err(|_| format!("{path}: {period} is not a refresh window id"))?;
            map.insert(period, int32(value, path)?);
            Ok(Delivery::Live)
        }
        ["mysekaiPhenomenaSchedules"] => {
            let rows = value
                .as_array()
                .ok_or("mysekaiPhenomenaSchedules must be an array of schedule rows")?;
            doc.schedules = document::parse_schedules(rows, "mysekaiPhenomenaSchedules")?;
            Ok(Delivery::Live)
        }
        ["mysekaiPhenomenaSchedules", index, key] => {
            let index = index
                .parse::<usize>()
                .map_err(|_| format!("{path}: {index} is not a row index"))?;
            let len = doc.schedules.len();
            let row = doc
                .schedules
                .get_mut(index)
                .ok_or_else(|| format!("{path}: the schedule has {len} rows"))?;
            match *key {
                "mysekaiPhenomenaId" => row.phenomena_id = int32(value, path)?,
                "mysekaiRefreshTimePeriodId" => row.refresh_time_period_id = int32(value, path)?,
                "scheduleDate" => row.schedule_date = int(value, path)?,
                _ => return Err(format!("{path} is not a schedule row field")),
            }
            Ok(Delivery::Live)
        }
        _ => Err(format!(
            "{path} is not an editable field of the server document"
        )),
    }
}

fn group_ids(value: Option<&Value>, what: &str) -> Result<Vec<i32>, String> {
    value
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{what} must be an array of unit group ids"))?
        .iter()
        .map(|id| int32(id, what))
        .collect()
}

fn only(command: &Map<String, Value>, allowed: &[&str]) -> Result<(), String> {
    match command.keys().find(|key| !allowed.contains(&key.as_str())) {
        Some(key) => Err(format!("server.edit field {key} is unknown")),
        None => Ok(()),
    }
}

/// The reply's talk list, as the document holds it.
fn reply_talks(doc: &ServerDocument) -> ReplyTalkList {
    match &doc.talk_histories {
        Some(Value::Array(rows)) => ReplyTalkList::Stated(
            rows.iter()
                .filter_map(|row| {
                    Some((
                        i32::try_from(row["mysekaiCharacterTalkId"].as_i64()?).ok()?,
                        row["isRead"].as_bool()?,
                    ))
                })
                .collect(),
        ),
        _ => ReplyTalkList::Policy,
    }
}

impl ServerModel {
    /// Validates and applies one `server.edit`. Returns a receipt.
    pub(crate) fn edit(&mut self, command: &Map<String, Value>) -> Result<Value, String> {
        if !self.joined {
            return Err(
                "server.edit is refused until the server has joined the client (masters loading)"
                    .into(),
            );
        }
        let now = self.now_ms();
        if let Some(action) = command.get("action") {
            let action = action
                .as_str()
                .ok_or("server.edit action must be a string")?;
            return self.action(action, command, now);
        }
        only(command, &["type", "path", "value"])?;
        let path = command
            .get("path")
            .and_then(Value::as_str)
            .ok_or("server.edit needs a string path or an action")?;
        let value = command
            .get("value")
            .ok_or_else(|| format!("server.edit {path} has no value"))?;
        let mut next = self.doc.clone();
        let delivery = edit_path(&mut next, &self.masters, now, path, value)
            .map_err(|reason| format!("server.edit {path}: {reason}"))?;
        next.check_structure()
            .and_then(|()| check_masters(&next, &self.masters))
            .map_err(|reason| format!("server.edit {path}: {reason}"))?;
        self.doc = next;
        match delivery {
            Delivery::Live => {
                self.commit();
                self.live_update();
            }
            Delivery::NextResponse(section) => {
                self.mark_pending(section);
                self.commit();
            }
        }
        info!("[server] server.edit {path} = {value} accepted ({delivery:?})");
        Ok(json!({
            "path": path,
            "delivery": match delivery {
                Delivery::Live => "live",
                Delivery::NextResponse(_) => "next-response",
            },
            "revision": self.revision,
        }))
    }

    fn action(
        &mut self,
        action: &str,
        command: &Map<String, Value>,
        now: i64,
    ) -> Result<Value, String> {
        match action {
            "sync" => {
                only(command, &["type", "action"])?;
                let pending = self.pending.clone();
                self.respond(ResponseKind::Sync, false, &[]);
                Ok(json!({"action": action, "delivered": pending, "revision": self.revision}))
            }
            "gate.reserve" => {
                only(
                    command,
                    &["type", "action", "mysekaiGameCharacterUnitGroupId"],
                )?;
                let group = int32(
                    command
                        .get("mysekaiGameCharacterUnitGroupId")
                        .unwrap_or(&Value::Null),
                    "gate.reserve mysekaiGameCharacterUnitGroupId",
                )?;
                let row = GateCharacter {
                    gate_id: self.doc.gate.gate_id,
                    unit_group_id: group,
                    is_reservation: true,
                    visit_count: 1,
                };
                let mut next = self.doc.clone();
                next.gate_characters.push(row);
                next.check_structure()
                    .and_then(|()| check_masters(&next, &self.masters))
                    .map_err(|reason| format!("server.edit gate.reserve: {reason}"))?;
                self.doc = next;
                let talks = reply_talks(&self.doc);
                self.gate_replies.push(GateReply::Reserve { row, talks });
                self.respond(ResponseKind::GateReserve, false, &[]);
                Ok(json!({"action": action, "row": {
                    "mysekaiGateId": row.gate_id,
                    "mysekaiGameCharacterUnitGroupId": row.unit_group_id,
                    "isReservation": true,
                    "visitCount": row.visit_count,
                }, "revision": self.revision}))
            }
            "gate.change" => {
                only(
                    command,
                    &[
                        "type",
                        "action",
                        "mysekaiGateId",
                        "mysekaiGateSkinId",
                        "mysekaiGameCharacterUnitGroupIds",
                    ],
                )?;
                let gate_id = int32(
                    command.get("mysekaiGateId").unwrap_or(&Value::Null),
                    "gate.change mysekaiGateId",
                )?;
                let skin_id = int32(
                    command.get("mysekaiGateSkinId").unwrap_or(&Value::Null),
                    "gate.change mysekaiGateSkinId",
                )?;
                let groups = group_ids(
                    command.get("mysekaiGameCharacterUnitGroupIds"),
                    "gate.change mysekaiGameCharacterUnitGroupIds",
                )?;
                if self.masters.gates.is_none() {
                    return Err("server.edit gate.change needs the master gate tables, which this root lacks".into());
                }
                let mut next = self.doc.clone();
                next.gate.gate_id = gate_id;
                next.gate.skin_id = skin_id;
                next.gate.last_refreshed_at = now;
                next.gate_characters = groups
                    .iter()
                    .map(|group| GateCharacter {
                        gate_id,
                        unit_group_id: *group,
                        is_reservation: false,
                        visit_count: 1,
                    })
                    .collect();
                next.check_structure()
                    .and_then(|()| check_masters(&next, &self.masters))
                    .map_err(|reason| format!("server.edit gate.change: {reason}"))?;
                self.doc = next;
                let talks = reply_talks(&self.doc);
                self.gate_replies.push(GateReply::Change {
                    gate_id,
                    skin_id,
                    rows: self.doc.gate_characters.clone(),
                    talks,
                });
                self.respond(ResponseKind::GateChange, false, &[]);
                Ok(
                    json!({"action": action, "mysekaiGateId": gate_id, "mysekaiGateSkinId": skin_id,
                    "mysekaiGameCharacterUnitGroupIds": groups, "revision": self.revision}),
                )
            }
            other => Err(format!("server.edit action {other} is unknown")),
        }
    }
}

// ---------------------------------------------------------------------------
// Schema
// ---------------------------------------------------------------------------

fn field(path: &str, kind: &str, extra: Value) -> Value {
    let mut value = json!({"path": path, "type": kind});
    if let (Some(target), Some(extra)) = (value.as_object_mut(), extra.as_object()) {
        for (key, entry) in extra {
            target.insert(key.clone(), entry.clone());
        }
    }
    value
}

/// The masters part of the schema.
fn masters_value(masters: &Masters) -> Value {
    json!({
        "refreshTimePeriods": masters.periods.as_ref().map(|periods| periods.iter().map(|period| json!({
            "id": period.id, "startHour": period.start_hour, "endHour": period.end_hour,
        })).collect::<Vec<_>>()),
        "phenomena": masters.phenomena.as_ref().map(|rows| rows.iter().map(|(id, name)| json!({"id": id, "name": name})).collect::<Vec<_>>()),
        "maxStamina": masters.stamina_max.map(|max| json!({
            "normal": max.normal, "enhance": max.enhance, "boost": max.boost,
        })),
        "recoveryBoostStamina": masters.boost_grant,
        "ranks": masters.ranks.as_ref().map(|rows| rows.iter().map(|(rank, exp)| json!({"mysekaiRank": rank, "totalExp": exp})).collect::<Vec<_>>()),
        "gates": masters.gates.as_ref().map(|(gates, _)| gates.clone()),
        "gateSkins": masters.gates.as_ref().map(|(_, skins)| skins.clone()),
        "musicRecords": masters.music_records.as_ref().map(Vec::len),
        "configs": masters.configs.as_ref().map(|configs| configs.len()),
        "avatar": {
            "costumes": masters.avatar.costumes.as_ref().map(|rows| rows.len()),
            "accessories": masters.avatar.accessories.as_ref().map(|rows| rows.len()),
            "skinColors": masters.avatar.skin_colors.as_ref().map(|rows| rows.len()),
            "coordinates": masters.avatar.coordinates.as_ref().map(|rows| rows.len()),
        },
        "craft": {
            "blueprints": masters.craft.blueprints.as_ref().map(|rows| rows.len()),
            "costBlueprints": masters.craft.costs.as_ref().map(|rows| rows.len()),
            "terms": masters.craft.terms.as_ref().map(Vec::len),
            "firstCraftBonus": masters.craft.first_craft_bonus,
            "materials": masters.craft.material_types.as_ref().map(|rows| rows.len()),
            "whiteBlueprintItem": masters.craft.white_blueprint_item,
        },
        "possession": {
            "fixture": masters.possession.fixture.as_ref().map(|rows| rows.iter().map(|row| json!({"level": row.level, "possessionLimit": row.possession_limit})).collect::<Vec<_>>()),
            "material": masters.possession.material.as_ref().map(|rows| rows.iter().map(|row| json!({"level": row.level, "possessionLimit": row.possession_limit})).collect::<Vec<_>>()),
        },
        "missing": masters.missing,
    })
}

fn sections(masters: &Masters) -> Value {
    let max = masters.stamina_max;
    let ranks = masters.ranks.as_ref();
    json!([
        {
            "key": "clock",
            "title": "Server clock",
            "delivery": "live",
            "fields": [
                field("clock.mode", "enum", json!({"values": ["device", "fixed"]})),
                field("clock.offsetMs", "int", json!({"when": {"clock.mode": "device"}, "unit": "ms"})),
                field("clock.at", "epoch-ms", json!({"when": {"clock.mode": "fixed"}})),
            ],
        },
        {
            "key": "policies",
            "title": "Server policies",
            "delivery": "live",
            "fields": [
                field("policies.staminaRefresh", "enum", json!({"values": [
                    {"value": document::REFILL_POLICY, "label": document::REFILL_POLICY_TEXT},
                    {"value": document::NO_REFILL_POLICY, "label": "no automatic refill"},
                ]})),
            ],
        },
        {
            "key": SECTION_GAMEDATA,
            "title": "Game data",
            "delivery": "next-response",
            "fields": [
                field("userMysekaiGamedata.mysekaiRank", "int", json!({
                    "min": ranks.and_then(|rows| rows.iter().map(|(rank, _)| *rank).min()),
                    "max": ranks.and_then(|rows| rows.iter().map(|(rank, _)| *rank).max()),
                    "sets": "userMysekaiGamedata.totalExp",
                })),
                field("userMysekaiGamedata.totalExp", "int", json!({"min": 0, "sets": "userMysekaiGamedata.mysekaiRank"})),
                field("userMysekaiGamedata.refreshedAt", "epoch-ms", json!({"readOnly": true, "by": "the stamina refresh policy's clock"})),
                field("userMysekaiGamedata.isMysekaiTutorialEnd", "bool", json!({"readOnly": true, "by": "the tutorial is out of scope"})),
            ],
        },
        {
            "key": SECTION_STAMINA,
            "title": "Stamina",
            "delivery": "next-response",
            "fields": [
                field("userMysekaiStamina.normalStamina", "int", json!({"min": 0, "max": max.map(|m| m.normal)})),
                field("userMysekaiStamina.enhanceStamina", "int", json!({"min": 0, "max": max.map(|m| m.enhance)})),
                field("userMysekaiStamina.boostStamina", "int", json!({"min": 0, "max": max.map(|m| m.boost)})),
            ],
        },
        {
            "key": SECTION_PASS,
            "title": "Colorful pass",
            "delivery": "next-response",
            "fields": [
                field("userMysekaiColorfulPass", "object-or-null", json!({"object": {"mysekaiColorfulPassId": "int", "expiredAt": "epoch-ms"}})),
                field("userMysekaiColorfulPass.expiredAt", "epoch-ms", json!({"when": {"userMysekaiColorfulPass": "object"}})),
            ],
        },
        {
            "key": "phenomenaSchedulePolicy",
            "title": "Phenomenon schedule",
            "delivery": "live",
            "fields": [
                field("phenomenaSchedulePolicy.kind", "enum", json!({"values": ["daily", "stated"]})),
                field("phenomenaSchedulePolicy.byRefreshTimePeriod.<refreshTimePeriodId>", "phenomenon", json!({"when": {"phenomenaSchedulePolicy.kind": "daily"}})),
                field("mysekaiPhenomenaSchedules", "rows", json!({"when": {"phenomenaSchedulePolicy.kind": "stated"},
                    "row": {"mysekaiRefreshTimePeriodId": "refreshTimePeriod", "scheduleDate": "epoch-ms", "mysekaiPhenomenaId": "phenomenon"}})),
                field("mysekaiPhenomenaSchedules.<index>.mysekaiPhenomenaId", "phenomenon", json!({"when": {"phenomenaSchedulePolicy.kind": "stated"}})),
            ],
        },
    ])
}

fn actions() -> Value {
    json!([
        {"action": "gate.reserve", "title": "Invite a visitor (reserve reply)", "fields": [
            {"name": "mysekaiGameCharacterUnitGroupId", "type": "int", "min": 1}]},
        {"action": "gate.change", "title": "Change the gate (change reply)", "fields": [
            {"name": "mysekaiGateId", "type": "gate"},
            {"name": "mysekaiGateSkinId", "type": "gateSkin", "min": 0},
            {"name": "mysekaiGameCharacterUnitGroupIds", "type": "int-list"}]},
        {"action": "sync", "title": "Deliver pending sections now", "fields": [],
            "note": "stands in for the next request the client makes; the source has no push"},
    ])
}

fn policies() -> Value {
    json!([
        {"name": document::REFILL_POLICY_TEXT, "key": "policies.staminaRefresh", "rule": "when the server clock enters a master refresh window (phenomena index refreshTimePeriods, device local time) later than refreshedAt, the server stamps refreshedAt with its clock, sets the normal pool to the master normal maxStamina, and the response says isRefreshed"},
        {"name": "initial stamina", "rule": "a document without stamina gets normal at the master maximum, enhance 0 and boost of one recovery grant"},
        {"name": "rank from experience", "rule": "mysekaiRank is the master rank of totalExp; editing either sets the other"},
        {"name": "harvest stamina", "rule": "the harvest reply carries the rests the client sent, unless a panel stamina edit is pending"},
        {"name": "gate reserve", "rule": "the unit group joins the home gate's characters (isReservation true, visitCount 1)"},
        {"name": "gate change", "rule": "the home gate takes the gate and skin; the unit groups become its characters (isReservation false, visitCount 1)"},
        {"name": "daily schedule", "rule": "rows for the local day before, of and after the server clock, one per refresh window, schedule date at local midnight"},
    ])
}

/// The document's sections: the core ones, the delivery's, the home
/// actions', the music settings' and the avatar's.
fn other_sections(core: Value, delivery: Value) -> Value {
    [
        delivery,
        super::home_action::schema_sections(),
        super::music::schema_sections(),
        super::avatar::schema_sections(),
        super::inventory::schema_sections(),
        super::craft::schema_sections(),
    ]
    .into_iter()
    .fold(core, concat)
}

/// Two JSON arrays as one.
fn concat(first: Value, second: Value) -> Value {
    let mut out = match first {
        Value::Array(rows) => rows,
        other => vec![other],
    };
    if let Value::Array(rows) = second {
        out.extend(rows);
    }
    Value::Array(out)
}

pub(crate) fn schema(model: &ServerModel) -> Value {
    json!({
        "schemaVersion": document::SCHEMA_VERSION,
        "command": "server.edit",
        "sections": other_sections(sections(&model.masters), super::delivery::schema_sections(Some(model))),
        "actions": actions(),
        "policies": concat(concat(concat(concat(policies(), super::delivery::schema_policies()), super::home_action::schema_policies()), super::inventory::schema_policies()), super::craft::schema_policies()),
        "masters": masters_value(&model.masters),
        "joined": model.joined,
    })
}

pub(crate) fn schema_without_model() -> Value {
    json!({
        "schemaVersion": document::SCHEMA_VERSION,
        "command": "server.edit",
        "sections": other_sections(sections(&Masters::default()), super::delivery::schema_sections(None)),
        "actions": actions(),
        "policies": concat(concat(concat(concat(policies(), super::delivery::schema_policies()), super::home_action::schema_policies()), super::inventory::schema_policies()), super::craft::schema_policies()),
        "masters": null,
        "joined": false,
    })
}

#[cfg(test)]
mod tests {
    use super::super::clock::RefreshPeriod;
    use super::super::{Origin, StaminaMax};
    use super::*;

    fn joined() -> ServerModel {
        let doc = document::migrate_v1(
            include_str!("../server_panel/npc.json"),
            document::Migration::CheckedInDefault,
        )
        .unwrap();
        let mut model = ServerModel::new(doc, Origin::CheckedInDefault, Vec::new(), false);
        model.masters.periods = Some(vec![
            RefreshPeriod {
                id: 1,
                start_hour: 5,
                end_hour: 17,
            },
            RefreshPeriod {
                id: 2,
                start_hour: 17,
                end_hour: 29,
            },
        ]);
        model.masters.stamina_max = Some(StaminaMax {
            normal: 1000,
            enhance: 1000,
            boost: 10000,
        });
        model.masters.boost_grant = Some(100);
        model.masters.ranks = Some(vec![(1, 0), (2, 48_000), (3, 172_300), (4, 250_000)]);
        model.masters.phenomena = Some(vec![(1, "a".into()), (2, "b".into())]);
        model.masters.gates = Some((vec![1, 2], vec![5]));
        model.masters_ready = true;
        model.join();
        model
    }

    fn edit(model: &mut ServerModel, text: &str) -> Result<Value, String> {
        let value: Value = serde_json::from_str(text).unwrap();
        model.edit(value.as_object().unwrap())
    }

    #[test]
    fn stamina_above_the_master_maximum_is_refused_by_name() {
        let mut model = joined();
        let error = edit(
            &mut model,
            r#"{"type":"server.edit","path":"userMysekaiStamina.normalStamina","value":1001}"#,
        )
        .unwrap_err();
        assert!(
            error.contains("normalStamina = 1001 is above the master maxStamina 1000"),
            "{error}"
        );
    }

    #[test]
    fn a_stamina_edit_waits_for_the_next_response() {
        let mut model = joined();
        model.responses.clear();
        edit(
            &mut model,
            r#"{"type":"server.edit","path":"userMysekaiStamina.normalStamina","value":10}"#,
        )
        .unwrap();
        assert!(model.responses.is_empty());
        assert_eq!(model.pending, vec![SECTION_STAMINA.to_owned()]);
        edit(&mut model, r#"{"type":"server.edit","action":"sync"}"#).unwrap();
        assert_eq!(model.responses.last().unwrap().stamina.unwrap().normal, 10);
    }

    #[test]
    fn a_rank_edit_sets_the_experience() {
        let mut model = joined();
        edit(
            &mut model,
            r#"{"type":"server.edit","path":"userMysekaiGamedata.mysekaiRank","value":4}"#,
        )
        .unwrap();
        assert_eq!(model.doc.gamedata.total_exp, 250_000);
        let error = edit(
            &mut model,
            r#"{"type":"server.edit","path":"userMysekaiGamedata.mysekaiRank","value":9}"#,
        )
        .unwrap_err();
        assert!(error.contains("9 is not a master rank"), "{error}");
    }

    #[test]
    fn an_unknown_phenomenon_is_refused() {
        let mut model = joined();
        let error = edit(&mut model, r#"{"type":"server.edit","path":"phenomenaSchedulePolicy.byRefreshTimePeriod.1","value":7}"#).unwrap_err();
        assert!(error.contains("= 7 is not a master phenomenon"), "{error}");
    }

    #[test]
    fn a_gate_change_replies_and_persists_the_gate() {
        let mut model = joined();
        edit(&mut model, r#"{"type":"server.edit","action":"gate.change","mysekaiGateId":2,"mysekaiGateSkinId":0,"mysekaiGameCharacterUnitGroupIds":[4,5]}"#).unwrap();
        assert_eq!(model.doc.gate.gate_id, 2);
        assert_eq!(model.doc.gate_characters.len(), 2);
        assert!(matches!(
            model.gate_replies.last(),
            Some(GateReply::Change {
                gate_id: 2,
                talks: ReplyTalkList::Policy,
                ..
            })
        ));
        let error = edit(&mut model, r#"{"type":"server.edit","action":"gate.change","mysekaiGateId":3,"mysekaiGateSkinId":0,"mysekaiGameCharacterUnitGroupIds":[]}"#).unwrap_err();
        assert!(error.contains("3 is not a master gate"), "{error}");
    }

    #[test]
    fn a_repeated_reservation_is_refused() {
        let mut model = joined();
        let error = edit(
            &mut model,
            r#"{"type":"server.edit","action":"gate.reserve","mysekaiGameCharacterUnitGroupId":1}"#,
        )
        .unwrap_err();
        assert!(error.contains("repeats unit group 1"), "{error}");
    }

    #[test]
    fn an_unknown_path_is_refused() {
        let mut model = joined();
        let error = edit(
            &mut model,
            r#"{"type":"server.edit","path":"userMysekaiSiteHousingLayouts","value":[]}"#,
        )
        .unwrap_err();
        assert!(error.contains("is not an editable field"), "{error}");
    }
}
