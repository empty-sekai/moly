# Child particle runtime plan (current JP 6.8.1)

This is an implementation plan and a gate ledger. It does **not** admit a
source system. In particular, the 009 `pt_ground -> flash` command receipt is
a current-`libunity.so` replay with four explicit parent particles, explicit
RNG words, identity owners and explicit update flags. It is not evidence that
the live `pt_fall -> pt_ground` collision chain, scene instance enumeration or
the original client seed order is available.

Authoritative inputs inspected for this plan:

- Current source: `<source-lane>/player-jp-6.8.1`.
- Runtime: this directory, especially
  `crates/moly-game/src/particle_runtime.rs`,
  `crates/moly-game/src/particle_runtime/birth.rs`,
  `crates/moly-law/src/particle/sub_emission.rs` and `buffer.rs`.
- Current receipts: `<weather-lane>/child-command-current.json`
  and `<source-lane>/render-integration-20260919/full-scope-20260920/child-emit-native.md`.
- Current source graph census: `<weather-lane>/rain-child-chain-audit.json`.

The receipt's current `libunity.so` SHA-256 is
`937c6d28193ba1bea76fc86ffecd6bc6dd215c6e89fecfc99bc56ffc475badd9`.
The receipt forwards eight commands (four zero-count, four count-one) into one
native child pool, ending at four particles with no failures. Zero-count
commands leave both the child pool and the child Initial RNG unchanged.

## Implemented bounded command replay

`particle_runtime/child.rs::apply_command` now accepts the captured 0x78-byte
command plus twelve-byte emission payload into a shared Runtime pool under
explicit Local identity owner and neutral inheritance. The current ignored
replay retains the selected source Initial/Color/CustomData and compares all
20 captured logical particle snapshots, including nonzero CustomData after one
catch-up step. It preserves old particles, autonomous clocks and legacy RNG.
It still refuses overflow, death during birth/catch-up, nonneutral inheritance,
Shape/Noise/Collision/Trail and recursive events; it has no weather admission
caller. `child-runtime-replay-current.json` identifies the exact verified build.
The original design requirements below remain for expanding this boundary.

## What the current runtime can already carry

`Runtime` already has the right **single-system storage shape**:

- `pool: Vec<Particle>` and `side: Vec<Side>` are parallel and are moved
  together by `compact_with_side` and `finish_births`.
- `Particle` carries position, persistent velocity, lifetime, inverse lifetime
  and age. `Side` carries seed, birth size/rotation/colour, total velocity and
  two CustomData streams. These cover the fields observed in the flash child
  output. CustomData is written in the native pre-simulation phase; Color is
  evaluated separately for rendering. A render read must not recompute
  CustomData at the later presentation age.
- `birth_capacity` performs the pre-initialization capacity decision and
  `finish_births` implements current four-wide alignment rotation. The gap
  rotation applies even with `RingBufferMode::Disabled`; it must not be replaced
  with a linear append.
- `InitialLaw::start_group_native` consumes all four SIMD lanes for a nonempty
  group, including inactive tail lanes. The caller may retain only the active
  lanes, but must keep the four-lane RNG transition and padded storage order.
- `BirthDistribution`, `BirthTiming`, `BirthBatch` and the bounded catch-up
  iterator in `moly-law::particle::sub_emission` are reusable arithmetic laws.
  They do not own a source graph, child pool, transform or RNG owner.

The current path is still autonomous-birth oriented. `birth.rs::start_explicit`
has no parent command position/velocity, inherited parameter block, child-owner
inverse, update flags or child catch-up state. Its `NativeBirthState` owns one
system seed owner, Initial `ModuleRandom` and autonomous emission state; it is
not a child-event state. `validate_emitter` also rejects any nonempty
`sub_emitters`, collision, trails, noise or inherited velocity. Those refusals
are correct until the corresponding event owner is installed.

## Data model for the smallest future interface

Do not put this state into `EmitterParams`. `EmitterParams` is a source-data
decoder and currently preserves the serialized edge as:

```text
SubEmitterParams {
    emitter: Option<String>,
    source_pointer: Missing | Null | Pointer { file_id, path_id },
    trigger: Birth | Collision | Death | Trigger | Manual,
    properties: u32,
    probability: f32,
}
```

