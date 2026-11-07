//! Where the assets come from: the native resource root and the four remote
//! bases the product reads at runtime.
//!
//! The native game and the server resolve the root here, through one
//! function: the configured `resources.root` when it is set, else
//! `resources.store`, else the resources the launcher installed. A browser
//! page has no configuration file; its own resolution (the `?assets=` prefix
//! and the game seed) stays in the entry crate and reuses the admission below.
//!
//! Every web path is admitted once, here: a same-origin absolute prefix, or
//! an HTTPS root this module names. `resources.catalog` (a full lowercase
//! SHA-256) selects a release of the content-addressed store, and
//! `resources.cache` names the native directory that keeps what a store read
//! fetched.
//!
//! The remote bases are configuration keys too (`resources.storage`,
//! `resources.storage_mirror`, `masters.base`, `masters.mirror`,
//! `masters.repository` and `masters.repository_<region>`): a key that is not
//! set names no host, and a host or region that has none is not read.

use crate::AssetSource;

fn validate_catalog_id(catalog: &Option<String>) -> Result<(), String> {
    if catalog.as_deref().is_some_and(|id| {
        id.len() != 64
            || !id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err("asset catalog must be a full lowercase SHA-256".into());
    }
    Ok(())
}

/// Only immutable public snapshots and the content-addressed store may use
/// CDN. `relative` is the path below the configured resource root.
fn is_public_asset_directory(relative: &str) -> bool {
    if relative == "asset-store/" {
        return true;
    }
    let Some(id) = relative
        .strip_prefix("snapshots/")
        .and_then(|rest| rest.strip_suffix("/assets/"))
    else {
        return false;
    };
    id.len() <= 96
        && id.as_bytes().first().is_some_and(u8::is_ascii_alphanumeric)
        && id.bytes().all(|b| {
            b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-')
        })
}

/// Split the strict HTTPS representation supplied by the host. Browser Url
/// parsing is authoritative for host/port/IP canonicalization; this shared
/// admission rejects ambiguous separators and credentials beforehand.
pub fn public_asset_parts(base: &str) -> Option<(&str, &str)> {
    let authority_and_path = base.strip_prefix("https://")?;
    let slash = authority_and_path.find('/')?;
    let authority = &authority_and_path[..slash];
    if authority.is_empty()
        || authority.bytes().any(|b| {
            !b.is_ascii_lowercase()
                && !b.is_ascii_digit()
                && !matches!(b, b'.' | b'-' | b'_' | b':' | b'[' | b']')
        })
    {
        return None;
    }
    let offset = "https://".len() + slash;
    Some((&base[..offset], &base[offset..]))
}

pub fn is_canonical_directory(path: &str) -> bool {
    path.starts_with('/')
        && !path.starts_with("//")
        && path.ends_with('/')
        && !path
            .bytes()
            .any(|b| b <= b' ' || b == 0x7f || matches!(b, b'\\' | b'%' | b'?' | b'#'))
        && !path.split('/').any(|part| matches!(part, "." | ".."))
}

/// An origin alone selects only the original `/moly/` deployment; any other
/// prefix arrives as the complete `resource_base`.
const ORIGIN_ONLY_ROOT: &str = "/moly/";

/// The HTTPS origin and path prefix that remote assets must live below.
fn resource_root<'a>(
    configured_origin: Option<&'a str>,
    configured_base: Option<&'a str>,
) -> Result<Option<(&'a str, &'a str)>, String> {
    let Some(base) = configured_base else {
        return Ok(configured_origin.map(|origin| (origin, ORIGIN_ONLY_ROOT)));
    };
    let (origin, path) = public_asset_parts(base)
        .filter(|(_, path)| is_canonical_directory(path))
        .ok_or("resource_base must be a canonical HTTPS directory ending in /")?;
    if configured_origin.is_some_and(|configured| configured != origin) {
        return Err("resource_origin must match resource_base".into());
    }
    Ok(Some((origin, if path == "/" { ORIGIN_ONLY_ROOT } else { path })))
}

