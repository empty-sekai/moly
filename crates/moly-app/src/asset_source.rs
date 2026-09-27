//! 资产根的解析——全树唯一允许的拒绝点。
//! native 读 `MOLY_ASSET_ROOT`；web 没有 env，读页面的 `?assets=` URL 前缀，
//! 浏览器游戏读启动种子里的同名字段，两条路走同一套准入。
//!
//! web 选「URL 参数」不选「同目录约定」：约定无法同步判「缺」——要探测就得
//! 发请求，失败会散成逐资产 404，唯一拒绝点就没了；参数缺了当场拒绝。
//! 前缀是同源绝对路径或明确许可的公共资源 HTTPS 根，且以 `/` 结尾。
//! 玩家数据与页面通信仍在同源 iframe 内，只有公开资源走 CDN。

use moly_assets::AssetSource;

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
/// parsing below is authoritative for host/port/IP canonicalization; this
/// shared admission rejects ambiguous separators and credentials beforehand.
fn public_asset_parts(base: &str) -> Option<(&str, &str)> {
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

fn is_canonical_directory(path: &str) -> bool {
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
/// the same layout.
pub const MASTER_BASE: &str = "https://metadata.pjsk.moe/";
pub const MASTER_MIRROR_BASE: &str = "https://metadata.exmeaning.com/";

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

/// Configured replacements of the four remote bases (each `None` keeps its
/// named default). Which host is tried first is the consumer's rule.
#[derive(Default)]
pub struct RemoteBaseOverrides<'a> {
    pub storage: Option<&'a str>,
    pub storage_mirror: Option<&'a str>,
    pub master: Option<&'a str>,
    pub master_mirror: Option<&'a str>,
}

/// The four remote bases, each defaulting to its named host.
pub fn remote_bases(
    overrides: RemoteBaseOverrides<'_>,
) -> Result<moly_assets::remote::RemoteBases, String> {
    let bases = moly_assets::remote::RemoteBases {
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
    };
    #[cfg(target_arch = "wasm32")]
    for base in [
        &bases.storage,
        &bases.storage_mirror,
        &bases.master,
        &bases.master_mirror,
    ] {
        let url = web_sys::Url::new(base).map_err(|_| format!("Invalid remote base {base}"))?;
        if url.href() != *base || !url.search().is_empty() || !url.hash().is_empty() {
            return Err(format!("remote base {base} must be its canonical HTTPS directory"));
        }
    }
    Ok(bases)
}

/// Native: `MOLY_STORAGE_BASE`, `MOLY_STORAGE_MIRROR_BASE`,
/// `MOLY_MASTER_BASE` and `MOLY_MASTER_MIRROR_BASE` replace the hosts.
#[cfg(not(target_arch = "wasm32"))]
pub fn resolve_remote() -> Result<moly_assets::remote::RemoteBases, String> {
    let read = |name: &str| std::env::var(name).ok();
    let (storage, storage_mirror, master, master_mirror) = (
        read("MOLY_STORAGE_BASE"),
        read("MOLY_STORAGE_MIRROR_BASE"),
        read("MOLY_MASTER_BASE"),
        read("MOLY_MASTER_MIRROR_BASE"),
    );
    remote_bases(RemoteBaseOverrides {
        storage: storage.as_deref(),
        storage_mirror: storage_mirror.as_deref(),
        master: master.as_deref(),
        master_mirror: master_mirror.as_deref(),
    })
}

/// Pages: `?storage_base=`, `?storage_mirror_base=`, `?master_base=` and
/// `?master_mirror_base=` replace the hosts.
#[cfg(target_arch = "wasm32")]
pub fn resolve_remote() -> Result<moly_assets::remote::RemoteBases, String> {
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    let search = window
        .location()
        .search()
        .map_err(|e| format!("could not read the page query string: {e:?}"))?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|e| format!("could not parse the query string {search:?}: {e:?}"))?;
    let (storage, storage_mirror, master, master_mirror) = (
        params.get("storage_base"),
        params.get("storage_mirror_base"),
        params.get("master_base"),
        params.get("master_mirror_base"),
    );
    remote_bases(RemoteBaseOverrides {
        storage: storage.as_deref(),
        storage_mirror: storage_mirror.as_deref(),
        master: master.as_deref(),
        master_mirror: master_mirror.as_deref(),
    })
}

