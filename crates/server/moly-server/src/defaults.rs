//! The document a user starts from: the asset source's slice when the root
//! has one, else the checked-in slice, migrated and overlaid with the
//! developer instruments.

use bevy::asset::io::{AssetReaderError, AssetSourceId, Reader};
use bevy::prelude::*;
use bevy::tasks::{block_on, futures_lite::future, IoTaskPool, Task};

use crate::instrument::instrument_env;
use crate::model::avatar;
use crate::model::document::{self, Clock, Migration, ServerDocument, Stamina};
use crate::model::{inventory, music_play, phenomena, tools, Origin};
use crate::model::music;

/// Asset source and path of a slice that replaces the checked-in one.
const SLICE_SOURCE: &str = "moly";
const SLICE_FILE: &str = "server-panel/npc.json";
/// The checked-in slice (the product default).
pub(crate) const CHECKED_IN_SLICE: &str = include_str!("server_panel/npc.json");

/// The document a user with no stored one starts from.
#[derive(Resource)]
pub(crate) struct Defaults {
    pub(crate) doc: ServerDocument,
    pub(crate) origin: Origin,
    /// The developer avatar instrument, resolved once the masters are in.
    pub(crate) avatar_instrument: avatar::AvatarInstrument,
}

/// The slice read, while it runs.
#[derive(Resource)]
struct SliceRead(Task<Result<Option<String>, String>>);

pub(crate) fn install(app: &mut App) {
    app.add_systems(Startup, load).add_systems(PreUpdate, resolve);
}

fn load(mut commands: Commands, server: Option<Res<AssetServer>>) {
    let Some(server) = server else {
        // No asset source (a harness that drives the model directly): the
        // checked-in slice is the product default.
        commands.insert_resource(from_slice(
            CHECKED_IN_SLICE.to_owned(),
            Migration::CheckedInDefault,
            Origin::CheckedInDefault,
        ));
        return;
    };
    let reader = server.clone();
    commands.insert_resource(SliceRead(IoTaskPool::get().spawn(async move {
        read_slice(reader).await
    })));
}

