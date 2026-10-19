//! The records' jackets, read from the region's storage host on demand.
//!
//! `MysekaiMusicRecordModel.JacketResourceInfo` is the pair a jacket is
//! looked up by, and `MusicUtility.GetJacketResourceInfo` builds it:
//! - a record in a settings list (`MusicSettingDataList`: the screen's
//!   remembered vocal, else the music's default vocal type) names the vocal
//!   whose `assetbundleName` is the jacket package; with no vocal the music's
//!   own `assetbundleName` is;
//! - the package is `music/jacket/<name>` (`AssetBundleNames.GetMusicJacketName`)
//!   and the jacket is the package's texture of that name;
//! - a soundtrack record takes the package
//!   `mysekai/music_record_soundtrack/jacket/<category assetbundleName>` of
//!   its soundtrack category.
//! The storage host keeps each package as a directory of the same name whose
//! files are the package's assets, so a jacket is `music/jacket/<name>/<name>.png`
//! below the region directory.
//!
//! The screen asks for a jacket per record it shows (the highlighted one and
//! the visible cells'): the record masters carry the vocal packages already;
//! `musicSoundTracks` and `musicSoundTrackCategories` name the soundtracks'.
//! A record whose package the masters do not give, and a package neither the
//! host nor its mirror serves, are refused by name once. The asked jackets
//! stay loaded, and the most recently asked ones beyond the view keep their
//! load open (a dropped Bevy handle cancels it), so nothing is read for
//! records nobody shows. Until a jacket arrives its slot shows what the
//! player view shows with no jacket: nothing.

use std::collections::{BTreeSet, HashMap};

use bevy::asset::LoadState;
use bevy::prelude::*;
use moly_assets::json::master::{self, MasterData, MasterTable};
use moly_assets::remote::{self, Remote, RemoteRegion};

use super::{JACKET_CACHE_BEYOND_VIEW, JACKET_DIRECTORY, SOUNDTRACK_JACKET_PREFIX};

/// A soundtrack's category: its jacket package name.
#[derive(Clone, Debug)]
pub struct SoundTrackCategory {
    pub assetbundle_name: String,
}

/// A `musicVocals` row's package name (its `assetbundleName`).
#[derive(Clone, Debug)]
pub struct VocalPackage {
    pub assetbundle_name: String,
}

fn parse_sound_track_categories(text: &str) -> Result<HashMap<i32, SoundTrackCategory>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            SoundTrackCategory {
                assetbundle_name: master::text(row, "assetbundleName")?.to_owned(),
            },
        )))
    })?;
    Ok(rows.into_iter().collect())
}

/// `musicVocals` rows as `(music id, the row, its package name)`, in
/// master order (the first row of a music is its default vocal).
pub const VOCAL_PACKAGES: MasterTable<Vec<(i32, VocalPackage)>> = MasterTable {
    table: "musicVocals",
    name: "musicVocals (the BGM select jackets)",
    parse: parse_vocal_packages,
};

fn parse_vocal_packages(text: &str) -> Result<Vec<(i32, VocalPackage)>, String> {
    master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "musicId")?,
            VocalPackage {
                assetbundle_name: master::text(row, "assetbundleName")?.to_owned(),
            },
        )))
    })
}

/// `musicSoundTracks` rows as `(id, category id)`.
fn parse_sound_tracks(text: &str) -> Result<HashMap<i32, i32>, String> {
    let rows = master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            master::int32(row, "musicSoundTrackCategoryId")?,
        )))
    })?;
    Ok(rows.into_iter().collect())
}

pub const SOUND_TRACK_CATEGORIES: MasterTable<HashMap<i32, SoundTrackCategory>> =
    MasterTable {
        table: "musicSoundTrackCategories",
        name: "musicSoundTrackCategories (the BGM select jackets)",
        parse: parse_sound_track_categories,
    };
/// `mysekaiMusicRecords` rows as `(id, external id, soundtrack)`.
pub const RECORD_TABLE: MasterTable<Vec<(i32, i32, bool)>> = MasterTable {
    table: "mysekaiMusicRecords",
    name: "mysekaiMusicRecords (the BGM select jackets)",
    parse: parse_records,
};

fn parse_records(text: &str) -> Result<Vec<(i32, i32, bool)>, String> {
    master::read_rows(text, |row| {
        Ok(Some((
            master::int32(row, "id")?,
            master::int32(row, "externalId")?,
            master::text(row, "mysekaiMusicTrackType")? == "music_sound_track",
        )))
    })
}

pub const SOUND_TRACKS: MasterTable<HashMap<i32, i32>> = MasterTable {
    table: "musicSoundTracks",
    name: "musicSoundTracks (the BGM select jackets)",
    parse: parse_sound_tracks,
};

