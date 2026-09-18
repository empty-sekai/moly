//! Resolve logical asset paths through versioned manifests without preloading a package.

use crate::read_limits::{
    self, Budget, Buffer, SharedReader, MAX_ASSET_BYTES, MAX_BLOB_BYTES, MAX_DOCUMENT_BYTES,
};
use async_lock::OnceCell;
use bevy::asset::io::{
    AssetReader, AssetReaderError, ErasedAssetReader, PathStream, Reader, VecReader,
};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeSet, HashMap},
    io::{Error, ErrorKind, Read},
    path::{Component, Path, PathBuf},
    sync::{Arc, Mutex, Weak},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogDocument {
    schema: String,
    version: String,
    #[serde(default)]
    region: Option<String>,
    #[serde(default)]
    provenance: Option<serde_json::Value>,
    packages: Vec<PackageDocument>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PackageDocument {
    id: String,
    #[serde(rename = "kind")]
    _kind: String,
    #[serde(rename = "download_bytes")]
    _download_bytes: u64,
    #[serde(rename = "content_bytes")]
    _content_bytes: u64,
    manifest: String,
    dependencies: Vec<String>,
    paths: Vec<String>,
    #[serde(default)]
    content_id: Option<String>,
}

struct Package {
    document: PackageDocument,
    loaded: OnceCell<Manifest>,
}

struct Catalog {
    packages: Vec<Package>,
    paths: HashMap<String, usize>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ManifestDocument {
    schema: String,
    #[serde(rename = "download_bytes")]
    _download_bytes: u64,
    #[serde(rename = "resident_bytes")]
    _resident_bytes: u64,
    #[serde(rename = "content_bytes")]
    _content_bytes: u64,
    #[serde(rename = "logical_bytes")]
    _logical_bytes: u64,
    #[serde(default)]
    content_id: Option<String>,
    #[serde(default)]
    transforms: serde_json::Value,
    #[serde(default)]
    encoders: serde_json::Value,
    entries: Vec<Entry>,
}

struct Manifest {
    entries: HashMap<String, Entry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    path: String,
    blob: String,
    blob_bytes: usize,
    bytes: usize,
    blob_sha256: String,
    content_sha256: String,
    codec: String,
    http_encoding: String,
    #[serde(deserialize_with = "required_transform")]
    xf: Option<String>,
}

fn required_transform<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<Option<String>, D::Error> {
    Option::<String>::deserialize(deserializer)
}

impl Entry {
    fn buffer_peak(&self) -> Result<usize, AssetReaderError> {
        let transfer = self.blob_bytes.checked_add(1);
        let peak = match self.codec.as_str() {
            "identity" if self.blob_bytes == self.bytes => transfer,
            "gzip" => transfer.and_then(|value| value.checked_add(self.bytes)?.checked_add(1)),
            _ => {
                return Err(invalid(
                    "Packed representation has incompatible codec or lengths",
                ))
            }
        };
        peak.ok_or_else(|| invalid("Asset buffer size overflow"))
    }
}

#[derive(Hash, PartialEq, Eq)]
struct Representation {
    path: String,
    codec: String,
    blob_bytes: usize,
    bytes: usize,
    blob_sha256: String,
    content_sha256: String,
}

type InFlight = OnceCell<Arc<Buffer>>;

enum Source {
    Reader(Box<dyn ErasedAssetReader>),
    #[cfg(target_arch = "wasm32")]
    Http(String),
}

pub(crate) struct PackReader {
    source: Source,
    catalog: OnceCell<Catalog>,
    catalog_path: String,
    in_flight: Mutex<HashMap<Representation, Weak<InFlight>>>,
    budget: Arc<Budget>,
}

fn invalid(message: impl Into<String>) -> AssetReaderError {
    Error::new(ErrorKind::InvalidData, message.into()).into()
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn serialized_path(value: &str) -> Result<(), AssetReaderError> {
    if value.is_empty()
        || value
            .bytes()
            .any(|b| b < 32 || b == 127 || matches!(b, b'\\' | b':'))
        || value.split('/').any(|part| matches!(part, "" | "." | ".."))
    {
        return Err(invalid("non-canonical path in packed document"));
    }
    Ok(())
}

fn address<'a>(path: &'a str, directory: &str) -> Result<&'a str, AssetReaderError> {
    let digest = path
        .strip_prefix(&format!("{directory}/"))
        .and_then(|s| s.strip_suffix(".json"))
        .filter(|s| is_sha256(s))
        .ok_or_else(|| invalid("invalid immutable document address"))?;
    Ok(digest)
}

fn check_address(path: &str, directory: &str, bytes: &[u8]) -> Result<(), AssetReaderError> {
    if address(path, directory)? != format!("{:x}", Sha256::digest(bytes)) {
        return Err(invalid(format!(
            "{directory} content address mismatch: {path}"
        )));
    }
    Ok(())
}

// The v2 projection consists of strings, unsigned sizes, null and transform
// data. Canonical key order is explicit even with serde_json/preserve_order.
fn canonical_json(value: &serde_json::Value) -> serde_json::Value {
    match value {
        serde_json::Value::Object(values) => {
            let mut names: Vec<_> = values.keys().collect();
            names.sort();
            serde_json::Value::Object(
                names
                    .into_iter()
                    .map(|key| (key.clone(), canonical_json(&values[key])))
                    .collect(),
            )
        }
        serde_json::Value::Array(values) => {
            serde_json::Value::Array(values.iter().map(canonical_json).collect())
        }
        other => other.clone(),
    }
}

fn interoperable_value(value: &serde_json::Value) -> bool {
    match value {
        serde_json::Value::Number(number) => {
            number.as_u64().is_some_and(|n| n <= 9_007_199_254_740_991)
        }
        serde_json::Value::Array(values) => values.iter().all(interoperable_value),
        serde_json::Value::Object(values) => values.values().all(interoperable_value),
        _ => true,
    }
}

fn check_content_identity(
    document: &ManifestDocument,
    expected: &str,
) -> Result<(), AssetReaderError> {
    if !interoperable_value(&document.transforms) {
        return Err(invalid(
            "transform identity requires nonnegative interoperable integer parameters",
        ));
    }
    let mut entries: Vec<_> = document.entries.iter().collect();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    let projection = serde_json::json!({"schema":"moly-asset-content/1", "transforms":document.transforms,
        "entries":entries.into_iter().map(|e| serde_json::json!({"path":e.path,"bytes":e.bytes,
            "content_sha256":e.content_sha256,"xf":e.xf})).collect::<Vec<_>>()});
    let mut bytes =
        serde_json::to_vec(&canonical_json(&projection)).map_err(|e| invalid(e.to_string()))?;
    bytes.push(b'\n');
    if !is_sha256(expected) || format!("{:x}", Sha256::digest(&bytes)) != expected {
        return Err(invalid("decoded package content identity mismatch"));
    }
    Ok(())
}

fn key(path: &Path) -> Result<String, AssetReaderError> {
    let mut parts = Vec::new();
    for part in path.components() {
        match part {
            Component::Normal(value) => parts.push(
                value
                    .to_str()
                    .ok_or_else(|| invalid("asset path is not UTF-8"))?,
            ),
            Component::CurDir => {}
            Component::ParentDir if !parts.is_empty() => {
                parts.pop();
            }
            _ => return Err(invalid("asset path leaves its source")),
        }
    }
    Ok(parts.join("/"))
}

impl PackReader {
    pub(crate) fn new(inner: Box<dyn ErasedAssetReader>) -> Self {
        Self {
            source: Source::Reader(inner),
            catalog: OnceCell::new(),
            catalog_path: "asset-packs.json".into(),
            in_flight: Mutex::new(HashMap::new()),
            budget: read_limits::shared_budget(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn http(root: String) -> Self {
        Self {
            source: Source::Http(root),
            catalog: OnceCell::new(),
            catalog_path: "asset-packs.json".into(),
            in_flight: Mutex::new(HashMap::new()),
            budget: read_limits::shared_budget(),
        }
    }

    /// Select a pinned release under the SAME store; never embed this address
    /// into a moly:// logical path. Validation occurs before the first I/O.
    pub(crate) fn with_catalog(mut self, catalog: Option<String>) -> Self {
        self.catalog_path = catalog
            .map(|id| format!("catalogs/{id}.json"))
            .unwrap_or_else(|| "asset-packs.json".into());
        self
    }

    async fn bytes(&self, path: &str, limit: usize) -> Result<Vec<u8>, AssetReaderError> {
        let path = key(Path::new(path))?;
        let inner = match &self.source {
            Source::Reader(inner) => inner,
            #[cfg(target_arch = "wasm32")]
            Source::Http(root) => return crate::http::read_bytes(root, &path, limit).await,
        };
        let mut reader = inner.read(Path::new(&path)).await?;
        let mut bytes = read_limits::buffer(
            limit
                .checked_add(1)
                .ok_or_else(|| invalid("Asset byte limit overflow"))?,
        )?;
        // This scratch buffer crosses await points. Keeping 64 KiB inline
        // multiplies every containing async state and overflows the default
        // WASM stack before the first request in unoptimized builds.
        let mut chunk = read_limits::buffer(64 * 1024)?;
        chunk.resize(64 * 1024, 0);
        loop {
            let remaining = (limit + 1 - bytes.len()).min(chunk.len());
            let count =
                futures_lite::AsyncReadExt::read(&mut reader, &mut chunk[..remaining]).await?;
            if count == 0 {
                break;
            }
            bytes.extend_from_slice(&chunk[..count]);
            if bytes.len() > limit {
                return Err(invalid(format!("Asset exceeds its byte limit: {path}")));
            }
        }
        Ok(bytes)
    }

    async fn document_bytes(&self, path: &str) -> Result<Buffer, AssetReaderError> {
        let _slot = self.budget.slots.acquire().await;
        let reservation = self.budget.reserve(MAX_DOCUMENT_BYTES + 1).await?;
        Ok(Buffer {
            bytes: self.bytes(path, MAX_DOCUMENT_BYTES).await?,
            _reservation: reservation,
        })
    }

    async fn catalog(&self) -> Result<&Catalog, AssetReaderError> {
        self.catalog
            .get_or_try_init(|| async {
                let pinned = self.catalog_path != "asset-packs.json";
                if pinned {
                    address(&self.catalog_path, "catalogs")?;
                }
                let bytes = self.document_bytes(&self.catalog_path).await?;
                if pinned {
                    check_address(&self.catalog_path, "catalogs", &bytes.bytes)?;
                }
                let document: CatalogDocument = serde_json::from_slice(&bytes.bytes)
                    .map_err(|e| invalid(format!("asset pack catalog: {e}")))?;
                if document.schema != "moly-asset-packs/2"
                    || document.version.is_empty()
                    || document.packages.is_empty()
                {
                    return Err(invalid(
                        "requires a nonempty v2 catalog; migrate legacy packs with pack.migrate",
                    ));
                }
                if !document.region.as_deref().is_some_and(|r| {
                    r.len() >= 2
                        && r.len() <= 16
                        && r.as_bytes()[0].is_ascii_lowercase()
                        && r.bytes()
                            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
                }) || !document.provenance.as_ref().is_some_and(|p| p.is_object())
                {
                    return Err(invalid("v2 catalog lacks release region/provenance"));
                }
                let ids: HashMap<_, _> = document
                    .packages
                    .iter()
                    .enumerate()
                    .map(|(i, p)| (p.id.as_str(), i))
                    .collect();
                if ids.len() != document.packages.len() {
                    return Err(invalid("duplicate asset package id"));
                }
                // Iterative Kahn walk: cyclic or very deep graphs cannot recurse
                // until the browser/native stack overflows.
                let mut incoming = vec![0usize; ids.len()];
                let mut outgoing = vec![Vec::new(); ids.len()];
                for (index, package) in document.packages.iter().enumerate() {
                    serialized_path(&package.id)?;
                    if package._kind.is_empty() {
                        return Err(invalid("empty package kind"));
                    }
                    serialized_path(&package.manifest)?;
                    address(&package.manifest, "packages")?;
                    if !package.content_id.as_deref().is_some_and(is_sha256) {
                        return Err(invalid("v2 catalog lacks package content identity"));
                    }
                    if package.paths.is_empty() {
                        return Err(invalid("empty asset package"));
                    }
                    let mut unique = BTreeSet::new();
                    for dependency in &package.dependencies {
                        let Some(&dependency_index) = ids.get(dependency.as_str()) else {
                            return Err(invalid(format!(
                                "package {} has missing dependency {dependency}",
                                package.id
                            )));
                        };
                        if !unique.insert(dependency) {
                            return Err(invalid("duplicate package dependency"));
                        }
                        incoming[index] += 1;
                        outgoing[dependency_index].push(index);
                    }
                }
                let mut ready: Vec<_> = incoming
                    .iter()
                    .enumerate()
                    .filter_map(|(i, &n)| (n == 0).then_some(i))
                    .collect();
                let mut visited = 0;
                while let Some(index) = ready.pop() {
                    visited += 1;
                    for &child in &outgoing[index] {
                        incoming[child] -= 1;
                        if incoming[child] == 0 {
                            ready.push(child);
                        }
                    }
                }
                if visited != ids.len() {
                    return Err(invalid("asset package dependency cycle"));
                }
                let mut paths = HashMap::new();
                for (index, package) in document.packages.iter().enumerate() {
                    key(Path::new(&package.manifest))?;
                    for path in &package.paths {
                        serialized_path(path)?;
                        let path = key(Path::new(path))?;
                        if paths.insert(path, index).is_some() {
                            return Err(invalid("asset belongs to more than one package"));
                        }
                    }
                }
                Ok(Catalog {
                    packages: document
                        .packages
                        .into_iter()
                        .map(|document| Package {
                            document,
                            loaded: OnceCell::new(),
                        })
                        .collect(),
                    paths,
                })
            })
            .await
    }

    async fn manifest<'a>(&self, package: &'a Package) -> Result<&'a Manifest, AssetReaderError> {
        package
            .loaded
            .get_or_try_init(|| async {
                let bytes = self.document_bytes(&package.document.manifest).await?;
                check_address(&package.document.manifest, "packages", &bytes.bytes)?;
                let document: ManifestDocument = serde_json::from_slice(&bytes.bytes)
                    .map_err(|e| invalid(format!("package {}: {e}", package.document.id)))?;
                if document.schema != "moly-asset-manifest/2"
                    || document.content_id != package.document.content_id
                    || !document.encoders.is_object()
                    || !document
                        .transforms
                        .as_object()
                        .is_some_and(|recipes| recipes.is_empty())
                    || document.entries.windows(2).any(|e| e[0].path >= e[1].path)
                {
                    return Err(invalid("incompatible v2 package manifest"));
                }
                check_content_identity(&document, document.content_id.as_deref().unwrap_or(""))?;
                let mut entries = HashMap::new();
                for entry in document.entries {
                    serialized_path(&entry.path)?;
                    serialized_path(&entry.blob)?;
                    if !is_sha256(&entry.blob_sha256) || !is_sha256(&entry.content_sha256) {
                        return Err(invalid("invalid packed checksum"));
                    }
                    let suffix = if entry.codec == "gzip" { "gzz" } else { "bin" };
                    let expected = format!(
                        "{}/{}.{}",
                        &entry.blob_sha256[..2],
                        entry.blob_sha256,
                        suffix
                    );
                    if entry.blob != expected {
                        return Err(invalid("blob address differs from its checksum/codec"));
                    }
                    let path = key(Path::new(&entry.path))?;
                    // Authenticated exact lengths plus bounded decoding cap memory.
                    // A compression-ratio heuristic rejects valid compressible
                    // artifacts and overflows usize for 16 MiB blobs on wasm32.
                    if entry.blob_bytes > MAX_BLOB_BYTES || entry.bytes > MAX_ASSET_BYTES {
                        return Err(invalid(format!(
                            "Packed asset exceeds transfer or decoded-size budget: {path}"
                        )));
                    }
                    if entry.xf.is_some()
                        || entry.http_encoding != "identity"
                        || !matches!(entry.codec.as_str(), "identity" | "gzip")
                    {
                        return Err(invalid(format!(
                            "unsupported packed representation for {path}"
                        )));
                    }
                    key(Path::new(&format!("blobs/{}", entry.blob)))?;
                    if entries.insert(path, entry).is_some() {
                        return Err(invalid("duplicate path in package manifest"));
                    }
                }
                if entries.len() != package.document.paths.len()
                    || package
                        .document
                        .paths
                        .iter()
                        .any(|path| !entries.contains_key(path))
                {
                    return Err(invalid(format!(
                        "package {} differs from its catalog",
                        package.document.id
                    )));
                }
                bevy::log::info!(
                    "asset package {}: manifest ready; payloads load on demand",
                    package.document.id
                );
                Ok(Manifest { entries })
            })
            .await
    }

    async fn payload(&self, path: &str, entry: &Entry) -> Result<Arc<Buffer>, AssetReaderError> {
        let identity = Representation {
            path: key(Path::new(path))?,
            codec: entry.codec.clone(),
            blob_bytes: entry.blob_bytes,
            bytes: entry.bytes,
            blob_sha256: entry.blob_sha256.clone(),
            content_sha256: entry.content_sha256.clone(),
        };
        let flight = {
            let mut flights = self
                .in_flight
                .lock()
                .map_err(|_| invalid("asset request state unavailable"))?;
            if let Some(flight) = flights.get(&identity).and_then(Weak::upgrade) {
                flight
            } else {
                flights.retain(|_, value| value.strong_count() != 0);
                let flight = Arc::new(InFlight::new());
                flights.insert(identity, Arc::downgrade(&flight));
                flight
            }
        };
        let bytes = flight
            .get_or_try_init(|| async {
                let _slot = self.budget.slots.acquire().await;
                let mut reservation = self.budget.reserve(entry.buffer_peak()?).await?;
                let bytes = self.bytes(path, entry.blob_bytes).await?;
                if bytes.len() != entry.blob_bytes
                    || format!("{:x}", Sha256::digest(&bytes)) != entry.blob_sha256
                {
                    return Err(invalid(format!(
                        "packed asset checksum differs: {}",
                        entry.path
                    )));
                }
                let decoded = if entry.codec == "gzip" {
                    let mut decoded = read_limits::buffer(entry.bytes + 1)?;
                    flate2::read::GzDecoder::new(bytes.as_slice())
                        .take(entry.bytes as u64 + 1)
                        .read_to_end(&mut decoded)?;
                    drop(bytes);
                    decoded
                } else {
                    bytes
                };
                if decoded.len() != entry.bytes
                    || format!("{:x}", Sha256::digest(&decoded)) != entry.content_sha256
                {
                    return Err(invalid(format!(
                        "decoded asset checksum differs: {}",
                        entry.path
                    )));
                }
                reservation.shrink_to(decoded.capacity());
                Ok::<Arc<Buffer>, AssetReaderError>(Arc::new(Buffer {
                    bytes: decoded,
                    _reservation: reservation,
                }))
            })
            .await?;
        Ok(bytes.clone())
    }
}

impl AssetReader for PackReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let path_key = key(path)?;
        let catalog = self.catalog().await?;
        let index = catalog
            .paths
            .get(&path_key)
            .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
        let manifest = self.manifest(&catalog.packages[*index]).await?;
        let entry = manifest
            .entries
            .get(&path_key)
            .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
        let path = format!("blobs/{}", entry.blob);
        let bytes = self.payload(&path, entry).await?;
        Ok(SharedReader::new(bytes))
    }

    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }

    async fn read_directory<'a>(
        &'a self,
        path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        let prefix = key(path)?;
        let prefix = if prefix.is_empty() {
            prefix
        } else {
            format!("{prefix}/")
        };
        let mut children = BTreeSet::new();
        for file in self.catalog().await?.paths.keys() {
            if let Some(tail) = file.strip_prefix(&prefix) {
                if let Some(child) = tail.split('/').next() {
                    children.insert(PathBuf::from(format!("{prefix}{child}")));
                }
            }
        }
        Ok(Box::new(futures_lite::stream::iter(children)))
    }

    async fn is_directory<'a>(&'a self, path: &'a Path) -> Result<bool, AssetReaderError> {
        let prefix = key(path)?;
        let prefix = if prefix.is_empty() {
            prefix
        } else {
            format!("{prefix}/")
        };
        Ok(self
            .catalog()
            .await?
            .paths
            .keys()
            .any(|p| p.starts_with(&prefix)))
    }
}

#[cfg(test)]
#[path = "packs/tests.rs"]
mod tests;