/// The browser game: the seed's `storageBase`, `storageMirrorBase`,
/// `masterBase` and `masterMirrorBase` mean what the page parameters mean.
#[cfg(target_arch = "wasm32")]
pub fn resolve_remote_seed(
    seed: &moly_game::GameSeed,
) -> Result<moly_assets::remote::RemoteBases, String> {
    remote_bases(RemoteBaseOverrides {
        storage: seed.storage_base.as_deref(),
        storage_mirror: seed.storage_mirror_base.as_deref(),
        master: seed.master_base.as_deref(),
        master_mirror: seed.master_mirror_base.as_deref(),
    })
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
            "https://assets-one.example/moly/snapshots/cn-6.0.0-a/assets/",
            "https://cdn-two.example:8443/moly/asset-store/",
            "https://assets.pjsk.moe/sekai-extra-assets/snapshots/cn-6.0.0-a/assets/",
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
}

#[cfg(not(target_arch = "wasm32"))]
pub fn resolve() -> Result<AssetSource, String> {
    let raw = std::env::var("MOLY_ASSET_ROOT").map_err(|_| {
        "MOLY_ASSET_ROOT is not set; point it at the extracted-asset directory".to_owned()
    })?;
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

/// One admission for the page query and the game seed.
#[cfg(target_arch = "wasm32")]
fn resolve_selection(
    base: String,
    packs: bool,
    catalog: Option<String>,
    configured_origin: Option<&str>,
    configured_base: Option<&str>,
) -> Result<AssetSource, String> {
    validate_asset_selection(&base, configured_origin, configured_base)?;
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    if let Some(root) = configured_base {
        let url = web_sys::Url::new(root).map_err(|_| "Invalid resource_base")?;
        if url.href() != root || !url.search().is_empty() || !url.hash().is_empty() {
            return Err("resource_base must be its canonical HTTPS directory".into());
        }
    }
    let location = window.location();
    let page = location.href().map_err(|_| "Could not read page URL")?;
    let url = web_sys::Url::new_with_base(&base, &page).map_err(|_| "Invalid asset URL prefix")?;
    let public_parts = public_asset_parts(&base);
    let allowed_origin = public_parts.map(|(origin, _)| origin.to_owned()).unwrap_or(
        location
            .origin()
            .map_err(|_| "Could not read page origin")?,
    );
    if url.origin() != allowed_origin
        || url.pathname() != public_parts.map(|(_, path)| path).unwrap_or(&base)
        || (public_parts.is_some() && url.href() != base)
        || !url.search().is_empty()
        || !url.hash().is_empty()
    {
        return Err("?assets= must resolve to its canonical trusted resource path".into());
    }
    validate_catalog_id(&catalog)?;
    match (packs, catalog) {
        (true, catalog) => Ok(AssetSource::HttpPacks { url: base, catalog }),
        (false, None) => Ok(AssetSource::HttpBase { url: base }),
        (false, Some(_)) => Err("asset_catalog requires packs=1".into()),
    }
}

#[cfg(target_arch = "wasm32")]
pub fn resolve() -> Result<AssetSource, String> {
    let window =
        web_sys::window().ok_or_else(|| "the wasm build only runs inside a page".to_owned())?;
    let search = window
        .location()
        .search()
        .map_err(|e| format!("could not read the page query string: {e:?}"))?;
    let params = web_sys::UrlSearchParams::new_with_str(&search)
        .map_err(|e| format!("could not parse the query string {search:?}: {e:?}"))?;
    let base = params.get("assets").ok_or_else(|| {
        "missing ?assets=<url-prefix>/ — the HTTP base assets are fetched from".to_owned()
    })?;
    let packs = match params.get("packs").as_deref() {
        Some("1") => true,
        None | Some("0") => false,
        _ => return Err("?packs= must be 0 or 1".into()),
    };
    resolve_selection(
        base,
        packs,
        params.get("asset_catalog"),
        params.get("resource_origin").as_deref(),
        params.get("resource_base").as_deref(),
    )
}

/// The browser game's asset source: the seed's fields mean what the
/// same-named page parameters mean, and pass the same admission.
#[cfg(target_arch = "wasm32")]
pub fn resolve_seed(seed: &moly_game::GameSeed) -> Result<AssetSource, String> {
    resolve_selection(
        seed.assets.clone(),
        seed.packs,
        seed.asset_catalog.clone(),
        seed.resource_origin.as_deref(),
        seed.resource_base.as_deref(),
    )
}