/// Asset roots are canonical paths, optionally on a configured HTTPS CDN.
fn validate_asset_prefix(base: &str, root: Option<(&str, &str)>) -> Result<(), String> {
    let path = if base.starts_with("https://") {
        let (origin, path) =
            public_asset_parts(base).ok_or("Invalid public Moly resource directory")?;
        let (root_origin, root_path) =
            root.ok_or("Remote assets must match the configured resource_origin")?;
        if origin != root_origin {
            return Err("Remote assets must match the configured resource_origin".into());
        }
        if !path
            .strip_prefix(root_path)
            .is_some_and(is_public_asset_directory)
        {
            return Err("Invalid public Moly resource directory".into());
        }
        path
    } else {
        base
    };
    if !is_canonical_directory(path) {
        return Err("?assets= must be a canonical same-origin directory or trusted public resource root ending in /".into());
    }
    Ok(())
}

/// The selected remote assets must lie below the resource root explicitly
/// passed by the same-origin host: its complete `resource_base`, or the
/// original deployment of its `resource_origin`. A second remote origin
/// cannot arrive through assets alone.
pub fn validate_asset_selection(
    base: &str,
    configured_origin: Option<&str>,
    configured_base: Option<&str>,
) -> Result<(), String> {
    validate_asset_prefix(base, resource_root(configured_origin, configured_base)?)
}

/// Whether `base` is a `file://` base: on native, an existing local
/// directory ending in `/`, read in the host's own layout. The browser reads
/// HTTPS hosts only. `Ok(false)` when `base` is not a `file://` address.
fn file_base(key: &str, base: &str) -> Result<bool, String> {
    let Some(path) = crate::remote::file_directory(base) else {
        return Ok(false);
    };
    if cfg!(target_arch = "wasm32") {
        return Err(format!(
            "`{key}` names a file directory, which only the native program reads"
        ));
    }
    let directory = path.split("{repo}").next().unwrap_or(path);
    if !path.ends_with('/') || !std::path::Path::new(directory).is_dir() {
        return Err(format!(
            "`{key}` must name an existing local directory ending in /"
        ));
    }
    Ok(true)
}

/// One remote base: a configured canonical HTTPS directory like
/// `resource_base`, a native `file://` directory, or `None` when the key
/// `key` is not set.
fn remote_base(key: &str, configured: Option<&str>) -> Result<Option<String>, String> {
    let Some(base) = configured else {
        return Ok(None);
    };
    if file_base(key, base)? {
        return Ok(Some(base.to_owned()));
    }
    public_asset_parts(base)
        .filter(|(_, path)| is_canonical_directory(path))
        .map(|_| Some(base.to_owned()))
        .ok_or_else(|| format!("`{key}` must be a canonical HTTPS directory ending in /"))
}

/// A master host whose paths name the region's repository: the configured
/// template must be a canonical HTTPS directory (or a native `file://`
/// directory) that names `{repo}` exactly once, as a whole path segment;
/// `None` when it is not configured.
fn repository_base(key: &str, configured: Option<&str>) -> Result<Option<String>, String> {
    let Some(template) = configured else {
        return Ok(None);
    };
    let path = match crate::remote::file_directory(template) {
        Some(path) if file_base(key, template)? => path,
        _ => {
            public_asset_parts(template)
                .filter(|(_, path)| is_canonical_directory(path))
                .ok_or_else(|| {
                    format!(
                        "`{key}` must be a canonical HTTPS directory ending in /, \
                         with {{repo}} as a path segment"
                    )
                })?
                .1
        }
    };
    if path.split('/').filter(|part| *part == "{repo}").count() != 1 {
        return Err(format!(
            "`{key}` must name {{repo}} exactly once, as a whole path segment"
        ));
    }
    Ok(Some(template.to_owned()))
}

/// A region's configured repository name: one URL-safe path segment (ASCII
/// letters, digits, `.`, `_` and `-`); `None` when it is not configured.
fn repository_name(key: &str, configured: Option<&str>) -> Result<Option<String>, String> {
    let Some(name) = configured else {
        return Ok(None);
    };
    if name.is_empty()
        || !name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'_' | b'-'))
    {
        return Err(format!(
            "`{key}` must be one URL-safe path segment (letters, digits, `.`, `_` or `-`)"
        ));
    }
    Ok(Some(name.to_owned()))
}

