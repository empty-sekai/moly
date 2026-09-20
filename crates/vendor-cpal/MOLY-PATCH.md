CPAL 0.15.3, from the crates.io archive (Apache-2.0; see LICENSE).
Upstream git commit: ac6cbb2ba55e61665a35ab88ae136a83380d1354.
Archive SHA-256: 873dab07c8f743075e57f524c583985fbaf745602acbe916a01539364369a779.

Local change: `src/host/webaudio/mod.rs` uses `scheduling.rs` to recover an
expired AudioBufferSource deadline before and after decoding. CPAL's existing
25 ms initial lead is reused; valid deadlines, 2048-frame buffers, the mixer,
decoded samples and native backends are unchanged. A delayed callback no
longer schedules multiple buffers into the past.

The whole directory is beneath `crates/`, so the existing build-source
fingerprint includes the dependency and its patch. Cargo uses the root
`[patch.crates-io]` entry. This is a host transport correction, not a game
timeline or sound-resource rule.
