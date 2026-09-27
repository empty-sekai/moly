//! Mesh shape in its Triangle placement: ShapeModule's mesh cache and the
//! triangle sample of one four-lane birth group.
//! Transcribed from the current JP client's engine library.
//!
//! The cache is what the module rebuilds whenever its mesh data changes.
//! Each included submesh (every submesh, or only the one the material index
//! names when the material filter is on) contributes its triangles in index
//! order, each with its area and the submesh's dense ordinal among the
//! included ones as its material. The area is half the square root of
//! `cz^2 + (cx^2 + cy^2)` for the cross product `(b - a) x (c - a)`. The
//! areas are summed per submesh from zero, and each submesh sum is added to
//! the running total. A lookup table of `min(T, 50)` entries follows: entry
//! `i` targets `step * i` with `step = total / n`, and holds the cumulative
//! area and the index of the first triangle whose inclusion would pass that
//! target. Every included submesh gets the module's default material colour,
//! opaque white, written by the module's static initializer; the renderer
//! materials replace it only when the module uses mesh colours, which this
//! envelope refuses.
//!
//! One group draws once from the Shape stream (the table pick of all four
//! lanes), walks from the table entry to the triangle, then draws twice (the
//! two barycentric weights). A walk that starts above the target goes back
//! until the cumulative area falls below it; otherwise it goes forward until
//! it reaches it; a walk off either end yields the first triangle. The
//! corners are blended with weights `(a, b, (1 - a) - b)`, where `a, b` are
//! the two draws or their complements when they sum past one; the normal is
//! blended the same way (a mesh without a normal channel reads +Z at every
//! corner) and is not normalised here. The position is pushed along that
//! normal by the normal offset. The caller's Store step then follows as for
//! every other kernel, and afterwards each lane's particle colour is
//! multiplied by the triangle's material colour; a lane whose alpha byte
//! becomes zero is given an age past the newborn kill.
/// Lookup table size cap.
pub const LOOKUP_ENTRIES: usize = 50;
/// The module's default material colour (RGBA bytes).
pub const DEFAULT_MATERIAL_COLOUR: [u8; 4] = [0xff; 4];
/// The normalized age a lane is given when its colour alpha becomes zero;
/// greater than 100, so the newborn kill removes it.
pub const DEAD_AGE_PERCENT: f32 = f32::from_bits(0x42c8_0001);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MeshTriangle {
    pub area: f32,
    pub corners: [u32; 3],
    /// Dense ordinal of its submesh among the included submeshes.
    pub material: u32,
}

/// Why a mesh cannot be cached for the Triangle kernel.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MeshCacheRefused {
    /// A corner index outside the vertex array.
    IndexOutOfRange,
    /// A normal channel whose length differs from the position channel.
    NormalCount,
    /// A submesh whose index count is not a multiple of three.
    PartialTriangle,
    /// No triangle is included. ShapeModule::Start then stores nothing and
    /// draws nothing for the group; that path is not transcribed.
    NoTriangles,
}

#[derive(Clone, Debug)]
pub struct MeshShapeCache {
    positions: Vec<[f32; 3]>,
    normals: Option<Vec<[f32; 3]>>,
    triangles: Vec<MeshTriangle>,
    total_area: f32,
    lookup: Vec<(f32, u32)>,
    material_colours: Vec<[u8; 4]>,
    material_filter: Option<u32>,
}

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

/// The triangle area of the cache, in the native operand order.
pub fn triangle_area(a: [f32; 3], b: [f32; 3], c: [f32; 3]) -> f32 {
    let e1 = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
    let e2 = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
    let cx = e1[1] * e2[2] - e1[2] * e2[1];
    let cy = e1[2] * e2[0] - e1[0] * e2[2];
    let cz = e1[0] * e2[1] - e1[1] * e2[0];
    (cz * cz + (cx * cx + cy * cy)).sqrt() * 0.5
}

