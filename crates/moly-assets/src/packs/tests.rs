use super::*;
use futures_lite::future::{block_on, poll_once};
use tokio::sync::Semaphore;

#[derive(Clone)]
struct DelayedReader {
    files: Arc<HashMap<String, Vec<u8>>>,
    gates: Arc<HashMap<String, Semaphore>>,
    opened: Arc<Mutex<Vec<String>>>,
}

impl AssetReader for DelayedReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let key = path.to_string_lossy().to_string();
        self.opened.lock().unwrap().push(key.clone());
        self.gates[&key].acquire().await.unwrap().forget();
        Ok(VecReader::new(self.files[&key].clone()))
    }
    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }
    async fn read_directory<'a>(
        &'a self,
        _: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Ok(Box::new(futures_lite::stream::empty()))
    }
    async fn is_directory<'a>(&'a self, _: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

fn entry(path: &str, bytes: &[u8]) -> Entry {
    let hash = format!("{:x}", Sha256::digest(bytes));
    Entry {
        path: path.into(),
        blob: path.into(),
        blob_bytes: bytes.len(),
        bytes: bytes.len(),
        blob_sha256: hash.clone(),
        content_sha256: hash,
        codec: "identity".into(),
        http_encoding: "identity".into(),
        xf: None,
    }
}

fn delayed(count: usize, budget: usize) -> (PackReader, DelayedReader, Vec<Entry>) {
    let files: HashMap<_, _> = (0..count)
        .map(|i| (i.to_string(), vec![i as u8; 16]))
        .collect();
    let entries = (0..count)
        .map(|i| entry(&i.to_string(), &files[&i.to_string()]))
        .collect();
    let source = DelayedReader {
        gates: Arc::new(
            files
                .keys()
                .map(|key| (key.clone(), Semaphore::new(0)))
                .collect(),
        ),
        files: Arc::new(files),
        opened: Default::default(),
    };
    let mut reader = PackReader::new(Box::new(source.clone()));
    reader.budget = Arc::new(Budget::new(budget, 8));
    (reader, source, entries)
}

async fn consume(reader: &PackReader, entry: &Entry) -> Result<Vec<u8>, AssetReaderError> {
    let data = reader.payload(&entry.path, entry).await?;
    let mut stream = SharedReader::new(data);
    let mut output = Vec::new();
    stream.read_to_end(&mut output).await?;
    Ok(output)
}

#[test]
fn legal_delayed_reads_queue_in_both_completion_orders() {
    for reverse in [false, true] {
        block_on(async {
            let (reader, source, entries) = delayed(6, 34);
            let mut tasks: Vec<_> = entries
                .iter()
                .map(|entry| Some(Box::pin(consume(&reader, entry))))
                .collect();
            for task in &mut tasks {
                assert!(poll_once(task.as_mut().unwrap()).await.is_none());
            }
            assert_eq!(source.opened.lock().unwrap().as_slice(), ["0", "1"]);
            let mut finished: Vec<String> = Vec::new();
            while finished.len() < tasks.len() {
                let opened = source.opened.lock().unwrap().clone();
                let mut active: Vec<_> = opened
                    .iter()
                    .filter(|key| !finished.contains(*key))
                    .cloned()
                    .collect();
                if reverse {
                    active.reverse();
                }
                let key = active
                    .first()
                    .expect("queued reads continue after a buffer is released")
                    .clone();
                source.gates[&key].add_permits(1);
                let mut progressed = false;
                for (index, task) in tasks.iter_mut().enumerate() {
                    let Some(pending) = task else {
                        continue;
                    };
                    if let Some(result) = poll_once(pending).await {
                        assert_eq!(result.unwrap(), vec![index as u8; 16]);
                        finished.push(index.to_string());
                        *task = None;
                        progressed = true;
                    }
                }
                assert!(progressed);
            }
            assert_eq!(source.opened.lock().unwrap().len(), 6);
        });
    }
}

#[test]
fn cancelling_a_transfer_releases_bytes_and_slots() {
    block_on(async {
        let (reader, source, entries) = delayed(2, 17);
        let mut first = Box::pin(consume(&reader, &entries[0]));
        let mut second = Box::pin(consume(&reader, &entries[1]));
        assert!(poll_once(&mut first).await.is_none());
        assert!(poll_once(&mut second).await.is_none());
        assert_eq!(source.opened.lock().unwrap().as_slice(), ["0"]);
        drop(first);
        assert!(poll_once(&mut second).await.is_none());
        assert_eq!(source.opened.lock().unwrap().as_slice(), ["0", "1"]);
        source.gates["1"].add_permits(1);
        assert_eq!(second.await.unwrap(), vec![1; 16]);
    });
}