/// Configured remote bases and repository names; `None` says that the key
/// does not name one, and a host or a region with none is not read.
/// Which host is tried first is the consumer's rule.
#[derive(Default)]
pub struct RemoteBaseOverrides<'a> {
    pub storage: Option<&'a str>,
    pub storage_mirror: Option<&'a str>,
    pub master: Option<&'a str>,
    pub master_mirror: Option<&'a str>,
    /// A template naming `{repo}` where the repository host's region
    /// repository goes.
    pub master_repository: Option<&'a str>,
    /// The repository name of each region on the repository host.
    pub master_repository_jp: Option<&'a str>,
    pub master_repository_cn: Option<&'a str>,
    pub master_repository_tw: Option<&'a str>,
    pub master_repository_kr: Option<&'a str>,
    pub master_repository_en: Option<&'a str>,
}

/// The remote bases and repository names. The browser entry additionally
/// requires each configured base to be its canonical URL.
pub fn remote_bases(
    overrides: RemoteBaseOverrides<'_>,
) -> Result<crate::remote::RemoteBases, String> {
    let names = [
        ("masters.repository_jp", overrides.master_repository_jp),
        ("masters.repository_cn", overrides.master_repository_cn),
        ("masters.repository_tw", overrides.master_repository_tw),
        ("masters.repository_kr", overrides.master_repository_kr),
        ("masters.repository_en", overrides.master_repository_en),
    ];
    let mut repositories = std::collections::BTreeMap::new();
    for (region, (key, configured)) in crate::remote::ALL.into_iter().zip(names) {
        if let Some(name) = repository_name(key, configured)? {
            repositories.insert(region, name);
        }
    }
    Ok(crate::remote::RemoteBases {
        storage: remote_base("resources.storage", overrides.storage)?,
        storage_mirror: remote_base("resources.storage_mirror", overrides.storage_mirror)?,
        master: remote_base("masters.base", overrides.master)?,
        master_mirror: remote_base("masters.mirror", overrides.master_mirror)?,
        master_repository: repository_base("masters.repository", overrides.master_repository)?,
        repositories,
    })
}

/// Native: `resources.storage`, `resources.storage_mirror`, `masters.base`,
/// `masters.mirror` and `masters.repository` (a template naming `{repo}`)
/// name the hosts, and `masters.repository_jp` … `masters.repository_en` the
/// repository name of each region on the repository host. Each key is read
/// from the configuration with its environment variable taking precedence;
/// a key that is not set names no host or repository.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_remote() -> Result<crate::remote::RemoteBases, String> {
    use moly_config::{keys, text};
    let (
        storage,
        storage_mirror,
        master,
        master_mirror,
        master_repository,
        master_repository_jp,
        master_repository_cn,
        master_repository_tw,
        master_repository_kr,
        master_repository_en,
    ) = (
        text(&keys::STORAGE_BASE),
        text(&keys::STORAGE_MIRROR),
        text(&keys::MASTER_BASE),
        text(&keys::MASTER_MIRROR),
        text(&keys::MASTER_REPOSITORY),
        text(&keys::MASTER_REPOSITORY_JP),
        text(&keys::MASTER_REPOSITORY_CN),
        text(&keys::MASTER_REPOSITORY_TW),
        text(&keys::MASTER_REPOSITORY_KR),
        text(&keys::MASTER_REPOSITORY_EN),
    );
    remote_bases(RemoteBaseOverrides {
        storage: storage.as_deref(),
        storage_mirror: storage_mirror.as_deref(),
        master: master.as_deref(),
        master_mirror: master_mirror.as_deref(),
        master_repository: master_repository.as_deref(),
        master_repository_jp: master_repository_jp.as_deref(),
        master_repository_cn: master_repository_cn.as_deref(),
        master_repository_tw: master_repository_tw.as_deref(),
        master_repository_kr: master_repository_kr.as_deref(),
        master_repository_en: master_repository_en.as_deref(),
    })
}

