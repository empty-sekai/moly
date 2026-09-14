//! Resolve logical asset paths through versioned manifests without preloading a package.

use crate::read_limits::{
    self, Budget, Buffer, SharedReader, MAX_ASSET_BYTES, MAX_BLOB_BYTES, MAX_DOCUMENT_BYTES,
    MAX_EXPANSION_RATIO,
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
struct CatalogDocument {
    schema: String,
    version: String,
    packages: Vec<PackageDocument>,
}

#[derive(Deserialize)]
struct PackageDocument {
    id: String,
    manifest: String,
    dependencies: Vec<String>,
    paths: Vec<String>,
}

struct Package {
    document: PackageDocument,
    loaded: OnceCell<Manifest>,
}

struct Catalog {
    version: String,
    packages: Vec<Package>,
    paths: HashMap<String, usize>,
}

#[derive(Deserialize)]
struct ManifestDocument {
    schema: String,
    version: String,
    blob_prefix: String,
    entries: Vec<Entry>,
}

struct Manifest {
    blob_prefix: String,
    entries: HashMap<String, Entry>,
}

#[derive(Deserialize)]
struct Entry {
    path: String,
    blob: String,
    blob_bytes: usize,
    bytes: usize,
    blob_sha256: String,
    content_sha256: String,
    codec: String,
    http_encoding: String,
    xf: Option<String>,
}

impl Entry {
    fn buffer_peak(&self) -> Result<usize, AssetReaderError> {
        let transfer = self.blob_bytes.checked_add(1);
        let peak = match self.codec.as_str() {
            "identity" if self.blob_bytes == self.bytes => transfer,
            "gzip" => transfer.and_then(|value| value.checked_add(self.bytes)?.checked_add(1)),
            _ => return Err(invalid("Packed representation has incompatible codec or lengths")),
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
    in_flight: Mutex<HashMap<Representation, Weak<InFlight>>>,
    budget: Arc<Budget>,
}

fn invalid(message: impl Into<String>) -> AssetReaderError {
    Error::new(ErrorKind::InvalidData, message.into()).into()
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
            in_flight: Mutex::new(HashMap::new()),
            budget: read_limits::shared_budget(),
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn http(root: String) -> Self {
        Self {
            source: Source::Http(root),
            catalog: OnceCell::new(),
            in_flight: Mutex::new(HashMap::new()),
            budget: read_limits::shared_budget(),
        }
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
        let mut chunk = [0u8; 64 * 1024];
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
                let bytes = self.document_bytes("asset-packs.json").await?;
                let document: CatalogDocument = serde_json::from_slice(&bytes.bytes)
                    .map_err(|e| invalid(format!("asset pack catalog: {e}")))?;
                if document.schema != "moly-asset-packs/1" {
                    return Err(invalid("unsupported asset pack catalog"));
                }
                let ids: BTreeSet<_> = document.packages.iter().map(|p| p.id.as_str()).collect();
                if ids.len() != document.packages.len() {
                    return Err(invalid("duplicate asset package id"));
                }
                for package in &document.packages {
                    for dependency in &package.dependencies {
                        if !ids.contains(dependency.as_str()) {
                            return Err(invalid(format!(
                                "package {} has missing dependency {dependency}",
                                package.id
                            )));
                        }
                    }
                }
                let mut paths = HashMap::new();
                for (index, package) in document.packages.iter().enumerate() {
                    key(Path::new(&package.manifest))?;
                    for path in &package.paths {
                        let path = key(Path::new(path))?;
                        if paths.insert(path, index).is_some() {
                            return Err(invalid("asset belongs to more than one package"));
                        }
                    }
                }
                Ok(Catalog {
                    version: document.version,
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

    async fn manifest<'a>(
        &self,
        package: &'a Package,
        version: &str,
    ) -> Result<&'a Manifest, AssetReaderError> {
        package
            .loaded
            .get_or_try_init(|| async {
                let bytes = self.document_bytes(&package.document.manifest).await?;
                let document: ManifestDocument = serde_json::from_slice(&bytes.bytes)
                    .map_err(|e| invalid(format!("package {}: {e}", package.document.id)))?;
                if document.schema != "moly-asset-manifest/1" || document.version != version {
                    return Err(invalid(format!(
                        "package {} has an incompatible manifest",
                        package.document.id
                    )));
                }
                let mut entries = HashMap::new();
                for entry in document.entries {
                    let path = key(Path::new(&entry.path))?;
                    let expansion_limit = entry.blob_bytes.checked_mul(MAX_EXPANSION_RATIO)
                        .and_then(|value| value.checked_add(64 * 1024))
                        .ok_or_else(|| invalid("Asset expansion limit overflow"))?;
                    if entry.blob_bytes > MAX_BLOB_BYTES || entry.bytes > MAX_ASSET_BYTES || entry.bytes > expansion_limit {
                        return Err(invalid(format!("Packed asset exceeds transfer, decoded-size or expansion budget: {path}")));
                    }
                    if entry.xf.is_some()
                        || entry.http_encoding != "identity"
                        || !matches!(entry.codec.as_str(), "identity" | "gzip")
                    {
                        return Err(invalid(format!(
                            "unsupported packed representation for {path}"
                        )));
                    }
                    key(Path::new(&format!(
                        "{}{}",
                        document.blob_prefix, entry.blob
                    )))?;
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
                Ok(Manifest {
                    blob_prefix: document.blob_prefix,
                    entries,
                })
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
        let manifest = self
            .manifest(&catalog.packages[*index], &catalog.version)
            .await?;
        let entry = manifest
            .entries
            .get(&path_key)
            .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))?;
        let path = format!("{}{}", manifest.blob_prefix, entry.blob);
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
mod tests;
