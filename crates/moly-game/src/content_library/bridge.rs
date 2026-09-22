//! Versioned browser transport over the existing content-library lifecycle.
//!
//! JS only enqueues validated intent and reads an immutable projection. Bevy
//! consumes intent in PreUpdate, before gameplay, through the native action
//! dispatcher. Neither the transport nor the DOM has a second playback owner.
use super::input::apply_action;
use super::*;
#[path = "bridge_export.rs"]
mod export;
#[path = "bridge_presentation.rs"]
mod presentation;
use crate::fixture_activity_data::{ActivityKey, ActivityOrigin};
pub use export::library_catalog;
use serde_json::{json, Value};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex, OnceLock,
};

const SCHEMA: u64 = 1;
const MAX_COMMAND_BYTES: usize = 16_384;
const MAX_PENDING_COMMANDS: usize = 128;
static COMMANDS: OnceLock<Mutex<Vec<BrowserCommand>>> = OnceLock::new();
static SNAPSHOT: OnceLock<Mutex<String>> = OnceLock::new();
// Set synchronously by JS, before the next input schedule. The queued focus
// command also updates the ordinary world/talk gates in ContentLibrary.
static INPUT_CAPTURED: AtomicBool = AtomicBool::new(false);

#[derive(Debug)]
pub(super) enum BrowserCommand {
    Open,
    Close,
    Action(LibraryAction),
    Query(String),
    Select(EntryKey),
    Play(Option<EntryKey>),
    Preview(Option<EntryKey>),
    Mode(ExperienceMode),
    Scope(Scope),
    Page(usize),
    PageSize(usize),
    Related(i32, LibraryTab),
    Fixture(Option<i32>),
    Focus(bool),
    Settings,
    Sound(bool),
    Weather(i32),
}
fn commands() -> &'static Mutex<Vec<BrowserCommand>> {
    COMMANDS.get_or_init(|| Mutex::new(Vec::new()))
}
fn snapshot_cell() -> &'static Mutex<String> {
    SNAPSHOT.get_or_init(|| {
        Mutex::new(
            json!({
                "schemaVersion":SCHEMA,"ready":false,"loading":true,"revision":0,
                "open":false,"rows":[],"selected":null,"tabs":[],"characters":[],
                "total":0,"page":0,"pageSize":24,"issues":[],
                "status":{"phase":"preparing","label":"正在载入内容…","error":null,"canStop":false}
            })
            .to_string(),
        )
    })
}

/// Select DOM-owned chrome before Startup or any asset parsing can run.
pub fn configure_browser_library(app: &mut App) {
    let mut state = app.world_mut().resource_mut::<ContentLibrary>();
    state.external_ui = true;
    state.page_size = 24;
}