The only complete authored-null proof is `Pointer { file_id: 0, path_id: "0" }`.
Do not turn `Null`, `Missing`, a failed lookup, or a synthetic node path into a
child. Keep the serialized `source_pointer` in the graph census and resolve it
to an actual source identity before building a runtime edge.

The runtime needs an external, source-qualified graph and one shared target
pool. A minimal design is:

```text
ChildTargetKey {
    effect_id, node_path, system_path_id, renderer_path_id,
    scene_instance_id,              // required once live instance enumeration exists
}

ChildEdgeKey {
    source_effect_id, source_node_path, source_system_path_id,
    source_pointer, ordinal, trigger, properties,
}

ChildRuntime {
    key: ChildTargetKey,
    emitter: EmitterParams,          // target source configuration, immutable
    runtime: Runtime,                // exactly one shared pool for this target instance
    owner: SeedOwnerState,
    initial_random: ModuleRandom,
    pending: VecDeque<ChildCommand>,
    parent_carries: map<ParentParticleId, EmissionState>,
}

ChildCommand {
    edge: ChildEdgeKey,
    sequence: u64,                   // RecordEmit arrival order, never vector index
    count: u64,
    rate_count: u64,
    emission: [f32; 3],              // spacing, offset, burstFraction
    position: [f32; 3],              // command +0x08..+0x13, opaque until owner is proven
    velocity: [f32; 3],              // command +0x14..+0x1f, opaque until owner is proven
    inherited_words: [u32; 13],     // command +0x20..+0x53; preserve raw words
    dt: f32,
    previous: f32,
    current: f32,
    catch_up: f32,
    update_flags: u32,               // UpdateState +0x1c; flag names are unproven
    frame_dt: f32,
    simulation_speed: f32,
    world_playing: bool,
    upper_lifetime_slot: f32,
    parent_owner: Affine3x4,
    child_owner: Affine3x4,
    inverse_child_owner: Affine3x4,
}
```

The raw command layout above is the one observed by
`child-command-current.py`: the emission pointer is at `+0x00` and the three
emission floats are the pointed-to 12 bytes; position is at `+0x08`, velocity
at `+0x14`, count at `+0x58`, rate count at `+0x60`, and
`[dt, previous, current, catch_up]` at `+0x68`. The inherited block is
`raw[0x20:0x54]` (13 words). Its final word at `+0x50` carries the parent
particle seed: `module-controls-native.py::SubNative.step` supplies
`[17, 19, 127, 2471805022]`, and the eight captured distance/time commands
preserve those four words in pairs. Keep the entire block bit-for-bit.
That parent-seed field is not the child birth seed, child system seed or an
Initial RNG reset. The child birth seeds in the receipt come from the child's
separate 16-word Initial `ModuleRandom` stream.

`ParentParticleId` must be a stable identity plus generation, not a `Vec`
index. Native compaction swaps lanes. Any per-parent distance/time carry,
collision/death event or recursive edge would otherwise attach to a different
particle after `swap_remove`. `Side` currently has no such identity, so full
SubModule integration requires a parallel identity sidecar (or an equivalent
stable handle) that moves with `Side` through every packing and death path.

## Command and phase barrier

The child queue is consumed in strict `sequence` order. A command with
`count == 0` is retained for diagnostics but is a no-op: it must not consume
child Initial RNG, mutate carry, run child modules or alter packing.

For each positive command, the future adapter must perform this barrier; no
renderer write may observe the child half-way through it:

1. Validate the source-qualified edge, target key, finite scalar fields,
   owner snapshots, update owner and current source module gates. Convert `u64`
   counts only after checking the exact supported range.
2. Resolve the child simulation-space inputs. For Local/Custom, apply the
   captured inverse child owner to command position, velocity and gravity at
   the native boundary. For World, preserve command world coordinates. Do not
   use a later frame's owner transform.
