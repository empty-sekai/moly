//! Process-owned seed manager for explicit, qualified particle reset events.
//!
//! The resource outlives weather live/retired collections. Creating, retiring or
//! resuming a collection must not replace this manager. The caller supplies an
//! actual first-reset/reset event; this module does not infer Play/resume rules.
//! OS entropy initializes the shared scalar manager once. Each automatic reset
//! consumes one manager word, then the law expands independent module streams.

use bevy::prelude::Resource;
use moly_law::particle::seed_owner::{
    ModuleRandom, ParticleSeedManager, ResetStreams, ScalarRandom, SeedOwner,
};

#[derive(Debug)]
pub(crate) enum SeedError {
    MissingRandomSeed,
    MissingAutoRandomSeed,
    EntropyUnavailable(getrandom::Error),
}

impl std::fmt::Display for SeedError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MissingRandomSeed => f.write_str("source randomSeed is unknown"),
            Self::MissingAutoRandomSeed => f.write_str("source autoRandomSeed is unknown"),
            Self::EntropyUnavailable(error) => {
                write!(f, "particle seed entropy unavailable: {error}")
            }
        }
    }
}

impl std::error::Error for SeedError {}

/// One shared resource. Default is deliberately lazy: manual source seeds do
/// not require OS entropy. Failed entropy reads leave the resource uninitialized
/// and the owner unchanged; they never substitute a fixed seed or frame counter.
#[derive(Resource, Default)]
pub(crate) struct SystemSeedManager {
    manager: Option<ParticleSeedManager>,
}

impl SystemSeedManager {
    #[cfg(test)]
    pub(crate) fn manager_words_for_test(&self) -> [u32; 4] {
        self.manager.map(|manager| manager.words()).unwrap_or([0; 4])
    }
    /// Captured original manager entropy for deterministic replay. These are
    /// manager-state words, not a source system's serialized randomSeed.
    pub(crate) fn from_entropy_words(words: [u32; 4]) -> Self {
        Self {
            manager: Some(ParticleSeedManager::from_entropy_words(words)),
        }
    }

    /// Idempotently initialize from 16 OS entropy bytes. getrandom's native OS
    /// backend and wasm JS crypto backend share this fallible interface.
    pub(crate) fn try_init(&mut self) -> Result<(), SeedError> {
        self.try_init_with(|| {
            let mut bytes = [0_u8; 16];
            getrandom::getrandom(&mut bytes)?;
            Ok(bytes)
        })
    }

    fn try_init_with(
        &mut self,
        entropy: impl FnOnce() -> Result<[u8; 16], getrandom::Error>,
    ) -> Result<(), SeedError> {
        if self.manager.is_none() {
            let bytes = entropy().map_err(SeedError::EntropyUnavailable)?;
            // Current source ARM64 and wasm/native targets use little-endian
            // words; do not reinterpret bytes through alignment-sensitive casts.
            let words = std::array::from_fn(|word| {
                u32::from_le_bytes(bytes[word * 4..word * 4 + 4].try_into().unwrap())
            });
            self.manager = Some(ParticleSeedManager::from_entropy_words(words));
        }
        Ok(())
    }

    /// Construct and perform the first explicit reset. Missing fields are
    /// rejected before any entropy request or shared-manager consumption.
    pub(crate) fn create_owner(
        &mut self,
        random_seed: Option<u32>,
        automatic: Option<bool>,
    ) -> Result<(SeedOwner, ResetStreams), SeedError> {
        let random_seed = random_seed.ok_or(SeedError::MissingRandomSeed)?;
        let automatic = automatic.ok_or(SeedError::MissingAutoRandomSeed)?;
        let mut owner = SeedOwner::from_serialized(random_seed, automatic);
        let streams = self.reset_owner(&mut owner)?;
        Ok((owner, streams))
    }