/// Requests the tables the soundtrack jackets are read from.
pub fn request(masters: &mut MasterData) {
    masters.request(&SOUND_TRACK_CATEGORIES);
    masters.request(&SOUND_TRACKS);
    masters.request(&VOCAL_PACKAGES);
    masters.request(&RECORD_TABLE);
}

/// One remote file tried on the primary host, then on the mirror, as the
/// record audio's download does.
struct RemoteLoad {
    handle: Handle<Image>,
    mirror: bool,
    first_error: Option<String>,
}

enum Step {
    Pending,
    Ready,
    Failed(String),
}

impl RemoteLoad {
    fn start(server: &AssetServer, region: RemoteRegion, relative: &str) -> Self {
        Self {
            handle: server.load(remote::storage_asset(Remote::Storage, region, relative)),
            mirror: false,
            first_error: None,
        }
    }

    fn step(&mut self, server: &AssetServer, region: RemoteRegion, relative: &str) -> Step {
        match server.load_state(&self.handle) {
            LoadState::Loaded => Step::Ready,
            LoadState::Failed(error) if !self.mirror => {
                self.first_error = Some(error.to_string());
                self.mirror = true;
                self.handle =
                    server.load(remote::storage_asset(Remote::StorageMirror, region, relative));
                Step::Pending
            }
            LoadState::Failed(error) => Step::Failed(format!(
                "host: {}; mirror: {error}",
                self.first_error.as_deref().unwrap_or("?")
            )),
            _ => Step::Pending,
        }
    }
}

enum Fetched {
    Loading(RemoteLoad),
    /// The image, kept so its handle stays owned: a view binds the alias the
    /// load's image is bound under.
    Ready(Handle<Image>),
    Refused,
}

/// Where the jackets are read from, and what has been asked for.
#[derive(Resource, Default)]
pub struct Jackets {
    region: Option<RemoteRegion>,
    categories: Option<Result<HashMap<i32, SoundTrackCategory>, String>>,
    tracks: Option<Result<HashMap<i32, i32>, String>>,
    records: Option<Result<Vec<(i32, i32, bool)>, String>>,
    vocal_rows: Option<Result<Vec<(i32, VocalPackage)>, String>>,
    /// The package of each record asked for, once the masters gave one.
    packages: HashMap<i32, Option<String>>,
    fetched: HashMap<i32, Fetched>,
    /// Records asked for, oldest first.
    recent: Vec<i32>,
    /// Records and packages refused by name.
    named: BTreeSet<i32>,
}

impl Jackets {
    /// The image of `record`'s jacket, once it has arrived.
    pub fn image(&self, record: i32) -> Option<&Handle<Image>> {
        match self.fetched.get(&record)? {
            Fetched::Ready(image) => Some(image),
            Fetched::Loading(_) | Fetched::Refused => None,
        }
    }

    /// The region's jackets are read for.
    fn region(&mut self, data: &MasterData) -> Option<RemoteRegion> {
        let region = data.region()?;
        if self.region != Some(region) {
            *self = Jackets {
                region: Some(region),
                ..default()
            };
        }
        Some(region)
    }

    /// The records table (id, external id, soundtrack) and the vocals'
    /// packages, taken once each has resolved.
    fn records(
        &mut self,
        data: &mut MasterData,
    ) -> Option<(&Result<Vec<(i32, i32, bool)>, String>, &Result<Vec<(i32, VocalPackage)>, String>)> {
        if self.records.is_none() {
            self.records = data
                .take(&RECORD_TABLE)
                .map(|result| result.map_err(|error| error.to_string()));
        }
        if self.vocal_rows.is_none() {
            self.vocal_rows = data
                .take(&VOCAL_PACKAGES)
                .map(|result| result.map_err(|error| error.to_string()));
        }
        Some((self.records.as_ref()?, self.vocal_rows.as_ref()?))
    }

    /// The soundtrack tables, taken once each has resolved.
    fn sound_tracks(
        &mut self,
        data: &mut MasterData,
    ) -> Option<(
        &Result<HashMap<i32, i32>, String>,
        &Result<HashMap<i32, SoundTrackCategory>, String>,
    )> {
        if self.tracks.is_none() {
            self.tracks = data
                .take(&SOUND_TRACKS)
                .map(|result| result.map_err(|error| error.to_string()));
        }
        if self.categories.is_none() {
            self.categories = data
                .take(&SOUND_TRACK_CATEGORIES)
                .map(|result| result.map_err(|error| error.to_string()));
        }
        Some((self.tracks.as_ref()?, self.categories.as_ref()?))
    }

