//! The server tab: the server document's sections as `moly.server.schema`
//! names them, with the document's values from `moly.server.document`, and
//! edits sent as `moly.server.edit`. Every check of a value is the
//! server's; a refused edit shows its refusal in the footer.
//!
//! Enums and master-backed ids are chips, integers and times are steppers;
//! row tables and nullable objects are shown, and edited by an external
//! client.

use super::widgets::{button, card, pair, row, text, Disabled, Kind, Selected, ACCENT, INK, MUTED};
use super::{ControlPanel, PanelAction, Reply as PanelReply};
use bevy::prelude::*;
use bevy_remote::BrpResult;
use serde_json::{json, Value};

/// How often the document is read again while the tab is shown, in seconds.
const READ_EVERY: f64 = 2.;
const HOUR_MS: i64 = 3_600_000;
const DAY_MS: i64 = 24 * HOUR_MS;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Adds `by` to an integer field.
    Step { field: u16, by: i32, stamp: u32 },
    /// Moves a time field by `by` milliseconds.
    Shift { field: u16, by: i64, stamp: u32 },
    /// Sets a time field to the server clock.
    Now { field: u16, stamp: u32 },
    /// Sets a field to one of its choices.
    Choose { field: u16, choice: u16, stamp: u32 },
    /// Runs a server action without fields.
    Run { action: u16 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Schema,
    Document,
    Edit,
}

#[derive(Default)]
pub struct View {
    schema: Value,
    read: Value,
    read_at: f64,
}

/// One drawn field: a schema field with its placeholders expanded.
struct Field {
    path: String,
    label: String,
    kind: String,
    spec: Value,
    value: Value,
}

impl View {
    fn document(&self) -> &Value {
        &self.read["document"]
    }

    /// The document revision a drawn control belongs to.
    fn stamp(&self) -> u32 {
        self.read["revision"].as_u64().unwrap_or(0) as u32
    }

    fn masters(&self) -> &Value {
        &self.schema["masters"]
    }

    /// The fields of every section, in order, for the current document.
    fn fields(&self) -> Vec<(usize, Field)> {
        let mut out = Vec::new();
        for (section, entry) in self.schema["sections"].as_array().map(Vec::as_slice).unwrap_or_default().iter().enumerate() {
            for field in entry["fields"].as_array().map(Vec::as_slice).unwrap_or_default() {
                expand(field, self.document(), self.masters(), &mut |field| out.push((section, field)));
            }
        }
        out
    }

    /// `(value, label)` choices of a field, or `None` when it has none (or
    /// its master is absent).
    fn choices(&self, field: &Field) -> Option<Vec<(Value, String)>> {
        let masters = self.masters();
        let ids = |key: &str| -> Option<Vec<(Value, String)>> {
            Some(masters[key].as_array()?.iter().map(|id| (id.clone(), id.to_string())).collect())
        };
        match field.kind.as_str() {
            "enum" => Some(
                field.spec["values"]
                    .as_array()?
                    .iter()
                    .map(|entry| match entry {
                        Value::String(value) => (entry.clone(), value.clone()),
                        _ => (entry["value"].clone(), entry["label"].as_str().map_or_else(|| entry["value"].to_string(), str::to_owned)),
                    })
                    .collect(),
            ),
            "phenomenon" => Some(
                masters["phenomena"]
                    .as_array()?
                    .iter()
                    .map(|row| (row["id"].clone(), format!("{} {}", row["id"], row["name"].as_str().unwrap_or_default()).trim().to_owned()))
                    .collect(),
            ),
            "refreshTimePeriod" => Some(
                masters["refreshTimePeriods"]
                    .as_array()?
                    .iter()
                    .map(|row| (row["id"].clone(), format!("{} ({}-{} h)", row["id"], row["startHour"], row["endHour"])))
                    .collect(),
            ),
            "gate" => ids("gates"),
            "gateSkin" => {
                let mut choices = ids("gateSkins")?;
                if field.spec["min"] == 0 && !choices.iter().any(|(id, _)| *id == 0) {
                    choices.insert(0, (json!(0), "0".into()));
                }
                Some(choices)
            }
            "bool" => Some(vec![(json!(true), "是".into()), (json!(false), "否".into())]),
            _ => None,
        }
    }
}

