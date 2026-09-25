//! The runtime's `System.Random` and the pick that draws from it.
//!
//! One of the two pick overloads the NPC talk lotteries use does not draw
//! from the engine's random stream. The overload over a plain sequence
//! builds a **fresh** `System.Random` for every pick, counts the sequence,
//! raises when it is empty, and otherwise takes one `Next(count)`. The other
//! overload (over a list or an array) draws one engine integer range and is
//! not modelled here.
//!
//! The generator is Knuth's subtractive generator as the runtime's class
//! library ships it: a 56-slot table seeded from one integer, `Next()`
//! returns the raw sample, and `Next(max)` scales the sample by `1 / (2^31 - 1)`
//! in double precision and truncates. A parameterless generator takes its
//! seed from a per-thread seeder: the seeder is itself a `System.Random`,
//! seeded once from the process-wide generator, and every new generator on
//! that thread consumes the seeder's next `Next()`. No clock enters the seed,
//! so two picks in the same frame are successive draws of one stream, not
//! repeats of one seed. The process-wide generator is seeded from operating
//! system random bytes, which no replay reproduces; the host chooses the root.

/// `Int32.MaxValue`, the generator's modulus.
const MBIG: i32 = i32::MAX;
/// The generator's seed constant.
const MSEED: i32 = 161_803_398;
/// `1 / (2^31 - 1)`, the sample scale, as the runtime writes it.
const SAMPLE_SCALE: f64 = 4.656_612_875_245_797e-10;

/// One `System.Random` instance.
#[derive(Debug, Clone)]
pub struct NetRandom {
    inext: usize,
    inextp: usize,
    seed_array: [i32; 56],
}

impl NetRandom {
    /// `new Random(seed)`: `Int32.MinValue` is treated as `Int32.MaxValue`,
    /// any other seed by its absolute value; arithmetic wraps.
    pub fn new(seed: i32) -> Self {
        let subtraction = if seed == i32::MIN { i32::MAX } else { seed.wrapping_abs() };
        let mut mj = MSEED.wrapping_sub(subtraction);
        let mut seed_array = [0_i32; 56];
        seed_array[55] = mj;
        let mut mk: i32 = 1;
        for i in 1..55 {
            let ii = (21 * i) % 55;
            seed_array[ii] = mk;
            mk = mj.wrapping_sub(mk);
            if mk < 0 {
                mk = mk.wrapping_add(MBIG);
            }
            mj = seed_array[ii];
        }
        for _ in 1..5 {
            for i in 1..56 {
                seed_array[i] = seed_array[i].wrapping_sub(seed_array[1 + (i + 30) % 55]);
                if seed_array[i] < 0 {
                    seed_array[i] = seed_array[i].wrapping_add(MBIG);
                }
            }
        }
        Self {
            inext: 0,
            inextp: 21,
            seed_array,
        }
    }

    fn internal_sample(&mut self) -> i32 {
        let mut loc_inext = self.inext + 1;
        if loc_inext >= 56 {
            loc_inext = 1;
        }
        let mut loc_inextp = self.inextp + 1;
        if loc_inextp >= 56 {
            loc_inextp = 1;
        }
        let mut ret = self.seed_array[loc_inext].wrapping_sub(self.seed_array[loc_inextp]);
        if ret == MBIG {
            ret -= 1;
        }
        if ret < 0 {
            ret = ret.wrapping_add(MBIG);
        }
        self.seed_array[loc_inext] = ret;
        self.inext = loc_inext;
        self.inextp = loc_inextp;
        ret
    }

    /// `Next()`: the raw sample in `[0, 2^31 - 1)`.
    pub fn next(&mut self) -> i32 {
        self.internal_sample()
    }

    /// `Next(max)`: `(int)(Sample() * max)`. A negative `max` raises in the
    /// runtime; the callers here only pass counts.
    pub fn next_below(&mut self, max: i32) -> i32 {
        assert!(max >= 0, "Next(max) with a negative max raises");
        (f64::from(self.internal_sample()) * SAMPLE_SCALE * f64::from(max)) as i32
    }
}

/// The per-thread seeder of parameterless generators.
#[derive(Debug, Clone)]
pub struct NetThreadSeeder {
    seeder: NetRandom,
}

impl NetThreadSeeder {
    /// The thread's seeder, seeded from the process-wide generator's next
    /// value; `root` stands for that generator's operating-system seed.
    pub fn from_root(root: i32) -> Self {
        let mut global = NetRandom::new(root);
        Self {
            seeder: NetRandom::new(global.next()),
        }
    }

    /// `new Random()`: a generator seeded from the seeder's next `Next()`.
    pub fn fresh(&mut self) -> NetRandom {
        NetRandom::new(self.seeder.next())
    }
}

/// A pick over a plain sequence: one fresh generator, then the count; an
/// empty sequence raises (`None`) after the generator was built; otherwise
/// one `Next(count)` gives the index.
pub trait EnumerablePick {
    fn pick(&mut self, count: usize) -> Option<usize>;
}

impl EnumerablePick for NetThreadSeeder {
    fn pick(&mut self, count: usize) -> Option<usize> {
        let mut random = self.fresh();
        if count == 0 {
            return None;
        }
        let count = i32::try_from(count).expect("a pick counts at most Int32.MaxValue elements");
        Some(random.next_below(count) as usize)
    }
}
