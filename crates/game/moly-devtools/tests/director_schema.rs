//! `moly.director.schema` against the steps the game registers.
//!
//! The schema the control surface answers for `moly.director.schema` lists
//! every step a script can write. The steps the game registers are the ones
//! in the world's action registry, which the registering crates fill at
//! startup. The script runner's own steps are the flow steps (`wait`,
//! `wait_until`, `assert`, `log`, `where`, `capture`, `parallel`, `repeat`)
//! and the macros that stand for flow around an interaction (`player.talk_to`,
//! `player.craft`, `site.switch`, `site.enter_room`, `site.go_home`). The
//! plugin document is checked with them: every step the schema lists has its
//! row there.

use std::collections::BTreeSet;

use bevy::prelude::*;
use serde_json::Value;

/// The script runner's own steps.
const FLOW: &[&str] = &[
    "wait",
    "wait_until",
    "assert",
    "log",
    "where",
    "capture",
    "parallel",
    "repeat",
    "player.talk_to",
    "player.craft",
    "site.switch",
    "site.enter_room",
    "site.go_home",
];

/// The steps the registering crates give the hub's registry: the hub's own
/// state, then each crate that registers steps.
fn registered_steps() -> BTreeSet<String> {
    let mut app = App::new();
    moly_features::act::install(&mut app);
    moly_act::install(&mut app);
    moly_fixture_edit::layout_actions::install(&mut app);
    moly_harvest::gather_actions::install(&mut app);
    moly_delivery::deliver_action::install(&mut app);
    // The sequence player registers `sequence.play` and `sequence.stop` at
    // startup; its kinds are the ones it registers then.
    app.world_mut()
        .resource_mut::<moly_features::act::Actions>()
        .register_kinds(moly_sequence_player::kinds());
    let actions = app.world().resource::<moly_features::act::Actions>();
    actions.names().map(str::to_owned).collect()
}

/// The plugin document's step tables: the steps a table row names.
fn documented_steps() -> BTreeSet<String> {
    let document = include_str!("../../../../docs/plugins.md");
    let mut steps = BTreeSet::new();
    for name in document
        .lines()
        .filter_map(|line| line.strip_prefix("| `"))
        .filter_map(|rest| rest.split_once('`'))
        .map(|(name, _)| name)
    {
        if name.bytes().next().is_some_and(|first| first.is_ascii_lowercase())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'.' | b'_'))
        {
            steps.insert(name.to_owned());
        }
    }
    steps
}

fn schema() -> Value {
    serde_json::from_str(moly_devtools::director::SCHEMA).expect("the director schema is JSON")
}

fn schema_steps(schema: &Value) -> Vec<String> {
    schema["steps"]
        .as_array()
        .expect("the director schema lists its steps")
        .iter()
        .map(|step| step["name"].as_str().expect("every schema step has a name").to_owned())
        .collect()
}

#[test]
fn the_director_schema_names_every_registered_step() {
    let schema = schema();
    let steps = schema_steps(&schema);
    let registered = registered_steps();
    // A positive control: the registry is filled, so a registry nothing
    // reaches would not pass by both sides being empty.
    assert!(
        ["player.walk_to", "npc.ragdoll", "sequence.play", "layout.put", "player.gather", "player.deliver"]
            .iter()
            .all(|name| registered.contains(*name)),
        "the registry lacks the steps of a registering crate: {registered:?}"
    );
    let unlisted: Vec<&str> = registered.iter().filter(|name| !steps.contains(name)).map(String::as_str).collect();
    assert!(unlisted.is_empty(), "registered steps with no moly.director.schema entry: {}", unlisted.join(", "));

    let flow: BTreeSet<&str> = FLOW.iter().copied().collect();
    let unregistered: Vec<&str> = steps
        .iter()
        .map(String::as_str)
        .filter(|name| !registered.contains(*name) && !flow.contains(name))
        .collect();
    assert!(
        unregistered.is_empty(),
        "moly.director.schema steps that are neither registered nor flow steps: {}",
        unregistered.join(", ")
    );
}

#[test]
fn the_director_schema_names_every_step_once_and_offers_it() {
    let schema = schema();
    let steps = schema_steps(&schema);
    let offered: BTreeSet<&str> = schema["common"]["properties"]["do"]["enum"]
        .as_array()
        .expect("the action name lists its values")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    let mut listed = BTreeSet::new();
    let repeated: Vec<&str> = steps.iter().map(String::as_str).filter(|name| !listed.insert(*name)).collect();
    assert!(repeated.is_empty(), "steps listed twice in moly.director.schema: {}", repeated.join(", "));
    let unoffered: Vec<&str> = steps.iter().map(String::as_str).filter(|name| !offered.contains(name)).collect();
    assert!(
        unoffered.is_empty(),
        "moly.director.schema steps missing from the action name's values: {}",
        unoffered.join(", ")
    );
}

#[test]
fn the_director_examples_are_laid_out_as_the_schema_lays_them_out() {
    let schema = schema();
    let common: BTreeSet<&str> = schema["common"]["properties"]
        .as_object()
        .expect("the common fields are an object")
        .keys()
        .map(String::as_str)
        .collect();
    // A positive control: the examples are there to be read.
    let examples = moly_devtools::director::examples();
    assert!(!examples.is_empty(), "the director has no bundled examples");
    let mut wrong = Vec::new();
    for (title, script) in &examples {
        for step in script["steps"].as_array().expect("an example script lists its steps") {
            let name = step["do"].as_str().expect("an example step names its action");
            let Some(layout) = schema["steps"]
                .as_array()
                .and_then(|steps| steps.iter().find(|entry| entry["name"] == name))
            else {
                wrong.push(format!("{title}: {name} is not a step of the schema"));
                continue;
            };
            let fields: BTreeSet<&str> = step.as_object().expect("an example step is an object").keys().map(String::as_str).collect();
            let params = layout["params"]["properties"]
                .as_object()
                .expect("a step's params list their fields")
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>();
            // `server.edit` passes its fields through to the server (the
            // schema says so with open properties).
            if layout["params"]["additionalProperties"].as_bool() != Some(true) {
                for field in fields.difference(&common).filter(|field| !params.contains(*field)) {
                    wrong.push(format!("{title}: {name} has {field}, which its schema entry does not"));
                }
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn the_plugin_document_names_every_schema_step() {
    let steps = schema_steps(&schema());
    let documented = documented_steps();
    // A positive control: the document's tables are read as step rows.
    assert!(documented.contains("player.walk_to"), "the plugin document's step rows are not read");
    let undocumented: Vec<String> = steps.into_iter().filter(|name| !documented.contains(name)).collect();
    assert!(
        undocumented.is_empty(),
        "moly.director.schema steps with no row in docs/plugins.md: {}",
        undocumented.join(", ")
    );
}