/// The value at a dotted path, array indexes as numbers.
fn value_at<'a>(root: &'a Value, path: &str) -> &'a Value {
    path.split('.').fold(root, |value, part| match value {
        Value::Array(items) => part.parse::<usize>().ok().and_then(|index| items.get(index)).unwrap_or(&Value::Null),
        _ => &value[part],
    })
}

/// A field's `when`: every named path holds its value ("object": any object).
fn holds(when: &Value, document: &Value) -> bool {
    let Some(when) = when.as_object() else {
        return true;
    };
    when.iter().all(|(path, expected)| {
        let value = value_at(document, path);
        if expected == "object" {
            value.is_object()
        } else {
            value == expected
        }
    })
}

/// The concrete fields of one schema field: `<refreshTimePeriodId>` over the
/// master windows (else the document's own keys), `<index>` over the
/// document's rows.
fn expand(field: &Value, document: &Value, masters: &Value, out: &mut dyn FnMut(Field)) {
    if !holds(&field["when"], document) {
        return;
    }
    let path = field["path"].as_str().unwrap_or_default();
    let kind = field["type"].as_str().unwrap_or_default().to_owned();
    let make = |path: String, label: String| Field {
        value: value_at(document, &path).clone(),
        path,
        label,
        kind: kind.clone(),
        spec: field.clone(),
    };
    if let Some(at) = path.find("<refreshTimePeriodId>") {
        let prefix = &path[..at];
        let periods: Vec<(String, String)> = match masters["refreshTimePeriods"].as_array() {
            Some(rows) => rows
                .iter()
                .map(|row| (row["id"].to_string(), format!(" ({}-{} h)", row["startHour"], row["endHour"])))
                .collect(),
            None => value_at(document, prefix.trim_end_matches('.'))
                .as_object()
                .map(|keys| keys.keys().map(|key| (key.clone(), String::new())).collect())
                .unwrap_or_default(),
        };
        for (id, hours) in periods {
            out(make(format!("{prefix}{id}"), format!("时段 {id}{hours}")));
        }
        return;
    }
    if let Some((prefix, suffix)) = path.split_once("<index>") {
        let rows = value_at(document, prefix.trim_end_matches('.')).as_array().map(Vec::len).unwrap_or(0);
        for index in 0..rows {
            let path = format!("{prefix}{index}{suffix}");
            out(make(path.clone(), path));
        }
        return;
    }
    out(make(path.to_owned(), path.to_owned()));
}

fn read_document(panel: &mut ControlPanel, now: f64) {
    if !panel.waiting(PanelReply::Server(Reply::Document)) {
        panel.server.read_at = now;
        panel.send(PanelReply::Server(Reply::Document), "moly.server.document", None, now);
    }
}

pub(super) fn enter(panel: &mut ControlPanel, now: f64) {
    if !panel.waiting(PanelReply::Server(Reply::Schema)) {
        panel.send(PanelReply::Server(Reply::Schema), "moly.server.schema", None, now);
    }
    read_document(panel, now);
}

pub(super) fn tick(panel: &mut ControlPanel, now: f64) {
    if now - panel.server.read_at >= READ_EVERY {
        read_document(panel, now);
    }
}

