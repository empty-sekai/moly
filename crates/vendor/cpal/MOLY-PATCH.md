CPAL 0.17.1, from the crates.io archive (Apache-2.0; see LICENSE).
Upstream git commit: ddad8bc0a085fb88d1ff897c760c00425535d145.
Archive SHA-256: 5b1f9c7312f19fc2fa12fd7acaf38de54e8320ba10d1a02dcbe21038def51ccb.

Local change: `src/host/webaudio/mod.rs` uses `scheduling.rs` to recover an
expired AudioBufferSource deadline before and after decoding. CPAL's existing
25 ms initial lead is reused; valid deadlines, buffer sizes, the mixer,
decoded samples and native backends are unchanged. A delayed callback no
longer schedules multiple buffers into the past.

The whole directory is beneath `crates/`, so the existing build-source
fingerprint includes the dependency and its patch. Cargo uses the root
`[patch.crates-io]` entry. This is a host transport correction, not a game
timeline or sound-resource rule.

Local change: with the `custom` feature, `set_default_output_device`
(`src/default_output.rs`) stores a `CustomDevice` that every host's
`default_output_device()` (the `impl_platform_host!` wrapper in
`src/platform/mod.rs`) returns ahead of its own default. `None` restores the
hosts' defaults. A program uses it to give a library that opens the default
output (rodio, and through it bevy_audio) a device the program renders
itself. Without the feature, or with no device set, the hosts behave as
upstream.
