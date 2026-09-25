//! Owner words on the environment chains against native rows.
//!
//! The rows (path by `MOLY_SKY_OWNER_ROWS`) come from the engine's own
//! instructions: for the sky chain the owner update over the site root, the
//! environment view controller at a root position, the sky view, its effect
//! root and the prefab chain; for the camera chain the rotation cancel in the
//! effector's order (the parent's world rotation from the transform getter,
//! its inverse, the local-rotation setter on the prefab root) and then the
//! owner update. Every output word is compared bit for bit with this host's
//! composition, and so are the cancel's three intermediate quaternions.
//!
//! With `MOLY_SKY_OWNER_CORPUS` (a phenomena directory) the sky rows of the
//! exported chains are also rebuilt from the export through this host's own
//! chain reader, and the controller translation is checked to be the only
//! non-identity word above the prefab.
use super::*;
use moly_law::particle::owner::{cancel_rotation, global_rotation, OwnerScaling, SourceTrs};

fn words(value: &Value) -> Vec<u32> {
    value.as_array().expect("word array").iter().map(|v| v.as_u64().expect("word") as u32).collect()
}
fn trs(node: &Value) -> SourceTrs {
    let f = |key: &str| words(&node[key]).into_iter().map(f32::from_bits).collect::<Vec<_>>();
    let (t, q, s) = (f("t"), f("q"), f("s"));
    SourceTrs { t: [t[0], t[1], t[2]], q: [q[0], q[1], q[2], q[3]], s: [s[0], s[1], s[2]] }
}
fn bits(values: &[f32]) -> Vec<u32> {
    values.iter().map(|v| v.to_bits()).collect()
}
fn same_words(ours: &moly_law::particle::owner::OwnerMatrices, native: &Value) -> bool {
    bits(&ours.local_to_world) == words(&native["localToWorld"])
        && bits(&ours.world_to_local) == words(&native["worldToLocal"])
        && bits(&ours.rotation) == words(&native["rotation"])
        && bits(&ours.local_rotation) == words(&native["localRotation3x3"])
        && bits(&ours.emitter_scale) == words(&native["emitterScale"])
        && bits(&ours.local_to_world) == words(&native["copy"])
}