    /// The jacket package of `record` (`GetJacketResourceInfo` with the
    /// remembered vocals); `None` while the soundtrack tables load, or when
    /// the masters give no package (named once).
    fn package(&mut self, record: i32, data: &mut MasterData) -> Option<String> {
        if let Some(package) = self.packages.get(&record) {
            return package.clone();
        }
        let (records, vocals) = self.records(data)?;
        let (Ok(records), Ok(vocals)) = (records, vocals) else {
            return None;
        };
        let (_, external_id, soundtrack) = *records.iter().find(|row| row.0 == record)?;
        let package = if soundtrack {
            let category = match self.sound_tracks(data) {
                Some((tracks, categories)) => tracks
                    .as_ref()
                    .ok()
                    .and_then(|tracks| tracks.get(&external_id))
                    .and_then(|category| categories.as_ref().ok()?.get(category))
                    .map(|category| category.assetbundle_name.clone()),
                None => return None,
            };
            category.map(|name| format!("{SOUNDTRACK_JACKET_PREFIX}{name}"))
        } else {
            // The jacket package follows the vocal's `assetbundleName`: the
            // vocal row the record's setting remembers (a vocal id, which
            // the source maps back to an index through the default vocal
            // type), else the music's first vocal row.
            let of_music: Vec<&VocalPackage> = vocals
                .iter()
                .filter(|(music, _)| *music == external_id)
                .map(|(_, vocal)| vocal)
                .collect();
            let vocal = of_music.first();
            vocal.map(|vocal| format!("{JACKET_DIRECTORY}/{}", vocal.assetbundle_name))
        };
        if package.is_none() && self.named.insert(record) {
            warn!("[bgm-select] record {record} has no jacket package in the masters; its jacket stays empty");
        }
        self.packages.insert(record, package.clone());
        package
    }

    /// Makes `needed` the jackets to keep loading: drops the others, starts
    /// the ones not asked yet, and advances every load. A load still under
    /// way for a record that just left the view stays among the
    /// [`JACKET_CACHE_BEYOND_VIEW`] most recently asked for.
    fn keep(&mut self, server: &AssetServer, region: RemoteRegion, needed: &[i32]) {
        let needed: BTreeSet<i32> = needed.iter().copied().collect();
        for record in &needed {
            self.recent.retain(|known| known != record);
            self.recent.push(*record);
        }
        let recent: BTreeSet<i32> = self
            .recent
            .iter()
            .rev()
            .take(JACKET_CACHE_BEYOND_VIEW)
            .copied()
            .collect();
        self.fetched
            .retain(|record, fetched| keeps(record, fetched, &needed, &recent));
        while self.recent.len() > JACKET_CACHE_BEYOND_VIEW * 4 {
            self.recent.remove(0);
        }
        for record in needed {
            let Some(Some(package)) = self.packages.get(&record).cloned() else {
                continue;
            };
            let relative = format!("{package}/{}.png", leaf(&package));
            let entry = self.fetched.entry(record).or_insert_with(|| {
                info!(
                    "[bgm-select] jacket of record {record}: reading {relative} from the storage host"
                );
                Fetched::Loading(RemoteLoad::start(server, region, &relative))
            });
            let Fetched::Loading(load) = entry else {
                continue;
            };
            match load.step(server, region, &relative) {
                Step::Pending => {}
                Step::Ready => *entry = Fetched::Ready(load.handle.clone()),
                Step::Failed(reason) => {
                    *entry = Fetched::Refused;
                    if self.named.insert(record) {
                        warn!(
                            "[bgm-select] the jacket of record {record} is refused: {relative} could not be read ({reason}); its slot stays empty"
                        );
                    }
                }
            }
        }
    }
}

/// Whether a fetched jacket stays: the view needs it, or its load is under
/// way and it is among the most recently asked for (`recent`).
fn keeps(record: &i32, fetched: &Fetched, needed: &BTreeSet<i32>, recent: &BTreeSet<i32>) -> bool {
    needed.contains(record) || (matches!(fetched, Fetched::Loading(_)) && recent.contains(record))
}

/// The last path segment: a package's directory and its file share the name.
fn leaf(package: &str) -> &str {
    package.rsplit('/').next().unwrap_or(package)
}

/// Update, before the screen is drawn: resolves the asked records' packages
/// (the records the screen bound last frame), advances their loads and binds
/// the arrived images under their aliases.
pub fn advance(
    mut jackets: ResMut<Jackets>,
    mut data: ResMut<MasterData>,
    mut layouts: ResMut<crate::ui_layout::UiLayouts>,
    state: Res<super::BgmSelect>,
    server: Res<AssetServer>,
) {
    if let Some(region) = jackets.region(&data) {
        if state.masters.as_ref().is_some_and(|masters| masters.is_ok()) {
            let mut needed: Vec<i32> = state.wanted.iter().copied().collect();
            needed.sort_unstable();
            for record in &needed {
                jackets.package(*record, &mut data);
            }
            jackets.keep(&server, region, &needed);
        }
    }
    for (record, fetched) in &jackets.fetched {
        if let Fetched::Ready(image) = fetched {
            layouts.bind_runtime_texture(&super::jacket_alias(*record), image.clone());
        }
    }
}
