//! Public remote resources the product reads at runtime instead of carrying
//! them in its asset root: asset storage (song and soundtrack audio, below
//! one directory per region) and masters (master tables, below
//! `<region>/master/`). Each kind has a primary host and a mirror host; every
//! host is an asset source of its own with one admitted HTTPS base: a path
//! below the source is joined to that base, and nothing outside the base is
//! reachable through it. Which host a consumer tries first, and whether it
//! falls back to the other, is the consumer's rule. The transfer is Bevy's
//! web asset reader (`ureq` on native, the page's `fetch` on wasm).
//!
//! The bases are admitted by the entry crate, the one refusal point for asset
//! locations, once the app is built and before it runs. A read before
//! admission, or of a path that is not a plain relative path, is refused by
//! name. Remote files carry no Bevy `.meta` files: a meta read answers
//! "not found" without a request.

use bevy::app::App;
use bevy::asset::io::{
    web::WebAssetReader, AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, Reader,
    VecReader,
};
use bevy::asset::{AssetApp, AssetPath};
use bevy::prelude::Resource;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// The admitted bases, each a canonical HTTPS directory ending in `/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBases {
    pub storage: String,
    pub storage_mirror: String,
    pub master: String,
    pub master_mirror: String,
}

/// One admitted host.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    Storage,
    StorageMirror,
    Master,
    MasterMirror,
}

impl Remote {
    pub const ALL: [Remote; 4] = [
        Remote::Storage,
        Remote::StorageMirror,
        Remote::Master,
        Remote::MasterMirror,
    ];

    /// The asset source id (`<id>://…`).
    pub const fn source(self) -> &'static str {
        match self {
            Remote::Storage => "moly-storage",
            Remote::StorageMirror => "moly-storage-mirror",
            Remote::Master => "moly-master",
            Remote::MasterMirror => "moly-master-mirror",
        }
    }

    fn base(self, bases: &RemoteBases) -> &str {
        match self {
            Remote::Storage => &bases.storage,
            Remote::StorageMirror => &bases.storage_mirror,
            Remote::Master => &bases.master,
            Remote::MasterMirror => &bases.master_mirror,
        }
    }
}

/// A region's directory on the storage and master hosts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RemoteRegion {
    Jp,
    Cn,
}

impl RemoteRegion {
    /// The region's asset directory on a storage host.
    fn storage_directory(self) -> &'static str {
        match self {
            Self::Jp => "sekai-jp-assets",
            Self::Cn => "sekai-cn-assets",
        }
    }

    /// The region's directory on a master host.
    fn master_directory(self) -> &'static str {
        match self {
            Self::Jp => "jp",
            Self::Cn => "cn",
        }
    }
}

/// One asset of a region on a storage host (`Storage` or `StorageMirror`);
/// `relative` is the asset's path below the region directory
/// (`music/long/0001_01/0001_01.mp3`).
pub fn storage_asset(host: Remote, region: RemoteRegion, relative: &str) -> AssetPath<'static> {
    debug_assert!(matches!(host, Remote::Storage | Remote::StorageMirror));
    AssetPath::from(format!(
        "{}://{}/{relative}",
        host.source(),
        region.storage_directory()
    ))
}

/// One master table of a region on a master host (`Master` or
/// `MasterMirror`), by its upstream table name (`musicVocals`).
pub fn master_table(host: Remote, region: RemoteRegion, table: &str) -> AssetPath<'static> {
    debug_assert!(matches!(host, Remote::Master | Remote::MasterMirror));
    AssetPath::from(format!(
        "{}://{}/master/{table}.json",
        host.source(),
        region.master_directory()
    ))
}

/// The bases the entry crate admitted; shared by the readers.
#[derive(Resource, Clone, Default)]
pub struct RemoteAdmission(Arc<RwLock<Option<RemoteBases>>>);

impl RemoteAdmission {
    fn set(&self, bases: RemoteBases) {
        *self
            .0
            .write()
            .unwrap_or_else(|poisoned| poisoned.into_inner()) = Some(bases);
    }

    /// The admitted bases, if admitted.
    pub fn bases(&self) -> Option<RemoteBases> {
        self.0
            .read()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone()
    }

    fn base(&self, remote: Remote) -> Option<String> {
        self.bases().map(|bases| remote.base(&bases).to_owned())
    }
}

/// Registers the four sources; must run before `AssetPlugin` is built.
pub(crate) fn register(app: &mut App) {
    let admission = RemoteAdmission::default();
    for remote in Remote::ALL {
        let admission = admission.clone();
        app.register_asset_source(
            remote.source(),
            AssetSourceBuilder::new(move || {
                Box::new(RemoteReader {
                    admission: admission.clone(),
                    remote,
                })
            }),
        );
    }
    app.insert_resource(admission);
}

/// Admits the bases of a built app, before it runs.
pub fn admit(app: &mut App, bases: RemoteBases) {
    app.world()
        .get_resource::<RemoteAdmission>()
        .expect("the remote sources are registered with the asset root")
        .set(bases);
}

struct RemoteReader {
    admission: RemoteAdmission,
    remote: Remote,
}

impl RemoteReader {
    /// `host/path` of the request (the web reader prefixes the scheme).
    fn target(&self, path: &Path) -> Result<PathBuf, AssetReaderError> {
        let refuse = |reason: String| {
            AssetReaderError::Io(Arc::new(std::io::Error::other(format!(
                "remote {:?} read of {}: {reason}",
                self.remote,
                path.display()
            ))))
        };
        let base = self
            .admission
            .base(self.remote)
            .ok_or_else(|| refuse("no base was admitted".into()))?;
        let host_and_path = base
            .strip_prefix("https://")
            .ok_or_else(|| refuse(format!("the admitted base {base} is not HTTPS")))?;
        let relative =
            plain_relative(path).ok_or_else(|| refuse("not a plain relative path".into()))?;
        Ok(PathBuf::from(format!("{host_and_path}{relative}")))
    }
}

/// The path as `a/b/c` when every component is a plain name of URL-safe
/// characters; `None` otherwise.
fn plain_relative(path: &Path) -> Option<String> {
    let mut parts = Vec::new();
    for component in path.components() {
        let Component::Normal(part) = component else {
            return None;
        };
        let part = part.to_str()?;
        if part.is_empty()
            || !part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
        {
            return None;
        }
        parts.push(part);
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

impl AssetReader for RemoteReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let target = self.target(path)?;
        let web = WebAssetReader::Https;
        let mut reader = AssetReader::read(&web, &target).await?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        Ok(VecReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Err(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}
