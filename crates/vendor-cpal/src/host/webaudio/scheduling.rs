// Host transport recovery only. This leaves the samples, source cursor and
// valid queued deadlines unchanged. The lead is CPAL 0.15.3's existing initial
// 25 ms allowance, reused only when there is no future buffer deadline.
pub(super) fn next_buffer_time(deadline: f64, now: f64) -> f64 {
    if deadline > now {
        deadline
    } else {
        now + 0.025
    }
}

#[cfg(test)]
mod tests {
    use super::next_buffer_time;

    #[test]
    fn uninterrupted_buffers_preserve_their_sample_clock() {
        assert_eq!(next_buffer_time(12.046_439_909, 12.01), 12.046_439_909);
    }

    #[test]
    fn cold_start_and_expired_deadlines_share_the_existing_startup_lead() {
        assert_eq!(next_buffer_time(0.0, 0.0), 0.025);
        assert_eq!(next_buffer_time(1.0, 17.5), 17.525);
        assert_eq!(next_buffer_time(17.5, 17.5), 17.525);
    }

    #[test]
    fn two_late_callbacks_resume_in_order_without_overlap() {
        let duration = 2048.0 / 44_100.0;
        let first = next_buffer_time(1.0, 17.5);
        let second = next_buffer_time(first + duration, 17.501);
        assert_eq!(second, first + duration);
        assert!(second > first);
    }

    #[test]
    fn decoder_work_that_misses_a_deadline_is_recovered_before_start() {
        let before_decode = next_buffer_time(0.0, 1.0);
        let after_decode = next_buffer_time(before_decode, 1.1);
        assert_eq!(after_decode, 1.125);
    }
}
