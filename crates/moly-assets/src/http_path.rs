//! URL and buffer bounds shared by the browser readers; pure logic is tested
//! on native builds without requiring a browser or replacing fetch globally.
use std::{
    io,
    path::{Component, Path},
};

pub(crate) const MAX_HTTP_READS: usize = 4;

pub(crate) fn canonical_path(path: &Path) -> io::Result<String> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(value) => {
                let value = value
                    .to_str()
                    .ok_or_else(|| io::Error::other("Asset path is not UTF-8"))?;
                if value.contains('\\') || value.chars().any(char::is_control) {
                    return Err(io::Error::other("Asset path has invalid characters"));
                }
                parts.push(value);
            }
            Component::CurDir => {}
            Component::ParentDir if !parts.is_empty() => {
                parts.pop();
            }
            _ => return Err(io::Error::other("Asset path leaves its source")),
        }
    }
    if parts.is_empty() {
        return Err(io::Error::other("Asset path is empty"));
    }
    Ok(parts.join("/"))
}

pub(crate) fn encode_path(path: &str) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut result = String::with_capacity(path.len());
    for byte in path.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.' | b'~' | b'/') {
            result.push(byte as char);
        } else {
            result.push('%');
            result.push(HEX[(byte >> 4) as usize] as char);
            result.push(HEX[(byte & 15) as usize] as char);
        }
    }
    result
}

/// An unencoded Content-Length is an exact stream bound. With transfer
/// compression it is a wire length, so retain the decoded-size limit instead.
pub(crate) fn response_bound(
    length: Option<u64>,
    encoded: bool,
    limit: usize,
) -> io::Result<usize> {
    if length.is_some_and(|size| size > limit as u64) {
        return Err(io::Error::other("Asset response exceeds its byte limit"));
    }
    if encoded {
        return Ok(limit);
    }
    Ok(length.map(|size| size as usize).unwrap_or(limit))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn source_paths_normalize_parents_but_cannot_escape() {
        assert_eq!(
            canonical_path(Path::new("models/../textures/image.png")).unwrap(),
            "textures/image.png"
        );
        for value in ["../image.png", "/image.png", "models/../../image.png", ""] {
            assert!(canonical_path(Path::new(value)).is_err());
        }
    }
    #[test]
    fn reserved_url_characters_and_unicode_remain_filename_bytes() {
        assert_eq!(
            encode_path("textures/音 #1?%.png"),
            "textures/%E9%9F%B3%20%231%3F%25.png"
        );
        assert_eq!(
            encode_path("phenomena/001_sunny/fx/effects.json"),
            "phenomena/001_sunny/fx/effects.json"
        );
    }
    #[test]
    fn small_files_do_not_reserve_the_entire_asset_limit() {
        assert_eq!(
            response_bound(Some(130063), false, 256 * 1024 * 1024).unwrap(),
            130063
        );
        assert_eq!(response_bound(Some(40), true, 100).unwrap(), 100);
        assert_eq!(response_bound(None, false, 100).unwrap(), 100);
        assert!(response_bound(Some(101), false, 100).is_err());
    }
    #[test]
    fn transfer_slots_bound_bursts_and_release_cancelled_waiters() {
        use futures_lite::future::{block_on, poll_once};
        block_on(async {
            let slots = async_lock::Semaphore::new(MAX_HTTP_READS);
            let mut active = Vec::new();
            for _ in 0..MAX_HTTP_READS {
                active.push(slots.acquire().await);
            }
            let mut waiting = Box::pin(slots.acquire());
            assert!(poll_once(&mut waiting).await.is_none());
            drop(waiting);
            drop(active.pop());
            assert!(poll_once(slots.acquire()).await.is_some());
        });
    }
}