3. Preserve the existing child prefix without advancing its age, position or
   modules. Current native child `Emit` does not run the old pool's normal
   update. Its caller owns that separate frame phase. Validate catch-up flags
   and the tested guard before birth; do not perform catch-up here. In the
   qualified world-playing/constant-lifetime branch,
   `catch_up >= upper_lifetime_slot` skips creation. Do not generalize that
   upper-lifetime cache to arbitrary curves without current evidence.
4. Decide `birth_capacity` against the existing pool before `InitialLaw`.
   Do not free capacity by simulating or killing existing particles inside
   `Emit`. Ring replacement remains a separate composition gate; never
   initialize particles that the capacity law would reject.
5. For each accepted command lane, construct `InitialGroupInput` with the
   command's birth fractions and curve times. A nonempty group still advances
   all four Initial RNG lanes, including padding. Commit Initial `ModuleRandom`
   only after the complete command succeeds.
6. Complete `StartModules` for the aligned newborn range, including Initial,
   StartVelocity and the native pre-simulation module write. The birth-time
   position and age update belongs to this newborn path. Only then run any
   catch-up steps, still restricted to this newborn range: each full step is
   `pre -> simulate -> post`. CustomData is written by `pre` using that phase's
   age; preserve the final written channels for rendering. Color is a separate
   render-side evaluation. The tested flags use `(update_flags & 5) != 0` for
   catch-up full-frame steps; this is a receipt boundary, not a named API. The
   tested catch-up range is at most one second; a sub-frame remainder is not
   simulated by this branch. The current receipt proves the birth position
   rule in its bounded constant branch, before additional catch-up movement:

   ```text
   position = emission_position - parent_velocity * birth_dt
              + born_velocity * birth_dt
   age_percent = birth_dt * 100 * inverse_lifetime
   ```

   Every multiply/add is a separate f32 operation. Do not reassociate it.
7. Keep padded newborn storage alive through the newborn phases, then call
   `finish_births` with the exact old count. Any future newborn-death path must
   preserve native four-wide retest and packing order; the current command
   receipt does not establish overflow/death/event composition during catch-up.
   Keep those configurations gated. It never authorizes compacting or advancing
   the old prefix inside `Emit`. `pool` and `side` must remain equal length at
   every barrier.
8. Publish the child pool and sidecar atomically, then enqueue recursive child
   commands only after their parent event phase is proven. A command that fails
   validation must leave pool, side, RNG words, carry and command sequence
   unchanged and increment a refusal diagnostic.

The existing autonomous `simulate_existing` ordering (old particles first,
then newborn span) differs from child `Emit`, which leaves the old prefix
unchanged and catches up only newborns after their birth phase. The 512-case
native replay checks unchanged old position, velocity, age and inverse lifetime;
the saved explicit catch-up trace records StartModules/Initial/StartVelocity,
newborn pre, then newborn pre/sim/post and CopyNewParticles. This closes the
bounded phase distinction, not source collision trajectory or recursive scene
scheduling. Keep normal old-pool updates in their caller-owned phase.

## Owner, RNG and lifecycle requirements

- A child target owns one `ModuleRandom` Initial stream and one effective
  `SeedOwner`; it must not derive a seed from parent index, edge ordinal,
  `Runtime.rng` SplitMix state, path hash or scene ordinal.
- A child reset consumes the shared `SystemSeedManager` only when the current
  source owner is automatic. Manual seed reset consumes no manager entropy.
  Parent and child reset order, live instance enumeration and original-client
  entropy are still unproven. Until captured, child runtime installation is a
  diagnostic fixture only.
- The command's 16-word probe state `17..32` is explicit test initialization,
  not a source seed. The current receipt records all 16-word transitions and
  padded lanes; it does not authorize replacing the real owner with those
  words.
- Preserve the parent particle seed at command `+0x50` independently of those
  child Initial words. Do not infer a child birth seed, child reset seed or RNG
  transition from that inherited parent field.
- Parent owner and child owner/inverse must be captured at command creation,
  together with parent particle position/velocity and simulation speed. The
  current `Runtime.node_affine` plus the frame `Context` can supply a transform
  for ordinary rendering, but cannot reconstruct a past collision/event owner
  after the parent pool is compacted.
