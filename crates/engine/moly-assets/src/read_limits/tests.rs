use super::*;
use futures_lite::future::{block_on, poll_once};

#[test]
fn byte_budget_fits_the_wasm32_semaphore_counter() {
    block_on(async {
        let budget = Arc::new(Budget::with_permit_limit(
            MAX_IN_FLIGHT_BYTES,
            8,
            (u32::MAX as usize) >> 3,
        ));
        let held = budget.reserve(MAX_IN_FLIGHT_BYTES).await.unwrap();
        let mut pending = Box::pin(budget.reserve(1));
        assert!(poll_once(&mut pending).await.is_none());
        drop(held);
        assert!(pending.await.is_ok());
    });
}

#[test]
fn temporary_pressure_waits_and_impossible_requests_fail() {
    block_on(async {
        let budget = Arc::new(Budget::new(10, 8));
        let held = budget.reserve(10).await.unwrap();
        let mut pending = Box::pin(budget.reserve(1));
        assert!(poll_once(&mut pending).await.is_none());
        assert!(budget.reserve(11).await.is_err());
        drop(held);
        let acquired = pending.await.unwrap();
        assert_eq!(budget.bytes.available_permits(), 9);
        drop(acquired);
        assert_eq!(budget.bytes.available_permits(), 10);
    });
}

#[test]
fn weighted_waiters_are_fair_and_cancellation_releases_the_queue() {
    block_on(async {
        let budget = Arc::new(Budget::new(10, 8));
        let held = budget.reserve(8).await.unwrap();
        let mut large = Box::pin(budget.reserve(3));
        let mut small = Box::pin(budget.reserve(2));
        assert!(poll_once(&mut large).await.is_none());
        assert!(poll_once(&mut small).await.is_none());
        drop(large);
        let small = small.await.unwrap();
        drop(small);
        let mut cancelled_after_wake = Box::pin(budget.reserve(10));
        assert!(poll_once(&mut cancelled_after_wake).await.is_none());
        drop(held);
        drop(cancelled_after_wake);
        assert_eq!(budget.bytes.available_permits(), 10);
    });
}

#[test]
fn shrinking_a_peak_wakes_waiters() {
    block_on(async {
        let budget = Arc::new(Budget::new(10, 8));
        let mut held = budget.reserve(10).await.unwrap();
        let mut pending = Box::pin(budget.reserve(6));
        assert!(poll_once(&mut pending).await.is_none());
        held.shrink_to(4);
        let acquired = pending.await.unwrap();
        assert_eq!(budget.bytes.available_permits(), 0);
        drop((acquired, held));
        assert_eq!(budget.bytes.available_permits(), 10);
    });
}

#[test]
fn consumed_reader_releases_budget_before_loading_a_dependency() {
    block_on(async {
        let budget = Arc::new(Budget::new(4, 8));
        let bytes = vec![1, 2, 3, 4];
        let pointer = bytes.as_ptr();
        let data = Arc::new(Buffer {
            bytes,
            _reservation: budget.reserve(4).await.unwrap(),
        });
        let mut reader = SharedReader::new(data);
        assert!(reader.seekable().is_err());
        let mut output = Vec::new();
        assert_eq!(reader.read_to_end(&mut output).await.unwrap(), 4);
        assert_eq!(output, [1, 2, 3, 4]);
        assert_eq!(output.as_ptr(), pointer);
        assert_eq!(budget.bytes.available_permits(), 4);
        let dependency = budget.reserve(4).await.unwrap();
        assert_eq!(reader.read_to_end(&mut output).await.unwrap(), 0);
        drop(dependency);
    });
}

#[test]
fn shared_buffers_are_released_after_the_last_consumer() {
    block_on(async {
        let budget = Arc::new(Budget::new(4, 8));
        let data = Arc::new(Buffer {
            bytes: vec![1, 2, 3, 4],
            _reservation: budget.reserve(4).await.unwrap(),
        });
        let mut first = SharedReader::new(data.clone());
        let mut second = SharedReader::new(data);
        let mut bytes = [0; 4];
        futures_lite::AsyncReadExt::read_exact(&mut first, &mut bytes)
            .await
            .unwrap();
        assert_eq!(bytes, [1, 2, 3, 4]);
        assert_eq!(budget.bytes.available_permits(), 0);
        let mut output = vec![9];
        second.read_to_end(&mut output).await.unwrap();
        assert_eq!(output, [9, 1, 2, 3, 4]);
        assert_eq!(budget.bytes.available_permits(), 4);
    });
}