#[test]
#[ignore = "needs MOLY_SKY_OWNER_ROWS (native owner rows on the environment chains)"]
fn environment_owner_words_match_native_rows() {
    let path = std::env::var_os("MOLY_SKY_OWNER_ROWS").expect("MOLY_SKY_OWNER_ROWS");
    let receipt: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    assert_eq!(receipt["sha256"].as_str(), Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
    let corpus = std::env::var_os("MOLY_SKY_OWNER_CORPUS").map(std::path::PathBuf::from);
    let mut effects: HashMap<(String, String), Value> = HashMap::new();
    let rows = receipt["rows"].as_array().unwrap();
    let (mut compared, mut mismatched, mut refused, mut cancels) = (0usize, 0usize, 0usize, 0usize);
    let (mut no_cancel_red, mut site_chain_red, mut site_chain_rows) = (0usize, 0usize, 0usize);
    let (mut corpus_rows, mut corpus_rebuilt) = (0usize, 0usize);
    let mut groups: HashMap<String, (usize, usize)> = HashMap::new();
    for (index, row) in rows.iter().enumerate() {
        let prefix: Vec<SourceTrs> = row["prefix"].as_array().unwrap().iter().map(trs).collect();
        let prefab: Vec<SourceTrs> = row["prefab"].as_array().unwrap().iter().map(trs).collect();
        let cancel = row["cancel"].as_bool().unwrap();
        let native = &row["native"];
        let group = row["group"].as_str().unwrap().to_owned();
        let label = row["label"].as_str().unwrap();
        let entry = groups.entry(group.clone()).or_default();
        entry.0 += 1;
        if cancel {
            cancels += 1;
            let parent = global_rotation(&prefix).expect("finite prefix");
            assert_eq!(bits(&parent), words(&native["parentRotation"]), "row {index} {label}: parent rotation");
            let [x, y, z, w] = parent;
            assert_eq!(bits(&[-x, -y, -z, w]), words(&native["inverse"]), "row {index} {label}: inverse");
            assert_eq!(bits(&cancel_rotation(&prefix).unwrap()), words(&native["rootLocalRotation"]),
                "row {index} {label}: stored local rotation");
        }
        // The native rows all have a finite, invertible owner; a refusal here
        // is a mismatch.
        let ours = match environment_owner(&prefix, &prefab, cancel, OwnerScaling::Local) {
            Ok(ours) => ours,
            Err(reason) => {
                refused += 1;
                eprintln!("row {index} {label}: refused {reason}");
                continue;
            }
        };
        compared += 1;
        if !same_words(&ours, native) {
            mismatched += 1;
            entry.1 += 1;
            eprintln!("row {index} {label}: mismatch");
        }
        // Mutant 1: the parent's rotation is not cancelled.
        if cancel && environment_owner(&prefix, &prefab, false, OwnerScaling::Local)
            .map_or(true, |mutant| !same_words(&mutant, native)) {
            no_cancel_red += 1;
        }
        // Mutant 2: the site chain (the prefab chain alone) instead of the
        // environment chain; red wherever the chain above moves anything.
        let identity = SourceTrs { t: [0.0; 3], q: [0.0, 0.0, 0.0, 1.0], s: [1.0; 3] };
        if prefix.iter().any(|node| node.t != identity.t || node.q != identity.q || node.s != identity.s) || cancel {
            site_chain_rows += 1;
            if environment_owner(&[], &prefab, false, OwnerScaling::Local).map_or(true, |mutant| !same_words(&mutant, native)) {
                site_chain_red += 1;
            }
        }
        // The exported sky chains through this host's reader and prefix.
        if group == "corpus-sky" {
            corpus_rows += 1;
            let anchor = prefix[1].t;
            assert_eq!(bits(&prefix.iter().flat_map(|n| n.t.iter().chain(&n.q).chain(&n.s)).copied().collect::<Vec<_>>()),
                bits(&sky_prefix(anchor).iter().flat_map(|n| n.t.iter().chain(&n.q).chain(&n.s)).copied().collect::<Vec<_>>()),
                "row {index} {label}: the chain above the prefab is not the sky prefix");
            if let Some(corpus) = corpus.as_ref() {
                let parts: Vec<&str> = label.split('|').collect();
                let (phenomenon, effect, node) = (parts[0], parts[1], parts[2]);
                let doc = effects.entry((phenomenon.to_owned(), effect.to_owned())).or_insert_with(|| {
                    let file = corpus.join(phenomenon).join("fx/effects.json");
                    let all: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
                    all["effects"][effect].clone()
                });
                let by_path: HashMap<String, &Value> = doc["nodes"].as_array().unwrap().iter()
                    .map(|node| (node["path"].as_str().unwrap().to_owned(), node)).collect();
                let (_, chain) = chain_owner(&by_path, node, EffectKind::Sky, OwnerScaling::Local).expect("sky chain owner");
                let chain = chain.expect("a sky owner keeps its chain").chain;
                assert_eq!(chain.len(), prefab.len(), "row {index} {label}: chain length");
                for (a, b) in chain.iter().zip(&prefab) {
                    assert_eq!(bits(&[a.t, [a.q[0], a.q[1], a.q[2]], a.s].concat()), bits(&[b.t, [b.q[0], b.q[1], b.q[2]], b.s].concat()),
                        "row {index} {label}: exported chain words");
                    assert_eq!(a.q[3].to_bits(), b.q[3].to_bits(), "row {index} {label}: exported chain w");
                }
                let rebuilt = environment_owner(&sky_prefix(anchor), &chain, false, OwnerScaling::Local).expect("rebuilt owner");
                assert!(same_words(&rebuilt, native), "row {index} {label}: rebuilt from the export");
                corpus_rebuilt += 1;
            }
        }
    }
    let mut groups: Vec<_> = groups.into_iter().collect();
    groups.sort();
    eprintln!("sky owner receipt: rows {}, compared {compared}, mismatched {mismatched}, refused {refused}; cancels {cancels}; \
        per group (rows, mismatched) {groups:?}; corpus rows {corpus_rows}, rebuilt from the export {corpus_rebuilt}; \
        mutants: no cancel red {no_cancel_red}/{cancels}, site chain red {site_chain_red}/{site_chain_rows}",
        rows.len());
    assert_eq!(compared, rows.len());
    assert_eq!(mismatched, 0);
    assert!(cancels > 0 && no_cancel_red > 0, "the no-cancel mutant must be red");
    assert!(site_chain_red > 0, "the site-chain mutant must be red");
    if corpus.is_some() {
        assert_eq!(corpus_rebuilt, corpus_rows);
    }
}

/// The Hierarchy owner words of the exported sub-emitter targets that use
/// that mode, rebuilt from the export through this host's chain reader and
/// compared with the native rows (path by `MOLY_HIER_OWNER_ROWS`, corpus by
/// `MOLY_SKY_OWNER_CORPUS`): the site targets on their authored chain, the sky
/// targets under the environment chain at every root position of the rows.
/// The law's own replay of every row is in moly-law; this is the product's
/// reader and prefix on the corpus rows.
#[test]
#[ignore = "needs MOLY_HIER_OWNER_ROWS and MOLY_SKY_OWNER_CORPUS"]
fn hierarchy_target_owner_words_rebuild_from_the_export() {
    let rows_path = std::env::var_os("MOLY_HIER_OWNER_ROWS").expect("MOLY_HIER_OWNER_ROWS");
    let corpus = std::path::PathBuf::from(std::env::var_os("MOLY_SKY_OWNER_CORPUS").expect("MOLY_SKY_OWNER_CORPUS"));
    let receipt: Value = serde_json::from_slice(&std::fs::read(rows_path).unwrap()).unwrap();
    assert_eq!(receipt["sha256"].as_str(), Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
    let mut effects: HashMap<String, Value> = HashMap::new();
    let (mut site, mut sky, mut local_red) = (0usize, 0usize, 0usize);
    for (index, row) in receipt["rows"].as_array().unwrap().iter().enumerate() {
        let group = row["group"].as_str().unwrap();
        if group != "corpus-site" && group != "corpus-sky" {
            continue;
        }
        let label = row["label"].as_str().unwrap();
        let parts: Vec<&str> = label.split('|').collect();
        let (phenomenon, effect, node) = (parts[0], parts[1], parts[2]);
        let doc = effects.entry(effect.to_owned()).or_insert_with(|| {
            let file = corpus.join(phenomenon).join("fx/effects.json");
            let all: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            all["effects"][effect].clone()
        });
        let by_path: HashMap<String, &Value> = doc["nodes"].as_array().unwrap().iter()
            .map(|node| (node["path"].as_str().unwrap().to_owned(), node)).collect();
        let system = doc["particles"].as_array().unwrap().iter()
            .find(|p| p["node"].as_str() == Some(node)).expect("target record")["system"].clone();
        assert_eq!(system["scalingMode"].as_u64(), Some(0), "row {index} {label}: not a Hierarchy target");
        let native = &row["native"];
        let (kind, prefix) = if group == "corpus-site" {
            site += 1;
            (EffectKind::Site, Vec::new())
        } else {
            sky += 1;
            (EffectKind::Sky, row["prefix"].as_array().unwrap().iter().map(trs).collect::<Vec<_>>())
        };
        let (owner, chain) = chain_owner(&by_path, node, kind, OwnerScaling::Hierarchy).expect("chain owner");
        let composed = match chain {
            None => owner,
            Some(chain) => {
                assert_eq!(chain.scaling, OwnerScaling::Hierarchy);
                let anchor = prefix[1].t;
                environment_owner(&sky_prefix(anchor), &chain.chain, false, chain.scaling).expect("rebuilt owner")
            }
        };
        assert!(same_words(&composed, native), "row {index} {label}: Hierarchy words from the export");
        let child = moly_law::particle::child_emit::ChildOwner::from_owner(&composed);
        assert_eq!(bits(&child.shape_scale), words(&native["shapeScale"]), "row {index} {label}: shape scale");
        if !same_words(&composed, &row["nativeLocal"]) {
            local_red += 1;
        }
    }
    eprintln!("hierarchy target owner words from the export: site {site}, sky {sky}, 0 mismatched; \
        rows whose native Local words differ {local_red}");
    assert!(site > 0 && sky > 0);
}