/// The per-byte colour product of the Store's colour step:
/// `(x + (x >> 8)) >> 8` with `x = a * b + 128`.
pub fn multiply_colour(start: [u8; 4], mesh: [u8; 4]) -> [u8; 4] {
    std::array::from_fn(|k| {
        let x = u32::from(start[k]) * u32::from(mesh[k]) + 128;
        ((x + (x >> 8)) >> 8) as u8
    })
}

/// Round to nearest even in f32, as the native pick does: add and subtract
/// 2^23 carrying the value's sign.
fn round_even(x: f32) -> f32 {
    let m = f32::from_bits(0x4b00_0000 | (x.to_bits() & 0x8000_0000));
    (x + m) - m
}

/// The vector minimum of the pick: NaN in either operand yields NaN.
fn native_min(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a < b {
        a
    } else {
        b
    }
}

impl MeshShapeCache {
    /// `submeshes` are index lists in the source's own triangle order.
    /// `material_filter` is the material index when the module's material
    /// filter is on.
    pub fn new(
        positions: Vec<[f32; 3]>,
        normals: Option<Vec<[f32; 3]>>,
        submeshes: &[Vec<u32>],
        material_filter: Option<u32>,
    ) -> Result<Self, MeshCacheRefused> {
        if normals.as_ref().is_some_and(|n| n.len() != positions.len()) {
            return Err(MeshCacheRefused::NormalCount);
        }
        let mut triangles = Vec::new();
        let mut total_area = 0.0f32;
        let mut included = 0u32;
        for (k, indices) in submeshes.iter().enumerate() {
            if material_filter.is_some_and(|m| u64::from(m) != k as u64) {
                continue;
            }
            if indices.len() % 3 != 0 {
                return Err(MeshCacheRefused::PartialTriangle);
            }
            let mut sum = 0.0f32;
            for corners in indices.chunks_exact(3) {
                let corners = [corners[0], corners[1], corners[2]];
                if corners.iter().any(|&i| i as usize >= positions.len()) {
                    return Err(MeshCacheRefused::IndexOutOfRange);
                }
                let [a, b, c] = corners.map(|i| positions[i as usize]);
                let area = triangle_area(a, b, c);
                sum += area;
                triangles.push(MeshTriangle { area, corners, material: included });
            }
            total_area = sum + total_area;
            included += 1;
        }
        if triangles.is_empty() {
            return Err(MeshCacheRefused::NoTriangles);
        }
        let n = triangles.len().min(LOOKUP_ENTRIES);
        let step = total_area / n as f32;
        let mut lookup = Vec::with_capacity(n);
        let (mut j, mut cumulative) = (0usize, 0.0f32);
        for i in 0..n {
            let target = step * i as f32;
            while j < triangles.len() {
                let next = cumulative + triangles[j].area;
                let within = if arms::on("lutStrict") { next < target } else { next <= target };
                if !within {
                    break;
                }
                cumulative = next;
                j += 1;
            }
            lookup.push((cumulative, j as u32));
        }
        Ok(Self {
            positions,
            normals,
            triangles,
            total_area,
            lookup,
            material_colours: vec![DEFAULT_MATERIAL_COLOUR; included as usize],
            material_filter,
        })
    }

    pub fn triangles(&self) -> &[MeshTriangle] {
        &self.triangles
    }
    pub fn total_area(&self) -> f32 {
        self.total_area
    }
    pub fn lookup(&self) -> &[(f32, u32)] {
        &self.lookup
    }
    pub fn material_colours(&self) -> &[[u8; 4]] {
        &self.material_colours
    }
    pub fn material_filter(&self) -> Option<u32> {
        self.material_filter
    }

