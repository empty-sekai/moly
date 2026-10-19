//! Case input and result output of the acceptance harness drivers: cases are
//! read from `<MOLY_NPC_HARNESS_DIR>/in/<name>.json` and results written to
//! `<MOLY_NPC_HARNESS_DIR>/out/<name>.json`.

use serde_json::Value;

pub fn case(name: &str) -> Value {
    let dir = std::env::var("MOLY_NPC_HARNESS_DIR").expect("MOLY_NPC_HARNESS_DIR");
    let text = std::fs::read_to_string(format!("{dir}/in/{name}.json"))
        .unwrap_or_else(|err| panic!("harness case {name}: {err}"));
    serde_json::from_str(&text).unwrap_or_else(|err| panic!("harness case {name}: {err}"))
}

pub fn emit(name: &str, value: &Value) {
    let dir = std::env::var("MOLY_NPC_HARNESS_DIR").expect("MOLY_NPC_HARNESS_DIR");
    std::fs::create_dir_all(format!("{dir}/out")).expect("harness out dir");
    std::fs::write(
        format!("{dir}/out/{name}.json"),
        serde_json::to_string(value).expect("harness result"),
    )
    .expect("write harness result");
}
