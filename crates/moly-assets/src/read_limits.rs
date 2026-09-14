//! Budgets for application-owned transfer and decoded asset buffers.

use async_lock::Semaphore;
use bevy::asset::io::{Reader, ReaderNotSeekableError, SeekableReader};
use futures_lite::io::{AsyncRead, AsyncSeek};
use std::{
    io::{self, SeekFrom},
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc, OnceLock,
    },
    task::{Context, Poll},
};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_BLOB_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_IN_FLIGHT_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_EXPANSION_RATIO: usize = 256;

pub(crate) struct Budget {
    used: AtomicUsize,
    pub(crate) slots: Semaphore,
}

pub(crate) fn shared_budget() -> Arc<Budget> {
    static BUDGET: OnceLock<Arc<Budget>> = OnceLock::new();
    BUDGET
        .get_or_init(|| {
            Arc::new(Budget {
                used: AtomicUsize::new(0),
                slots: Semaphore::new(8),
            })
        })
        .clone()
}

impl Budget {
    pub(crate) fn reserve(self: &Arc<Self>, bytes: usize) -> io::Result<Reservation> {
        self.used
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |used| {
                used.checked_add(bytes)
                    .filter(|total| *total <= MAX_IN_FLIGHT_BYTES)
            })
            .map_err(|_| {
                io::Error::other(
                    "Asset buffer budget exhausted (512 MiB); retry after pending reads finish",
                )
            })?;
        Ok(Reservation {
            budget: self.clone(),
            bytes,
        })
    }
}

pub(crate) struct Reservation {
    budget: Arc<Budget>,
    bytes: usize,
}

impl Reservation {
    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        assert!(bytes <= self.bytes);
        self.budget
            .used
            .fetch_sub(self.bytes - bytes, Ordering::AcqRel);
        self.bytes = bytes;
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.budget.used.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}

pub(crate) struct Buffer {
    pub(crate) bytes: Vec<u8>,
    pub(crate) _reservation: Reservation,
}

/// Shared buffers stay accounted for until the last reader releases them.
pub(crate) struct SharedReader {
    data: Arc<Buffer>,
    position: usize,
}

impl SharedReader {
    pub(crate) fn new(data: Arc<Buffer>) -> Self {
        Self { data, position: 0 }
    }
}

impl AsyncRead for SharedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let length = buffer
            .len()
            .min(self.data.bytes.len().saturating_sub(self.position));
        if length != 0 {
            buffer[..length]
                .copy_from_slice(&self.data.bytes[self.position..self.position + length]);
            self.position += length;
        }
        Poll::Ready(Ok(length))
    }
}

impl AsyncSeek for SharedReader {
    fn poll_seek(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        position: SeekFrom,
    ) -> Poll<io::Result<u64>> {
        let position = match position {
            SeekFrom::Start(value) => value as i128,
            SeekFrom::End(value) => self.data.bytes.len() as i128 + value as i128,
            SeekFrom::Current(value) => self.position as i128 + value as i128,
        };
        match usize::try_from(position) {
            Ok(position) => {
                self.position = position;
                Poll::Ready(Ok(position as u64))
            }
            Err(_) => Poll::Ready(Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Invalid asset seek",
            ))),
        }
    }
}

impl Reader for SharedReader {
    fn seekable(&mut self) -> Result<&mut dyn SeekableReader, ReaderNotSeekableError> {
        Ok(self)
    }
}

pub(crate) fn buffer(capacity: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| io::Error::other("Could not allocate bounded asset buffer"))?;
    Ok(bytes)
}