/// JSON is a deliberately small stable boundary; source identifiers remain
/// opaque strings so the host never needs to learn the master-table topology.
pub fn library_command(input: &str) -> Result<(), String> {
    let command = parse_command(input)?;
    let capture = match command {
        BrowserCommand::Focus(value) => Some(value),
        BrowserCommand::Close => Some(false),
        _ => None,
    };
    let mut queue = commands()
        .lock()
        .map_err(|_| "内容操作队列暂时不可用".to_owned())?;
    if queue.len() >= MAX_PENDING_COMMANDS {
        return Err("操作过于频繁，请稍候重试".into());
    }
    queue.push(command);
    if let Some(value) = capture {
        INPUT_CAPTURED.store(value, Ordering::Relaxed);
    }
    Ok(())
}
pub fn library_snapshot() -> String {
    snapshot_cell()
        .lock()
        .map(|s| s.clone())
        .unwrap_or_else(|_| "{\"schemaVersion\":1,\"ready\":false,\"loading\":true}".into())
}
pub(super) fn drain_commands() -> Vec<BrowserCommand> {
    commands()
        .lock()
        .map(|mut queue| std::mem::take(&mut *queue))
        .unwrap_or_default()
}
fn parse_tab(value: &str) -> Result<LibraryTab, String> {
    match value {
        "conversations" => Ok(LibraryTab::Conversations),
        "furniture" => Ok(LibraryTab::Furniture),
        "performances" => Ok(LibraryTab::Performances),
        "activities" => Ok(LibraryTab::Activities),
        _ => Err("未知的内容分类".into()),
    }
}
fn string<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("操作缺少字符串字段 {field}"))
}
fn parse_command(input: &str) -> Result<BrowserCommand, String> {
    if input.len() > MAX_COMMAND_BYTES {
        return Err("内容操作超过长度限制".into());
    }
    let value: Value = serde_json::from_str(input).map_err(|_| "内容操作格式不正确".to_owned())?;
    if value.get("schemaVersion").and_then(Value::as_u64) != Some(SCHEMA) {
        return Err("内容浏览器版本不匹配，请刷新页面".into());
    }
    Ok(match string(&value, "type")? {
        "open" => BrowserCommand::Open,
        "close" => BrowserCommand::Close,
        "settings" => {
            if value
                .get("value")
                .is_some_and(|v| v.as_str() != Some("toggle"))
            {
                return Err("未知的设置操作".into());
            }
            BrowserCommand::Settings
        }
        "sound" => BrowserCommand::Sound(
            value.get("value").and_then(Value::as_bool).ok_or("声音开关不正确")?,
        ),
        "stop" => BrowserCommand::Action(LibraryAction::Stop),
        "restore" => BrowserCommand::Action(LibraryAction::RestoreScene),
        "tab" => BrowserCommand::Action(LibraryAction::Tab(parse_tab(string(&value, "value")?)?)),
        "query" => BrowserCommand::Query(
            string(&value, "value")?
                .chars()
                .map(|ch| if ch.is_whitespace() { ' ' } else { ch })
                .filter(|ch| !ch.is_control())
                .take(MAX_QUERY)
                .collect(),
        ),
        "select" => BrowserCommand::Select(parse_key(string(&value, "key")?)?),
        "preview" => BrowserCommand::Preview(
            value
                .get("key")
                .map(|_| string(&value, "key").and_then(parse_key))
                .transpose()?,
        ),
        "play" => BrowserCommand::Play(
            value
                .get("key")
                .map(|_| string(&value, "key").and_then(parse_key))
                .transpose()?,
        ),
        "character" => {
            BrowserCommand::Action(LibraryAction::SetCharacter(match value.get("value") {
                Some(Value::Null) => None,
                Some(id) => Some(
                    id.as_u64()
                        .and_then(|id| u32::try_from(id).ok())
                        .filter(|id| *id > 0)
                        .ok_or("角色编号不正确")?,
                ),
                None => return Err("操作缺少角色字段".into()),
            }))
        }
        "fixture" => BrowserCommand::Fixture(match value.get("value") {
            Some(Value::Null) => None,
            Some(id) => Some(
                id.as_u64()
                    .and_then(|id| i32::try_from(id).ok())
                    .filter(|id| *id > 0)
                    .ok_or("家具编号不正确")?,
            ),
            None => return Err("操作缺少家具字段".into()),
        }),
        "availability" => BrowserCommand::Scope(match string(&value, "value")? {
            "all" => Scope::All,
            "here" => Scope::Here,
            "ready" => Scope::Ready,
            _ => return Err("未知的可用性筛选".into()),
        }),
        "mode" => BrowserCommand::Mode(match string(&value, "value")? {
            "independent" => ExperienceMode::Independent,
            "current" => ExperienceMode::CurrentScene,
            _ => return Err("未知的体验方式".into()),
        }),
        "page" => BrowserCommand::Page(
            value
                .get("value")
                .and_then(Value::as_u64)
                .and_then(|n| usize::try_from(n).ok())
                .ok_or("页码不正确")?,
        ),
        "pageSize" => BrowserCommand::PageSize(
            value
                .get("value")
                .and_then(Value::as_u64)
                .filter(|n| (1..=100).contains(n))
                .ok_or("每页数量必须在 1 到 100 之间")? as usize,
        ),
        "related" => BrowserCommand::Related(
            value
                .get("fixtureId")
                .and_then(Value::as_u64)
                .and_then(|id| i32::try_from(id).ok())
                .filter(|id| *id > 0)
                .ok_or("家具编号不正确")?,
            match parse_tab(string(&value, "tab")?)? {
                LibraryTab::Conversations => {
                    return Err("相关入口必须是家具、故事或角色互动".into())
                }
                tab => tab,
            },
        ),
        "focus" => BrowserCommand::Focus(
            value
                .get("value")
                .and_then(Value::as_bool)
                .ok_or("输入焦点值不正确")?,
        ),
        // The target档位, never an index: the host picks from the catalogue the
        // runtime published, so a wrong ID can only come from a stale page.
        "weather" => BrowserCommand::Weather(
            value
                .get("value")
                .and_then(Value::as_i64)
                .and_then(|id| i32::try_from(id).ok())
                .filter(|id| *id > 0)
                .ok_or("天气档位不正确")?,
        ),
        _ => return Err("未知的内容操作".into()),
    })
}
fn positive_id(value: &str) -> Result<i32, String> {
    value
        .parse::<i32>()
        .ok()
        .filter(|id| *id > 0)
        .ok_or_else(|| "内容编号不正确".into())
}
fn parse_key(value: &str) -> Result<EntryKey, String> {
    let parts: Vec<_> = value.split(':').collect();
    Ok(match parts.as_slice() {
        ["talk", "general", id] => EntryKey::Talk(TalkBackend::General, positive_id(id)?),
        ["talk", "fixture", id] => EntryKey::Talk(TalkBackend::Fixture, positive_id(id)?),
        ["fixture", id] => EntryKey::Fixture(positive_id(id)?),
        ["activity", origin, id, timeline] => EntryKey::Activity(ActivityKey {
            origin: match *origin {
                "notalk" => ActivityOrigin::NoTalk(positive_id(id)?),
                "preaction" => ActivityOrigin::PreAction(positive_id(id)?),
                _ => return Err("未知的角色互动来源".into()),
            },
            timeline_id: positive_id(timeline)?,
        }),
        _ => return Err("内容编号格式不正确".into()),
    })
}
fn key(value: EntryKey) -> String {
    match value {
        EntryKey::Talk(TalkBackend::General, id) => format!("talk:general:{id}"),
        EntryKey::Talk(TalkBackend::Fixture, id) => format!("talk:fixture:{id}"),
        EntryKey::Fixture(id) => format!("fixture:{id}"),
        EntryKey::Activity(activity) => format!(
            "activity:{}:{}:{}",
            match activity.origin {
                ActivityOrigin::NoTalk(_) => "notalk",
                ActivityOrigin::PreAction(_) => "preaction",
            },
            activity.source_id(),
            activity.timeline_id
        ),
    }
}
fn busy(state: &ContentLibrary) -> bool {
    state.active.is_some() || state.pending.is_some() || state.stopping || state.scene_owned
}
fn reject(state: &mut ContentLibrary, reason: &str) {
    state.status = reason.into();
    state.changed();
}
pub(super) fn apply_command(
    command: BrowserCommand,
    state: &mut ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
    io: &mut input::LibraryInput,
) {
    let preview_intent = matches!(&command, BrowserCommand::Preview(_));
    match command {
        BrowserCommand::Open => {
            if !state.external_ui {
                state.page_size = 24;
            }
            state.external_ui = true;
            state.external_open = true;
            state.open = false;
            state.search_focus = false;
            state.picker_open = false;
            state.ime_preedit.clear();
            state.changed();
        }
        BrowserCommand::Close => {
            apply_action(LibraryAction::Close, state, catalog, world, io);
            state.external_open = false;
            state.external_input_capture = false;
            // Once mounted, all catalogue chrome belongs to the host, even
            // while the host is collapsed and the original scene is restored.
            state.external_ui = true;
        }
        BrowserCommand::Settings => {
            io.settings
                .write(crate::game_settings::SettingsPanelRequest::Toggle);
        }
        BrowserCommand::Sound(enabled) => {
            io.audio_gate.enabled = enabled;
        }
        // Weather owns no catalogue state: the dial is queued for the weather
        // chain, which validates the档 and starts the same cross-fade as the
        // native key. A stale ID is refused there with a log line.
        BrowserCommand::Weather(id) => {
            io.weather.write(crate::weather::WeatherRequest(id));
        }
        BrowserCommand::Focus(value) => {
            state.external_input_capture = value;
        }
        BrowserCommand::Query(query) => {
            state.search = query;
            state.search_cursor = state.search.chars().count();
            state.reset_browse();
        }
        BrowserCommand::Select(key) => {
            if !state.filtered.contains(&key) {
                reject(state, "这项内容已不在当前结果中，请重新选择");
                return;
            }
            apply_action(LibraryAction::Select(key), state, catalog, world, io);
            state.select_index(
                state
                    .filtered
                    .iter()
                    .position(|candidate| *candidate == key)
                    .unwrap(),
            );
        }
        BrowserCommand::Play(selected) | BrowserCommand::Preview(selected) => {
            // A stale/missing key must never silently play a different row.
            if let Some(key) = selected {
                if !state.filtered.contains(&key) {
                    reject(state, "这项内容已不在当前结果中，请重新选择");
                    return;
                }
                apply_action(LibraryAction::Select(key), state, catalog, world, io);
            }
            if state.stopping {
                reject(state, "正在恢复场景，请稍候再播放");
                return;
            }
            apply_action(
                if preview_intent {
                    LibraryAction::Preview
                } else {
                    LibraryAction::Play
                },
                state,
                catalog,
                world,
                io,
            );
        }
        BrowserCommand::Mode(mode) => {
            if state.mode == mode {
                return;
            }
            if busy(state) {
                reject(state, "请先停止当前体验并等待场景恢复，再切换体验方式");
                return;
            }
            apply_action(LibraryAction::SetMode(mode), state, catalog, world, io);
            // Browse all authored content in either mode; availability is a
            // separate explicit filter, never an implicit empty catalogue.
            state.scope = Scope::All;
            state.reset_browse();
        }
        BrowserCommand::Scope(scope) => {
            if scope == Scope::Here && state.mode != ExperienceMode::CurrentScene && busy(state) {
                reject(state, "请先停止当前体验，再查看原场景中的内容");
                return;
            }
            apply_action(LibraryAction::SetScope(scope), state, catalog, world, io);
        }
        BrowserCommand::Page(page) => {
            let max_page = state.filtered.len().saturating_sub(1) / state.page_size.max(1);
            state.offset = page.min(max_page) * state.page_size.max(1);
            state.changed();
        }
        BrowserCommand::PageSize(size) => {
            let first = state.offset;
            state.page_size = size;
            state.offset = first / size * size;
            state.changed();
        }
        BrowserCommand::Fixture(id) => {
            state.related_fixture = id;
            state.reset_browse();
        }
        BrowserCommand::Related(id, tab) => {
            if catalog.fixture(id).is_none() {
                reject(state, "相关家具已不在当前图鉴中");
                return;
            }
            match tab {
                LibraryTab::Furniture => {
                    apply_action(LibraryAction::Tab(tab), state, catalog, world, io);
                    state.search = format!("#{id}");
                    state.scope = Scope::All;
                    state.reset_browse();
                    state.rebuild_results(catalog, world);
                    apply_action(
                        LibraryAction::Select(EntryKey::Fixture(id)),
                        state,
                        catalog,
                        world,
                        io,
                    );
                }
                LibraryTab::Activities => apply_action(
                    LibraryAction::RelatedActivities(id),
                    state,
                    catalog,
                    world,
                    io,
                ),
                _ => apply_action(LibraryAction::Related(id), state, catalog, world, io),
            }
        }
        BrowserCommand::Action(action) => apply_action(action, state, catalog, world, io),
    }
    // The DOM catalogue remains visible beside the canvas during playback;
    // its existence must not swallow the canvas's click-to-advance dialogue.
    if state.external_ui {
        state.open = false;
        state.search_focus = false;
    }
}
fn character(catalog: &LibraryCatalog, id: u32) -> Value {
    let original = catalog.character(id);
    let duplicated = catalog
        .character_names
        .values()
        .filter(|name| **name == original)
        .count()
        > 1;
    let group = catalog.character_groups.get(&id);
    let name = if duplicated {
        group
            .map(|group| format!("{original} · {group}"))
            .unwrap_or_else(|| original.clone())
    } else {
        original.clone()
    };
    json!({"id":id,"name":name,"originalName":original,"group":group,"color":catalog.character_colors.get(&id)})
}

