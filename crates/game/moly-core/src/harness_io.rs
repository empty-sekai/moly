//! Case input and result output of the acceptance harness drivers: cases are
//! read from `<dev.npc_harness_dir>/in/<name>.json` and results written to
//! `<dev.npc_harness_dir>/out/<name>.json`, where `dev.npc_harness_dir` is
//! `MOLY_NPC_HARNESS_DIR`.

use serde_json::Value;

pub fn case(name: &str) -> Value {
    let dir = harness_dir();
    let text = std::fs::read_to_string(format!("{dir}/in/{name}.json"))
        .unwrap_or_else(|err| panic!("harness case {name}: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("harness case {name}: {err}"))
}

fn harness_dir() -> String {
    moly_config::text(&moly_config::keys::DEV_NPC_HARNESS_DIR)
        .expect("dev.npc_harness_dir (MOLY_NPC_HARNESS_DIR) must be set")
}

pub fn emit(name: &str, value: &Value) {
    let dir = harness_dir();
    std::fs::create_dir_all(format!("{dir}/out")).expect("harness out dir");
    std::fs::write(
        format!("{dir}/out/{name}.json"),
        serde_json::to_string(value).expect("harness result"),
    )
    .expect("write harness result");
}
