//! The world seed: one number that every random source of the game draws
//! from, so that a run can be repeated.
//!
//! Without a seed each draw takes fresh bytes from the platform's random
//! source. A seed is given as `MOLY_SEED` (native) or the page's `seed`
//! query parameter (browser), a decimal u64, read once at the first draw;
//! anything else stops the game with a panic that names the value. With a
//! seed, every named [`Stream`] yields a fixed sequence: draw `n` of a stream
//! is SplitMix64 of the seed, the stream and `n`. The streams count their
//! draws separately, so the order in which unrelated systems draw does not
//! change one another's values.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

/// The random sources of the game, one per kind of draw.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Stream {
    /// The NPC generators: objectives, pairing, change-site draws.
    Npc,
    /// The particle seed manager.
    Particle,
    /// The site map's icon order.
    SiteMap,
    /// The cannon's landing draw.
    Landing,
    /// The audio sequence players.
    Audio,
}

const STREAMS: usize = 5;

static SEED: OnceLock<Option<u64>> = OnceLock::new();
static DRAWS: [AtomicU64; STREAMS] = [const { AtomicU64::new(0) }; STREAMS];

/// The world seed, when one was given.
pub(crate) fn seed() -> Option<u64> {
    *SEED.get_or_init(|| {
        let raw = given()?;
        let seed = raw
            .trim()
            .parse::<u64>()
            .unwrap_or_else(|_| panic!("[world-seed] the seed must be a decimal u64, not {raw:?}"));
        bevy::log::info!("[world-seed] seed {seed}: every random stream draws a fixed sequence");
        Some(seed)
    })
}

#[cfg(not(target_arch = "wasm32"))]
fn given() -> Option<String> {
    std::env::var("MOLY_SEED").ok()
}

#[cfg(target_arch = "wasm32")]
fn given() -> Option<String> {
    let search = web_sys::window()?.location().search().ok()?;
    web_sys::UrlSearchParams::new_with_str(&search).ok()?.get("seed")
}

/// Fills `bytes` from `stream`: the seeded sequence when there is a seed,
/// else the platform's random source.
pub(crate) fn fill(stream: Stream, bytes: &mut [u8]) -> Result<(), getrandom::Error> {
    let Some(seed) = seed() else {
        return getrandom::getrandom(bytes);
    };
    for chunk in bytes.chunks_mut(8) {
        let draw = DRAWS[stream as usize].fetch_add(1, Ordering::Relaxed);
        let word = splitmix64(seed ^ splitmix64(stream as u64 + 1) ^ splitmix64(draw.wrapping_add(0x5EED)));
        chunk.copy_from_slice(&word.to_le_bytes()[..chunk.len()]);
    }
    Ok(())
}

/// Eight bytes from `stream`; a platform without a random source is refused
/// with a panic naming `what`.
pub(crate) fn next_u64(stream: Stream, what: &str) -> u64 {
    let mut bytes = [0u8; 8];
    fill(stream, &mut bytes).unwrap_or_else(|error| panic!("{what} needs the platform random source: {error}"));
    u64::from_le_bytes(bytes)
}

/// The draws taken from each stream so far, by stream name.
pub(crate) fn draws() -> [(&'static str, u64); STREAMS] {
    let count = |stream: Stream| DRAWS[stream as usize].load(Ordering::Relaxed);
    [
        ("npc", count(Stream::Npc)),
        ("particle", count(Stream::Particle)),
        ("site_map", count(Stream::SiteMap)),
        ("landing", count(Stream::Landing)),
        ("audio", count(Stream::Audio)),
    ]
}

fn splitmix64(value: u64) -> u64 {
    let mut z = value.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

#[cfg(test)]
mod tests {
    use super::splitmix64;

    #[test]
    fn splitmix64_matches_the_reference_sequence() {
        // The first outputs of the reference SplitMix64 generator started at
        // state 0 (each output is the mix of state + k * golden gamma).
        let mut state = 0u64;
        let mut next = || {
            let out = splitmix64(state);
            state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
            out
        };
        assert_eq!(next(), 0xE220_A839_7B1D_CDAF);
        assert_eq!(next(), 0x6E78_9E6A_A1B9_65F4);
        assert_eq!(next(), 0x06C4_5D18_8009_454F);
    }
}
