//! Where the assets come from: the native resource root and the four remote
//! bases the product reads at runtime.
//!
//! The native game and the server resolve the root here, through one
//! function: `MOLY_ASSET_ROOT` when it is set, else `MOLY_ASSET_STORE_URL`,
//! else the resources the launcher installed. A browser page has no
//! environment; its own resolution (the `?assets=` prefix and the game seed)
//! stays in the entry crate and reuses the admission below.
//!
//! Every web path is admitted once, here: a same-origin absolute prefix, or
//! an HTTPS root this module names. `MOLY_ASSET_CATALOG` (a full lowercase
//! SHA-256) selects a release of the content-addressed store, and
//! `MOLY_ASSET_CACHE` names the native directory that keeps what a store
//! read fetched.

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

/// The public asset storage host the product reads song and soundtrack
/// audio from (one directory per region below it); these files are not in
/// the asset root. [`STORAGE_MIRROR_BASE`] serves the same layout.
pub const STORAGE_BASE: &str = "https://storage.pjsk.moe/";
pub const STORAGE_MIRROR_BASE: &str = "https://storage.exmeaning.com/";

/// The public master host the product reads master tables from
/// (`<region>/master/<table>.json` below it). [`MASTER_MIRROR_BASE`] serves
/// the same layout. Both carry the `jp` and `cn` directories only.
pub const MASTER_BASE: &str = "https://metadata.pjsk.moe/";
pub const MASTER_MIRROR_BASE: &str = "https://metadata.exmeaning.com/";

/// The first master host: the repositories of the master tables below
/// `{repo}/main/master/`, one repository per region (a region's repository
/// is [`crate::remote::Region::repository`]). A region whose table the host
/// does not serve is refused by name.
pub const MASTER_HARUKI_BASE: &str = "https://raw.githubusercontent.com/Team-Haruki/{repo}/";

/// One remote base: the named default, or a configured replacement that is a
/// canonical HTTPS directory like `resource_base`.
fn remote_base(name: &str, configured: Option<&str>, default: &str) -> Result<String, String> {
    let Some(base) = configured else {
        return Ok(default.to_owned());
    };
    public_asset_parts(base)
        .filter(|(_, path)| is_canonical_directory(path))
        .map(|_| base.to_owned())
        .ok_or_else(|| format!("{name} must be a canonical HTTPS directory ending in /"))
}

/// A master host whose paths name the region's repository: the configured
/// template must be a canonical HTTPS directory that names `{repo}` exactly
/// once, as a whole path segment.
fn repository_base(name: &str, configured: Option<&str>) -> Result<String, String> {
    let Some(template) = configured else {
        return Ok(MASTER_HARUKI_BASE.to_owned());
    };
    let (_, path) = public_asset_parts(template)
        .filter(|(_, path)| is_canonical_directory(path))
        .ok_or_else(|| {
            format!("{name} must be a canonical HTTPS directory ending in /, with {{repo}} as a path segment")
        })?;
    if path.split('/').filter(|part| *part == "{repo}").count() != 1 {
        return Err(format!(
            "{name} must name {{repo}} exactly once, as a whole path segment"
        ));
    }
    Ok(template.to_owned())
}

/// Configured replacements of the remote bases (each `None` keeps its named
/// default). Which host is tried first is the consumer's rule.
#[derive(Default)]
pub struct RemoteBaseOverrides<'a> {
    pub storage: Option<&'a str>,
    pub storage_mirror: Option<&'a str>,
    pub master: Option<&'a str>,
    pub master_mirror: Option<&'a str>,
    /// A template naming `{repo}` where the repository host's region
    /// repository goes.
    pub master_repository: Option<&'a str>,
}

/// The remote bases, each defaulting to its named host. The browser entry
/// additionally requires each to be its canonical URL.
pub fn remote_bases(
    overrides: RemoteBaseOverrides<'_>,
) -> Result<crate::remote::RemoteBases, String> {
    let master_repository = repository_base("master_haruki_base", overrides.master_repository)?;
    Ok(crate::remote::RemoteBases {
        storage: remote_base("storage_base", overrides.storage, STORAGE_BASE)?,
        storage_mirror: remote_base(
            "storage_mirror_base",
            overrides.storage_mirror,
            STORAGE_MIRROR_BASE,
        )?,
        master: remote_base("master_base", overrides.master, MASTER_BASE)?,
        master_mirror: remote_base(
            "master_mirror_base",
            overrides.master_mirror,
            MASTER_MIRROR_BASE,
        )?,
        master_repository,
    })
}

/// Native: `MOLY_STORAGE_BASE`, `MOLY_STORAGE_MIRROR_BASE`, `MOLY_MASTER_BASE`,
/// `MOLY_MASTER_MIRROR_BASE` and `MOLY_MASTER_HARUKI_BASE` (a template naming
/// `{repo}`) replace the hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_remote() -> Result<crate::remote::RemoteBases, String> {
    let read = |name: &str| std::env::var(name).ok();
    let (storage, storage_mirror, master, master_mirror, master_repository) = (
        read("MOLY_STORAGE_BASE"),
        read("MOLY_STORAGE_MIRROR_BASE"),
        read("MOLY_MASTER_BASE"),
        read("MOLY_MASTER_MIRROR_BASE"),
        read("MOLY_MASTER_HARUKI_BASE"),
    );
    remote_bases(RemoteBaseOverrides {
        storage: storage.as_deref(),
        storage_mirror: storage_mirror.as_deref(),
        master: master.as_deref(),
        master_mirror: master_mirror.as_deref(),
        master_repository: master_repository.as_deref(),
    })
}