pub(super) fn act(panel: &mut ControlPanel, action: Action, now: f64) {
    let view = &panel.server;
    let stamp = match action {
        Action::Step { stamp, .. } | Action::Shift { stamp, .. } | Action::Now { stamp, .. } | Action::Choose { stamp, .. } => Some(stamp),
        Action::Run { .. } => None,
    };
    if stamp.is_some_and(|stamp| stamp != view.stamp()) {
        panel.footer = Some(("服务器文档刚更新过，请再点一次".into(), false));
        panel.changed();
        return;
    }
    let edit = match action {
        Action::Run { action } => {
            let Some(name) = view.schema["actions"][action as usize]["action"].as_str() else {
                return;
            };
            json!({ "action": name })
        }
        Action::Step { field, .. } | Action::Shift { field, .. } | Action::Now { field, .. } | Action::Choose { field, .. } => {
            let fields = view.fields();
            let Some((_, field_entry)) = fields.get(field as usize) else {
                return;
            };
            let value = match action {
                Action::Step { by, .. } => json!(field_entry.value.as_i64().unwrap_or(0) + i64::from(by)),
                Action::Shift { by, .. } => {
                    let base = field_entry.value.as_i64().or_else(|| view.read["serverNowMs"].as_i64()).unwrap_or(0);
                    json!(base + by)
                }
                Action::Now { .. } => view.read["serverNowMs"].clone(),
                Action::Choose { choice, .. } => match view.choices(field_entry).and_then(|choices| choices.into_iter().nth(choice as usize)) {
                    Some((value, _)) => value,
                    None => return,
                },
                Action::Run { .. } => unreachable!(),
            };
            json!({ "path": field_entry.path, "value": value })
        }
    };
    panel.send(PanelReply::Server(Reply::Edit), "moly.server.edit", Some(edit), now);
}

pub(super) fn answer(panel: &mut ControlPanel, reply: Reply, result: BrpResult) -> bool {
    let Ok(value) = result else {
        return false;
    };
    match reply {
        Reply::Schema => {
            let changed = panel.server.schema != value;
            panel.server.schema = value;
            changed
        }
        Reply::Document => {
            // The clock moves every read; only the document itself redraws.
            let changed = ["document", "pending", "revision", "joined", "errors"]
                .iter()
                .any(|key| panel.server.read[*key] != value[*key]);
            panel.server.read = value;
            changed
        }
        Reply::Edit => {
            panel.server.read_at = f64::NEG_INFINITY;
            false
        }
    }
}

/// Epoch milliseconds as a UTC date and time.
fn date(ms: i64) -> String {
    let days = ms.div_euclid(DAY_MS);
    let in_day = ms.rem_euclid(DAY_MS) / 1000;
    // Civil date from days since 1970-01-01 (proleptic Gregorian).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!("{year}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC", in_day / 3600, in_day / 60 % 60, in_day % 60)
}

fn show(value: &Value, kind: &str) -> String {
    match (value, kind) {
        (Value::Null, _) => "—".into(),
        (Value::Number(ms), "epoch-ms") => ms.as_i64().map_or_else(|| ms.to_string(), date),
        (Value::String(text), _) => text.clone(),
        (value, "rows" | "object-or-null" | "int-list") => {
            let text = value.to_string();
            if text.chars().count() > 160 {
                text.chars().take(160).collect::<String>() + "…"
            } else {
                text
            }
        }
        (value, _) => value.to_string(),
    }
}

