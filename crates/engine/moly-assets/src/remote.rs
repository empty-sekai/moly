//! Public remote resources the product reads at runtime instead of carrying
//! them in its asset root: asset storage (song and soundtrack audio, below
//! one directory per region) and masters (master tables, from a repository
//! host that carries one repository per region, and from two master hosts
//! that carry a directory per region, `jp` and `cn`). Every host is an asset
//! source of its own with one admitted HTTPS base: a path below the source is
//! joined to that base, and nothing outside the base is reachable through it.
//! Which host a consumer tries first, and whether it falls back to the others,
//! is the consumer's rule.
//!
//! On native the whole transfer, request and body, runs on the blocking
//! thread pool through `ureq`, so a slow or stalled host never holds a thread
//! of the asset task pool. It fails as `TimedOut` when it has not completed
//! within [`NATIVE_READ_MS`], and a master read also when its response
//! headers do not arrive within [`MASTER_FIRST_BYTE_MS`]. On wasm a storage
//! read is Bevy's web asset reader (the page's `fetch`) and a master read is
//! its own `fetch`, which fails as `TimedOut` when the response headers do not
//! arrive within [`MASTER_FIRST_BYTE_MS`].
//!
//! The bases are admitted by the entry crate, the one refusal point for asset
//! locations, once the app is built and before it runs. A read before
//! admission, or of a path that is not a plain relative path, is refused by
//! name. Remote files carry no Bevy `.meta` files: a meta read answers
//! "not found" without a request.
//!
//! The same native agent also reads the public asset store a packed release
//! comes from, which has no admitted base: its root is the resolved asset
//! source, and `read_store_bytes` bounds every body by the caller's limit.

use bevy::app::App;
#[cfg(target_arch = "wasm32")]
use bevy::asset::io::web::WebAssetReader;
use bevy::asset::io::{
    AssetReader, AssetReaderError, AssetSourceBuilder, PathStream, Reader, VecReader,
};
use bevy::asset::{AssetApp, AssetPath};
use bevy::prelude::Resource;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, RwLock};

/// How long a master host has to send its response headers before the read
/// fails as `TimedOut` (on the browser the body is not bounded).
pub const MASTER_FIRST_BYTE_MS: i32 = 10_000;

/// How long a native remote read may take, from the request to the last body
/// byte, before it fails as `TimedOut`: the bound the browser puts on the
/// product's own asset requests.
pub const NATIVE_READ_MS: u64 = 60_000;

/// The admitted bases, each a canonical HTTPS directory ending in `/`.
///
/// `master` names the master host, whose per-region directory is `jp` or `cn`
/// to match it. `master_repository` names the first master host, whose path
/// names the region's repository: a table of `cn` there is
/// `haruki-sekai-sc-master/main/master/<table>.json`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteBases {
    pub storage: String,
    pub storage_mirror: String,
    pub master: String,
    pub master_mirror: String,
    pub master_repository: String,
}

/// One admitted host, each an asset source of its own.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Remote {
    /// The asset storage host.
    Storage,
    /// The asset storage mirror.
    StorageMirror,
    /// The master host: a directory per region (`jp`, `cn`).
    Master,
    /// The master mirror: the same layout as [`Remote::Master`].
    MasterMirror,
    /// The master host that names the region's repository.
    MasterRepository,
}

impl Remote {
    pub const ALL: [Remote; 5] = [
        Remote::Storage,
        Remote::StorageMirror,
        Remote::Master,
        Remote::MasterMirror,
        Remote::MasterRepository,
    ];

    /// The asset source id (`<id>://…`).
    pub const fn source(self) -> &'static str {
        match self {
            Remote::Storage => "moly-storage",
            Remote::StorageMirror => "moly-storage-mirror",
            Remote::Master => "moly-master",
            Remote::MasterMirror => "moly-master-mirror",
            Remote::MasterRepository => "moly-master-repository",
        }
    }

    /// Whether this host serves master tables.
    pub const fn is_master(self) -> bool {
        matches!(
            self,
            Remote::Master | Remote::MasterMirror | Remote::MasterRepository
        )
    }

    fn base(self, bases: &RemoteBases) -> &str {
        match self {
            Remote::Storage => &bases.storage,
            Remote::StorageMirror => &bases.storage_mirror,
            Remote::Master => &bases.master,
            Remote::MasterMirror => &bases.master_mirror,
            Remote::MasterRepository => &bases.master_repository,
        }
    }
}

