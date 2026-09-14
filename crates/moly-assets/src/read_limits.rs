//! Budgets for application-owned transfer and decoded asset buffers.

use async_lock::Semaphore;
use bevy::asset::io::{
    Reader, ReaderNotSeekableError, SeekableReader, StackFuture, STACK_FUTURE_SIZE,
};
use futures_lite::io::AsyncRead;
use std::{
    io,
    pin::Pin,
    sync::{Arc, OnceLock},
    task::{Context, Poll},
};
use tokio::sync::{OwnedSemaphorePermit, Semaphore as ByteSemaphore};

pub(crate) const MAX_DOCUMENT_BYTES: usize = 16 * 1024 * 1024;
pub(crate) const MAX_BLOB_BYTES: usize = 128 * 1024 * 1024;
pub(crate) const MAX_ASSET_BYTES: usize = 256 * 1024 * 1024;
pub(crate) const MAX_IN_FLIGHT_BYTES: usize = 512 * 1024 * 1024;
pub(crate) const MAX_EXPANSION_RATIO: usize = 256;

pub(crate) struct Budget {
    bytes: Arc<ByteSemaphore>,
    limit: usize,
    bytes_per_permit: usize,
    pub(crate) slots: Semaphore,
}

pub(crate) fn shared_budget() -> Arc<Budget> {
    static BUDGET: OnceLock<Arc<Budget>> = OnceLock::new();
    BUDGET
        .get_or_init(|| Arc::new(Budget::new(MAX_IN_FLIGHT_BYTES, 8)))
        .clone()
}

impl Budget {
    pub(crate) fn new(limit: usize, slots: usize) -> Self {
        Self::with_permit_limit(limit, slots, ByteSemaphore::MAX_PERMITS)
    }

    fn with_permit_limit(limit: usize, slots: usize, max_permits: usize) -> Self {
        assert!(limit <= u32::MAX as usize);
        // A byte-sized permit would exceed the semaphore's count on wasm32.
        // Round each request up and the capacity down to keep the byte cap.
        let bytes_per_permit = limit.div_ceil(max_permits).max(1);
        let permits = limit / bytes_per_permit;
        Self {
            bytes: Arc::new(ByteSemaphore::new(permits)),
            limit: permits * bytes_per_permit,
            bytes_per_permit,
            slots: Semaphore::new(slots),
        }
    }

    /// FIFO weighted admission. Dropping a pending acquisition removes it from
    /// the queue; a request that can never fit fails without waiting.
    pub(crate) async fn reserve(self: &Arc<Self>, bytes: usize) -> io::Result<Reservation> {
        if bytes > self.limit {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Asset buffer exceeds the total byte budget",
            ));
        }
        let permit = self
            .bytes
            .clone()
            .acquire_many_owned(bytes.div_ceil(self.bytes_per_permit) as u32)
            .await
            .map_err(|_| io::Error::other("Asset buffer budget is closed"))?;
        Ok(Reservation {
            permit,
            bytes_per_permit: self.bytes_per_permit,
        })
    }
}

pub(crate) struct Reservation {
    permit: OwnedSemaphorePermit,
    bytes_per_permit: usize,
}

impl Reservation {
    pub(crate) fn shrink_to(&mut self, bytes: usize) {
        let released = self
            .permit
            .num_permits()
            .checked_sub(bytes.div_ceil(self.bytes_per_permit))
            .expect("a decoded buffer fits its peak reservation");
        drop(
            self.permit
                .split(released)
                .expect("reservation owns its permits"),
        );
    }
}

pub(crate) struct Buffer {
    pub(crate) bytes: Vec<u8>,
    pub(crate) _reservation: Reservation,
}

/// Forward-only readers release their transfer buffer when consumed or dropped.
/// A loader can then await dependent assets without retaining this budget.
/// Seeking loaders use Bevy's documented VecReader fallback.
pub(crate) struct SharedReader {
    data: Option<Arc<Buffer>>,
    position: usize,
}

impl SharedReader {
    pub(crate) fn new(data: Arc<Buffer>) -> Self {
        Self {
            data: Some(data),
            position: 0,
        }
    }
}

impl AsyncRead for SharedReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        _: &mut Context<'_>,
        buffer: &mut [u8],
    ) -> Poll<io::Result<usize>> {
        let Some(data) = &self.data else {
            return Poll::Ready(Ok(0));
        };
        let length = buffer
            .len()
            .min(data.bytes.len().saturating_sub(self.position));
        let end = self.position + length;
        let consumed = end == data.bytes.len();
        if length != 0 {
            buffer[..length].copy_from_slice(&data.bytes[self.position..end]);
            self.position = end;
        }
        if consumed {
            self.data = None;
        }
        Poll::Ready(Ok(length))
    }
}

impl Reader for SharedReader {
    fn read_to_end<'a>(
        &'a mut self,
        output: &'a mut Vec<u8>,
    ) -> StackFuture<'a, io::Result<usize>, STACK_FUTURE_SIZE> {
        StackFuture::from(async move {
            let Some(data) = self.data.take() else {
                return Ok(0);
            };
            let count = data.bytes.len().saturating_sub(self.position);
            if self.position == 0 && output.is_empty() {
                match Arc::try_unwrap(data) {
                    Ok(Buffer {
                        bytes,
                        _reservation,
                    }) => {
                        *output = bytes;
                        drop(_reservation);
                    }
                    Err(data) => output.extend_from_slice(&data.bytes),
                }
            } else {
                output.extend_from_slice(&data.bytes[self.position..]);
            }
            self.position += count;
            Ok(count)
        })
    }

    fn seekable(&mut self) -> Result<&mut dyn SeekableReader, ReaderNotSeekableError> {
        Err(ReaderNotSeekableError)
    }
}

#[cfg(test)]
mod tests;

pub(crate) fn buffer(capacity: usize) -> io::Result<Vec<u8>> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| io::Error::other("Could not allocate bounded asset buffer"))?;
    Ok(bytes)
}
