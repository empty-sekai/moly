//! Whether the release root carries the site-move effect products.
//!
//! The pipeline ships them together: the cannon's particle document, the
//! three effect prefabs with their particle documents (flying, landing,
//! failed landing) and the FieldCamera prefab the speed lines read. One
//! presence check against data the runtime already loads decides whether
//! any of them is requested: the fixture-particles-v2 index must list the
//! cannon package and the three effect packages.
//!
//! - Listed: the moves play their effects from the effect pools (built
//!   from the effect table at startup, as the source builds them in the
//!   field scene's setup, before any move), the speed-line prefab is
//!   requested at once, and each move requests the cannon's particle
//!   document. A listed product whose file is missing is a release defect
//!   and fails loudly (the asset server's own ERROR lines).
//! - Not listed: one WARN naming the missing packages and no request at
//!   all; the moves run without those effects, the cannon's particles and
//!   the speed lines.

use bevy::prelude::*;
use moly_assets::json::JsonAsset;
use serde_json::Value;

pub(crate) const INDEX: &str = "moly://fixture-particles-v2/index.json";

/// The packages of `packages` the fixture-particles-v2 index does not list
/// with a file (or lists as missing).
pub(crate) fn unlisted(index: &Value, packages: impl IntoIterator<Item = String>) -> Vec<String> {
    packages
        .into_iter()
        .filter(|package| {
            let entry = &index["packages"][package.as_str()];
            !(entry["file"].is_string() && entry["missing"] != true)
        })
        .collect()
}

#[derive(Resource)]
pub(crate) enum SiteMoveProducts {
    Pending(Handle<JsonAsset>),
    Present,
    Absent,
}

impl SiteMoveProducts {
    /// None until the index has been read.
    pub(crate) fn present(&self) -> Option<bool> {
        match self {
            Self::Pending(_) => None,
            Self::Present => Some(true),
            Self::Absent => Some(false),
        }
    }
}

/// The packages the check requires.
fn packages() -> Vec<String> {
    let mut packages = vec![super::cannon::PARTICLE_PACKAGE.to_owned()];
    packages.extend(super::effects::packages());
    packages
}

pub(crate) fn request(mut commands: Commands, server: Res<AssetServer>) {
    commands.insert_resource(SiteMoveProducts::Pending(server.load(INDEX)));
}

pub(crate) fn pending(products: Option<Res<SiteMoveProducts>>) -> bool {
    products.is_some_and(|products| products.present().is_none())
}

/// Read the index once it has loaded and request the products it lists.
pub(crate) fn resolve(world: &mut World) {
    let Some(SiteMoveProducts::Pending(handle)) = world.get_resource::<SiteMoveProducts>() else {
        return;
    };
    let handle = handle.clone();
    let missing: Vec<String> = if world
        .resource::<AssetServer>()
        .load_state(&handle)
        .is_failed()
    {
        packages()
    } else {
        let Some(text) = world
            .resource::<Assets<JsonAsset>>()
            .get(&handle)
            .map(|json| json.0.clone())
        else {
            return;
        };
        let index: Value = serde_json::from_str(&text).unwrap_or(Value::Null);
        unlisted(&index, packages())
    };
    if missing.is_empty() {
        info!("[site-move] the release root lists the site-move effect products {:?}; requesting them", packages());
        world.insert_resource(SiteMoveProducts::Present);
        world.insert_resource(super::SiteMoveEffects(super::effects::Effects::default()));
        super::speed_lines::request(world);
    } else {
        warn!(
            "[site-move] release root predates the site-move effects: {INDEX} does not list {missing:?}; no request for the flying/landing effects, the cannon particles or the speed-line prefab, and the moves run without them"
        );
        world.insert_resource(SiteMoveProducts::Absent);
    }
}
