//! The ground half of the footstep (`FootSEController`): the site's sound
//! mesh, its vertex colours, and the two footstep master tables.
//!
//! - The controller takes the site view's sound mesh once, at construction:
//!   its vertex colours, and its vertices through the mesh filter's
//!   `TransformPoint` (world space, fixed from then on).
//! - `GetNearestVertexColor(position)` scans every vertex in order with the
//!   distance `sqrt(dz*dz + (dx*dx + dy*dy))` in single precision, keeps a
//!   vertex only when it is strictly closer than the best so far (starting
//!   from the largest finite float) and answers opaque white when nothing
//!   was kept. There is no search radius.
//! - The colour becomes three integers by truncating `channel * 255` (single
//!   precision). `PlayFootGroundSe` takes the first site footstep row, in the
//!   table's row order, whose red, green and blue equal them; the cue is
//!   `se_walk_` or, in the Dash state, `se_run_`, followed by that row's walk
//!   or run cue. With no row the suffix is empty, and the cue check
//!   (`ExistsCueName`) refuses the result, so nothing plays.
//! - `IsWater(position)` compares the same nearest colour with the row whose
//!   walk cue is `water`, taken with `First` when the controller is built (a
//!   table without such a row cannot build the controller).
//! - `PlayFootFixtureSE(fixtureId)`: the fixture master row's footstep id
//!   selects the first fixture footstep row with that id, with the same cue
//!   rule.
//!
//! The runtime frame reflects the source's x axis. Distances do not change
//! under it, and the vertices keep their authored order, so the nearest
//! vertex and its colour are the source's.

use bevy::prelude::*;
use serde_json::Value;
use std::collections::{HashMap, HashSet};

pub(crate) const SITE_FOOTSTEPS: &str = "moly://mysekai-site-footsteps.json";
pub(crate) const FIXTURE_FOOTSTEPS: &str = "moly://mysekai-fixture-footsteps.json";
pub(crate) const SOUND_WAVE: &str = "moly://site-sound-wave/site-sound-wave.json";

#[derive(Clone, Debug)]
pub(crate) struct SiteFootstepRow {
    pub id: i64,
    pub rgb: [i32; 3],
    pub walk: String,
    pub run: String,
}

#[derive(Clone, Debug)]
pub(crate) struct FixtureFootstepRow {
    pub id: i64,
    pub walk: String,
    pub run: String,
}

/// The two footstep master tables, rows in source order.
#[derive(Clone, Debug)]
pub(crate) struct Masters {
    pub site: Vec<SiteFootstepRow>,
    pub fixture: Vec<FixtureFootstepRow>,
}

/// Rows of an `{entries, rowOrder}` master document in row order.
fn ordered_rows(doc: &Value, table: &str) -> Result<Vec<Value>, String> {
    let entries = doc["entries"]
        .as_object()
        .ok_or_else(|| format!("{table}: entries must be an object"))?;
    let order = doc["rowOrder"]
        .as_array()
        .ok_or_else(|| format!("{table}: rowOrder must be a list"))?;
    let mut seen = HashSet::new();
    let mut rows = Vec::with_capacity(order.len());
    for id in order {
        let id = id
            .as_i64()
            .ok_or_else(|| format!("{table}: rowOrder holds a non-integer"))?;
        if !seen.insert(id) {
            return Err(format!("{table}: duplicate rowOrder id {id}"));
        }
        let row = entries
            .get(&id.to_string())
            .ok_or_else(|| format!("{table}: rowOrder names absent id {id}"))?;
        if row["id"].as_i64() != Some(id) {
            return Err(format!("{table}: entry {id} carries another id"));
        }
        rows.push(row.clone());
    }
    if rows.len() != entries.len() {
        return Err(format!("{table}: entries not covered by rowOrder"));
    }
    Ok(rows)
}

fn cue_field(row: &Value, table: &str, field: &str) -> Result<String, String> {
    row[field]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| format!("{table} row {}: {field} is not a string", row["id"]))
}