/// A region whose content the product reads: the storage directory of its
/// assets and the repository of its master tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Region {
    Jp,
    Cn,
    Tw,
    Kr,
    En,
}

/// A region's `Region::ALL`, in master directory order.
pub const ALL: [Region; 5] = [Region::Jp, Region::Cn, Region::Tw, Region::Kr, Region::En];

/// The regions the product reads: every master table is the JP rows plus
/// the exclusive rows of each of these. A region outside it is a known value
/// that is never requested. Another region joins once the asset store carries
/// the resources its exclusive rows name; until then the tables are JP's.
pub const ENABLED: &[Region] = &[Region::Jp];

/// A display language a region's text is written in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Language {
    Ja,
    ZhHans,
    ZhHant,
    Ko,
    En,
}

impl Region {
    /// The region's directory on a storage host.
    pub fn storage_directory(self) -> &'static str {
        match self {
            Self::Jp => "sekai-jp-assets",
            Self::Cn => "sekai-cn-assets",
            Self::Tw => "sekai-tw-assets",
            Self::Kr => "sekai-kr-assets",
            Self::En => "sekai-en-assets",
        }
    }

    /// The region's directory on a master host.
    pub fn directory(self) -> &'static str {
        match self {
            Self::Jp => "jp",
            Self::Cn => "cn",
            Self::Tw => "tw",
            Self::Kr => "kr",
            Self::En => "en",
        }
    }

    /// The region from its directory name (`cn`).
    pub fn from_directory(name: &str) -> Option<Region> {
        ALL.into_iter().find(|region| region.directory() == name)
    }

    /// The region's repository on the repository host: the path's
    /// repository segment.
    pub fn repository(self) -> &'static str {
        match self {
            Self::Jp => "haruki-sekai-master",
            Self::Cn => "haruki-sekai-sc-master",
            Self::Tw => "haruki-sekai-tc-master",
            Self::Kr => "haruki-sekai-kr-master",
            Self::En => "haruki-sekai-en-master",
        }
    }

    /// The language the region's own text is written in.
    pub const fn native_language(self) -> Language {
        match self {
            Self::Jp => Language::Ja,
            Self::Cn => Language::ZhHans,
            Self::Tw => Language::ZhHant,
            Self::Kr => Language::Ko,
            Self::En => Language::En,
        }
    }
}

/// Whether `region` is one of [`ENABLED`].
pub fn enabled(region: Region) -> bool {
    ENABLED.contains(&region)
}

/// One asset of a region on a storage host (`Storage` or `StorageMirror`);
/// `relative` is the asset's path below the region directory
/// (`music/long/0001_01/0001_01.mp3`).
pub fn storage_asset(host: Remote, region: Region, relative: &str) -> AssetPath<'static> {
    debug_assert!(matches!(host, Remote::Storage | Remote::StorageMirror));
    AssetPath::from(format!(
        "{}://{}/{relative}",
        host.source(),
        region.storage_directory()
    ))
}

/// The master path of `table` below the region directory the master hosts
/// carry (`jp/master/musicVocals.json`).
pub fn master_directory_table(region: Region, table: &str) -> String {
    format!("{}/master/{table}.json", region.directory())
}

/// The master path of `table` below the region's repository
/// (`haruki-sekai-master/main/master/musicVocals.json`).
pub fn master_repository_table(region: Region, table: &str) -> String {
    format!("{}/main/master/{table}.json", region.repository())
}

/// One master table of a region on a master host, by its upstream table name
/// (`musicVocals`): below the region directory on the master hosts, below the
/// region's repository on [`Remote::MasterRepository`].
pub fn master_table(host: Remote, region: Region, table: &str) -> AssetPath<'static> {
    debug_assert!(host.is_master());
    let path = match host {
        Remote::MasterRepository => master_repository_table(region, table),
        _ => master_directory_table(region, table),
    };
    AssetPath::from(format!("{}://{path}", host.source()))
}

