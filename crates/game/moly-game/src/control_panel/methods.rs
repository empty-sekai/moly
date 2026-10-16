//! The methods tab: every method of the control surface (`rpc.discover`,
//! with the moly methods' descriptions from `moly.methods`). A method that
//! needs no params can be called from here and its answer read.

use super::widgets::{button, card, row, text, Kind, Selected, ACCENT, INK, MUTED};
use super::{ControlPanel, PanelAction, Reply as PanelReply};
use bevy::prelude::*;
use bevy_remote::BrpResult;
use serde_json::Value;

/// Characters of an answer shown.
const SHOWN_ANSWER: usize = 4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    Select(usize),
    Call,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Reply {
    Discover,
    Described,
    Called,
}

#[derive(Default)]
pub(crate) struct View {
    names: Option<Vec<String>>,
    /// `moly.methods`: name → (description, params schema).
    described: Vec<(String, String, Value)>,
    selected: Option<usize>,
    answer: Option<String>,
}

impl View {
    fn selected_name(&self) -> Option<&str> {
        self.names.as_ref()?.get(self.selected?).map(String::as_str)
    }

    /// Whether the method takes a call without params: the built-in
    /// listing ones, and moly methods whose schema requires nothing.
    fn callable(&self, name: &str) -> bool {
        match self.described.iter().find(|(described, ..)| described == name) {
            Some((_, _, params)) => params.get("required").is_none_or(|required| required.as_array().is_some_and(Vec::is_empty)),
            None => matches!(name, "rpc.discover" | "world.list_components" | "world.list_resources" | "registry.schema"),
        }
    }
}

pub(super) fn enter(panel: &mut ControlPanel, now: f64) {
    if panel.methods.names.is_none() && !panel.waiting(PanelReply::Methods(Reply::Discover)) {
        panel.send(PanelReply::Methods(Reply::Discover), "rpc.discover", None, now);
        panel.send(PanelReply::Methods(Reply::Described), "moly.methods", None, now);
    }
}

pub(super) fn act(panel: &mut ControlPanel, action: Action, now: f64) {
    match action {
        Action::Select(index) => {
            panel.methods.selected = Some(index);
            panel.methods.answer = None;
            panel.changed();
        }
        Action::Call => {
            let Some(name) = panel.methods.selected_name().map(str::to_owned) else {
                return;
            };
            if panel.methods.callable(&name) {
                panel.send(PanelReply::Methods(Reply::Called), &name, None, now);
            }
        }
    }
}

pub(super) fn answer(panel: &mut ControlPanel, reply: Reply, result: BrpResult) -> bool {
    match (reply, result) {
        (Reply::Discover, Ok(discovered)) => {
            let mut names: Vec<String> = discovered["methods"]
                .as_array()
                .map(|methods| methods.iter().filter_map(|method| method["name"].as_str().map(str::to_owned)).collect())
                .unwrap_or_default();
            // The game's own verbs first, then the protocol's.
            names.sort_by_key(|name| (!name.starts_with("moly."), name.clone()));
            panel.methods.names = Some(names);
            true
        }
        (Reply::Described, Ok(described)) => {
            panel.methods.described = described["methods"]
                .as_array()
                .map(|methods| {
                    methods
                        .iter()
                        .map(|method| {
                            (
                                method["name"].as_str().unwrap_or_default().to_owned(),
                                method["description"].as_str().unwrap_or_default().to_owned(),
                                method["params"].clone(),
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            true
        }
        (Reply::Called, result) => {
            let shown = match result {
                Ok(value) => serde_json::to_string_pretty(&value).unwrap_or_default(),
                Err(error) => format!("错误 {}：{}", error.code, error.message),
            };
            let mut shown: String = shown.chars().take(SHOWN_ANSWER).collect();
            if shown.chars().count() == SHOWN_ANSWER {
                shown += "\n…";
            }
            panel.methods.answer = Some(shown);
            true
        }
        _ => false,
    }
}

pub(super) fn build(c: &mut Commands, body: Entity, font: &Handle<Font>, panel: &ControlPanel) {
    let view = &panel.methods;
    let Some(names) = &view.names else {
        text(c, body, font, "正在读取…", 12., MUTED);
        return;
    };
    let list = card(c, body, font, Some(&format!("方法（{}）", names.len())));
    let chips = row(c, list, 6.);
    for (index, name) in names.iter().enumerate() {
        let id = button(c, chips, font, name, PanelAction::Methods(Action::Select(index)), Kind::Chip);
        if view.selected == Some(index) {
            c.entity(id).insert(Selected);
        }
    }
    let Some(name) = view.selected_name() else {
        text(c, body, font, "选一个方法看它做什么、要什么参数。", 12., MUTED);
        return;
    };
    let detail = card(c, body, font, Some(name));
    match view.described.iter().find(|(described, ..)| described == name) {
        Some((_, description, params)) => {
            text(c, detail, font, description, 12., INK);
            text(c, detail, font, format!("参数：{}", params), 11., MUTED);
        }
        None => {
            text(c, detail, font, "Bevy 远程协议的内置方法。", 12., INK);
        }
    }
    if view.callable(name) {
        button(c, detail, font, "调用（无参数）", PanelAction::Methods(Action::Call), Kind::Primary);
    } else {
        text(c, detail, font, "这个方法需要参数，请经外部客户端调用。", 11., ACCENT);
    }
    if let Some(answer) = &view.answer {
        let reply = card(c, body, font, Some("回答"));
        text(c, reply, font, answer.clone(), 11., MUTED);
    }
}
