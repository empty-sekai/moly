//! Entry adapters around the shared source admission.
pub use moly_assets::browser_source::*;
#[cfg(target_arch = "wasm32")]
use moly_assets::AssetSource;

/// The browser game: the seed's `storageBase`, `storageMirrorBase`,
/// `masterBase` and `masterMirrorBase` mean what the page parameters mean.
#[cfg(target_arch = "wasm32")]
pub fn resolve_remote_seed(
    seed: &moly_game::GameSeed,
) -> Result<moly_assets::remote::RemoteBases, String> {
    let bases = moly_assets::asset_source::remote_bases(RemoteBaseOverrides {
        storage: seed.storage_base.as_deref(),
        storage_mirror: seed.storage_mirror_base.as_deref(),
        master: seed.master_base.as_deref(),
        master_mirror: seed.master_mirror_base.as_deref(),
        // The seed names no master repository: the built-in one.
        master_repository: None,
    })?;
    canonical_bases(bases)
}

/// The browser game's asset source: the seed's fields mean what the
/// same-named page parameters mean, and pass the same admission.
#[cfg(target_arch = "wasm32")]
pub fn resolve_seed(seed: &moly_game::GameSeed) -> Result<AssetSource, String> {
    resolve_selection(
        seed.assets.clone(),
        seed.packs,
        seed.asset_catalog.clone(),
        seed.resource_origin.as_deref(),
        seed.resource_base.as_deref(),
    )
}