fn image(catalog: &LibraryCatalog, id: i32) -> Option<String> {
    catalog
        .thumbnail_paths
        .get(&id)
        .map(|path| format!("fixture-thumbnails/{path}"))
}
fn fixture(catalog: &LibraryCatalog, id: i32) -> Value {
    json!({"id":id,"name":catalog.fixture_name(id),"image":image(catalog,id)})
}
fn row_reason(
    key: EntryKey,
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
) -> Option<String> {
    let probe = ContentLibrary {
        selected: Some(key),
        mode: state.mode,
        active: state.active.clone(),
        selected_uid: if state.selected == Some(key) {
            state.selected_uid.clone()
        } else {
            None
        },
        ..default()
    };
    context::selected_reason(&probe, catalog, world)
}
fn project_row(
    key_value: EntryKey,
    state: &ContentLibrary,
    catalog: &LibraryCatalog,
    world: &LibraryContext,
    detail: bool,
) -> Value {
    let reason = row_reason(key_value, state, catalog, world);
    let (title, subtitle, kind, units, fixture_ids, description, lines) = match key_value {
        EntryKey::Fixture(id) => match catalog.fixture(id) {
            Some(row) => (
                row.name.clone(),
                excerpt(&row.description, 130),
                row.action_label(),
                vec![],
                vec![id],
                row.description.clone(),
                vec![],
            ),
            None => (
                catalog.fixture_name(id),
                String::new(),
                "家具",
                vec![],
                vec![id],
                String::new(),
                vec![],
            ),
        },
        EntryKey::Activity(id) => match catalog.activity(id) {
            Some(row) => {
                let lines=row.spec.tweet.as_ref().map(|tweet|vec![json!({"speaker":catalog.character(row.spec.unit),"text":plain_text(&tweet.text)})]).unwrap_or_default();
                (
                    row.title.clone(),
                    row.spec
                        .tweet
                        .as_ref()
                        .map(|tweet| excerpt(&plain_text(&tweet.text), 130))
                        .unwrap_or_else(|| "欣赏角色与家具的动作演出".into()),
                    row.kind(),
                    vec![row.spec.unit],
                    vec![row.spec.fixture_id],
                    if row.spec.tweet.is_some() {
                        "这段家具互动包含角色动作与头顶气泡。"
                    } else {
                        "这段家具互动没有对白，角色将按照原始演出与家具互动。"
                    }
                    .into(),
                    lines,
                )
            }
            None => (
                "角色互动".into(),
                String::new(),
                "角色互动",
                vec![],
                vec![],
                String::new(),
                vec![],
            ),
        },
        _ => match catalog.talk(key_value) {
            Some(row) => (
                row.title.clone(),
                excerpt(&row.preview, 150),
                row.kind(),
                row.units.clone(),
                row.fixture_ids.clone(),
                String::new(),
                if detail {
                    row.lines
                        .iter()
                        .map(|line| json!({"speaker":line.speaker,"text":line.text}))
                        .collect()
                } else {
                    vec![]
                },
            ),
            None => (
                "对话".into(),
                String::new(),
                "对话",
                vec![],
                vec![],
                String::new(),
                vec![],
            ),
        },
    };
    let mut row = json!({"key":key(key_value),"title":title,"subtitle":subtitle,"image":fixture_ids.first().and_then(|id|image(catalog,*id)),
        "characters":units.iter().map(|id|character(catalog,*id)).collect::<Vec<_>>(),"kind":kind,"available":reason.is_none(),"reason":reason});
    if detail {
        let mut related = Vec::new();
        for id in &fixture_ids {
            if !matches!(key_value, EntryKey::Fixture(_)) {
                related.push(json!({"tab":"furniture","fixtureId":id,"label":catalog.fixture_name(*id),"count":1}));
            }
            let stories = catalog
                .talks
                .iter()
                .filter(|row| row.fixture_ids.contains(id) && row.furniture_related)
                .count();
            let activities = catalog
                .activities
                .iter()
                .filter(|row| row.spec.fixture_id == *id)
                .count();
            if stories > 0 {
                related.push(
                    json!({"tab":"performances","fixtureId":id,"label":"家具故事","count":stories}),
                );
            }
            if activities > 0 {
                related.push(json!({"tab":"activities","fixtureId":id,"label":"角色互动","count":activities}));
            }
        }
        row["description"] = json!(description);
        row["lines"] = json!(lines);
        row["fixtures"] = json!(fixture_ids
            .iter()
            .map(|id| fixture(catalog, *id))
            .collect::<Vec<_>>());
        row["related"] = json!(related);
    }
    presentation::enrich(key_value, state, catalog, &mut row);
    row
}
fn tab_name(tab: LibraryTab) -> &'static str {
    match tab {
        LibraryTab::Conversations => "conversations",
        LibraryTab::Furniture => "furniture",
        LibraryTab::Performances => "performances",
        LibraryTab::Activities => "activities",
    }
}
fn project(state: &ContentLibrary, catalog: &LibraryCatalog, world: &LibraryContext) -> Value {
    let rows: Vec<_> = state
        .filtered
        .iter()
        .skip(state.offset)
        .take(state.page_size)
        .map(|key| project_row(*key, state, catalog, world, false))
        .collect();
    let selected = state
        .selected
        .map(|key| project_row(key, state, catalog, world, true));
    let phase = if state.stopping
        || (state.scene_owned && state.pending.is_none() && state.active.is_none())
    {
        "restoring"
    } else if state.pending.is_some() || state.active.as_ref().is_some_and(|a| !a.started) {
        "preparing"
    } else if state.active.as_ref().is_some_and(|active| active.completed) {
        "completed"
    } else if state.active.is_some() {
        "playing"
    } else if state.last_error.is_some() {
        "error"
    } else {
        "idle"
    };
    let mut characters: Vec<_> = catalog.character_names.keys().copied().collect();
    characters.sort_unstable();
    let tabs=[LibraryTab::Conversations,LibraryTab::Furniture,LibraryTab::Performances,LibraryTab::Activities].map(|tab|json!({
        "id":tab_name(tab),"label":tab.title(),"count":match tab {
            LibraryTab::Conversations=>catalog.talks.len(),LibraryTab::Furniture=>catalog.fixtures.len(),
            LibraryTab::Performances=>catalog.talks.iter().filter(|row|row.furniture_related).count(),LibraryTab::Activities=>catalog.activities.len(),
        }
    }));
    let loading = match state.tab {
        LibraryTab::Furniture => !catalog.source_ready,
        LibraryTab::Activities => !catalog.activities_ready,
        _ => !catalog.talks_ready,
    };
    json!({"schemaVersion":SCHEMA,"ready":catalog.source_ready&&!loading,"loading":loading,
        "revision":format!("{}:{}:{}",state.revision,catalog.revision,world.revision),"open":state.external_open,
        "tab":tab_name(state.tab),"query":state.search,"character":state.character,
        "availability":match state.scope{Scope::All=>"all",Scope::Here=>"here",Scope::Ready=>"ready"},
        "mode":match state.mode{ExperienceMode::Independent=>"independent",ExperienceMode::CurrentScene=>"current"},
        "region":catalog.source_region,"version":catalog.source_version,"total":state.filtered.len(),
        "page":state.offset/state.page_size.max(1),"pageSize":state.page_size,"tabs":tabs,
        "characters":characters.into_iter().map(|id|character(catalog,id)).collect::<Vec<_>>(),
        "rows":rows,"selected":selected,"relatedFixture":state.related_fixture,
        "inspection":state.inspection.map(|(id,fixture_id)|json!({"id":id,"fixtureId":fixture_id})),
        "status":{"phase":phase,"label":state.status,"error":state.last_error,
            "preview":state.active.as_ref().is_some_and(|active|active.choice.preview),
            "activeKey":state.active.as_ref().map(|a|key(a.choice.key)).or_else(||state.pending.as_ref().map(|p|key(p.key))),
            "activeTitle":state.active.as_ref().map(|a|a.title.clone()).or_else(||state.pending.as_ref().map(|p|catalog.title(p.key))),
            "canStop":busy(state)},"issues":catalog.data_issues})
}