/// Native: `resources.root`, else `resources.store`, else the resources the
/// launcher installed. Each key is read from the configuration with its
/// environment variable (`MOLY_ASSET_ROOT`, `MOLY_ASSET_STORE_URL`, ...) taking
/// precedence.
///
/// `resources.root` is the extracted-asset directory. `resources.store` is a
/// public asset store directory over HTTPS (`.../moly/asset-store/`), read
/// from the network as the browser reads it; setting both is an error,
/// because one run reads one source. `resources.catalog` (a full lowercase
/// SHA-256) selects a release of either. `resources.cache` is the directory a
/// store read keeps the immutable documents and blobs it fetched in, and is
/// created when missing; it applies to `resources.store` only.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve() -> Result<AssetSource, String> {
    let config = moly_config::global();
    let root = config.text(&moly_config::keys::RESOURCE_ROOT);
    let store = config.text(&moly_config::keys::RESOURCE_STORE);
    match (root, store) {
        (Some(_), Some(_)) => Err(
            "resources.root (MOLY_ASSET_ROOT) and resources.store (MOLY_ASSET_STORE_URL) are both set; keep exactly one".into(),
        ),
        (Some(raw), None) => local_root(raw),
        (None, Some(url)) => public_store(url),
        (None, None) => installed_resources(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn local_root(raw: String) -> Result<AssetSource, String> {
    if raw.trim().is_empty() {
        return Err("resources.root (MOLY_ASSET_ROOT) is empty; point it at the extracted-asset directory".into());
    }
    let path = std::path::PathBuf::from(raw);
    if !path.is_dir() {
        return Err(format!(
            "resources.root (MOLY_ASSET_ROOT) is not a directory: {}",
            path.display()
        ));
    }
    let catalog = moly_config::global().text(&moly_config::keys::RESOURCE_CATALOG);
    validate_catalog_id(&catalog)?;
    if catalog.is_some() || path.join("asset-packs.json").is_file() {
        Ok(AssetSource::NativePacks { path, catalog })
    } else {
        Ok(AssetSource::NativeDir { path })
    }
}

/// A public asset store directory: a canonical HTTPS directory ending in the
/// content-addressed `asset-store/`. The URL is admitted against its own
/// origin with the publication prefix that directory sits in as the resource
/// root, by the same rule a browser root passes, so
/// `https://cdn.example/moly/asset-store/` and a CDN publication
/// (`https://cdn.example/release/asset-store/`) both pass, while a query,
/// fragment, credentials, an escaped path or another directory is refused.
#[cfg(not(target_arch = "wasm32"))]
fn store_root(url: &str) -> Result<(), String> {
    let (origin, path) = public_asset_parts(url)
        .filter(|(_, path)| is_canonical_directory(path))
        .ok_or("resources.store must be a canonical HTTPS asset store directory")?;
    let publication = path
        .strip_suffix("asset-store/")
        .map(|prefix| format!("{origin}{prefix}"))
        .ok_or("resources.store must be a canonical HTTPS asset store directory")?;
    validate_asset_selection(url, Some(origin), Some(&publication))
        .map_err(|error| format!("resources.store: {error}"))
}

/// The public asset store as an asset source, admitted by [`store_root`].
/// `resources.cache` names the directory that keeps the immutable files a
/// run fetched; it is created when missing.
#[cfg(not(target_arch = "wasm32"))]
fn public_store(url: String) -> Result<AssetSource, String> {
    if url.trim().is_empty() {
        return Err(
            "resources.store (MOLY_ASSET_STORE_URL) is empty; point it at the public asset store"
                .into(),
        );
    }
    store_root(&url)?;
    let catalog = moly_config::global().text(&moly_config::keys::RESOURCE_CATALOG);
    validate_catalog_id(&catalog)?;
    let cache = match moly_config::global().text(&moly_config::keys::RESOURCE_CACHE) {
        Some(value) if value.trim().is_empty() => {
            return Err(
                "resources.cache (MOLY_ASSET_CACHE) is empty; point it at a cache directory".into(),
            );
        }
        Some(value) => {
            let path = std::path::PathBuf::from(value);
            std::fs::create_dir_all(&path).map_err(|error| {
                format!(
                    "resources.cache (MOLY_ASSET_CACHE) is not usable: {}: {error}",
                    path.display()
                )
            })?;
            Some(path)
        }
        None => None,
    };
    Ok(AssetSource::HttpPacks {
        url,
        catalog,
        cache,
    })
}

/// Without `resources.root` or `resources.store`, the resources the
/// launcher installed: its resource directory and the selected (or newest)
/// complete snapshot there.
#[cfg(not(target_arch = "wasm32"))]
fn installed_resources() -> Result<AssetSource, String> {
    let (path, installed) = moly_store::config::default_selection()?.ok_or_else(|| {
        let dir = moly_store::config::LauncherConfig::load()
            .and_then(|config| config.resource_dir())
            .map(|dir| dir.display().to_string())
            .unwrap_or_else(|error| error);
        format!(
            "no resources are installed in {dir}; download them with moly-launcher, \
             or set resources.root (MOLY_ASSET_ROOT) to a resource directory, \
             or resources.store (MOLY_ASSET_STORE_URL) to a public asset store"
        )
    })?;
    let catalog = moly_config::global()
        .text(&moly_config::keys::RESOURCE_CATALOG)
        .or(Some(installed.catalog));
    validate_catalog_id(&catalog)?;
    Ok(AssetSource::NativePacks { path, catalog })
}

#[cfg(test)]
mod public_resource_tests {
    use super::{public_asset_parts, validate_asset_selection};

    /// Each remote case is selected by its own origin; the provider prefix
    /// arrives as its complete resource root.
    fn validate_asset_prefix(value: &str) -> Result<(), String> {
        let base = value
            .contains("/publication/")
            .then_some("https://resources.example/publication/");
        validate_asset_selection(value, public_asset_parts(value).map(|(origin, _)| origin), base)
    }

    #[test]
    fn public_immutable_roots_accept_different_configured_https_origins() {
        for value in [
            "/moly/sources/local/",
            "https://assets-one.example/moly/snapshots/cn-1.2.3-a/assets/",
            "https://cdn-two.example:8443/moly/asset-store/",
            "https://resources.example/publication/snapshots/cn-1.2.3-a/assets/",
            "https://resources.example/publication/asset-store/",
        ] {
            assert!(validate_asset_prefix(value).is_ok(), "{value}");
        }
        for value in [
            "http://cdn.example/moly/asset-store/",
            "https://user@cdn.example/moly/asset-store/",
            "https://cdn.example/api/player/",
            "https://cdn.example/moly/sources/private/",
            "https://cdn.example/moly/snapshots/../cn-a/assets/",
            "https://cdn.example/moly/snapshots/cn-a/assets/?token=private",
            "https://cdn.example/moly/snapshots/cn-a/assets/%2f",
            "https://cdn.example/moly/snapshots/cn-a/assets/#fragment",
            "https://resources.example/publication/releases/current/",
            "//cdn.example/moly/asset-store/",
        ] {
            assert!(validate_asset_prefix(value).is_err(), "{value}");
        }
    }

    #[test]
    fn remote_assets_require_the_selected_origin_without_a_baked_domain() {
        for origin in ["https://assets-one.example", "https://cdn-two.example:8443"] {
            let root = format!("{origin}/moly/asset-store/");
            assert!(validate_asset_selection(&root, Some(origin), None).is_ok());
            assert!(validate_asset_selection(&root, None, None).is_err());
            assert!(validate_asset_selection(&root, Some("https://unselected.example"), None).is_err());
        }
        assert!(validate_asset_selection("/moly/sources/local/", None, None).is_ok());
    }

    /// A store URL names the publication directory it sits in; anything that
    /// is not one public asset directory below that publication is refused.
    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn store_urls_name_one_public_asset_directory_below_a_publication() {
        use super::store_root;
        for value in [
            "https://cdn.example/moly/asset-store/",
            "https://cdn-two.example:8443/moly/asset-store/",
            "https://resources.example/publication/asset-store/",
        ] {
            assert!(store_root(value).is_ok(), "{value}");
        }
        for value in [
            "http://cdn.example/moly/asset-store/",
            "https://user@cdn.example/moly/asset-store/",
            "https://cdn.example/moly/",
            "https://cdn.example/moly/sources/private/",
            "https://cdn.example/moly/asset-store/blobs/",
            "https://cdn.example/moly/asset-store/?token=private",
            "https://cdn.example/moly/snapshots/cn-a/assets/",
            "https://cdn.example/moly/asset-store/%2f",
            "https://cdn.example/moly/asset-store/#fragment",
            "https://cdn.example/asset-store/",
            "https://cdn.example/",
            "//cdn.example/moly/asset-store/",
        ] {
            assert!(store_root(value).is_err(), "{value}");
        }
    }
}
