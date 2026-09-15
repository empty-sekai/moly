//! Bounded loose-file HTTP assets. This reader intentionally has no Bevy
//! .meta files: extracted runtime assets are complete, read-only outputs.
use crate::read_limits::{self, Budget, SharedReader, MAX_ASSET_BYTES};
use bevy::asset::io::{AssetReader, AssetReaderError, PathStream, Reader, VecReader};
use std::{path::Path, sync::Arc};

pub(crate) struct HttpDirectoryReader {
    root: String,
    budget: Arc<Budget>,
    slots: async_lock::Semaphore,
}
impl HttpDirectoryReader {
    pub(crate) fn new(root: String) -> Self {
        Self {
            root,
            budget: read_limits::shared_budget(),
            slots: async_lock::Semaphore::new(crate::http_path::MAX_HTTP_READS),
        }
    }
}
impl AssetReader for HttpDirectoryReader {
    async fn read<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        let path = crate::http_path::canonical_path(path)?;
        // Admission happens before creating the HTTP timeout. Queued assets
        // cannot expire merely because other transfers are using the slots.
        let _slot = self.slots.acquire().await;
        let bytes =
            crate::http::read_budgeted(&self.root, &path, MAX_ASSET_BYTES, &self.budget).await?;
        Ok(SharedReader::new(Arc::new(bytes)))
    }
    async fn read_meta<'a>(&'a self, path: &'a Path) -> Result<impl Reader + 'a, AssetReaderError> {
        Err::<VecReader, _>(AssetReaderError::NotFound(path.to_owned()))
    }
    async fn read_directory<'a>(
        &'a self,
        _path: &'a Path,
    ) -> Result<Box<PathStream>, AssetReaderError> {
        Ok(Box::new(futures_lite::stream::empty()))
    }
    async fn is_directory<'a>(&'a self, _path: &'a Path) -> Result<bool, AssetReaderError> {
        Ok(false)
    }
}
