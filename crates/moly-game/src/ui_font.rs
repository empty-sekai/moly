//! The UI font: the subset of the open font, embedded once for every text
//! path that reads it.

pub(crate) static UI_FONT: &[u8] = include_bytes!("../assets/font/ResourceHanRoundedSC-Medium.subset.ttf");