#[test]
fn oversized_reservation_is_rejected_before_io() {
    block_on(async {
        let (reader, source, entries) = delayed(1, 16);
        assert!(consume(&reader, &entries[0]).await.is_err());
        assert!(source.opened.lock().unwrap().is_empty());
    });
}

#[test]
fn identity_peak_does_not_reserve_a_decompression_buffer() {
    block_on(async {
        let mut item = entry("large", &[]);
        item.blob_bytes = 48 * 1024 * 1024;
        item.bytes = item.blob_bytes;
        assert_eq!(item.buffer_peak().unwrap(), 48 * 1024 * 1024 + 1);
        let budget = Arc::new(Budget::new(read_limits::MAX_IN_FLIGHT_BYTES, 8));
        let mut reservations = Vec::new();
        for _ in 0..6 {
            reservations.push(budget.reserve(item.buffer_peak().unwrap()).await.unwrap());
        }
        drop(reservations);
        item.codec = "gzip".into();
        assert_eq!(item.buffer_peak().unwrap(), 96 * 1024 * 1024 + 2);
    });
}

#[derive(Clone)]
struct MemoryStore(Arc<HashMap<String, Vec<u8>>>);
impl AssetReader for MemoryStore {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        self.0
            .get(&path.to_string_lossy().replace('\\', "/"))
            .cloned()
            .map(VecReader::new)
            .ok_or_else(|| AssetReaderError::NotFound(path.to_owned()))
    }
    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }
    async fn read_directory<'a>(
        &'a self,
        _: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Ok(Box::new(futures_lite::stream::empty()))
    }
    async fn is_directory<'a>(&'a self, _: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}

fn document_bytes(value: &serde_json::Value) -> Vec<u8> {
    let mut bytes = serde_json::to_vec(&canonical_json(value)).unwrap();
    bytes.push(b'\n');
    bytes
}
fn digest(value: &[u8]) -> String {
    format!("{:x}", Sha256::digest(value))
}

