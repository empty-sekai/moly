//! No finite export may make the admission of a sub-emitter target recurse
//! without end. The target's admission judges its parent's record, which
//! judges the parent's own parent when that parent is a target in turn; a
//! self-edge or a cycle of parents must be refused by name before anything
//! recurses. A chain that ends passes the same gate with the same system
//! fields, so the refusal below is the chain's and nothing else's.
use super::*;
use serde_json::json;

/// One system record: its node, the targets its SubModule names, and the
/// system fields the gate reads after the chain (all on their passing side).
fn record(node: &str, targets: &[&str]) -> Value {
    json!({
        "node": node,
        "system": {
            "useUnscaledTime": false,
            "prewarm": false,
            "scalingMode": 1,
            "subEmitters": targets.iter().map(|target| json!({ "emitter": target })).collect::<Vec<_>>(),
        },
    })
}

fn gate(records: &[Value], node: &str) -> Result<String, String> {
    let graph = source_sub_emitter_owners(records);
    let particle = records.iter().find(|record| record["node"] == node).expect("record of the node");
    sub_emitter_target_gate(&graph[node], particle, &graph, EffectKind::Site, None, false)
}

#[test]
fn a_chain_of_parents_that_ends_passes_the_target_gate() {
    let records = [record("a", &["a/b"]), record("a/b", &["a/b/c"]), record("a/b/c", &[])];
    assert_eq!(gate(&records, "a/b/c"), Ok("a/b".to_owned()));
    assert_eq!(gate(&records, "a/b"), Ok("a".to_owned()));
}

#[test]
fn a_self_edge_is_refused_before_admission_recurses() {
    let records = [record("a", &["a"])];
    assert_eq!(gate(&records, "a"), Err("sub-emitter chain revisits a".to_owned()));
}

#[test]
fn a_two_system_cycle_is_refused_from_either_side() {
    let records = [record("a", &["b"]), record("b", &["a"])];
    assert_eq!(gate(&records, "a"), Err("sub-emitter chain revisits a".to_owned()));
    assert_eq!(gate(&records, "b"), Err("sub-emitter chain revisits b".to_owned()));
}

#[test]
fn a_cycle_above_the_target_is_refused_where_it_closes() {
    // t's parent a sits on the cycle a <-> b; t itself is not on it.
    let records = [record("a", &["b", "t"]), record("b", &["a"]), record("t", &[])];
    assert_eq!(gate(&records, "t"), Err("sub-emitter chain revisits a".to_owned()));
}
