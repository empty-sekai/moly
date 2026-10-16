//! Region-correct offline HOME starter rows.
//!
//! The starter rows are a named mock of the server's housing layout. A server
//! only ever sends pieces the client of its own region ships, so every row
//! must name a piece of the loaded snapshot's own fixture master. The rows are
//! authored once (the compact rows in the server panel document, the full
//! showcase in this module's parent); on a snapshot whose fixture master lacks
//! an authored piece, the row takes a stand-in chosen from that master by one
//! rule, at load, never from a hand-kept table:
//!
//! 1. keep the pieces with the authored piece's grid size (width, depth,
//!    height), layout type and put type, whose model the fixture-model index
//!    exports with a fixture view;
//! 2. prefer the fewest differences in fixture type, handle type and player
//!    action type, so a stand-in neither adds nor drops an interaction;
//! 3. then the lowest master id.
//!
//! The row keeps its position, centre height, layout, direction and texture.
//! A non-zero fixture id anchor must be the authored piece's own master id and
//! follows the piece into the stand-in's id. A row whose piece the master has
//! keeps it unchanged, and its non-zero anchor must equal that master id.
//!
//! An absent piece needs the authored piece's traits (the loaded master
//! cannot supply them); a row without them, or with no candidate, is refused
//! by name. The master's own region must be the snapshot's region.

use std::collections::{HashMap, HashSet};
use std::sync::OnceLock;

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

use super::PlacementMock;

const FIXTURE_MASTER: &str = "moly://mysekai-fixtures.json";
const FIXTURE_MODELS: &str = "moly://fixture-models/index.json";
const PACKAGE_PREFIX: &str = "mysekai__fixture__";

/// The master traits the stand-in rule reads, in the master's own words.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PieceTraits {
    pub grid: [i32; 3],
    pub layout_type: String,
    pub put_type: String,
    pub fixture_type: String,
    pub handle_type: String,
    pub player_action_type: String,
}

impl PieceTraits {
    /// The traits block of a panel row (`authoredPiece`), in the master's
    /// field names.
    pub(crate) fn from_record(record: &Value) -> Result<Self, String> {
        let int = |key: &str| {
            record[key]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or_else(|| format!("authored piece {key} must be an integer"))
        };
        let text = |key: &str| {
            record[key]
                .as_str()
                .map(str::to_owned)
                .ok_or_else(|| format!("authored piece {key} must be a string"))
        };
        Ok(Self {
            grid: [int("gridWidth")?, int("gridDepth")?, int("gridHeight")?],
            layout_type: text("layoutType")?,
            put_type: text("putType")?,
            fixture_type: text("fixtureType")?,
            handle_type: text("handleType")?,
            player_action_type: text("playerActionType")?,
        })
    }

    fn differences(&self, other: &PieceTraits) -> usize {
        [
            self.fixture_type != other.fixture_type,
            self.handle_type != other.handle_type,
            self.player_action_type != other.player_action_type,
        ]
        .into_iter()
        .filter(|differs| *differs)
        .count()
    }

    fn fits(&self, other: &PieceTraits) -> bool {
        self.grid == other.grid
            && self.layout_type == other.layout_type
            && self.put_type == other.put_type
    }
}

struct Piece {
    id: i32,
    package: String,
    traits: PieceTraits,
}

/// The loaded snapshot's fixture master as the rule reads it.
pub(crate) struct RegionMaster {
    region: String,
    /// Master rows in id order.
    pieces: Vec<Piece>,
    /// Master rows by package, in id order. Both masters name some packages
    /// on more than one row (a wall and a floor appearance share one).
    by_package: HashMap<String, Vec<usize>>,
    /// Packages the fixture-model index exports with a fixture view.
    viewed: HashSet<String>,
}

static MASTER: OnceLock<RegionMaster> = OnceLock::new();

impl RegionMaster {
    fn parse(master: &str, models: &str) -> Result<Self, String> {
        let master: Value =
            serde_json::from_str(master).map_err(|error| format!("fixture master: {error}"))?;
        let region = master["region"]
            .as_str()
            .ok_or("fixture master has no region")?
            .to_owned();
        let rows = master["fixtures"]
            .as_array()
            .ok_or("fixture master has no fixtures array")?;
        let mut pieces = Vec::with_capacity(rows.len());
        for row in rows {
            let id = row["id"]
                .as_i64()
                .and_then(|value| i32::try_from(value).ok())
                .ok_or("fixture master row without an integer id")?;
            let name = row["assetbundleName"]
                .as_str()
                .ok_or_else(|| format!("fixture master row {id} has no assetbundleName"))?;
            pieces.push(Piece {
                id,
                package: format!("{PACKAGE_PREFIX}{name}"),
                traits: PieceTraits::from_record(row)
                    .map_err(|reason| format!("fixture master row {id}: {reason}"))?,
            });
        }
        pieces.sort_by_key(|piece| piece.id);
        let mut by_package: HashMap<String, Vec<usize>> = HashMap::new();
        for (index, piece) in pieces.iter().enumerate() {
            by_package.entry(piece.package.clone()).or_default().push(index);
        }
        let models: Value = serde_json::from_str(models)
            .map_err(|error| format!("fixture-model index: {error}"))?;
        let packages = models["packages"]
            .as_object()
            .ok_or("fixture-model index has no packages object")?;
        let viewed = packages
            .iter()
            .filter(|(_, entry)| {
                entry["status"].as_str() == Some("exported")
                    && entry["variants"].as_array().is_some_and(|variants| {
                        variants
                            .iter()
                            .any(|variant| variant["hasFixtureView"].as_bool() == Some(true))
                    })
            })
            .map(|(name, _)| name.clone())
            .collect();
        Ok(Self {
            region,
            pieces,
            by_package,
            viewed,
        })
    }