    /// Only call for a proven ResetSeeds lifecycle event. Toggling auto or
    /// resuming playback alone does not authorize a reset. On entropy failure,
    /// neither the owner nor module streams have been changed.
    pub(crate) fn reset_owner(&mut self, owner: &mut SeedOwner) -> Result<ResetStreams, SeedError> {
        if !owner.automatic {
            // Manual ResetSeeds expands the serialized owner directly and has
            // no global-manager dependency or draw. The layout is the law's
            // reset layout, with each module receiving independent storage.
            let module = ModuleRandom::from_owner_seed(owner.seed);
            return Ok(ResetStreams {
                scalar_birth: ScalarRandom::from_seed(owner.seed),
                initial: module,
                shape: module,
                collision: module,
                force: module,
                lights: ScalarRandom::from_seed(owner.seed),
                noise_scroll: 0.0,
            });
        }
        self.try_init()?;
        // SeedOwner::reset advances exactly once for an automatic owner.
        Ok(owner.reset(
            self.manager
                .as_mut()
                .expect("successful entropy initialization"),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENTROPY: [u32; 4] = [17, 19, 127, 2_471_805_022];

    #[test]
    fn first_automatic_reset_consumes_one_shared_word() {
        let mut manager = SystemSeedManager::from_entropy_words(ENTROPY);
        let mut expected = ParticleSeedManager::from_entropy_words(ENTROPY);
        let seed = expected.next_system_seed();
        let (owner, streams) = manager.create_owner(Some(0), Some(true)).unwrap();
        assert_eq!(owner, SeedOwner::from_serialized(seed, true));
        assert_eq!(manager.manager.unwrap().words(), expected.words());
        assert_eq!(streams.scalar_birth, ScalarRandom::from_seed(seed));
        assert_eq!(streams.initial, ModuleRandom::from_owner_seed(seed));
        assert_eq!(streams.initial, streams.shape);
        let mut advanced = streams;
        advanced.initial.next4_u32();
        assert_eq!(
            advanced.shape, streams.shape,
            "module streams own separate state"
        );
    }

    #[test]
    fn manual_seed_needs_no_entropy_and_never_consumes_shared_state() {
        let mut manager = SystemSeedManager::default();
        let (mut owner, streams) = manager.create_owner(Some(0), Some(false)).unwrap();
        assert!(manager.manager.is_none());
        assert_eq!(
            owner.seed, 0,
            "manual zero is valid and must not be randomized"
        );
        let mut reference = ParticleSeedManager::from_entropy_words(ENTROPY);
        let expected = owner.reset(&mut reference);
        assert_eq!(streams, expected);
        assert_eq!(reference.words(), ENTROPY);

        let mut manager = SystemSeedManager::from_entropy_words(ENTROPY);
        let (mut owner, initial) = manager.create_owner(Some(71), Some(false)).unwrap();
        assert_eq!(manager.reset_owner(&mut owner).unwrap(), initial);
        assert_eq!(manager.manager.unwrap().words(), ENTROPY);
    }

    #[test]
    fn shared_state_survives_owners_and_auto_flag_alone_does_not_draw() {
        let mut manager = SystemSeedManager::from_entropy_words(ENTROPY);
        let mut expected = ParticleSeedManager::from_entropy_words(ENTROPY);
        let first = expected.next_system_seed();
        let (mut retired_owner, retired_streams) =
            manager.create_owner(Some(999), Some(true)).unwrap();
        assert_eq!(retired_owner.seed, first);
        let before = manager.manager.unwrap().words();
        retired_owner.set_automatic(false);
        retired_owner.set_automatic(true);
        assert_eq!(manager.manager.unwrap().words(), before);
        // Retaining the old owner/streams while installing another owner shares
        // the manager; this operation does not reset the retired system.
        let second = expected.next_system_seed();
        let (live_owner, _) = manager.create_owner(Some(999), Some(true)).unwrap();
        assert_eq!(live_owner.seed, second);
        assert_eq!(retired_owner.seed, first);
        assert_eq!(
            retired_streams.initial,
            ModuleRandom::from_owner_seed(first)
        );
        let third = expected.next_system_seed();
        manager.reset_owner(&mut retired_owner).unwrap();
        assert_eq!(retired_owner.seed, third);
        assert_eq!(manager.manager.unwrap().words(), expected.words());
    }

    #[test]
    fn missing_fields_and_entropy_failure_do_not_create_a_fallback() {
        let mut manager = SystemSeedManager::default();
        assert!(matches!(
            manager.create_owner(None, Some(true)),
            Err(SeedError::MissingRandomSeed)
        ));
        assert!(matches!(
            manager.create_owner(Some(7), None),
            Err(SeedError::MissingAutoRandomSeed)
        ));
        assert!(manager.manager.is_none());
        assert!(matches!(
            manager.try_init_with(|| Err(getrandom::Error::UNSUPPORTED)),
            Err(SeedError::EntropyUnavailable(_))
        ));
        assert!(manager.manager.is_none());
        manager
            .try_init_with(|| {
                let mut bytes = [0_u8; 16];
                for (index, word) in ENTROPY.into_iter().enumerate() {
                    bytes[index * 4..index * 4 + 4].copy_from_slice(&word.to_le_bytes());
                }
                Ok(bytes)
            })
            .unwrap();
        manager
            .try_init_with(|| panic!("initialized manager must not request fresh entropy"))
            .unwrap();
        assert_eq!(manager.manager.unwrap().words(), ENTROPY);
    }
}