    /// The triangle one lane picks for its table draw `r` (in [0, 1]).
    pub fn pick(&self, r: f32) -> usize {
        let count = self.triangles.len();
        let n = self.lookup.len() as f32;
        let target = r * self.total_area;
        let scaled = r * n;
        let start = if arms::on("lutFloor") { scaled.floor() } else { round_even(scaled) };
        // Saturating conversion toward zero (NaN converts to zero). The draw
        // is in [0, 1], so the entry index is in [0, n - 1].
        let k = native_min(start, n + -1.0) as i32;
        let (mut cumulative, from) = self.lookup[k as usize];
        let from = from as usize;
        if cumulative <= target {
            let mut x = from;
            while x < count {
                cumulative = cumulative + self.triangles[x].area;
                if cumulative >= target {
                    return x;
                }
                x += 1;
            }
            0
        } else {
            if arms::on("noBackward") {
                return if from < count { from } else { 0 };
            }
            let mut x = from;
            while x > 0 {
                x -= 1;
                cumulative = cumulative - self.triangles[x].area;
                if cumulative < target {
                    return x;
                }
            }
            0
        }
    }

    /// One lane's local position and normal on triangle `triangle` for the
    /// barycentric draws `u`, `v`, pushed `normal_offset` along the normal.
    pub fn point(&self, triangle: usize, u: f32, v: f32, normal_offset: f32) -> ([f32; 3], [f32; 3]) {
        let corners = self.triangles[triangle].corners.map(|i| i as usize);
        let (u, v) = if arms::on("swapUV") { (v, u) } else { (u, v) };
        let (a, b) = if u + v > 1.0 { (1.0 - u, 1.0 - v) } else { (u, v) };
        let c = (1.0 - a) - b;
        let blend = |p: [[f32; 3]; 3]| -> [f32; 3] {
            std::array::from_fn(|axis| (p[0][axis] * a + p[1][axis] * b) + p[2][axis] * c)
        };
        let position = blend(corners.map(|i| self.positions[i]));
        let normal = match &self.normals {
            Some(normals) => blend(corners.map(|i| normals[i])),
            None => blend([[0.0, 0.0, 1.0]; 3]),
        };
        if arms::on("noNormalOffset") {
            return (position, normal);
        }
        (std::array::from_fn(|axis| normal[axis] * normal_offset + position[axis]), normal)
    }

    /// The material colour the Store multiplies a lane's colour with.
    pub fn material_colour(&self, triangle: usize) -> [u8; 4] {
        self.material_colours[self.triangles[triangle].material as usize]
    }

    /// The lane's particle colour after the Store's colour step, and whether
    /// the lane is then marked dead (its alpha byte became zero).
    pub fn store_colour(start: [u8; 4], mesh: [u8; 4]) -> ([u8; 4], bool) {
        let colour = multiply_colour(start, mesh);
        (colour, colour[3] == 0 && !arms::on("noDeadMark"))
    }
}