/// A region's `versions/current_version.json` on a master host.
pub fn master_version(host: Remote, region: Region) -> AssetPath<'static> {
    debug_assert!(host.is_master());
    let path = match host {
        Remote::MasterRepository => {
            format!("{}/main/versions/current_version.json", region.repository())
        }
        _ => format!("{}/versions/current_version.json", region.directory()),
    };
    AssetPath::from(format!("{}://{path}", host.source()))
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

    /// The HTTPS URL a remote asset path is read from, once admitted.
    pub fn url(&self, path: &AssetPath) -> Option<String> {
        let source = path.source().as_str()?;
        let remote = Remote::ALL
            .into_iter()
            .find(|remote| remote.source() == source)?;
        join(&self.base(remote)?, path.path().to_str()?)
    }
}

/// `base` followed by `relative`. A base that names `{repo}` (the repository
/// host's template) takes the first segment of `relative` there, and the rest
/// of `relative` after it.
fn join(base: &str, relative: &str) -> Option<String> {
    if !base.contains("{repo}") {
        return Some(format!("{base}{relative}"));
    }
    let (repo, rest) = relative.split_once('/')?;
    Some(format!("{}{rest}", base.replace("{repo}", repo)))
}

/// Registers the sources; must run before `AssetPlugin` is built.
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
        let joined = join(host_and_path, &relative)
            .ok_or_else(|| refuse(format!("{relative} names no repository")))?;
        Ok(PathBuf::from(joined))
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
        let url = format!("https://{}", target.display());
        // A master host that does not answer within the bound fails the read
        // as `TimedOut`, and the master layer tries its next host.
        let master = self.remote.is_master();
        #[cfg(not(target_arch = "wasm32"))]
        let bytes = native::read(url, master).await?;
        // Bevy's fetch has no bound: a connect the network drops waits for the
        // system's own connect limit (about two minutes).
        #[cfg(target_arch = "wasm32")]
        let bytes = if master {
            crate::http::read_first_byte_bounded(&url, MASTER_FIRST_BYTE_MS).await?
        } else {
            let web = WebAssetReader::Https;
            let mut reader = AssetReader::read(&web, &target).await?;
            let mut bytes = Vec::new();
            reader.read_to_end(&mut bytes).await?;
            bytes
        };
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

/// The bytes of a public asset store path below its HTTPS root, read on
/// native; the browser store read is `crate::http::read_bytes`.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use native::read_store_bytes;

/// Native transfers: one `ureq` agent per bound, so connections are reused
/// across the reads of a host kind.
#[cfg(not(target_arch = "wasm32"))]
mod native {
    use super::{AssetReaderError, MASTER_FIRST_BYTE_MS, NATIVE_READ_MS};
    use std::io::{self, BufReader, Read};
    use std::path::PathBuf;
    use std::sync::{Arc, LazyLock};
    use std::time::Duration;
    use ureq::tls::{RootCerts, TlsConfig};
    use ureq::Agent;

    fn agent(first_byte: Option<Duration>) -> Agent {
        Agent::config_builder()
            .tls_config(
                TlsConfig::builder()
                    .root_certs(RootCerts::PlatformVerifier)
                    .build(),
            )
            .timeout_global(Some(Duration::from_millis(NATIVE_READ_MS)))
            .timeout_recv_response(first_byte)
            .build()
            .new_agent()
    }

    static MASTER: LazyLock<Agent> = LazyLock::new(|| {
        agent(Some(Duration::from_millis(MASTER_FIRST_BYTE_MS as u64)))
    });
    static STORAGE: LazyLock<Agent> = LazyLock::new(|| agent(None));

    /// A failed transfer of `url` as an I/O error, `TimedOut` when the
    /// agent's bound expired.
    fn failure(url: &str, error: ureq::Error) -> AssetReaderError {
        transfer_failure("remote read", url, error)
    }

    /// A failed store transfer of `url`, with the same kinds.
    fn store_failure(url: &str, error: ureq::Error) -> AssetReaderError {
        transfer_failure("asset store read", url, error)
    }

