//! Minimal current JP seed ownership laws. No entropy provider or runtime wiring.
//!
//! Transcribed from the current JP client's engine library: ParticleSystem::ResetSeeds,
//! RandN.SetSeed and RandomizeState. Reset evidence has
//! 192 exact native cases + 32 entropy-success passthrough cases.
//! The global seed manager's pointer lies at another place in the current
//! library than in the bundled symbol reference. The owner seed is the random
//! seed field of the system's read-only state, beside its auto-seed flag.

const EXPANSION: u32 = 0x6c07_8965;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ScalarRandom {
    pub words: [u32; 4],
}
impl ScalarRandom {
    pub fn from_seed(seed: u32) -> Self {
        let mut words = [seed; 4];
        for i in 1..4 {
            words[i] = words[i - 1].wrapping_mul(EXPANSION).wrapping_add(1);
        }
        Self { words }
    }
    pub fn next_u32(&mut self) -> u32 {
        let t = self.words[0] ^ self.words[0].wrapping_shl(11);
        let w = self.words[3];
        let next = w ^ (w >> 19) ^ t ^ (t >> 8);
        self.words = [self.words[1], self.words[2], w, next];
        next
    }
}

/// Shared across source ParticleSystems, distinct from each system birth stream.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleSeedManager {
    random: ScalarRandom,
}
impl ParticleSeedManager {
    /// Current RandomizeState copies 16 OS-entropy bytes directly on success.
    /// Callers supply the captured/replayable words. Never synthesize an ordinal
    /// or claim a fixed value reproduces an original client's entropy sequence.
    pub fn from_entropy_words(words: [u32; 4]) -> Self {
        Self {
            random: ScalarRandom { words },
        }
    }
    pub fn words(&self) -> [u32; 4] {
        self.random.words
    }
    pub fn next_system_seed(&mut self) -> u32 {
        self.random.next_u32()
    }
}

/// Source RandN layout: state word first, then four SIMD lanes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ModuleRandom {
    pub words: [[u32; 4]; 4],
}
impl ModuleRandom {
    pub fn from_owner_seed(seed: u32) -> Self {
        let lanes = [0_u32, 367, 734, 1101]
            .map(|offset| ScalarRandom::from_seed(seed.wrapping_add(offset)));
        Self {
            words: std::array::from_fn(|word| std::array::from_fn(|lane| lanes[lane].words[word])),
        }
    }
    pub fn next4_u32(&mut self) -> [u32; 4] {
        let mut out = [0; 4];
        for (lane, value) in out.iter_mut().enumerate() {
            let mut random = ScalarRandom {
                words: std::array::from_fn(|word| self.words[word][lane]),
            };
            *value = random.next_u32();
            for word in 0..4 {
                self.words[word][lane] = random.words[word];
            }
        }
        out
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SeedOwner {
    /// Effective source randomSeed. Auto resets replace this value.
    pub seed: u32,
    pub automatic: bool,
}
impl SeedOwner {
    pub fn from_serialized(random_seed: u32, auto_random_seed: bool) -> Self {
        Self {
            seed: random_seed,
            automatic: auto_random_seed,
        }
    }
    /// SetRandomSeed disables auto even when the numeric seed is unchanged.
    /// Native skips writes only for the same seed with auto already false.
    /// It does not itself call ResetSeeds; this pure law has no dirty notification.
    pub fn set_manual_seed(&mut self, seed: u32) {
        self.automatic = false;
        self.seed = seed;
    }
    /// SetAutoRandomSeed only changes the flag; next reset performs the draw.
    pub fn set_automatic(&mut self, automatic: bool) {
        self.automatic = automatic;
    }
    /// One actual source ResetSeeds event. Keep owner independent of streams.
    /// Child recursion/order belongs to the caller, not to this root-only law.
    pub fn reset(&mut self, manager: &mut ParticleSeedManager) -> ResetStreams {
        if self.automatic {
            self.seed = manager.next_system_seed();
        }
        let module = ModuleRandom::from_owner_seed(self.seed);
        ResetStreams {
            scalar_birth: ScalarRandom::from_seed(self.seed),
            initial: module,
            shape: module,
            collision: module,
            force: module,
            lights: ScalarRandom::from_seed(self.seed),
            noise_scroll: 0.0,
        }
    }
}

/// Each copy owns independent mutable storage although reset contents match.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ResetStreams {
    pub scalar_birth: ScalarRandom,
    pub initial: ModuleRandom,
    pub shape: ModuleRandom,
    pub collision: ModuleRandom,
    pub force: ModuleRandom,
    pub lights: ScalarRandom,
    pub noise_scroll: f32,
}