/// Receipt rows of the native mesh run: the module cache, every group's
/// Store boundary and every stored lane, colours and dead marks included.
#[cfg(test)]
pub(crate) fn replay_mesh_rows(text: &str) -> MeshReplay {
    use super::json::{parse, Value};
    use super::schema::{ShapeControls, ShapeParams, ShapeTexture};
    use super::seed_owner::ModuleRandom;
    use super::shape::ArcLoopClock;
    use super::shape_birth::{ShapeBatch, ShapeBirthLaw};
    let doc = parse(text.as_bytes()).expect("receipt json");
    let words = |v: &Value| -> Vec<u32> {
        v.as_array().expect("word list").iter().map(|x| x.as_f64().expect("word") as u32).collect()
    };
    let bits3 = |v: &Value| -> [f32; 3] {
        let w = words(v);
        [f32::from_bits(w[0]), f32::from_bits(w[1]), f32::from_bits(w[2])]
    };
    let random = |w: &[u32]| ModuleRandom { words: std::array::from_fn(|word| std::array::from_fn(|lane| w[word * 4 + lane])) };
    let bytes = |w: u32| w.to_le_bytes();
    let mut out = MeshReplay::default();
    for (case, row) in doc.get("rows").and_then(Value::as_array).expect("rows").iter().enumerate() {
        let get = |k: &str| row.get(k).unwrap_or_else(|| panic!("case {case}: {k}"));
        let shape = get("shape");
        let s = |k: &str| shape.get(k).unwrap_or_else(|| panic!("case {case}: shape.{k}"));
        let num = |v: &Value| v.as_f64().expect("number") as u32;
        let use_material = num(s("useMaterialIndex")) != 0;
        let params = ShapeParams {
            shape_type: "Mesh".into(),
            radius: 1.0,
            radius_thickness: 1.0,
            arc: 360.0,
            rotation: bits3(s("rotation")),
            position: bits3(s("position")),
            controls: ShapeControls {
                source_version: Some(1),
                scale: Some(bits3(s("scale"))),
                arc_mode: Some(super::schema::ShapeMode::Random),
                align_to_direction: Some(num(s("align")) != 0),
                random_direction: Some(f32::from_bits(num(s("randomDirection")))),
                spherical_direction: Some(f32::from_bits(num(s("spherical")))),
                random_position: Some(f32::from_bits(num(s("randomPosition")))),
                texture: Some(ShapeTexture::None),
                mesh_placement: Some(num(s("placement"))),
                mesh_normal_offset: Some(f32::from_bits(num(s("normalOffset")))),
                mesh_use_colors: Some(num(s("useColors")) != 0),
                mesh_use_material_index: Some(use_material),
                mesh_material_index: use_material.then(|| num(s("materialIndex"))),
                ..Default::default()
            },
        };
        let law = ShapeBirthLaw::from_params(&params)
            .unwrap_or_else(|refused| panic!("case {case}: refused a row inside the executed envelope: {refused:?}"));
        let positions: Vec<[f32; 3]> = get("positions").as_array().unwrap().iter().map(bits3).collect();
        let normals = match get("normals") {
            Value::Null => None,
            list => Some(list.as_array().unwrap().iter().map(bits3).collect::<Vec<_>>()),
        };
        let submeshes: Vec<Vec<u32>> = get("submeshes").as_array().unwrap().iter().map(words).collect();
        let cache = MeshShapeCache::new(positions, normals, &submeshes, use_material.then(|| num(s("materialIndex"))))
            .unwrap_or_else(|refused| panic!("case {case}: cache refused {refused:?}"));
        let native = get("native");
        let n = |k: &str| native.get(k).unwrap_or_else(|| panic!("case {case}: native.{k}"));
        let ncache = n("cache");
        let triangles: Vec<Vec<u32>> = ncache.get("triangles").unwrap().as_array().unwrap().iter().map(words).collect();
        let lut: Vec<Vec<u32>> = ncache.get("lut").unwrap().as_array().unwrap().iter().map(words).collect();
        let mut red = Vec::new();
        let law_triangles: Vec<Vec<u32>> = cache
            .triangles()
            .iter()
            .map(|t| vec![t.area.to_bits(), t.corners[0], t.corners[1], t.corners[2], t.material])
            .collect();
        if law_triangles != triangles {
            red.push("triangles".to_string());
        }
        if cache.total_area().to_bits() != num(ncache.get("totalArea").unwrap()) {
            red.push("total".into());
        }
        let law_lut: Vec<Vec<u32>> = cache.lookup().iter().map(|&(c, j)| vec![c.to_bits(), j]).collect();
        if law_lut != lut {
            red.push("lut".into());
        }
        let colours: Vec<u32> = cache.material_colours().iter().map(|c| u32::from_le_bytes(*c)).collect();
        if colours != words(ncache.get("materialColors").unwrap()) {
            red.push("materialColours".into());
        }
        let owner16 = words(get("owner"));
        let owner: [f32; 16] = std::array::from_fn(|i| f32::from_bits(owner16[i]));
        let world = get("ownerName").as_str() != Some("identity");
        let emitter_scale = bits3(get("emitterScale"));
        let axis_flag = num(get("axisFlag")) != 0;
        let start_colours = words(get("colors"));
        let count = num(get("count")) as usize;
        let first = num(get("first")) as usize;
        let arrays = n("arrays");
        let axis_arrays = n("axisArrays");
        let array = |set: &Value, key: &str| words(set.get(key).unwrap_or_else(|| panic!("case {case}: array {key}")));
        let stored: Vec<Vec<u32>> = ["0x0", "0x20", "0x40", "0x60", "0x80", "0xa0", "0x360", "0x3c0"]
            .iter()
            .map(|k| array(arrays, k))
            .collect();
        let axes: Option<Vec<Vec<u32>>> =
            axis_flag.then(|| ["0x180", "0x1a0", "0x1c0"].iter().map(|k| array(axis_arrays, k)).collect());
        let mut stream = random(&words(get("rng")));
        let mut batch = ShapeBatch::new(std::num::NonZeroU32::MIN, 0.0, 0.0, ArcLoopClock::default());
        let groups = n("groups").as_array().unwrap();
        let mut slot = first;
        let mut index = 0;
        while slot < count {
            let g = &groups[index];
            assert_eq!(num(g.get("first").unwrap()) as usize, slot, "case {case}: group order");
            let group = law
                .evaluate_group_with_mesh(&batch, stream, owner, world, emitter_scale, axis_flag, Some(&cache))
                .unwrap_or_else(|refused| panic!("case {case}: group refused {refused:?}"));
            let local_position: Vec<Vec<u32>> = g.get("localPosition").unwrap().as_array().unwrap().iter().map(words).collect();
            let local_direction: Vec<Vec<u32>> = g.get("localDirection").unwrap().as_array().unwrap().iter().map(words).collect();
            let mesh_colour = group.mesh_colour.expect("a mesh group carries its colours");
            for lane in 0..4 {
                let at = slot + lane;
                for axis in 0..3 {
                    if group.raw_position[lane][axis].to_bits() != local_position[axis][lane] {
                        red.push(format!("pos@{at}"));
                    }
                    if group.raw_direction[lane][axis].to_bits() != local_direction[axis][lane] {
                        red.push(format!("dir@{at}"));
                    }
                    if group.samples[lane].position[axis].to_bits() != stored[axis][at] {
                        red.push(format!("storedPos@{at}"));
                    }
                    if group.samples[lane].direction[axis].to_bits() != stored[3 + axis][at] {
                        red.push(format!("storedDir@{at}"));
                    }
                    if let (Some(axes), Some(law_axes)) = (&axes, &group.axis_of_rotation) {
                        if law_axes[lane][axis].to_bits() != axes[axis][at] {
                            red.push(format!("axis@{at}"));
                        }
                    }
                }
                let start = start_colours.get(at).copied().unwrap_or(0x0102_0304);
                let (colour, dead) = MeshShapeCache::store_colour(bytes(start), mesh_colour[lane]);
                if u32::from_le_bytes(colour) != stored[6][at] {
                    red.push(format!("colour@{at}"));
                }
                let age = if dead { DEAD_AGE_PERCENT.to_bits() } else { 0 };
                if age != stored[7][at] {
                    red.push(format!("age@{at}"));
                }
            }
            if group.before_store != random(&words(g.get("rngBeforeStore").unwrap())) {
                red.push(format!("rngBeforeStore@{slot}"));
            }
            if group.after_rng != random(&words(g.get("rngAfterStore").unwrap())) {
                red.push(format!("rngAfterStore@{slot}"));
            }
            // The refusing entry point must accept every finite native group
            // and advance the stream exactly as native.
            let mut advanced = stream;
            law.sample_group_with_mesh(&mut batch, &mut advanced, owner, world, emitter_scale, axis_flag, Some(&cache))
                .unwrap_or_else(|refused| panic!("case {case}: finite native group refused {refused:?}"));
            assert_eq!(advanced, group.after_rng, "case {case}: stream after the group");
            stream = group.after_rng;
            slot += 4;
            index += 1;
            out.groups += 1;
        }
        assert_eq!(index, groups.len(), "case {case}: native group count");
        if stream != random(&words(n("rngAfter"))) {
            red.push("rngAfter".into());
        }
        out.cases += 1;
        if !red.is_empty() {
            out.red_cases += 1;
            out.mismatches += red.len();
            if out.first_red.is_none() {
                out.first_red = Some(format!("case {case} {}: {:?}", get("label").as_str().unwrap_or(""), &red[..red.len().min(8)]));
            }
        }
    }
    out
}