/// Native: `MOLY_ASSET_ROOT`, else `MOLY_ASSET_STORE_URL`, else the
/// resources the launcher installed.
///
/// `MOLY_ASSET_ROOT` is the extracted-asset directory. `MOLY_ASSET_STORE_URL`
/// is a public asset store directory over HTTPS (`.../moly/asset-store/`),
/// read from the network as the browser reads it; setting both is an error,
/// because one run reads one source. `MOLY_ASSET_CATALOG` (a full lowercase
/// SHA-256) selects a release of either. `MOLY_ASSET_CACHE` is the directory
/// a store read keeps the immutable documents and blobs it fetched in, and
/// is created when missing; it applies to `MOLY_ASSET_STORE_URL` only.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve() -> Result<AssetSource, String> {
    let root = std::env::var("MOLY_ASSET_ROOT").ok();
    let store = std::env::var("MOLY_ASSET_STORE_URL").ok();
    match (root, store) {
        (Some(_), Some(_)) => Err(
            "MOLY_ASSET_ROOT and MOLY_ASSET_STORE_URL are both set; keep exactly one".into(),
        ),
        (Some(raw), None) => local_root(raw),
        (None, Some(url)) => public_store(url),
        (None, None) => installed_resources(),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn local_root(raw: String) -> Result<AssetSource, String> {
    if raw.trim().is_empty() {
        return Err("MOLY_ASSET_ROOT is empty; point it at the extracted-asset directory".into());
    }
    let path = std::path::PathBuf::from(raw);
    if !path.is_dir() {
        return Err(format!(
            "MOLY_ASSET_ROOT is not a directory: {}",
            path.display()
        ));
    }
    let catalog = std::env::var("MOLY_ASSET_CATALOG").ok();
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
        .ok_or("MOLY_ASSET_STORE_URL must be a canonical HTTPS asset store directory")?;
    let publication = path
        .strip_suffix("asset-store/")
        .map(|prefix| format!("{origin}{prefix}"))
        .ok_or("MOLY_ASSET_STORE_URL must be a canonical HTTPS asset store directory")?;
    validate_asset_selection(url, Some(origin), Some(&publication))
        .map_err(|error| format!("MOLY_ASSET_STORE_URL: {error}"))
}

/// The public asset store as an asset source, admitted by [`store_root`].
/// `MOLY_ASSET_CACHE` names the directory that keeps the immutable files a
/// run fetched; it is created when missing.
#[cfg(not(target_arch = "wasm32"))]
fn public_store(url: String) -> Result<AssetSource, String> {
    if url.trim().is_empty() {
        return Err(
            "MOLY_ASSET_STORE_URL is empty; point it at the public asset store directory".into(),
        );
    }
    store_root(&url)?;
    let catalog = std::env::var("MOLY_ASSET_CATALOG").ok();
    validate_catalog_id(&catalog)?;
    let cache = match std::env::var("MOLY_ASSET_CACHE") {
        Ok(value) if value.trim().is_empty() => {
            return Err("MOLY_ASSET_CACHE is empty; point it at a cache directory".into());
        }
        Ok(value) => {
            let path = std::path::PathBuf::from(value);
            std::fs::create_dir_all(&path).map_err(|error| {
                format!("MOLY_ASSET_CACHE is not usable: {}: {error}", path.display())
            })?;
            Some(path)
        }
        Err(_) => None,
    };
    Ok(AssetSource::HttpPacks {
        url,
        catalog,
        cache,
    })
}

/// Without `MOLY_ASSET_ROOT` or `MOLY_ASSET_STORE_URL`, the resources the
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
             or set MOLY_ASSET_ROOT to a resource directory, \
             or MOLY_ASSET_STORE_URL to a public asset store"
        )
    })?;
    let catalog = std::env::var("MOLY_ASSET_CATALOG").ok().or(Some(installed.catalog));
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
            .contains("/sekai-extra-assets/")
            .then_some("https://assets.pjsk.moe/sekai-extra-assets/");
        validate_asset_selection(value, public_asset_parts(value).map(|(origin, _)| origin), base)
    }

    #[test]
    fn public_immutable_roots_accept_different_configured_https_origins() {
        for value in [
            "/moly/sources/local/",
            "https://assets-one.example/moly/snapshots/cn-1.2.3-a/assets/",
            "https://cdn-two.example:8443/moly/asset-store/",
            "https://assets.pjsk.moe/sekai-extra-assets/snapshots/cn-1.2.3-a/assets/",
            "https://assets.pjsk.moe/sekai-extra-assets/asset-store/",
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
            "https://assets.pjsk.moe/sekai-extra-assets/releases/current/",
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
            "https://assets.pjsk.moe/sekai-extra-assets/asset-store/",
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
