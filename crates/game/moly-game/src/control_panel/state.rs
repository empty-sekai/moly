//! The state tab: `moly.state` read twice a second while it is shown.

use super::widgets::{card, pair, text, MUTED};
use super::{ControlPanel, Reply};
use bevy::prelude::*;
use bevy_remote::BrpResult;
use serde_json::Value;

const READ_EVERY: f64 = 0.5;

#[derive(Default)]
pub(crate) struct View {
    read: Value,
    read_at: f64,
}

pub(super) fn enter(panel: &mut ControlPanel, now: f64) {
    panel.state.read_at = f64::NEG_INFINITY;
    tick(panel, now);
}

pub(super) fn tick(panel: &mut ControlPanel, now: f64) {
    if now - panel.state.read_at >= READ_EVERY && !panel.waiting(Reply::State) {
        panel.state.read_at = now;
        panel.send(Reply::State, "moly.state", None, now);
    }
}

pub(super) fn answer(panel: &mut ControlPanel, result: BrpResult) -> bool {
    let Ok(read) = result else {
        return false;
    };
    let changed = panel.state.read != read;
    panel.state.read = read;
    changed
}

/// A value as short text: strings bare, numbers to two places, arrays of
/// numbers as a tuple.
fn show(value: &Value) -> String {
    match value {
        Value::Null => "—".into(),
        Value::String(text) => text.clone(),
        Value::Number(number) if number.is_f64() => format!("{:.2}", number.as_f64().unwrap_or_default()),
        Value::Array(items) if items.iter().all(Value::is_number) => {
            let parts: Vec<String> = items.iter().map(show).collect();
            format!("({})", parts.join(", "))
        }
        other => other.to_string(),
    }
}

fn local_time(ms: &Value) -> String {
    let Some(ms) = ms.as_i64() else {
        return "—".into();
    };
    let seconds = ms.div_euclid(1000);
    let (hours, minutes, secs) = (seconds.div_euclid(3600).rem_euclid(24), seconds.div_euclid(60).rem_euclid(60), seconds.rem_euclid(60));
    format!("{hours:02}:{minutes:02}:{secs:02} UTC ({ms})")
}

pub(super) fn build(c: &mut Commands, body: Entity, font: &Handle<Font>, panel: &ControlPanel) {
    let read = &panel.state.read;
    if read.is_null() {
        text(c, body, font, "正在读取…", 12., MUTED);
        return;
    }
    let game = card(c, body, font, Some("游戏"));
    pair(c, game, font, "帧", show(&read["frame"]));
    let time = &read["time"];
    pair(
        c,
        game,
        font,
        "时间",
        format!(
            "{} 秒 · 倍速 {}{}",
            show(&time["elapsed"]),
            show(&time["scale"]),
            if time["paused"] == true { " · 已暂停" } else { "" }
        ),
    );
    pair(c, game, font, "服务器时钟", local_time(&read["server_clock_ms"]));
    pair(c, game, font, "入场", if read["entry_done"] == true { "完成" } else { "进行中" });
    pair(c, game, font, "换站", show(&read["site_moving"]));
    pair(c, game, font, "对话", show(&read["talk_active"]));
    let site = &read["site"];
    let place = card(c, body, font, Some("站点与天气"));
    pair(
        c,
        place,
        font,
        "站点",
        format!("{} · {} · id {} · 等级 {}", show(&site["site_type"]), show(&site["scene"]), show(&site["site_id"]), show(&site["level"])),
    );
    pair(c, place, font, "室内", show(&site["indoor"]));
    let weather = &read["weather"];
    pair(c, place, font, "天气", format!("{} {}", show(&weather["phenomenon"]), show(&weather["name"])));
    let player = &read["player"];
    let you = card(c, body, font, Some("玩家"));
    pair(c, you, font, "实体", show(&player["entity"]));
    pair(c, you, font, "位置", show(&player["position"]));
    pair(c, you, font, "状态", show(&player["state"]));
    let camera = &read["camera"];
    pair(
        c,
        you,
        font,
        "相机",
        format!("{} · 距离 {} · 偏航 {} · 俯仰 {}", show(&camera["state"]), show(&camera["distance"]), show(&camera["yaw"]), show(&camera["pitch"])),
    );
    let screens = &read["screens"];
    let ui = card(c, body, font, Some("界面"));
    pair(c, ui, font, "当前", show(&screens["current"]));
    let stack: Vec<String> = screens["stack"].as_array().map(|items| items.iter().map(show).collect()).unwrap_or_default();
    pair(c, ui, font, "返回栈", if stack.is_empty() { "—".into() } else { stack.join(" → ") });
    for dialog in screens["dialogs"].as_array().map(Vec::as_slice).unwrap_or_default() {
        pair(c, ui, font, "对话框", format!("{} · {} · {}", show(&dialog["dialog"]), show(&dialog["layer"]), show(&dialog["state"])));
    }
    let characters = read["characters"].as_array().map(Vec::as_slice).unwrap_or_default();
    let cast = card(c, body, font, Some(&format!("角色（{}）", characters.len())));
    for character in characters {
        let mut line = format!("{} · {}", show(&character["action"]), show(&character["objective"]));
        if character["away"] == true {
            line = "不在本站".into();
        } else if character["talking"] == true {
            line += " · 对话中";
        } else if character["tweeting"] == true {
            line += " · 气泡";
        }
        line += &format!(" · {}", show(&character["position"]));
        pair(c, cast, font, &format!("角色 {}", show(&character["unit"])), line);
    }
}
