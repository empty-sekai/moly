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