impl Masters {
    pub(crate) fn parse(site: &Value, fixture: &Value) -> Result<Self, String> {
        let site = ordered_rows(site, "mysekaiSiteFootsteps")?
            .iter()
            .map(|row| {
                let channel = |name: &str| {
                    row[name]
                        .as_i64()
                        .and_then(|v| i32::try_from(v).ok())
                        .ok_or_else(|| {
                            format!(
                                "mysekaiSiteFootsteps row {}: {name} is not an integer",
                                row["id"]
                            )
                        })
                };
                Ok(SiteFootstepRow {
                    id: row["id"].as_i64().expect("checked by ordered_rows"),
                    rgb: [channel("red")?, channel("green")?, channel("blue")?],
                    walk: cue_field(row, "mysekaiSiteFootsteps", "walkCue")?,
                    run: cue_field(row, "mysekaiSiteFootsteps", "runCue")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        let fixture = ordered_rows(fixture, "mysekaiFixtureFootsteps")?
            .iter()
            .map(|row| {
                Ok(FixtureFootstepRow {
                    id: row["id"].as_i64().expect("checked by ordered_rows"),
                    walk: cue_field(row, "mysekaiFixtureFootsteps", "walkCue")?,
                    run: cue_field(row, "mysekaiFixtureFootsteps", "runCue")?,
                })
            })
            .collect::<Result<Vec<_>, String>>()?;
        Ok(Self { site, fixture })
    }

    /// `GetMasterMysekaiSiteFootStep(r, g, b)`: the first row in row order.
    pub(crate) fn site_row(&self, rgb: [i32; 3]) -> Option<&SiteFootstepRow> {
        self.site.iter().find(|row| row.rgb == rgb)
    }

    /// The fixture footstep row with that id (first in row order).
    pub(crate) fn fixture_row(&self, id: i64) -> Option<&FixtureFootstepRow> {
        self.fixture.iter().find(|row| row.id == id)
    }

    /// The water colour the controller captures: the first site row whose
    /// walk cue is `water` (`First`, which throws on a table without one).
    pub(crate) fn water(&self) -> Result<[i32; 3], String> {
        self.site
            .iter()
            .find(|row| row.walk == "water")
            .map(|row| row.rgb)
            .ok_or_else(|| "mysekaiSiteFootsteps has no row whose walkCue is water".into())
    }
}

/// The cue for a footstep: `se_walk_`/`se_run_` plus the row's cue, or the
/// bare prefix when there is no row (which the cue check refuses).
pub(crate) fn cue(dash: bool, walk: Option<&str>, run: Option<&str>) -> String {
    let (prefix, suffix) = if dash {
        ("se_run_", run)
    } else {
        ("se_walk_", walk)
    };
    format!("{prefix}{}", suffix.unwrap_or(""))
}

/// `(int)(channel * 255f)`: single-precision product, truncated toward zero.
/// A value outside the int range (never a vertex colour) gives the integer
/// minimum, as the source's conversion does.
pub(crate) fn channel_byte(channel: f32) -> i32 {
    let scaled = channel * 255.0f32;
    if scaled.is_finite() && scaled > -2_147_483_649.0 && scaled < 2_147_483_648.0 {
        scaled as i32
    } else {
        i32::MIN
    }
}

pub(crate) fn colour_bytes(colour: [f32; 4]) -> [i32; 3] {
    [
        channel_byte(colour[0]),
        channel_byte(colour[1]),
        channel_byte(colour[2]),
    ]
}

/// The sound mesh as the controller holds it.
#[derive(Clone, Debug)]
pub(crate) struct SoundWave {
    pub world: Vec<Vec3>,
    pub colours: Vec<[f32; 4]>,
}

impl SoundWave {
    /// `GetNearestVertexColor`.
    pub(crate) fn nearest_colour(&self, position: Vec3) -> ([f32; 4], Option<usize>) {
        let mut best = f32::MAX;
        let mut colour = [1.0, 1.0, 1.0, 1.0];
        let mut index = None;
        for (i, (vertex, c)) in self.world.iter().zip(&self.colours).enumerate() {
            let dx = position.x - vertex.x;
            let dy = position.y - vertex.y;
            let dz = position.z - vertex.z;
            let distance = (dz * dz + (dx * dx + dy * dy)).sqrt();
            if distance < best {
                best = distance;
                colour = *c;
                index = Some(i);
            }
        }
        (colour, index)
    }
}

/// Where the site's sound mesh lives in the release root.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct SoundMeshSource {
    /// Asset path of the glb that holds it.
    pub glb: String,
    /// Mesh name inside that glb.
    pub mesh: String,
    /// Vertex count the source mesh has.
    pub vertices: usize,
    /// Whether the mesh comes from a room kit shared by several rooms (its
    /// placement node is not in the glb, see [`crate::footstep`]).
    pub kit: bool,
}

/// Resolve a scene's sound mesh: the sound-wave document names the mesh and
/// its package; a mesh in the scene's own package is one of the scene's
/// collision rows in the site index, and a mesh in the room kit's package is
/// a mesh of the kit geometry.
pub(crate) fn sound_mesh_source(
    sound_wave: &Value,
    site_index: &Value,
    scene: &str,
) -> Result<SoundMeshSource, String> {
    let entry = &sound_wave["scenes"][scene];
    if entry.is_null() {
        return Err(format!("the sound-wave document has no scene {scene}"));
    }
    let mesh = &entry["mesh"];
    let name = mesh["name"]
        .as_str()
        .ok_or_else(|| format!("sound-wave scene {scene}: mesh has no name"))?;
    let vertices = mesh["vertices"]
        .as_u64()
        .ok_or_else(|| format!("sound-wave scene {scene}: mesh has no vertex count"))?
        as usize;
    let package = mesh["package"].as_str().unwrap_or_default();
    if mesh["inScenePackage"] == true {
        let rows = site_index["scenes"][scene]["collision"]
            .as_array()
            .ok_or_else(|| format!("site index scene {scene} has no collision rows"))?;
        let row = rows.iter().find(|row| row["mesh"] == name).ok_or_else(|| {
            format!("site index scene {scene} has no collision row for mesh {name}")
        })?;
        let file = row["file"]
            .as_str()
            .ok_or_else(|| format!("site index scene {scene}: collision row {name} has no file"))?;
        return Ok(SoundMeshSource {
            glb: format!("moly://site/{file}"),
            mesh: name.to_owned(),
            vertices,
            kit: false,
        });
    }
    let kit = &site_index["indoor"]["kit"];
    if kit["package"] == package {
        let geometry = kit["geometry"]
            .as_str()
            .ok_or_else(|| "site index indoor kit has no geometry".to_owned())?;
        return Ok(SoundMeshSource {
            glb: format!("moly://site/indoor/kit/{geometry}"),
            mesh: name.to_owned(),
            vertices,
            kit: true,
        });
    }
    Err(format!(
        "sound-wave scene {scene}: mesh {name} lives in package {package}, which is neither the scene's nor the room kit's"
    ))
}

/// The distinct colours of a vertex colour list with their vertex counts,
/// for comparison with the document's own census.
pub(crate) fn colour_census(colours: &[[f32; 4]]) -> HashMap<[u32; 4], usize> {
    let mut census = HashMap::new();
    for c in colours {
        *census.entry(c.map(f32::to_bits)).or_insert(0) += 1;
    }
    census
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Value check against a reference computed from the release glb data
    /// (not in the repository): `MOLY_FOOT_REFERENCE` names a JSON file with
    /// `{sites: [{scene, vertices: [[x,y,z]...], colours: [[r,g,b,a]...],
    /// samples: [{p: [x,y,z], index, bytes: [r,g,b]}]}], bytes: [[b, out]...]}`
    /// computed independently (numpy float32 in the same operation order).
    #[test]
    #[ignore = "needs a reference file computed from release data (MOLY_FOOT_REFERENCE)"]
    fn nearest_colour_matches_the_reference() {
        let path = std::env::var("MOLY_FOOT_REFERENCE").expect("MOLY_FOOT_REFERENCE");
        let text = std::fs::read_to_string(&path).expect("reference readable");
        let reference: Value = serde_json::from_str(&text).expect("reference json");
        let floats = |v: &Value| -> Vec<f32> {
            v.as_array()
                .unwrap()
                .iter()
                .map(|x| x.as_f64().unwrap() as f32)
                .collect()
        };
        let mut checked = 0;
        for site in reference["sites"].as_array().unwrap() {
            let world: Vec<Vec3> = site["vertices"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| Vec3::from_slice(&floats(v)))
                .collect();
            let colours: Vec<[f32; 4]> = site["colours"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| floats(v).try_into().unwrap())
                .collect();
            let mesh = SoundWave { world, colours };
            for sample in site["samples"].as_array().unwrap() {
                let p = Vec3::from_slice(&floats(&sample["p"]));
                let (colour, index) = mesh.nearest_colour(p);
                let expect_index = sample["index"].as_i64().map(|i| i as usize);
                assert_eq!(index, expect_index, "{} sample {p:?}", site["scene"]);
                let bytes: Vec<i32> = sample["bytes"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|b| b.as_i64().unwrap() as i32)
                    .collect();
                assert_eq!(
                    colour_bytes(colour).to_vec(),
                    bytes,
                    "{} sample {p:?}",
                    site["scene"]
                );
                checked += 1;
            }
        }
        for pair in reference["bytes"].as_array().unwrap() {
            let input = pair[0].as_f64().unwrap() as f32;
            assert_eq!(
                channel_byte(input) as i64,
                pair[1].as_i64().unwrap(),
                "byte of {input}"
            );
        }
        eprintln!(
            "checked {checked} samples and {} channel values",
            reference["bytes"].as_array().unwrap().len()
        );
        assert!(checked > 0);
    }
}