pub(super) fn build(c: &mut Commands, body: Entity, font: &Handle<Font>, panel: &ControlPanel) {
    let view = &panel.server;
    if view.schema.is_null() || view.read.is_null() {
        text(c, body, font, "正在读取…", 12., MUTED);
        return;
    }
    if view.read["joined"] != true {
        text(c, body, font, "服务器还没接上客户端（主数据载入中），编辑会被拒绝。", 12., ACCENT);
    }
    let pending: Vec<String> = view.read["pending"].as_array().map(|items| items.iter().map(|item| show(item, "")).collect()).unwrap_or_default();
    if !pending.is_empty() {
        text(c, body, font, format!("待下发：{}（下一次请求带上，或点「立即下发」）", pending.join("、")), 12., ACCENT);
    }
    let actions = view.schema["actions"].as_array().map(Vec::as_slice).unwrap_or_default();
    let stamp = view.stamp();
    let sections = view.schema["sections"].as_array().map(Vec::as_slice).unwrap_or_default();
    let fields = view.fields();
    for (section, entry) in sections.iter().enumerate() {
        let drawn: Vec<(usize, &Field)> = fields
            .iter()
            .enumerate()
            .filter(|(_, (owner, _))| *owner == section)
            .map(|(index, (_, field))| (index, field))
            .collect();
        if drawn.is_empty() {
            continue;
        }
        let delivery = match entry["delivery"].as_str() {
            Some("live") => "即时生效",
            Some("next-response") => "下一次回应下发",
            _ => "",
        };
        let title = format!("{} · {delivery}", entry["title"].as_str().unwrap_or_default());
        let group = card(c, body, font, Some(&title));
        for (index, field) in drawn {
            let index = index as u16;
            pair(c, group, font, &field.label, show(&field.value, &field.kind));
            if field.spec["readOnly"] == true {
                if let Some(by) = field.spec["by"].as_str() {
                    text(c, group, font, format!("只读：{by}"), 11., MUTED);
                }
                continue;
            }
            let controls = row(c, group, 6.);
            if let Some(choices) = view.choices(field) {
                for (choice, (value, label)) in choices.iter().enumerate() {
                    let id = button(
                        c,
                        controls,
                        font,
                        label,
                        PanelAction::Server(Action::Choose { field: index, choice: choice as u16, stamp }),
                        Kind::Chip,
                    );
                    if *value == field.value {
                        c.entity(id).insert(Selected);
                    }
                }
                continue;
            }
            match field.kind.as_str() {
                "int" => {
                    let current = field.value.as_i64().unwrap_or(0);
                    for by in [-100, -10, -1, 1, 10, 100] {
                        let id = button(
                            c,
                            controls,
                            font,
                            &format!("{by:+}"),
                            PanelAction::Server(Action::Step { field: index, by, stamp }),
                            Kind::Chip,
                        );
                        let next = current + i64::from(by);
                        let out = field.spec["min"].as_i64().is_some_and(|min| next < min)
                            || field.spec["max"].as_i64().is_some_and(|max| next > max);
                        if out {
                            c.entity(id).insert(Disabled);
                        }
                    }
                }
                "epoch-ms" => {
                    for (label, by) in [("−1 天", -DAY_MS), ("−1 时", -HOUR_MS), ("+1 时", HOUR_MS), ("+1 天", DAY_MS)] {
                        button(c, controls, font, label, PanelAction::Server(Action::Shift { field: index, by, stamp }), Kind::Chip);
                    }
                    button(c, controls, font, "现在", PanelAction::Server(Action::Now { field: index, stamp }), Kind::Chip);
                }
                _ => {
                    text(c, controls, font, "经外部客户端编辑（moly.server.edit）", 11., MUTED);
                }
            }
        }
    }
    if !actions.is_empty() {
        let group = card(c, body, font, Some("服务器动作"));
        for (index, action) in actions.iter().enumerate() {
            let title = action["title"].as_str().unwrap_or_default();
            let takes_fields = action["fields"].as_array().is_some_and(|fields| !fields.is_empty());
            if takes_fields {
                pair(c, group, font, action["action"].as_str().unwrap_or_default(), format!("{title}（带参数，经外部客户端调用）"));
            } else {
                let label = if action["action"] == "sync" { "立即下发".to_owned() } else { title.to_owned() };
                button(c, group, font, &label, PanelAction::Server(Action::Run { action: index as u16 }), Kind::Plain);
            }
        }
    }
    let errors: Vec<String> = view.read["errors"].as_array().map(|items| items.iter().map(|item| show(item, "")).collect()).unwrap_or_default();
    if !errors.is_empty() {
        let group = card(c, body, font, Some("具名拒绝"));
        for error in errors {
            text(c, group, font, error, 11., INK);
        }
    }
}