#[cfg(test)]
#[derive(Debug, Default)]
pub(crate) struct MeshReplay {
    pub cases: usize,
    pub groups: usize,
    pub red_cases: usize,
    pub mismatches: usize,
    pub first_red: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_product_keeps_white_and_zero_alpha_marks_dead() {
        for a in 0..=255u8 {
            assert_eq!(multiply_colour([a, 255 - a, a / 2, a], DEFAULT_MATERIAL_COLOUR), [a, 255 - a, a / 2, a]);
        }
        assert_eq!(MeshShapeCache::store_colour([9, 8, 7, 0], DEFAULT_MATERIAL_COLOUR), ([9, 8, 7, 0], true));
        assert_eq!(MeshShapeCache::store_colour([9, 8, 7, 1], DEFAULT_MATERIAL_COLOUR), ([9, 8, 7, 1], false));
        assert!(DEAD_AGE_PERCENT > 100.0);
    }

    #[test]
    fn empty_filtered_mesh_is_refused_by_name() {
        let positions = vec![[0.0; 3], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let cache = MeshShapeCache::new(positions.clone(), None, &[vec![0, 1, 2]], Some(3));
        assert_eq!(cache.err(), Some(MeshCacheRefused::NoTriangles));
        let cache = MeshShapeCache::new(positions, None, &[vec![0, 1, 3]], None);
        assert_eq!(cache.err(), Some(MeshCacheRefused::IndexOutOfRange));
    }

    fn rows(variable: &str) -> String {
        let path = std::env::var_os(variable).unwrap_or_else(|| panic!("{variable} is unset"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", std::path::Path::new(&path).display()))
    }

    /// The native rows of the mesh run: 212 cases, 380 groups, the cache and
    /// every stored lane bit for bit. Each named mutant of the transcription
    /// must go red somewhere, except the table-start rounding, which the rows
    /// cannot observe (the walk reaches the same triangle from either start
    /// away from exact boundaries).
    #[test]
    #[ignore = "set MOLY_MESH_SHAPE_ROWS to the native mesh-shape receipt rows (json)"]
    fn current_mesh_shape_rows_bit_exact() {
        let text = rows("MOLY_MESH_SHAPE_ROWS");
        arms::set(None);
        let base = replay_mesh_rows(&text);
        assert_eq!((base.cases, base.groups), (212, 380), "row inventory");
        assert_eq!(base.mismatches, 0, "first red: {:?}", base.first_red);
        let mut report = Vec::new();
        for mutant in ["lutStrict", "noBackward", "swapUV", "noNormalOffset", "noDeadMark", "lutFloor"] {
            arms::set(Some(mutant));
            let run = replay_mesh_rows(&text);
            arms::set(None);
            report.push((mutant, run.red_cases));
        }
        eprintln!("mesh-shape rows: cases {} groups {} mismatches 0; mutants red cases {report:?}", base.cases, base.groups);
        for (mutant, red) in &report {
            if *mutant != "lutFloor" {
                assert!(*red > 0, "mutant {mutant} stayed green");
            }
        }
    }
}
