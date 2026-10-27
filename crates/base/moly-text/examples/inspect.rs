//! Validate caller-provided text documents without starting the game.
use moly_text::{Overlay, SourceCatalog, TextSession};
use std::sync::Arc;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args().skip(1);
    let source_path = args.next().ok_or("usage: inspect SOURCE OVERLAY LOCALE")?;
    let overlay_path = args.next().ok_or("missing overlay path")?;
    let locale = args.next().ok_or("missing locale")?;
    let source = Arc::new(SourceCatalog::parse(&std::fs::read_to_string(source_path)?)?);
    let overlay = Arc::new(Overlay::parse(&std::fs::read_to_string(overlay_path)?)?);
    let session = TextSession::new(source.clone(), &locale, Some(overlay.clone()), None)?;
    println!("source={} text={} overlay={}", source.region, session.locale, overlay.revision);
    Ok(())
}
