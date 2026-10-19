//! The director tab: the bundled example scripts, pause, resume and stop,
//! and the run's status, current steps and log, all through
//! `moly.director.*`.

use super::widgets::{button, card, row, text, Disabled, Kind, BAD, INK, MUTED};
use super::{ControlPanel, PanelAction, Reply as PanelReply};
use bevy::prelude::*;
use bevy_remote::BrpResult;
use serde_json::{json, Value};

/// Log records shown, newest first.
const SHOWN_RECORDS: usize = 40;
/// How often the snapshot is read while the tab is shown, in seconds.
const SNAPSHOT_EVERY: f64 = 0.25;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Runs the example of this index.
    Run(usize),
    Pause,
    Resume,
    Stop,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Reply {
    Examples,
    Snapshot,
    Command,
}

#[derive(Default)]
pub struct View {
    /// `(title, script)` of each example, once read.
    examples: Option<Vec<(String, Value)>>,
    snapshot: Value,
    read_at: f64,
}

impl View {
    fn status(&self) -> &str {
        self.snapshot["status"].as_str().unwrap_or("idle")
    }

    fn allows(&self, action: Action) -> bool {
        match action {
            Action::Run(_) => true,
            Action::Pause => self.status() == "running",
            Action::Resume => self.status() == "paused",
            Action::Stop => matches!(self.status(), "running" | "paused" | "queued"),
        }
    }
}

pub(super) fn enter(panel: &mut ControlPanel, now: f64) {
    if panel.director.examples.is_none() && !panel.waiting(PanelReply::Director(Reply::Examples)) {
        panel.send(PanelReply::Director(Reply::Examples), "moly.director.examples", None, now);
    }
    tick(panel, now);
}

pub(super) fn tick(panel: &mut ControlPanel, now: f64) {
    if now - panel.director.read_at >= SNAPSHOT_EVERY && !panel.waiting(PanelReply::Director(Reply::Snapshot)) {
        panel.director.read_at = now;
        panel.send(PanelReply::Director(Reply::Snapshot), "moly.director.snapshot", None, now);
    }
}

pub(super) fn act(panel: &mut ControlPanel, action: Action, now: f64) {
    let command = match action {
        Action::Run(index) => {
            let Some((_, script)) = panel.director.examples.as_ref().and_then(|examples| examples.get(index)) else {
                return;
            };
            json!({ "run": script })
        }
        Action::Pause => json!({ "pause": true }),
        Action::Resume => json!({ "resume": true }),
        Action::Stop => json!({ "stop": true }),
    };
    panel.send(PanelReply::Director(Reply::Command), "moly.director.command", Some(command), now);
    // A run drives the player through the field's own input: the drawer
    // makes way so its taps reach the field.
    if matches!(action, Action::Run(_) | Action::Resume) {
        panel.close();
    }
}

pub(super) fn answer(panel: &mut ControlPanel, reply: Reply, result: BrpResult) -> bool {
    let Ok(value) = result else {
        return reply != Reply::Snapshot;
    };
    match reply {
        Reply::Examples => {
            let examples = value
                .as_array()
                .map(|examples| {
                    examples
                        .iter()
                        .map(|example| {
                            (example["title"].as_str().unwrap_or_default().to_owned(), example["script"].clone())
                        })
                        .collect()
                })
                .unwrap_or_default();
            panel.director.examples = Some(examples);
            true
        }
        Reply::Snapshot => {
            let changed = panel.director.snapshot != value;
            panel.director.snapshot = value;
            changed
        }
        Reply::Command => false,
    }
}

fn status_word(status: &str) -> &str {
    match status {
        "idle" => "空闲",
        "queued" => "等待下一帧",
        "running" => "运行中",
        "paused" => "已暂停",
        "done" => "已完成",
        "failed" => "失败",
        "stopped" => "已停止",
        other => other,
    }
}

fn strings(value: &Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| items.iter().map(|item| item.as_str().map_or_else(|| item.to_string(), str::to_owned)).collect())
        .unwrap_or_default()
}

pub(super) fn build(c: &mut Commands, body: Entity, font: &Handle<Font>, panel: &ControlPanel) {
    let view = &panel.director;
    let examples = card(c, body, font, Some("示例脚本"));
    match &view.examples {
        None => {
            text(c, examples, font, "正在读取…", 12., MUTED);
        }
        Some(list) => {
            let chips = row(c, examples, 8.);
            for (index, (title, _)) in list.iter().enumerate() {
                button(c, chips, font, title, PanelAction::Director(Action::Run(index)), Kind::Chip);
            }
            text(c, examples, font, "运行或继续时面板会收起，按 F9 回来看进度。", 11., MUTED);
        }
    }
    let run = card(c, body, font, Some("当前运行"));
    let snapshot = &view.snapshot;
    let status = view.status();
    let mut line = status_word(status).to_owned();
    if let Some(script) = snapshot["script"].as_str() {
        line += &format!(" · {script}");
    }
    if status != "idle" {
        line += &format!(" · {:.1} 秒", snapshot["elapsed"].as_f64().unwrap_or(0.));
    }
    text(c, run, font, line, 14., INK);
    let steps = strings(&snapshot["current"]);
    if !steps.is_empty() {
        text(c, run, font, format!("正在：{}", steps.join(" · ")), 12., MUTED);
    }
    let owned = strings(&snapshot["owned"]);
    if !owned.is_empty() {
        text(c, run, font, format!("接管角色 {}", owned.join("、")), 12., MUTED);
    }
    if let Some(error) = snapshot["error"].as_str() {
        text(c, run, font, format!("错误：{error}"), 12., BAD);
    }
    let controls = row(c, run, 8.);
    for (title, action, kind) in [
        ("暂停", Action::Pause, Kind::Plain),
        ("继续", Action::Resume, Kind::Plain),
        ("停止", Action::Stop, Kind::Danger),
    ] {
        let id = button(c, controls, font, title, PanelAction::Director(action), kind);
        if !view.allows(action) {
            c.entity(id).insert(Disabled);
        }
    }
    let log = card(c, body, font, Some("日志（新的在上）"));
    let records = snapshot["log"].as_array().map(Vec::as_slice).unwrap_or_default();
    if records.is_empty() {
        text(c, log, font, "还没有记录。", 12., MUTED);
    }
    for record in records.iter().rev().take(SHOWN_RECORDS) {
        let mut line = format!(
            "{:>6.1}  {}  {}",
            record["t"].as_f64().unwrap_or(0.),
            record["step"].as_str().unwrap_or_default(),
            record["event"].as_str().unwrap_or_default(),
        );
        if let Some(note) = record["note"].as_str().filter(|note| !note.is_empty()) {
            line += &format!("  {note}");
        }
        text(c, log, font, line, 11., MUTED);
    }
}