struct StoreFixture {
    files: HashMap<String, Vec<u8>>,
    catalog_id: String,
    package: String,
    blob: String,
    paths: Vec<String>,
    payload: Vec<u8>,
}
impl StoreFixture {
    fn new(v2: bool, gzip: bool) -> Self {
        let payload = b"representative logical asset payload".to_vec();
        let encoded = if gzip {
            use std::io::Write;
            let mut encoder =
                flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
            encoder.write_all(&payload).unwrap();
            encoder.finish().unwrap()
        } else {
            payload.clone()
        };
        let hash = digest(&encoded);
        let blob = format!(
            "{}/{}.{}",
            &hash[..2],
            hash,
            if gzip { "gzz" } else { "bin" }
        );
        let mut paths: Vec<String> = [
            "fixture-models/test/model.glb",
            "sd_104.glb",
            "site/scenes/site01/site01.glb",
            "ui/action-icon/Talk.png",
            "phenomena/audio/voice.ogg",
            "phenomena/001_sunny/config.json",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        paths.sort();
        let entries: Vec<_> = paths
            .iter()
            .map(|path| {
                serde_json::json!({"path":path,
            "blob":blob,"blob_bytes":encoded.len(),"bytes":payload.len(),"blob_sha256":hash,
            "content_sha256":digest(&payload),"codec":if gzip {"gzip"} else {"identity"},
            "http_encoding":"identity","xf":null})
            })
            .collect();
        let semantic = serde_json::json!({"schema":"moly-asset-content/1","transforms":{},
            "entries": paths.iter().map(|path| serde_json::json!({"path":path,"bytes":payload.len(),
                "content_sha256":digest(&payload),"xf":null})).collect::<Vec<_>>()});
        let content = digest(&document_bytes(&semantic));
        let mut manifest = serde_json::json!({"schema":if v2 {"moly-asset-manifest/2"} else {"moly-asset-manifest/1"},
            "transforms":{},"entries":entries,"download_bytes":encoded.len(),
            "resident_bytes":payload.len(),"content_bytes":payload.len(),"logical_bytes":payload.len()*paths.len()});
        if v2 {
            manifest["content_id"] = content.clone().into();
            manifest["encoders"] = serde_json::json!({});
        } else {
            manifest["version"] = "legacy".into();
            manifest["generated_utc"] = "old-time".into();
            manifest["blob_prefix"] = "blobs/".into();
        }
        let bytes = document_bytes(&manifest);
        let package = format!("packages/{}.json", digest(&bytes));
        let mut row = serde_json::json!({"id":"common/test","kind":"common","manifest":package,"paths":paths,"dependencies":[],
            "download_bytes":encoded.len(),"content_bytes":payload.len()});
        if v2 {
            row["content_id"] = content.into();
        }
        let mut catalog = serde_json::json!({"schema":if v2 {"moly-asset-packs/2"} else {"moly-asset-packs/1"},
            "version":if v2 {"release-unrelated-to-package"} else {"legacy"},"packages":[row]});
        if v2 {
            catalog["region"] = "cn".into();
            catalog["provenance"] = serde_json::json!({"source":"receipt-only"});
        }
        let catalog_bytes = document_bytes(&catalog);
        let catalog_id = digest(&catalog_bytes);
        let blob = format!("blobs/{blob}");
        let files = [
            (package.clone(), bytes),
            (blob.clone(), encoded),
            ("asset-packs.json".into(), catalog_bytes.clone()),
            (format!("catalogs/{catalog_id}.json"), catalog_bytes),
        ]
        .into_iter()
        .collect();
        Self {
            files,
            catalog_id,
            package,
            blob,
            paths,
            payload,
        }
    }
    fn reader(&self, pinned: bool) -> PackReader {
        PackReader::new(Box::new(MemoryStore(Arc::new(self.files.clone()))))
            .with_catalog(pinned.then(|| self.catalog_id.clone()))
    }
}
async fn logical_read(reader: &PackReader, path: &str) -> Result<Vec<u8>, AssetReaderError> {
    let logical = bevy::asset::AssetPath::from(format!("moly://{path}"));
    let mut stream = AssetReader::read(reader, logical.path()).await?;
    let mut bytes = Vec::new();
    stream.read_to_end(&mut bytes).await?;
    Ok(bytes)
}

#[test]
fn v2_resolves_moly_fixture_character_site_ui_audio_weather() {
    for gzip in [false, true] {
        for pinned in [false, true] {
            block_on(async {
                let fixture = StoreFixture::new(true, gzip);
                let reader = fixture.reader(pinned);
                for path in &fixture.paths {
                    assert_eq!(logical_read(&reader, path).await.unwrap(), fixture.payload);
                }
                assert!(AssetReader::is_directory(&reader, Path::new("site/scenes"))
                    .await
                    .unwrap());
                assert!(logical_read(&reader, "does-not-exist").await.is_err());
            });
        }
    }
}

#[test]
fn v2_release_version_and_region_do_not_bind_package() {
    block_on(async {
        let mut fixture = StoreFixture::new(true, false);
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fixture.files["asset-packs.json"]).unwrap();
        catalog["region"] = "jp".into();
        catalog["version"] = "different-release".into();
        fixture
            .files
            .insert("asset-packs.json".into(), document_bytes(&catalog));
        assert_eq!(
            logical_read(&fixture.reader(false), &fixture.paths[0])
                .await
                .unwrap(),
            fixture.payload
        );
    });
}

#[test]
fn corrupt_missing_package_blob_and_catalog_are_errors_not_fallbacks() {
    for damage in [
        "package",
        "blob",
        "catalog",
        "missing-package",
        "missing-blob",
        "missing-catalog",
    ] {
        block_on(async {
            let mut fixture = StoreFixture::new(true, false);
            let path = match damage {
                "package" | "missing-package" => fixture.package.clone(),
                "blob" | "missing-blob" => fixture.blob.clone(),
                _ => format!("catalogs/{}.json", fixture.catalog_id),
            };
            if damage.starts_with("missing") {
                fixture.files.remove(&path);
            } else {
                fixture.files.get_mut(&path).unwrap().push(b' ');
            }
            assert!(
                logical_read(&fixture.reader(true), &fixture.paths[0])
                    .await
                    .is_err(),
                "{damage}"
            );
        });
    }
}

#[test]
fn legacy_catalog_requires_explicit_offline_migration() {
    block_on(async {
        let mut fixture = StoreFixture::new(false, false);
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fixture.files["asset-packs.json"]).unwrap();
        catalog["version"] = "legacy".into();
        fixture
            .files
            .insert("asset-packs.json".into(), document_bytes(&catalog));
        assert!(logical_read(&fixture.reader(false), &fixture.paths[0])
            .await
            .is_err());
    });
}

