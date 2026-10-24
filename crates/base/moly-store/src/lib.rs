//! The local resource store.
//!
//! A CDN publishes `game/manifest.json` ([`publication`]), which lists the
//! resource snapshots it serves; each snapshot names a content-addressed
//! catalog in an asset store (`catalogs/<sha256>.json`, `packages/`,
//! `blobs/`). [`install`] mirrors one catalog into a local directory with the
//! same layout, verifying every file against its SHA-256, and [`record`]
//! notes what a directory holds. [`location`] fixes where the files live by
//! default, so a game started without configuration reads what the launcher
//! downloaded. [`upgrade`] carries an earlier install's settings and saves over
//! to a new data directory, and [`relocate`] moves a folder tree to another place.
//! [`update`] installs the newest published program release into a versions folder.

pub mod config;
pub mod location;
pub mod publication;
pub mod record;
pub mod store;

#[cfg(not(target_arch = "wasm32"))]
pub mod install;
#[cfg(not(target_arch = "wasm32"))]
pub mod migrate;
#[cfg(not(target_arch = "wasm32"))]
pub mod relocate;
#[cfg(not(target_arch = "wasm32"))]
pub mod tls;
#[cfg(not(target_arch = "wasm32"))]
pub mod update;
#[cfg(not(target_arch = "wasm32"))]
pub mod upgrade;

/// The resource CDN used when none is configured.
pub const DEFAULT_CDN: &str = "https://assets.pjsk.moe/sekai-extra-assets/";

/// A full lowercase SHA-256 in hex.
pub fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// The lowercase hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for byte in digest {
        out.push(char::from_digit((byte >> 4) as u32, 16).unwrap());
        out.push(char::from_digit((byte & 15) as u32, 16).unwrap());
    }
    out
}

/// Check a CDN base: HTTPS (or plain HTTP to a loopback host), no query,
/// fragment or credentials, and a trailing slash.
pub fn check_cdn(base: &str) -> Result<(), String> {
    let rest = base
        .strip_prefix("https://")
        .or_else(|| {
            base.strip_prefix("http://").filter(|rest| {
                ["localhost", "127.0.0.1", "[::1]"]
                    .iter()
                    .any(|host| rest.starts_with(&format!("{host}:")) || rest.starts_with(&format!("{host}/")))
            })
        })
        .ok_or_else(|| format!("CDN address must start with https:// : {base}"))?;
    if !base.ends_with('/') {
        return Err(format!("CDN address must end with '/': {base}"));
    }
    let authority = rest.split('/').next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(format!("CDN address has no plain host: {base}"));
    }
    if base.bytes().any(|b| b <= b' ' || matches!(b, b'?' | b'#' | b'\\')) {
        return Err(format!("CDN address may not hold spaces, '?', '#' or '\\': {base}"));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_hex_matches_the_standard_vector() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(is_sha256(&sha256_hex(b"")));
        assert!(!is_sha256("ABC"));
    }

    #[test]
    fn cdn_addresses_are_https_directories() {
        assert!(check_cdn(DEFAULT_CDN).is_ok());
        assert!(check_cdn("http://127.0.0.1:8080/store/").is_ok());
        for bad in [
            "http://cdn.example/",
            "https://cdn.example",
            "https://user@cdn.example/",
            "https://cdn.example/?a=1/",
            "ftp://cdn.example/",
        ] {
            assert!(check_cdn(bad).is_err(), "{bad}");
        }
    }
}