pub(crate) fn publish(
    state: Res<ContentLibrary>,
    catalog: Res<LibraryCatalog>,
    world: Res<LibraryContext>,
    mut previous: Local<String>,
    site_ready: Option<Res<crate::site::SiteScenesReady>>,
    site_materials: Option<Res<crate::site_material::SiteMaterialsSwapped>>,
    player_visual: Query<
        (),
        (
            With<crate::player::PlayerControlled>,
            With<crate::player_avatar::AvatarDriver>,
        ),
    >,
    text_art: Option<Res<crate::balloon::BalloonArt>>,
    layouts: Option<Res<crate::ui_layout::UiLayouts>>,
    phenomenon: Option<Res<crate::weather::CurrentPhenomenonId>>,
    catalogue: Option<Res<crate::weather::PhenomenonCatalogue>>,
    weather_transition: Option<Res<crate::weather_transition::WeatherTransition>>,
    server: Res<AssetServer>,
    mut audio_startup: Option<ResMut<crate::audio_startup::BrowserAudioStartup>>,
    preview: Option<Res<staging::ScenePreview>>,
    staged: Query<(&Transform, &GlobalTransform)>,
) {
    if !state.external_ui {
        return;
    }
    export::publish_if_requested(&catalog);
    // No per-frame elapsed counter is exposed. Serialize a page only when a
    // catalogue, context, owner, or visible status actually changes.
    let text_ready = text_art.is_some()
        && layouts
            .as_deref()
            .is_some_and(|layouts| layouts.ready("Talk", &server));
    let mut scene_ready = site_ready.is_some()
        && site_materials.is_some()
        && !player_visual.is_empty()
        && world.actors.values().all(|ready| *ready)
        && text_ready;
    if let Some(startup) = audio_startup.as_mut() {
        // A requested independent scene must finish its own real staging before
        // its audio or script starts; readiness of the empty base is not enough.
        startup.prepared = scene_ready && state.pending.as_ref().is_none_or(|choice| {
            preview.as_ref().is_some_and(|preview| preview.ready(choice, &staged))
        });
        scene_ready &= startup.released();
    }
    // 天气档位进戳：宿主画的那颗钮读同一份投影，切档必须重发一页，否则
    // 「档位变了但页面没变」会一直停在旧值上。
    let weather_id = phenomenon
        .as_deref()
        .map(|phenomenon| phenomenon.0)
        .unwrap_or_default();
    let weather_view = weather_transition.as_deref().map(|phase| phase.presentation());
    let stamp = format!(
        "{}:{}:{}:{}:{}:{}:{}:{}:{}:{}:{:?}",
        state.revision,
        catalog.revision,
        world.revision,
        state.status,
        state.stopping,
        state.scene_owned,
        state.active.is_some(),
        state.pending.is_some(),
        scene_ready,
        weather_id, weather_view
    );
    if *previous == stamp {
        return;
    }
    *previous = stamp;
    if let Ok(mut slot) = snapshot_cell().lock() {
        let mut snapshot = project(&state, &catalog, &world);
        let mut actors: Vec<_> = world.actors.keys().copied().collect();
        actors.sort_unstable();
        let mut fixture_ids: Vec<_> = world.instances.keys().copied().collect();
        fixture_ids.sort_unstable();
        snapshot["scene"] =
            json!({"ready":scene_ready,"actorUnits":actors,"fixtureIds":fixture_ids});
        // The weather dial rides the same projection as the catalogue: current
        // 档 plus the ordered list the runtime actually resolved. An unresolved
        // catalogue omits the block instead of publishing an empty dial, so the
        // host never renders a control with nothing to choose.
        if let Some(catalogue) = catalogue.as_deref().filter(|c| !c.0.is_empty()) {
            let current = catalogue
                .0
                .iter()
                .find(|option| option.id == weather_id)
                .or_else(|| catalogue.0.first());
            snapshot["weather"] = json!({
                "id": current.map(|option| option.id).unwrap_or_default(),
                "name": current.map(|option| option.name.clone()).unwrap_or_default(),
                "transition": weather_view,
                "options": catalogue
                    .0
                    .iter()
                    .map(|option| json!({
                        "id": option.id, "name": option.name,
                        "icon": option.icon, "metadata": option.metadata,
                        "iconSource": option.icon_source,
                    }))
                    .collect::<Vec<_>>(),
            });
        }
        *slot = snapshot.to_string();
    }
}
pub(crate) fn gate_input(
    mut keys: ResMut<ButtonInput<KeyCode>>,
    mut buttons: ResMut<ButtonInput<MouseButton>>,
) {
    if INPUT_CAPTURED.load(Ordering::Relaxed) {
        // Also clears held movement when focus moves from canvas to search.
        keys.reset_all();
        buttons.reset_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exact_keys_preserve_backend_origin_and_timeline_variant() {
        let keys = [
            EntryKey::Talk(TalkBackend::General, 7),
            EntryKey::Talk(TalkBackend::Fixture, 7),
            EntryKey::Fixture(7),
            EntryKey::Activity(ActivityKey {
                origin: ActivityOrigin::NoTalk(7),
                timeline_id: 8,
            }),
            EntryKey::Activity(ActivityKey {
                origin: ActivityOrigin::PreAction(7),
                timeline_id: 8,
            }),
        ];
        for value in keys {
            assert_eq!(parse_key(&key(value)).unwrap(), value);
        }
        assert_ne!(key(keys[0]), key(keys[1]));
        assert_ne!(key(keys[3]), key(keys[4]));
        for bad in [
            "fixture:0",
            "fixture:1:2",
            "talk:general:-1",
            "activity:7:8",
        ] {
            assert!(parse_key(bad).is_err());
        }
    }
    #[test]
    fn malformed_commands_fail_before_queueing_or_playback() {
        for bad in [
            r#"{"schemaVersion":2,"type":"play"}"#,
            r#"{"schemaVersion":1,"type":"play","key":"missing"}"#,
            r#"{"schemaVersion":1,"type":"focus","value":"true"}"#,
            r#"{"schemaVersion":1,"type":"pageSize","value":0}"#,
        ] {
            assert!(parse_command(bad).is_err());
        }
        let input =
            json!({"schemaVersion":1,"type":"query","value":"奏\n".repeat(500)}).to_string();
        let BrowserCommand::Query(query) = parse_command(&input).unwrap() else {
            panic!()
        };
        assert_eq!(query.chars().count(), MAX_QUERY);
        assert!(!query.contains('\n'));
    }
    fn talk(id: i32, backend: TalkBackend) -> LibraryTalk {
        LibraryTalk {
            preview_tweet: None,
            content: TalkContent {
                master_id: id,
                backend,
                is_general: Some(true),
            },
            units: vec![1],
            fixture_ids: vec![],
            title: format!("对话 {id}"),
            preview: "一歌：第一句".into(),
            lines: vec![DialogueLine {
                speaker: "一歌".into(),
                text: "第一句\n长台词".into(),
            }],
            search: format!("对话 {id}"),
            furniture_related: false,
            drives_fixture: false,
        }
    }
    #[test]
    fn projection_paginates_without_truncating_selected_transcript() {
        let mut catalog = LibraryCatalog::default();
        catalog.talks = (1..=70).map(|id| talk(id, TalkBackend::General)).collect();
        let mut state = ContentLibrary {
            page_size: 24,
            ..default()
        };
        state.rebuild_results(&catalog, &LibraryContext::default());
        state.offset = 24;
        state.selected = Some(EntryKey::Talk(TalkBackend::General, 30));
        let output = project(&state, &catalog, &LibraryContext::default());
        assert_eq!(output["total"], 70);
        assert_eq!(output["rows"].as_array().unwrap().len(), 24);
        assert_eq!(output["page"], 1);
        assert_eq!(output["selected"]["key"], "talk:general:30");
        assert_eq!(output["selected"]["lines"][0]["text"], "第一句\n长台词");
        assert_eq!(output["mode"], "independent");
    }
    #[test]
    fn current_scene_unavailable_rows_report_their_own_cast() {
        let mut catalog = LibraryCatalog::default();
        catalog.talks = vec![talk(1, TalkBackend::General), talk(2, TalkBackend::General)];
        catalog.talks[1].units = vec![2];
        catalog.character_names.insert(2, "奏".into());
        let mut context = LibraryContext::default();
        context.actors.insert(1, true);
        let state = ContentLibrary {
            mode: ExperienceMode::CurrentScene,
            ..default()
        };
        let first = project_row(
            EntryKey::Talk(TalkBackend::General, 1),
            &state,
            &catalog,
            &context,
            false,
        );
        let second = project_row(
            EntryKey::Talk(TalkBackend::General, 2),
            &state,
            &catalog,
            &context,
            false,
        );
        assert_eq!(first["available"], true);
        assert_eq!(second["available"], false);
        assert!(second["reason"].as_str().unwrap().contains("奏"));
    }
    fn apply_in_test(
        command: BrowserCommand,
        state: &mut ContentLibrary,
        catalog: &LibraryCatalog,
    ) -> (usize, usize) {
        use bevy::ecs::system::SystemState;
        let mut world = World::new();
        world.init_resource::<Messages<KeyboardInput>>();
        world.init_resource::<Messages<Ime>>();
        world.init_resource::<Messages<MouseWheel>>();
        world.init_resource::<Messages<TalkCancelRequest>>();
        world.init_resource::<Messages<PlayerFixtureRequest>>();
        world.init_resource::<Messages<crate::game_settings::SettingsPanelRequest>>();
        world.init_resource::<Messages<crate::weather::WeatherRequest>>();
        world.init_resource::<input::PasteInbox>();
        // 声音闸是 `LibraryInput` 的一个 `ResMut`，缺席时取参会 panic。
        // 它随声音开关那条改动进入 `LibraryInput`，而这个最小 World 没跟着
        // 补——四条 bridge 判据因此红，且红的话说的是「资源不存在」，
        // 与被测的命令语义无关。
        world.init_resource::<crate::audio::AudioGate>();
        let mut system = SystemState::<input::LibraryInput>::new(&mut world);
        {
            let mut io = system.get_mut(&mut world);
            apply_command(command, state, catalog, &LibraryContext::default(), &mut io);
        }
        system.apply(&mut world);
        (
            world.resource::<Messages<TalkCancelRequest>>().len(),
            world.resource::<Messages<PlayerFixtureRequest>>().len(),
        )
    }
    fn playing() -> ContentLibrary {
        let choice = PlaybackChoice {
            preview: false,
            key: EntryKey::Talk(TalkBackend::General, 1),
            target: None,
            ticket: 77,
            mode: ExperienceMode::Independent,
        };
        ContentLibrary {
            external_ui: true,
            external_open: true,
            watching: true,
            scene_owned: true,
            selected: Some(choice.key),
            filtered: vec![choice.key],
            active: Some(ActiveChoice {
                choice,
                title: "正在播放".into(),
                started: true,
                completed: false,
                elapsed: 3.,
                effect_owner: None,
                static_view: false,
            }),
            ..default()
        }
    }
    #[test]
    fn browse_does_not_cancel_and_busy_mode_change_does_not_abandon_owner() {
        let mut state = playing();
        let catalog = LibraryCatalog::default();
        assert_eq!(
            apply_in_test(
                BrowserCommand::Action(LibraryAction::Tab(LibraryTab::Furniture)),
                &mut state,
                &catalog
            ),
            (0, 0)
        );
        assert_eq!(state.active.as_ref().unwrap().choice.ticket, 77);
        assert_eq!(
            apply_in_test(
                BrowserCommand::Mode(ExperienceMode::CurrentScene),
                &mut state,
                &catalog
            ),
            (0, 0)
        );
        assert_eq!(state.mode, ExperienceMode::Independent);
        assert!(state.scene_owned);
        assert_eq!(state.active.as_ref().unwrap().choice.ticket, 77);
        assert!(state.status.contains("先停止"));
    }
    #[test]
    fn stale_play_key_never_plays_the_current_selection() {
        let mut state = playing();
        let mut catalog = LibraryCatalog::default();
        catalog.talks = vec![talk(1, TalkBackend::General)];
        let emitted = apply_in_test(
            BrowserCommand::Play(Some(EntryKey::Talk(TalkBackend::General, 999))),
            &mut state,
            &catalog,
        );
        assert_eq!(emitted, (0, 0));
        assert!(state.pending.is_none());
        assert_eq!(state.active.as_ref().unwrap().choice.ticket, 77);
    }
    #[test]
    fn exact_play_enters_the_shared_cancel_and_pending_lifecycle() {
        let mut state = playing();
        let mut catalog = LibraryCatalog::default();
        catalog.talks = vec![talk(1, TalkBackend::General), talk(1, TalkBackend::Fixture)];
        let requested = EntryKey::Talk(TalkBackend::Fixture, 1);
        state.filtered.push(requested);
        assert_eq!(
            apply_in_test(BrowserCommand::Play(Some(requested)), &mut state, &catalog),
            (1, 1)
        );
        assert_eq!(state.pending.as_ref().unwrap().key, requested);
        assert_eq!(state.cleanup_frames, 2);
        assert_eq!(state.active.as_ref().unwrap().choice.ticket, 77);
    }
    #[test]
    fn closing_cancels_owners_but_retains_scene_ownership_until_restore() {
        let mut state = playing();
        let catalog = LibraryCatalog::default();
        assert_eq!(
            apply_in_test(BrowserCommand::Close, &mut state, &catalog),
            (1, 1)
        );
        assert!(!state.external_open);
        assert!(!state.open);
        assert!(state.external_ui);
        assert!(state.stopping);
        assert!(state.scene_owned);
        assert!(state.active.is_some());
        let snapshot = project(&state, &catalog, &LibraryContext::default());
        assert_eq!(snapshot["status"]["phase"], "restoring");
        assert_eq!(snapshot["status"]["canStop"], true);
    }
    #[test]
    fn dom_focus_blocks_input_while_canvas_can_advance_an_owned_talk() {
        let mut state = playing();
        state.open = true;
        state.release_guard = 0;
        assert!(state.blocks_world_input());
        assert!(!state.blocks_talk_input());
        state.external_input_capture = true;
        assert!(state.blocks_world_input());
        assert!(state.blocks_talk_input());
        state.external_input_capture = false;
        state.active = None;
        state.scene_owned = false;
        assert!(!state.blocks_world_input());
        assert!(!state.blocks_talk_input());
        state.external_ui = false;
        assert!(state.blocks_world_input());
        assert!(state.blocks_talk_input());
    }
    #[test]
    fn furniture_and_activity_details_keep_artwork_and_original_bubble() {
        use crate::fixture_activity_data::{ActivitySpec, ActivityTimeline};
        let mut catalog = LibraryCatalog::default();
        catalog.fixtures.push(LibraryFixture {
            id: 837,
            name: "摩托车".into(),
            description: "喜欢的摩托车".into(),
            action: "timeline".into(),
            search: "摩托车".into(),
            thumbnail: None,
            presentation: FixturePresentation::Model,
            source: Some(FixtureSource {
                package: "bike".into(),
                grid_size: moly_law::fixture::Vector3Int::new(2, 1, 1),
                exported: true,
                layout: 0,
                center_y: 0,
            }),
        });
        catalog.thumbnail_paths.insert(837, "images/837.png".into());
        let activity_key = ActivityKey {
            origin: ActivityOrigin::PreAction(4),
            timeline_id: 9,
        };
        catalog.activities.push(activities::LibraryActivity {
            unavailable: None,
            title: "一歌 · 摩托车".into(),
            search: String::new(),
            spec: ActivitySpec {
                key: activity_key,
                unit: 1,
                fixture_id: 837,
                point: 1,
                timeline: ActivityTimeline {
                    id: 9,
                    group_id: 2,
                    asset_name: "bike".into(),
                    action_point_definition: 1,
                },
                tweet: Some(moly_law::talk::TweetRef {
                    id: 1,
                    text: "<color=red>出发吧！</color>".into(),
                    motion: String::new(),
                    eye: String::new(),
                    mouth: String::new(),
                }),
                related_talk: None,
                variant: 1,
                variants: 1,
            },
        });
        let state = ContentLibrary::default();
        let context = LibraryContext::default();
        let furniture = project_row(EntryKey::Fixture(837), &state, &catalog, &context, true);
        assert_eq!(furniture["description"], "喜欢的摩托车");
        assert_eq!(furniture["image"], "fixture-thumbnails/images/837.png");
        assert_eq!(furniture["related"][0]["tab"], "activities");
        assert_eq!(furniture["related"][0]["count"], 1);
        let activity = project_row(
            EntryKey::Activity(activity_key),
            &state,
            &catalog,
            &context,
            true,
        );
        assert_eq!(activity["available"], true);
        assert_eq!(activity["lines"][0]["text"], "出发吧！");
        assert_eq!(activity["fixtures"][0]["name"], "摩托车");
        assert_eq!(activity["related"][0]["tab"], "furniture");
    }
    #[test]
    fn late_activity_catalogue_does_not_prematurely_admit_deep_links() {
        let state = ContentLibrary {
            tab: LibraryTab::Activities,
            ..default()
        };
        let mut catalog = LibraryCatalog {
            source_ready: true,
            talks_ready: true,
            ..default()
        };
        let before = project(&state, &catalog, &LibraryContext::default());
        assert_eq!(before["ready"], false);
        assert_eq!(before["loading"], true);
        catalog.activities_ready = true;
        let after = project(&state, &catalog, &LibraryContext::default());
        assert_eq!(after["ready"], true);
        assert_eq!(after["loading"], false);
    }
    #[test]
    fn duplicate_character_variants_have_world_labels_without_rewriting_dialogue_names() {
        let mut catalog = LibraryCatalog::default();
        catalog.character_names.insert(21, "初音未来".into());
        catalog.character_names.insert(27, "初音未来".into());
        catalog.character_groups.insert(21, "VIRTUAL SINGER".into());
        catalog.character_groups.insert(27, "Leo/need".into());
        assert_eq!(character(&catalog, 27)["name"], "初音未来 · Leo/need");
        assert_eq!(character(&catalog, 27)["originalName"], "初音未来");
        assert_eq!(catalog.character(27), "初音未来");
        assert!(matches!(
            parse_command(r#"{"schemaVersion":1,"type":"settings","value":"toggle"}"#),
            Ok(BrowserCommand::Settings)
        ));
    }

    #[test]
    fn npc_performance_allows_spectator_movement_without_unlocking_scene_mutation() {
        let mut state = playing();
        state.active.as_mut().unwrap().choice.key = EntryKey::Activity(ActivityKey {
            origin: ActivityOrigin::NoTalk(1),
            timeline_id: 1,
        });
        state.release_guard = 0;
        assert!(!state.blocks_exploration_input());
        assert!(!state.blocks_camera_input());
        assert!(state.blocks_world_input());
        state.stopping = true;
        assert!(state.blocks_exploration_input());
        state.stopping = false;
        state.external_input_capture = true;
        assert!(state.blocks_camera_input());
        state.external_input_capture = false;
        state.active.as_mut().unwrap().choice.key = EntryKey::Fixture(1);
        assert!(state.blocks_exploration_input());
    }
}
