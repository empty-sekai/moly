//! The client's clock, re-exported under the server model's module tree.

pub use crate::clock::{
    device_now_ms, device_utc_offset_ms, refresh_window_start, seat_periods, seat_server_date,
    server_now_ms,
};