- `update_flags`, `frame_dt`, world-playing state and the upper-lifetime slot
  are command owner inputs. Do not silently map them to `dt`, `duration` or
  `simulation_speed`.

## Capacity, death and packing invariants

The following are hard invariants for every future child target:

1. One target instance has one shared `Runtime.pool`; never allocate a pool per
   parent particle or per edge. `sub_emission.rs` explicitly places carries per
   parent/edge but sends resulting batches to the child's shared pool.
2. Capacity is checked per command with the target's authored maximum and ring
   mode. Overflow increments the target's `full_total`; it is not silently
   dropped or routed to a second pool.
3. Nonempty commands allocate/retain aligned four-lane storage. Tail lanes may
   advance RNG and be tested for death, but only accepted live lanes count
   towards the logical particle count. The gap rotation in `finish_births`
   determines final order and therefore seed/age/CustomData association.
4. `pool.len() == side.len() == identity_sidecar.len()` before and after every
   child command. Any swap/removal moves all three together.
5. Catch-up belongs only to the new aligned birth block. Existing child
   particles do not age or die as part of `Emit`; their normal lifecycle is
   caller-owned. Newborn death/overflow during catch-up and death-triggered
   children stay gated until native event timing and packing composition are
   captured. The independent `compact_with_side` law is not sufficient proof
   of that composition.
6. Ring mode is not inferred from the parent. The seven 009 flash candidates
   are ring mode 0; other targets with ring replacement remain refused until
   child overflow/death composition is replayed.

## 009 flash: smallest *implementation* slice, still not admission

The seven current source copies of `root/pt_fall/pt_ground/flash` share the
same initial/color/custom configuration hash
`883be7da0182590f37e611aa2c694407d157710f08e993c172d1a98f48e4c7b5` and have
shape disabled, Local simulation, ring mode 0 and a one-count burst. Their
actual target modules are Initial, Emission, Color and CustomData. This makes
them the smallest **consumer integration** slice once a real parent command is
available.

The target still needs the following before source admission:

- real `pt_fall -> pt_ground` collision arrival and parent trajectory;
- source-qualified scene instance and renderer owner identity;
- child seed/reset order and live owner state;
- actual parent/child/inverse transforms and update flags;
- shared-pool catch-up, death and overflow behavior under source timing;
- current renderer/shader and original-client visual equivalence.

The receipt's four explicit parent particles, identity owner, flags `0`,
identity seed setup and the separate synthetic Local catch-up fixture must stay
labelled as probes. They may drive a test adapter, never `admitted` counts.
The second `pt_ground -> pt` edge and the upstream collision edge are separate
gates; forwarding `flash` commands cannot close either one.

The minimal code seam after those gates are evidenced is therefore a new
child-specific command entry (conceptually `apply_child_command`) that accepts
`ChildCommand`, resolves a target `Runtime`, calls the existing Initial/CustomData
and buffer laws, and commits a single shared pool. It must not reuse
`step_explicit`'s autonomous clock or invent an emission schedule from the
child's own `EmitterParams`; RecordEmit already supplied the command count and
emission state.

## Global admission gates

Keep the following gates in the corpus audit even if a bounded child test is
green:

- `authoredNull`: only `(fileId=0,pathId="0")` is an empty edge; no synthetic
  self-emission.
- `collision`: all seven current collision edges require scene broadphase/BVH,
  PhysX filter/owner and event timing evidence.
- `death`: rain/rain-night/rain-meteor death chains require exact native death
  event ordering and pool ownership; collision callbacks are not substitutes.
- `properties != 0`, non-unit probability, nonconstant curves/bursts,
  repeating/multiple bursts, nonzero inherited modules and custom simulation
  owners stay refused until independently replayed.
- Shape, Noise, Trail, Collision, ring replacement, prewarm and recursive
  scheduling remain module-specific gates. A target's passing Initial replay
  does not compose those modules automatically.
- A static census condition (`67` bounded birth edges, `7` shape-disabled 009
  flash copies) is coverage information, not live source admission.

The original plan preceded the child implementation. Follow-up implementation
should use the dedicated task target/cache from `INTEGRATION.md`, run only
incremental targeted tests, and keep synthetic command receipts separate from
the current admission audit.