#[test]
fn invalid_catalog_cycle_duplicate_and_noncanonical_paths_are_rejected() {
    for damage in [
        "cycle",
        "duplicate-dependency",
        "bad-path",
        "bad-manifest",
        "ownership",
    ] {
        block_on(async {
            let mut fixture = StoreFixture::new(true, false);
            let mut catalog: serde_json::Value =
                serde_json::from_slice(&fixture.files["asset-packs.json"]).unwrap();
            match damage {
                "cycle" => {
                    catalog["packages"][0]["dependencies"] = serde_json::json!(["common/test"])
                }
                "duplicate-dependency" => {
                    catalog["packages"][0]["dependencies"] =
                        serde_json::json!(["common/test", "common/test"])
                }
                "bad-path" => catalog["packages"][0]["paths"][0] = "site/../outside".into(),
                "bad-manifest" => {
                    catalog["packages"][0]["manifest"] = "packages/not-a-hash.json".into()
                }
                _ => {
                    let second = catalog["packages"][0].clone();
                    catalog["packages"].as_array_mut().unwrap().push(second);
                }
            }
            fixture
                .files
                .insert("asset-packs.json".into(), document_bytes(&catalog));
            assert!(
                logical_read(&fixture.reader(false), &fixture.paths[0])
                    .await
                    .is_err(),
                "{damage}"
            );
        });
    }
}

#[test]
fn semantic_identity_is_checked_independently_of_representation_address() {
    block_on(async {
        let mut fixture = StoreFixture::new(true, false);
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fixture.files[&fixture.package]).unwrap();
        manifest["entries"][0]["content_sha256"] = "0".repeat(64).into();
        let raw = document_bytes(&manifest);
        let package = format!("packages/{}.json", digest(&raw));
        fixture.files.insert(package.clone(), raw);
        let mut catalog: serde_json::Value =
            serde_json::from_slice(&fixture.files["asset-packs.json"]).unwrap();
        catalog["packages"][0]["manifest"] = package.into();
        fixture
            .files
            .insert("asset-packs.json".into(), document_bytes(&catalog));
        assert!(logical_read(&fixture.reader(false), &fixture.paths[0])
            .await
            .is_err());
    });
}

#[test]
fn catalog_and_asset_future_frames_do_not_embed_transfer_scratch() {
    // A scratch array nested in each async state overflowed the default WASM
    // stack before the first HTTP request. Browser conformance also runs the
    // debug WASM build with default linker settings.
    let fixture = StoreFixture::new(true, false);
    let reader = fixture.reader(true);
    assert!(std::mem::size_of_val(&reader.catalog()) < 16 * 1024);
    assert!(
        std::mem::size_of_val(&AssetReader::read(&reader, Path::new(&fixture.paths[0])))
            < 32 * 1024
    );
}

#[test]
fn release_fields_cannot_enter_v2_package_even_with_a_valid_address() {
    for field in [
        "version",
        "generated_utc",
        "region",
        "provenance",
        "blob_prefix",
    ] {
        block_on(async {
            let mut fixture = StoreFixture::new(true, false);
            let mut manifest: serde_json::Value =
                serde_json::from_slice(&fixture.files[&fixture.package]).unwrap();
            manifest[field] = serde_json::Value::Null;
            let raw = document_bytes(&manifest);
            let package = format!("packages/{}.json", digest(&raw));
            fixture.files.insert(package.clone(), raw);
            let mut catalog: serde_json::Value =
                serde_json::from_slice(&fixture.files["asset-packs.json"]).unwrap();
            catalog["packages"][0]["manifest"] = package.into();
            fixture
                .files
                .insert("asset-packs.json".into(), document_bytes(&catalog));
            assert!(
                logical_read(&fixture.reader(false), &fixture.paths[0])
                    .await
                    .is_err(),
                "{field}"
            );
        });
    }
}

#[test]
fn transform_field_is_required_even_for_identity_representation() {
    let fixture = StoreFixture::new(true, false);
    let manifest: serde_json::Value =
        serde_json::from_slice(&fixture.files[&fixture.package]).unwrap();
    let mut entry = manifest["entries"][0].clone();
    entry.as_object_mut().unwrap().remove("xf");
    assert!(serde_json::from_value::<Entry>(entry).is_err());
}