    fn transfer_failure(what: &str, url: &str, error: ureq::Error) -> AssetReaderError {
        let kind = if matches!(error, ureq::Error::Timeout(_)) {
            io::ErrorKind::TimedOut
        } else {
            io::ErrorKind::Other
        };
        AssetReaderError::Io(Arc::new(io::Error::new(
            kind,
            format!("{what} of {url}: {error}"),
        )))
    }

    /// An oversized asset store body, refused before it is buffered whole.
    fn oversized(path: &str) -> AssetReaderError {
        AssetReaderError::Io(Arc::new(io::Error::new(
            io::ErrorKind::InvalidData,
            format!("asset store read of {path}: response exceeds its byte limit"),
        )))
    }

    /// The body of `url`, read on the blocking thread pool. A 404 is "not
    /// found" and another error status an HTTP error, as Bevy's web reader
    /// answers them; a timeout is `TimedOut`.
    pub(super) async fn read(url: String, master: bool) -> Result<Vec<u8>, AssetReaderError> {
        blocking::unblock(move || {
            let agent = if master { &*MASTER } else { &*STORAGE };
            let mut response = match agent.get(&url).call() {
                Ok(response) => response,
                Err(ureq::Error::StatusCode(404)) => {
                    return Err(AssetReaderError::NotFound(PathBuf::from(&url)));
                }
                Err(ureq::Error::StatusCode(code)) => return Err(AssetReaderError::HttpError(code)),
                Err(error) => return Err(failure(&url, error)),
            };
            let mut bytes = Vec::new();
            BufReader::new(response.body_mut().with_config().reader())
                .read_to_end(&mut bytes)
                .map_err(|error| failure(&url, error.into()))?;
            Ok(bytes)
        })
        .await
    }

    /// One request for an asset store path. The body is requested identity
    /// encoded: the store serves its files as they are, and an identity body
    /// keeps `Content-Length` an exact body size. A 403 or 404 is "not found",
    /// as the browser store read answers it.
    fn store_response(url: &str) -> Result<ureq::http::Response<ureq::Body>, AssetReaderError> {
        match STORAGE.get(url).header("Accept-Encoding", "identity").call() {
            Ok(response) => Ok(response),
            Err(ureq::Error::StatusCode(403 | 404)) => {
                Err(AssetReaderError::NotFound(PathBuf::from(url)))
            }
            Err(ureq::Error::StatusCode(code)) => Err(AssetReaderError::HttpError(code)),
            Err(error) => Err(store_failure(url, error)),
        }
    }

    /// The bytes of the asset store path `path` below the HTTPS `root`, read
    /// on the blocking thread pool. A body larger than `limit` is refused
    /// rather than buffered whole. `exact` says that `limit` is the
    /// authenticated size, so the body buffer is allocated once, as the
    /// browser allocates it.
    pub(crate) async fn read_store_bytes(
        root: String,
        path: String,
        limit: usize,
        exact: bool,
    ) -> Result<Vec<u8>, AssetReaderError> {
        blocking::unblock(move || {
            let url = format!("{root}{path}");
            // One byte over the limit tells an oversized body apart from one
            // that is exactly at the limit.
            let bound = limit.checked_add(1).ok_or_else(|| oversized(&path))?;
            let mut response = store_response(&url)?;
            if response
                .body()
                .content_length()
                .is_some_and(|length| length > limit as u64)
            {
                return Err(oversized(&path));
            }
            let mut bytes = Vec::new();
            if exact {
                bytes.try_reserve_exact(limit).map_err(|_| {
                    AssetReaderError::Io(Arc::new(io::Error::other(format!(
                        "asset store read of {path}: could not allocate its buffer"
                    ))))
                })?;
            }
            response
                .body_mut()
                .with_config()
                .limit(bound as u64)
                .reader()
                .read_to_end(&mut bytes)
                .map_err(|error| match ureq::Error::from(error) {
                    ureq::Error::BodyExceedsLimit(_) => oversized(&path),
                    error => store_failure(&url, error),
                })?;
            if bytes.len() > limit {
                return Err(oversized(&path));
            }
            Ok(bytes)
        })
        .await
    }
}