    /// The master row of a package; a package named by more than one row
    /// anchors no single id.
    fn piece(&self, package: &str) -> Result<Option<&Piece>, String> {
        match self.by_package.get(package).map(Vec::as_slice) {
            None | Some([]) => Ok(None),
            Some([index]) => Ok(Some(&self.pieces[*index])),
            Some(indices) => Err(format!(
                "starter row {package}: this snapshot's ({}) fixture master names that package on {} rows (ids {:?})",
                self.region,
                indices.len(),
                indices.iter().map(|index| self.pieces[*index].id).collect::<Vec<_>>()
            )),
        }
    }

    /// The row as this master's region has it (see the module notes).
    fn resolve(
        &self,
        mut row: PlacementMock<String>,
        authored: Option<&PieceTraits>,
    ) -> Result<PlacementMock<String>, String> {
        if let Some(piece) = self.piece(&row.package)? {
            if !self.viewed.contains(&row.package) {
                return Err(format!(
                    "starter row {}: the fixture-model index does not export it with a fixture view",
                    row.package
                ));
            }
            if row.fixture_id != 0 && row.fixture_id != piece.id {
                return Err(format!(
                    "starter row {} carries fixture id {}, but this snapshot's master gives it id {}",
                    row.package, row.fixture_id, piece.id
                ));
            }
            return Ok(row);
        }
        let authored = authored.ok_or_else(|| {
            format!(
                "starter row {} is not in this snapshot's ({}) fixture master, and the row does not state the authored piece's traits",
                row.package, self.region
            )
        })?;
        let mut candidates: Vec<(usize, i32, &Piece)> = self
            .pieces
            .iter()
            .filter(|piece| piece.traits.fits(authored) && self.viewed.contains(&piece.package))
            .map(|piece| (authored.differences(&piece.traits), piece.id, piece))
            .collect();
        candidates.sort_by_key(|(differences, id, _)| (*differences, *id));
        let Some(&(differences, _, stand_in)) = candidates.first() else {
            return Err(format!(
                "starter row {}: no piece of this snapshot's ({}) fixture master has its grid size, layout type and put type",
                row.package, self.region
            ));
        };
        info!(
            "[fixture-region] starter row {} is not in this snapshot's ({}) fixture master: stand-in {} (id {}) of {} fitting pieces, {} trait differences",
            row.package,
            self.region,
            stand_in.package,
            stand_in.id,
            candidates.len(),
            differences
        );
        if row.fixture_id != 0 {
            row.fixture_id = stand_in.id;
        }
        row.package = stand_in.package.clone();
        Ok(row)
    }
}

/// Whether the loaded snapshot's fixture master is installed.
pub(crate) fn ready() -> bool {
    MASTER.get().is_some()
}

/// The starter rows as the loaded snapshot's region has them. Each row comes
/// with the authored piece's traits when the row states them. `region` is the
/// snapshot's source region, which the master must share.
pub(super) fn resolve_rows(
    rows: Vec<(PlacementMock<String>, Option<PieceTraits>)>,
    region: moly_law::carve::NavMeshRegion,
) -> Result<Vec<PlacementMock<String>>, String> {
    let master = MASTER
        .get()
        .ok_or("the snapshot's fixture master is not installed yet")?;
    let expected = match region {
        moly_law::carve::NavMeshRegion::Cn => "cn",
        moly_law::carve::NavMeshRegion::Jp => "jp",
    };
    if master.region != expected {
        return Err(format!(
            "the fixture master is region {} but the snapshot's source region is {expected}",
            master.region
        ));
    }
    rows.into_iter()
        .map(|(row, traits)| master.resolve(row, traits.as_ref()))
        .collect()
}

#[derive(Resource)]
pub(crate) struct RegionMasterHandles {
    master: Handle<JsonAsset>,
    models: Handle<JsonAsset>,
}

/// Startup: request the fixture master and the fixture-model index.
pub(crate) fn load(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(RegionMasterHandles {
        master: server.load(FIXTURE_MASTER),
        models: server.load(FIXTURE_MODELS),
    });
}

/// Installs the master once both documents are in.
pub(crate) fn install(
    mut commands: Commands,
    handles: Option<Res<RegionMasterHandles>>,
    json: Res<Assets<JsonAsset>>,
) {
    let Some(handles) = handles else {
        return;
    };
    let (Some(master), Some(models)) = (json.get(&handles.master), json.get(&handles.models))
    else {
        return;
    };
    let parsed = RegionMaster::parse(&master.0, &models.0)
        .unwrap_or_else(|reason| panic!("[fixture-region] {reason}"));
    info!(
        "[fixture-region] fixture master installed: region {}, {} pieces, {} exported with a fixture view",
        parsed.region,
        parsed.pieces.len(),
        parsed.viewed.len()
    );
    if MASTER.set(parsed).is_err() {
        warn!("[fixture-region] the fixture master was already installed; the first one stays");
    }
    commands.remove_resource::<RegionMasterHandles>();
}
