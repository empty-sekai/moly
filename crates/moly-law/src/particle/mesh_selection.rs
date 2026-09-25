//! Mesh render mode: which of the renderer's meshes a particle draws.
//!
//! Tier: engine native, instruction for instruction (constants pinned by bit).
//!
//! The renderer keeps a cache of its mesh slots: in slot order, every slot
//! whose mesh resolves and is drawable, packed to the front of four entries
//! (the rest null). The geometry preparation takes the mesh count as the run
//! of leading non-null cache entries; with the first entry null the count is
//! zero, no per-mesh data is prepared, and nothing is drawn for the system
//! (it still simulates: the particle update never reads its renderer). With
//! the non-uniform distribution the weight of cache entry `i` is the authored
//! weight at position `i` (the weights are indexed by cache position, not by
//! the slot the mesh came from), and the total is their running sum from +0,
//! each step `weight + total`.
//!
//! Per particle:
//! - fewer than two meshes: mesh 0;
//! - otherwise a hash of the particle seed, `h = seed + 0xbc524e5f`,
//!   `m = h * 0x6ab51b9d + 0x714acb3f`, `x = h ^ (h << 11)`,
//!   `r = m ^ x ^ (x >> 8) ^ (m >> 19)`;
//! - a total weight equal to zero (the uniform distribution leaves it at +0):
//!   `r % count`;
//! - otherwise `v = total * unit(r)` (`unit` = the low 23 bits times
//!   2^-23 (1 + 2^-23)), then for each mesh but the last, the first whose
//!   weight is above `v` (the running `v` loses each passed weight), else the
//!   last mesh. A NaN total takes this branch; a NaN `v` passes every mesh.
//!
//! A per-particle mesh index written through the script API overrides the
//! draw; this runtime has no such writer, so it is not modelled.

const HASH_T: u32 = 0xbc52_4e5f;
const HASH_M: u32 = 0x6ab5_1b9d;
const HASH_C: u32 = 0x714a_cb3f;
const UNIT: f32 = f32::from_bits(0x3400_0001);

/// The meshes a renderer draws and how a particle picks one.
#[derive(Clone, Debug, PartialEq)]
pub struct MeshSelection {
    weights: Vec<f32>,
    total: f32,
}

impl MeshSelection {
    /// `weights` holds one authored weight per cached mesh, the first
    /// `count` authored weights in position order (their count is the mesh
    /// count, at most four); `non_uniform` is the renderer's
    /// NonUniformRandom distribution. The uniform distribution never reads
    /// the weights and its total stays +0.
    pub fn new(weights: &[f32], non_uniform: bool) -> Result<Self, &'static str> {
        if weights.len() > 4 {
            return Err("a particle renderer caches at most four meshes");
        }
        let total = if non_uniform { weights.iter().fold(0.0_f32, |total, weight| weight + total) } else { 0.0 };
        Ok(Self { weights: weights.to_vec(), total })
    }

    pub fn count(&self) -> usize {
        self.weights.len()
    }

    /// The running total the selection scales its draw by.
    pub fn total(&self) -> f32 {
        self.total
    }

    /// The mesh index of a particle with this seed. A renderer with no mesh
    /// draws nothing, so the caller never asks; zero is returned for it.
    pub fn index(&self, seed: u32) -> usize {
        let count = self.weights.len();
        if count < 2 {
            return 0;
        }
        let h = seed.wrapping_add(HASH_T);
        let m = h.wrapping_mul(HASH_M).wrapping_add(HASH_C);
        let x = h ^ (h << 11);
        let r = m ^ x ^ (x >> 8) ^ (m >> 19);
        if self.total == 0.0 {
            return (r % count as u32) as usize;
        }
        let mut v = self.total * (((r & 0x007f_ffff) as f32) * UNIT);
        for (index, weight) in self.weights[..count - 1].iter().enumerate() {
            if v < *weight {
                return index;
            }
            v -= weight;
        }
        count - 1
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_or_no_mesh_draws_mesh_zero() {
        for weights in [&[][..], &[3.0][..]] {
            let selection = MeshSelection::new(weights, true).unwrap();
            assert!((0..64).all(|seed| selection.index(seed) == 0));
        }
    }

    #[test]
    fn uniform_selection_covers_every_mesh() {
        let selection = MeshSelection::new(&[1.0, 1.0, 1.0], false).unwrap();
        assert_eq!(selection.total(), 0.0);
        let mut seen = [false; 3];
        for seed in 0..256 { seen[selection.index(seed)] = true; }
        assert_eq!(seen, [true; 3]);
    }

    #[test]
    fn weighted_selection_never_picks_a_zero_weight_before_the_last() {
        let selection = MeshSelection::new(&[0.0, 2.0, 0.0], true).unwrap();
        assert!((0..512).all(|seed| selection.index(seed) == 1));
    }
}