async fn read_slice(server: AssetServer) -> Result<Option<String>, String> {
    let source = server
        .get_source(AssetSourceId::from(SLICE_SOURCE))
        .map_err(|error| error.to_string())?;
    let mut reader = match source.reader().read(std::path::Path::new(SLICE_FILE)).await {
        Ok(reader) => reader,
        Err(AssetReaderError::NotFound(_)) => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let mut bytes = Vec::new();
    reader
        .read_to_end(&mut bytes)
        .await
        .map_err(|error| error.to_string())?;
    String::from_utf8(bytes)
        .map(Some)
        .map_err(|error| error.to_string())
}

fn resolve(mut commands: Commands, read: Option<ResMut<SliceRead>>) {
    let Some(mut read) = read else {
        return;
    };
    let Some(result) = block_on(future::poll_once(&mut read.0)) else {
        return;
    };
    commands.remove_resource::<SliceRead>();
    let defaults = match result {
        Ok(Some(text)) => from_slice(text, Migration::AssetSource, Origin::AssetSource),
        Ok(None) => from_slice(
            CHECKED_IN_SLICE.to_owned(),
            Migration::CheckedInDefault,
            Origin::CheckedInDefault,
        ),
        Err(error) => {
            error!("[server] the default document {SLICE_SOURCE}://{SLICE_FILE} failed to load: {error}");
            from_slice(
                CHECKED_IN_SLICE.to_owned(),
                Migration::CheckedInDefault,
                Origin::CheckedInDefault,
            )
        }
    };
    commands.insert_resource(defaults);
}

fn from_slice(text: String, migration: Migration, origin: Origin) -> Defaults {
    let mut doc = document::migrate_v1(&text, migration).unwrap_or_else(|reason| {
        panic!(
            "the default server document ({}) is refused: {reason}",
            origin.name()
        )
    });
    let instrument = native_overlay(&mut doc);
    info!(
        "[server] default document from {} (clock {}, stamina {:?}, rank {:?}, totalExp {})",
        origin.name(),
        document::clock_value(doc.clock),
        doc.stamina,
        doc.gamedata.mysekai_rank,
        doc.gamedata.total_exp,
    );
    Defaults {
        doc,
        origin,
        avatar_instrument: instrument,
    }
}

/// The developer instruments over the default document: the screen pools,
/// the fixed birthday clock and the avatar wear's names and colour. The
/// avatar waits for the masters, so only its instrument is returned.
fn native_overlay(doc: &mut ServerDocument) -> avatar::AvatarInstrument {
    let int = |name: &str| -> Option<i32> {
        let raw = instrument_env(name)?;
        match raw.trim().parse::<i32>() {
            Ok(value) => Some(value),
            Err(_) => {
                warn!("[server] {name}={raw:?} is not an integer; the document keeps its value");
                None
            }
        }
    };
    if let Some(exp) = int("MOLY_MENU_MOCK_TOTAL_EXP") {
        info!("[server] native overlay: MOLY_MENU_MOCK_TOTAL_EXP -> userMysekaiGamedata.totalExp {exp} (the rank follows it)");
        doc.gamedata.total_exp = exp;
        doc.gamedata.mysekai_rank = None;
    }
    let pools = [
        int("MOLY_MENU_MOCK_STAMINA_NORMAL"),
        int("MOLY_MENU_MOCK_STAMINA_ENHANCE"),
        int("MOLY_MENU_MOCK_STAMINA_BOOST"),
    ];
    if pools.iter().any(Option::is_some) {
        let base = doc.stamina.unwrap_or(Stamina {
            normal: 0,
            enhance: 0,
            boost: 0,
        });
        let stamina = Stamina {
            normal: pools[0].unwrap_or(base.normal),
            enhance: pools[1].unwrap_or(base.enhance),
            boost: pools[2].unwrap_or(base.boost),
        };
        info!(
            "[server] native overlay: MOLY_MENU_MOCK_STAMINA_* -> userMysekaiStamina {stamina:?}"
        );
        doc.stamina = Some(stamina);
    }
    if let Some(raw) = instrument_env("MOLY_BIRTHDAY_NOW_MS") {
        let at_ms = raw
            .trim()
            .parse::<i64>()
            .ok()
            .filter(|at| *at > 0)
            .unwrap_or_else(|| {
                panic!("MOLY_BIRTHDAY_NOW_MS={raw:?} is not a positive epoch millisecond")
            });
        info!(
            "[server] native overlay: MOLY_BIRTHDAY_NOW_MS -> the server clock is fixed at {at_ms}"
        );
        doc.clock = Clock::Fixed { at_ms };
    }
    if instrument_env("MOLY_MENU_MOCK_STAMINA_MAX").is_some() {
        warn!("[server] MOLY_MENU_MOCK_STAMINA_MAX is not applied: the gauge maximum is the master maxStamina");
    }
    const MUSIC: &str = "MOLY_AUDIO_MOCK_MUSIC_RECORD";
    if let Some(raw) = instrument_env(MUSIC) {
        doc.music_settings = music::parse_instrument(MUSIC, &raw);
        info!(
            "[server] native overlay: {MUSIC} -> userMysekaiMusicPlayFixtureSettings ({} sites)",
            doc.music_settings.len()
        );
    }
    inventory::native_overlay(doc);
    music_play::native_overlay(doc);
    tools::native_overlay(doc);
    phenomena::native_overlay(doc);
    let text = |name: &str| {
        instrument_env(name)
            .map(|raw| raw.trim().to_owned())
            .filter(|raw| !raw.is_empty())
    };
    avatar::AvatarInstrument {
        coordinate: text("MOLY_AVATAR_MOCK_COORDINATE"),
        costume: text("MOLY_AVATAR_MOCK_COSTUME"),
        accessory: text("MOLY_AVATAR_MOCK_ACCESSORY"),
        skin_color: text("MOLY_AVATAR_MOCK_SKIN_COLOR"),
    }
}
