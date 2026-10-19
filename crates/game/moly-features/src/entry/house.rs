//! The player's house data and its tables now live in [`crate::house`], the
//! world layer's owning module; this path keeps the entry's own references
//! and the older `crate::entry::house` spelling working.

pub use crate::house::*;