/// Native rows of the per-particle mesh index loop of the geometry
/// preparation, run in the game's own JP libunity from its first instruction
/// to the index register: every row's mesh count, distribution, weights,
/// total weight (as the preparation accumulates it) and seeds, and the index
/// per seed. Rows with a script-API mesh index (not modelled) are skipped
/// and counted. A control file of mutated rows must fail.
#[cfg(test)]
mod native_rows {
    use super::*;
    use crate::particle::json::{parse, Value};

    fn number(value: &Value, key: &str) -> f64 {
        value.get(key).and_then(Value::as_f64).unwrap_or_else(|| panic!("{key}"))
    }

    /// (rows, seeds, skipped rows, mismatches, first mismatches)
    fn replay(path: &std::ffi::OsStr) -> (usize, usize, usize, usize, Vec<String>) {
        let receipt = parse(&std::fs::read(path).unwrap()).unwrap();
        assert_eq!(receipt.get("sourceSha256").and_then(Value::as_str),
            Some("937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9"));
        let (mut rows, mut seeds, mut skipped, mut bad) = (0, 0, 0, 0);
        let mut first = Vec::new();
        for row in receipt.get("rows").and_then(Value::as_array).expect("rows") {
            let name = row.get("name").and_then(Value::as_str).unwrap_or("?");
            if !matches!(row.get("meshIndex"), None | Some(Value::Null)) {
                skipped += 1;
                continue;
            }
            let count = number(row, "count") as usize;
            let weights: Vec<f32> = row.get("weightsBits").and_then(Value::as_array).expect("weights")
                .iter().map(|w| f32::from_bits(w.as_f64().unwrap() as u32)).collect();
            assert_eq!(weights.len(), count, "{name}");
            let selection = MeshSelection::new(&weights, number(row, "distribution") == 1.0).expect(name);
            if selection.total().to_bits() != number(row, "totalBits") as u32 {
                bad += 1;
                first.push(format!("{name} total {:08x}", selection.total().to_bits()));
            }
            let expected = row.get("index").and_then(Value::as_array).expect("index");
            for (seed, index) in row.get("seed").and_then(Value::as_array).expect("seed").iter().zip(expected) {
                let seed = seed.as_f64().unwrap() as u32;
                let got = selection.index(seed);
                if got as f64 != index.as_f64().unwrap() {
                    bad += 1;
                    if first.len() < 12 {
                        first.push(format!("{name} seed {seed:08x}: {got} native {}", index.as_f64().unwrap()));
                    }
                }
                seeds += 1;
            }
            rows += 1;
        }
        (rows, seeds, skipped, bad, first)
    }

    #[test]
    #[ignore = "MOLY_MESH_SELECT_ROWS identifies the current JP mesh selection rows"]
    fn mesh_selection_matches_native_rows() {
        let path = std::env::var_os("MOLY_MESH_SELECT_ROWS").expect("MOLY_MESH_SELECT_ROWS");
        let (rows, seeds, skipped, bad, first) = replay(&path);
        println!("mesh selection rows: {rows} rows ({skipped} with a script mesh index skipped), {seeds} seeds, {bad} mismatched {first:?}");
        assert!(rows > 0 && seeds > 0);
        assert_eq!(bad, 0, "{first:?}");
    }

    #[test]
    #[ignore = "MOLY_MESH_SELECT_ROWS_CONTROLS identifies mutated mesh selection rows"]
    fn mesh_selection_controls_fail() {
        let paths = std::env::var("MOLY_MESH_SELECT_ROWS_CONTROLS").expect("MOLY_MESH_SELECT_ROWS_CONTROLS");
        for path in paths.split(';').filter(|p| !p.is_empty()) {
            let (rows, _, _, bad, _) = replay(std::ffi::OsStr::new(path));
            println!("mesh selection control {path}: {rows} rows, {bad} mismatched");
            assert!(bad > 0, "control {path} must mismatch");
        }
    }
}
