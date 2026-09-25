//! The engine CollisionModule update for World type, 3D mode and High
//! quality: the query packs, the one broadphase overlap per call, the
//! per-particle hit selection against the returned colliders, the hits back
//! into simulation space, the response with the module's random advance, and
//! the Collision sub-emitter events.
//!
//! Every operation follows the ARM rules in the engine's order. The scene side
//! (the broadphase overlap, each collider's world bounds and the sphere sweep
//! against one collider) is the [`CollisionScene`] trait; the law reads only
//! what the engine reads back from it.
//!
//! Packing: a call over `[from, to)` loads `ceil((to - from) / 4)` packs of
//! four lanes starting at `from`. Only when `to` is not a multiple of four are
//! the lanes of the last pack from `to % 4` on rewritten with that pack's lane
//! 0 and the index `to`; otherwise nothing is rewritten, and lanes at or past
//! `to` are read as the slots hold them. Such lanes widen the broadphase box
//! and the pack test, never reach a sweep and are never written. A caller that
//! does not know those slots' contents returns `None` for them: the call is
//! then exact only when no queried lane can reach any collider the mask can
//! return, and refused otherwise.
//!
//! Several colliders: the selection takes the colliders in the order the
//! overlap stored its touches. The nearest hit wins only on a strictly
//! smaller travel, so a tie keeps the earlier collider; a touching start ends
//! the lane with no hit and a penetrating one returns at once, so which of two
//! such starts comes first decides the answer. A scene that cannot give the
//! broadphase order says so ([`CollisionScene::order_known`]); every lane is
//! then selected over all orders of the colliders that hit it. The answer is
//! kept when every order gives the same hit point and normal (bit for bit),
//! or, when the hits differ, when every order's hit gives the same written
//! particle (position, velocity, age) and the same event record (the random
//! advance follows the hit count, the events the written particle); the call
//! is refused by name when two orders write differently.

use crate::particle::armf as a;
use crate::particle::collision_event::{record_emit, CollisionEmitEdge, EventParent, RecordedEmit};
use crate::particle::collision_response::{
    build_query, world_hit_to_simulation, CollisionQuery, CollisionRandom, CollisionResponse,
    QueryAffine, QueryInput, Refusal as ResponseRefusal,
};
use crate::particle::schema::{CollisionMode, CollisionParams, CollisionQuality, CollisionType};

/// Margin added to the broadphase box and to every collider's bounds.
const MARGIN: f32 = f32::from_bits(0x3727_c5ac);
/// Floor of the lane dt, of the segment length that gives a direction, and of
/// the swept sphere's radius.
const TINY: f32 = f32::from_bits(0x3586_37bd);
/// The skin fraction: the sphere shrinks by it and the sweep reaches as far.
const SKIN: f32 = f32::from_bits(0x3e1a_9fbe);
/// A non-finite hit normal is rebuilt from the hit position and the collider
/// centre when their distance is above this, else it becomes +Z.
const REPAIR_MIN: f32 = f32::from_bits(0x3727_c5ac);

#[cfg(test)]
pub(crate) mod arms {
    use std::cell::Cell;
    thread_local! { static ARM: Cell<Option<&'static str>> = const { Cell::new(None) }; }
    pub fn set(arm: Option<&'static str>) {
        ARM.with(|a| a.set(arm));
    }
    pub fn on(name: &str) -> bool {
        ARM.with(|a| a.get() == Some(name))
    }
}
#[cfg(not(test))]
pub(crate) mod arms {
    #[inline(always)]
    pub fn on(_: &str) -> bool {
        false
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A module configuration the native path was not executed for.
    Unqualified(&'static str),
    Response(ResponseRefusal),
    /// The range ends before it starts (neither engine caller passes one).
    ReversedRange,
    /// An index past the 32-bit lane index.
    RangeTooLarge,
    /// A lane below `to` has no particle.
    MissingParticle,
    /// A Local system without its owner matrices.
    MissingOwner,
    /// Slots past the end are read, their contents are not known, and a
    /// queried lane reaches a collider the mask can return.
    PastEndLanes,
    /// The scene could not give the engine's answer to a query of this call.
    Scene(&'static str),
    /// The scene could not give the broadphase order, and on `dependent`
    /// lanes of the call two orders of the colliders that hit write the
    /// particle differently (`order_free` other lanes had several hits whose
    /// orders all agreed).
    OrderDependent { dependent: usize, order_free: usize },
    /// The scene could not give the broadphase order, and the order decides
    /// more than the law evaluates: more touches than the shape limit keeps,
    /// or more than six colliders hitting one lane.
    OrderUnbounded,
}

/// The most colliders hitting one lane whose orders are all evaluated.
const MAX_UNORDERED_HITS: usize = 6;

/// The particle-state flags the module reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParticleFlags {
    /// The query radius reads the current-size stream (else the start size).
    pub current_size: bool,
    /// The radius is the largest of the three size components (else X).
    pub size_3d: bool,
    /// The full velocity is scaled by the per-particle speed modifier.
    pub speed_modifier: bool,
}

/// One particle as the module reads it, in simulation coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParticleLane {
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub animated: [f32; 3],
    /// The size stream the current-size flag selects.
    pub size: [f32; 3],
    /// Read only with the speed-modifier flag.
    pub speed_modifier: f32,
    pub age_percent: f32,
    pub inverse_lifetime: f32,
    pub seed: u32,
}

/// The particle arrays of one call. `lane(i)` is `None` only for a slot past
/// the end whose contents the caller does not know.
pub trait CollisionParticles {
    fn lane(&self, index: usize) -> Option<ParticleLane>;
}

/// The module's persistent words: two range words the query rewrites on every
/// nonempty call (no output of this path reads them), the collision-event
/// flag and the collision-event count it clears when events are off.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CollisionState {
    pub range_words: [u64; 2],
    pub uses_events: bool,
    pub events: u64,
}

/// The owner of a Local system: its local-to-world and the inverse the
/// engine's owner update stores.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OwnerPair {
    pub local_to_world: QueryAffine,
    pub world_to_local: QueryAffine,
}

/// The broadphase query of one call.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct OverlapQuery {
    pub center: [f32; 3],
    pub extents: [f32; 3],
    pub collides_with: u32,
    pub dynamic: bool,
    pub max_shapes: i32,
}

/// One collider the overlap returned, with its world bounds as the physics
/// scene reports them at inflation 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Candidate {
    pub bounds_min: [f32; 3],
    pub bounds_max: [f32; 3],
    /// The collider is a trigger: its hits are dropped.
    pub is_trigger: bool,
    pub collider_id: i32,
    pub body_id: Option<i32>,
}

/// One sphere sweep: identity rotation, zero inflation, hit normal and
/// initial-overlap depth requested.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepRequest {
    /// Index into the candidates of this call's overlap.
    pub shape: usize,
    pub particle: u32,
    pub origin: [f32; 3],
    pub direction: [f32; 3],
    pub distance: f32,
    pub sphere_radius: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SweepHit {
    pub position: [f32; 3],
    pub normal: [f32; 3],
    pub distance: f32,
}

/// The physics scene at the module's boundary.
pub trait CollisionScene {
    /// The broadphase overlap, in the order the engine stores the touches,
    /// at most `max_shapes` of them.
    fn overlap(&mut self, query: &OverlapQuery) -> Vec<Candidate>;
    /// Every collider an overlap with this mask can return, in any order.
    fn reachable(&self, collides_with: u32) -> Vec<Candidate>;
    /// The sphere sweep against candidate `request.shape` of the last overlap.
    fn sweep_sphere(&mut self, request: &SweepRequest) -> Option<SweepHit>;
    /// Why the scene could not answer a query it was given, if it could not;
    /// the call then refuses and commits nothing.
    fn refusal(&self) -> Option<&'static str> {
        None
    }
    /// Whether the last overlap returned its colliders in the engine's
    /// order. When not, the law selects over every order of the colliders
    /// that hit a lane and refuses a lane the order decides.
    fn order_known(&self) -> bool {
        true
    }
}

/// One selected hit, in simulation coordinates after the owner inverse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRecord {
    pub index: u32,
    pub start: [f32; 3],
    pub direction: [f32; 3],
    pub normal: [f32; 3],
    pub point: [f32; 3],
    /// With `order_free`, the hit of the scene's listed order: the engine's
    /// order may pick another collider, with the same point and normal or
    /// with a hit that writes the particle the same, and no output reads the
    /// hit records' ids, points or normals while collision messages are
    /// refused.
    pub collider_id: i32,
    pub body_id: i32,
    /// Selected over every order of several colliders that hit the lane, all
    /// giving this point and normal or writing the particle the same.
    pub order_free: bool,
}

/// One particle the response wrote.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Written {
    pub index: usize,
    pub position: [f32; 3],
    pub velocity: [f32; 3],
    pub age_percent: f32,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct UpdateOutcome {
    /// In the response's (reverse hit) order; commit all of them.
    pub written: Vec<Written>,
    pub hits: Vec<HitRecord>,
    pub emits: Vec<RecordedEmit>,
    /// Random draws the response made (three per group of four hits).
    pub draws: usize,
    /// Slots past the end were not known and no queried lane reached a
    /// collider, so the call made no sweep.
    pub past_end_unreached: bool,
    /// Lanes selected over every order of several colliders that hit them
    /// (the scene could not give the broadphase order), all orders agreeing.
    pub order_free: usize,
}

/// One per-particle selection, in call order.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct IntersectRecord {
    pub particle: u32,
    pub start: [f32; 3],
    pub direction: [f32; 3],
    pub aabb: [f32; 6],
    pub length: f32,
    pub radius: f32,
    pub candidates: usize,
    pub returned: Option<usize>,
}

/// The intermediate words of one call, for comparison with the engine's.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Trace {
    pub packs: Vec<[u32; 32]>,
    pub overlap: Option<OverlapQuery>,
    pub bounds_calls: usize,
    pub pack_tests: Vec<bool>,
    pub intersects: Vec<IntersectRecord>,
    pub sweeps: Vec<SweepRequest>,
}

/// The inputs of one call beyond the particle arrays.
#[derive(Clone, Copy, Debug)]
pub struct UpdateInput<'a> {
    pub from: usize,
    pub to: usize,
    /// Lane `l` of every pack uses `dt[l]`.
    pub dt: [f32; 4],
    /// Required for a Local system, ignored for a World one.
    pub owner: Option<OwnerPair>,
    /// The Collision sub-emitter edges, in the module's order.
    pub edges: &'a [CollisionEmitEdge],
    /// The parent's emission state word the event seeds add to.
    pub emission_word: u32,
    /// The parent's system time still to simulate (the commands' catch-up).
    pub pending: f32,
}

/// The qualified module: World type, 3D mode, High quality, constant response
/// curves in [0, 1], no collider force, no collision messages, no interior
/// collisions.
#[derive(Clone, Copy, Debug)]
pub struct CollisionLaw {
    response: CollisionResponse,
    radius_scale: f32,
    collides_with: u32,
    dynamic: bool,
    max_shapes: i32,
    world: bool,
    flags: ParticleFlags,
}

#[derive(Clone, Copy)]
struct Lane {
    index: u32,
    query: CollisionQuery,
}

impl CollisionLaw {
    pub fn from_params(params: &CollisionParams, world: bool, flags: ParticleFlags) -> Result<Self, Refused> {
        if params.kind != CollisionType::World {
            return Err(Refused::Unqualified("Planes collision type"));
        }
        if params.mode != CollisionMode::ThreeDimensional {
            return Err(Refused::Unqualified("2D collision mode"));
        }
        if params.quality != CollisionQuality::High {
            return Err(Refused::Unqualified("Medium or Low collision quality (voxel cache)"));
        }
        if params.collider_force != 0.0 {
            return Err(Refused::Unqualified("collider force"));
        }
        if params.messages {
            return Err(Refused::Unqualified("collision messages"));
        }
        if params.interior {
            return Err(Refused::Unqualified("interior collisions"));
        }
        if !params.radius_scale.is_finite() {
            return Err(Refused::Unqualified("non-finite radius scale"));
        }
        let max_shapes = i32::try_from(params.max_shapes)
            .map_err(|_| Refused::Unqualified("collision shape limit past 32-bit signed"))?;
        Ok(Self {
            response: CollisionResponse::from_params(params).map_err(Refused::Response)?,
            radius_scale: params.radius_scale,
            collides_with: params.collides_with,
            dynamic: params.dynamic,
            max_shapes,
            world,
            flags,
        })
    }

    /// For native rows whose module words are given directly.
    #[allow(clippy::too_many_arguments)]
    pub fn from_words(response: CollisionResponse, radius_scale: f32, collides_with: u32, dynamic: bool,
        max_shapes: i32, world: bool, flags: ParticleFlags) -> Self {
        Self { response, radius_scale, collides_with, dynamic, max_shapes, world, flags }
    }

    pub fn collides_with(&self) -> u32 {
        self.collides_with
    }

    /// Whether the query radius reads the current-size stream.
    pub fn reads_current_size(&self) -> bool {
        self.flags.current_size
    }

    /// One `CollisionModule::Update` call over `[from, to)`. Nothing is
    /// committed on a refusal: `state` and `random` change only on success,
    /// and the caller commits `written` and delivers `emits`.
    pub fn update(
        &self,
        state: &mut CollisionState,
        random: Option<&mut CollisionRandom>,
        particles: &dyn CollisionParticles,
        input: &UpdateInput<'_>,
        scene: &mut dyn CollisionScene,
        mut trace: Option<&mut Trace>,
    ) -> Result<UpdateOutcome, Refused> {
        let (from, to) = (input.from, input.to);
        if to < from {
            return Err(Refused::ReversedRange);
        }
        if to == from {
            return Ok(UpdateOutcome::default());
        }
        if u32::try_from(to.saturating_add(3)).is_err() {
            return Err(Refused::RangeTooLarge);
        }
        let owner = match (self.world, input.owner) {
            (true, _) => None,
            (false, Some(owner)) => Some(owner),
            (false, None) => return Err(Refused::MissingOwner),
        };
        let mut next = *state;
        // Collision messages are refused at construction, so events stay off
        // and the count is cleared when a previous state had them on.
        if next.uses_events {
            next.uses_events = false;
            next.events = 0;
        }
        let count = (to - from) as u64;
        let [s10, s18] = state.range_words;
        let marker = if (from as u64) <= s18 && s18 < to as u64 { s18 } else { from as u64 };
        next.range_words = [range_word(s10, count), s10.wrapping_add(marker)];

        let packs = self.packs(particles, input, owner)?;
        let mut outcome = UpdateOutcome::default();
        let known: Option<Vec<[Lane; 4]>> =
            packs.iter().map(|pack| pack.iter().copied().collect::<Option<Vec<_>>>()
                .map(|lanes| [lanes[0], lanes[1], lanes[2], lanes[3]])).collect();
        let Some(packs) = known else {
            // Unknown past-end slots change only the overlap box and the pack
            // tests; with no queried lane near any collider there is no sweep.
            if self.max_shapes >= 1 {
                let reachable = scene.reachable(self.collides_with);
                for lane in packs.iter().flatten().flatten().filter(|lane| (lane.index as usize) < to) {
                    let (pc, pe) = lane_box(&lane.query);
                    if reachable.iter().any(|c| {
                        let (sc, se) = candidate_box(c);
                        (0..3).all(|k| a::abs(a::sub(sc[k], pc[k])) <= a::add(pe[k], se[k]))
                    }) {
                        return Err(Refused::PastEndLanes);
                    }
                }
            }
            *state = next;
            outcome.past_end_unreached = true;
            return Ok(outcome);
        };
        if let Some(trace) = trace.as_deref_mut() {
            trace.packs = packs.iter().map(pack_words).collect();
        }
        let found = self.find(&packs, input.dt, to, scene, trace.as_deref_mut());
        if let Some(reason) = scene.refusal() {
            return Err(Refused::Scene(reason));
        }
        let Found { choices, mut order_free } = found?;
        let mut hits = Vec::with_capacity(choices.len());
        let mut dependent = 0;
        for choice in choices {
            match choice {
                Choice::Hit(hit) => hits.push(hit),
                Choice::Alternatives(answers) => {
                    // What each order's answer writes: the particle and, from
                    // it, the event record; a no-hit writes nothing and
                    // changes the random advance.
                    let written = answers.iter().map(|answer| answer.map(|hit| self.written_words(particles, owner, hit))
                        .transpose().map(|words| (words, answer.filter(|_| arms::on("orderFreeComparesIds"))
                            .map(|hit| hit.collider_id))))
                        .collect::<Result<Vec<_>, _>>()?;
                    if !arms::on("orderFreeComparesHits") && written.iter().all(|w| *w == written[0]) {
                        order_free += 1;
                        hits.extend(answers[0].map(|hit| HitRecord { order_free: true, ..hit }));
                    } else {
                        dependent += 1;
                    }
                }
            }
        }
        if dependent > 0 {
            return Err(Refused::OrderDependent { dependent, order_free });
        }
        outcome.order_free = order_free;
        for hit in &mut hits {
            *hit = to_simulation(owner, *hit);
        }
        if !hits.is_empty() {
            outcome.draws = hits.len().div_ceil(4) * 3;
        }
        let order: Vec<&HitRecord> = if arms::on("forwardOrder") { hits.iter().collect() } else { hits.iter().rev().collect() };
        for hit in order {
            let index = hit.index as usize;
            let p = particles.lane(index).ok_or(Refused::MissingParticle)?;
            let modifier = self.flags.speed_modifier.then_some(p.speed_modifier);
            let out = self.response.respond(p.position, p.velocity, p.animated, modifier, p.age_percent, hit.point, hit.normal);
            outcome.written.push(Written { index, position: out.position, velocity: out.velocity, age_percent: out.age_percent });
            if !out.records_event || input.edges.is_empty() {
                continue;
            }
            let parent = EventParent {
                index,
                seed: p.seed,
                position: out.position,
                velocity: out.velocity,
                animated: p.animated,
                normalized_age: out.normalized_age,
                seconds: a::div(out.normalized_age, p.inverse_lifetime),
            };
            let pending = if arms::on("catchUpZero") { 0.0 } else { input.pending };
            for (k, edge) in input.edges.iter().enumerate() {
                outcome.emits.push(record_emit(edge, k, &parent, input.emission_word, pending,
                    owner.map(|owner| owner.local_to_world)));
            }
        }
        if let Some(random) = random {
            if !arms::on("noRandom") {
                random.advance_for_hits(hits.len());
            }
        }
        outcome.hits = hits;
        *state = next;
        Ok(outcome)
    }

    /// What the response writes for one hit (the world hit taken into
    /// simulation space): the particle's position, velocity and age, the
    /// normalized age and whether it records an event.
    fn written_words(&self, particles: &dyn CollisionParticles, owner: Option<OwnerPair>, hit: HitRecord)
        -> Result<[u32; 9], Refused> {
        let hit = to_simulation(owner, hit);
        let p = particles.lane(hit.index as usize).ok_or(Refused::MissingParticle)?;
        let modifier = self.flags.speed_modifier.then_some(p.speed_modifier);
        let out = self.response.respond(p.position, p.velocity, p.animated, modifier, p.age_percent, hit.point, hit.normal);
        let mut words = [0u32; 9];
        for k in 0..3 {
            words[k] = out.position[k].to_bits();
            words[3 + k] = out.velocity[k].to_bits();
        }
        words[6] = out.age_percent.to_bits();
        words[7] = out.normalized_age.to_bits();
        words[8] = u32::from(out.records_event);
        Ok(words)
    }

    /// The query packs, with `None` for a lane whose slot is not known.
    fn packs(&self, particles: &dyn CollisionParticles, input: &UpdateInput<'_>, owner: Option<OwnerPair>)
        -> Result<Vec<[Option<Lane>; 4]>, Refused> {
        let (from, to) = (input.from, input.to);
        let local = owner.map_or(QueryAffine::IDENTITY, |owner| owner.local_to_world);
        let radius_scale = if arms::on("radiusNoHalf") { a::mul(self.radius_scale, 2.0) } else { self.radius_scale };
        let mut packs = Vec::with_capacity((to - from).div_ceil(4));
        for p in 0..(to - from).div_ceil(4) {
            let mut pack = [None; 4];
            for (l, slot) in pack.iter_mut().enumerate() {
                let index = from + 4 * p + l;
                let Some(lane) = particles.lane(index) else {
                    if index < to {
                        return Err(Refused::MissingParticle);
                    }
                    continue;
                };
                let query = build_query(QueryInput {
                    position: lane.position,
                    persistent_velocity: lane.velocity,
                    animated_velocity: lane.animated,
                    speed_modifier: self.flags.speed_modifier.then_some(lane.speed_modifier),
                    dt: input.dt[l],
                    size: lane.size,
                    size_is_3d: self.flags.size_3d,
                    radius_scale,
                    world_space: self.world,
                    owner: local,
                });
                *slot = Some(Lane { index: index as u32, query });
            }
            packs.push(pack);
        }
        if arms::on("oldPadding") {
            for (p, pack) in packs.iter_mut().enumerate() {
                let base = pack[0];
                for (l, slot) in pack.iter_mut().enumerate() {
                    if from + 4 * p + l >= to {
                        *slot = base.map(|lane| Lane { index: to as u32, ..lane });
                    }
                }
            }
        } else if to & 3 != 0 {
            // Lane 0 of the last pack is always below `to`.
            let last = packs.last_mut().ok_or(Refused::MissingParticle)?;
            let base = last[0].ok_or(Refused::MissingParticle)?;
            for slot in &mut last[to & 3..] {
                *slot = Some(Lane { index: to as u32, query: base.query });
            }
        }
        Ok(packs)
    }

    /// FindParticleIntersections: the batch box, one overlap, the collider
    /// boxes, the pack tests and the per-lane selections.
    fn find(&self, packs: &[[Lane; 4]], dt: [f32; 4], to: usize, scene: &mut dyn CollisionScene,
        mut trace: Option<&mut Trace>) -> Result<Found, Refused> {
        let mut found = Found::default();
        if self.max_shapes < 1 {
            return Ok(found);
        }
        let mut lo = [f32::INFINITY; 3];
        let mut hi = [f32::NEG_INFINITY; 3];
        let mut radius = 0.0f32;
        for lane in packs.iter().flatten() {
            for k in 0..3 {
                lo[k] = a::min(a::min(lo[k], lane.query.start[k]), lane.query.end[k]);
                hi[k] = a::max(a::max(hi[k], lane.query.start[k]), lane.query.end[k]);
            }
            radius = a::max(radius, lane.query.radius);
        }
        let lo: [f32; 3] = std::array::from_fn(|k| a::sub(a::sub(lo[k], radius), MARGIN));
        let hi: [f32; 3] = std::array::from_fn(|k| a::add(a::add(hi[k], radius), MARGIN));
        let query = OverlapQuery {
            center: std::array::from_fn(|k| a::mul(a::add(lo[k], hi[k]), 0.5)),
            extents: std::array::from_fn(|k| a::mul(a::sub(hi[k], lo[k]), 0.5)),
            collides_with: self.collides_with,
            dynamic: self.dynamic,
            max_shapes: self.max_shapes,
        };
        if let Some(trace) = trace.as_deref_mut() {
            trace.overlap = Some(query);
        }
        let mut candidates = scene.overlap(&query);
        let ordered = scene.order_known() || arms::on("orderFreeTrustListed");
        if !ordered && candidates.len() > self.max_shapes as usize {
            return Err(Refused::OrderUnbounded);
        }
        candidates.truncate(self.max_shapes as usize);
        if candidates.is_empty() {
            return Ok(found);
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.bounds_calls = candidates.len();
        }
        let boxes: Vec<_> = candidates.iter().map(candidate_box).collect();
        for pack in packs {
            let lanes: [([f32; 3], [f32; 3]); 4] = std::array::from_fn(|l| lane_box(&pack[l].query));
            let passes = boxes.iter().any(|(sc, se)| {
                lanes.iter().any(|(pc, pe)| (0..3).all(|k| a::add(se[k], pe[k]) >= a::abs(a::sub(pc[k], sc[k]))))
            });
            if let Some(trace) = trace.as_deref_mut() {
                trace.pack_tests.push(passes);
            }
            if !passes {
                continue;
            }
            for (l, lane) in pack.iter().enumerate() {
                let d: [f32; 3] = std::array::from_fn(|k| a::sub(lane.query.end[k], lane.query.start[k]));
                let length = a::sqrt(a::add(a::mul(d[0], d[0]), a::add(a::mul(d[1], d[1]), a::mul(d[2], d[2]))));
                let direction = if length > TINY { d.map(|x| a::div(x, length)) } else { [0.0; 3] };
                if dt[l] < TINY || lane.index as usize >= to {
                    continue;
                }
                let selected = if ordered {
                    Selected::Hit(self.intersect(lane, direction, lanes[l], length, &candidates, &boxes,
                        scene, trace.as_deref_mut()))
                } else {
                    self.intersect_unordered(lane, direction, lanes[l], length, &candidates, &boxes,
                        scene, trace.as_deref_mut())
                };
                match selected {
                    Selected::Hit(hit) => found.choices.extend(hit.map(Choice::Hit)),
                    Selected::OrderFree(hit) => {
                        found.order_free += 1;
                        found.choices.extend(hit.map(Choice::Hit));
                    }
                    Selected::Alternatives(answers) => found.choices.push(Choice::Alternatives(answers)),
                    Selected::Unbounded => return Err(Refused::OrderUnbounded),
                }
            }
        }
        Ok(found)
    }

    /// ParticleIntersect: the nearest hit over the candidates whose boxes
    /// meet the lane's, with the skin; a touching start ends the lane with no
    /// hit, a penetrating start with a hit pushed out along the normal.
    #[allow(clippy::too_many_arguments)]
    fn intersect(&self, lane: &Lane, direction: [f32; 3], (pc, pe): ([f32; 3], [f32; 3]), length: f32,
        candidates: &[Candidate], boxes: &[([f32; 3], [f32; 3])], scene: &mut dyn CollisionScene,
        trace: Option<&mut Trace>) -> Option<HitRecord> {
        let start = lane.query.start;
        let r = lane.query.radius;
        let skin = if arms::on("noSkin") { 0.0 } else { a::mul(r, SKIN) };
        let sphere_radius = a::max_nm(a::sub(r, skin), TINY);
        let distance = a::add(skin, length);
        let mut record = IntersectRecord {
            particle: lane.index,
            start,
            direction,
            aabb: [pc[0], pc[1], pc[2], pe[0], pe[1], pe[2]],
            length,
            radius: r,
            candidates: candidates.len(),
            returned: None,
        };
        let mut sweeps = Vec::new();
        let mut best = f32::INFINITY;
        let mut result = None;
        let finish = |record: IntersectRecord, sweeps: Vec<SweepRequest>, trace: Option<&mut Trace>| {
            if let Some(trace) = trace {
                trace.intersects.push(record);
                trace.sweeps.extend(sweeps);
            }
        };
        for (i, candidate) in candidates.iter().enumerate() {
            let (sc, se) = boxes[i];
            if !(0..3).all(|k| a::abs(a::sub(sc[k], pc[k])) <= a::add(pe[k], se[k])) {
                continue;
            }
            let request = SweepRequest { shape: i, particle: lane.index, origin: start, direction, distance, sphere_radius };
            sweeps.push(request);
            let Some(hit) = scene.sweep_sphere(&request) else {
                continue;
            };
            let mut normal = hit.normal;
            if normal.iter().any(|v| !v.is_finite()) {
                let v: [f32; 3] = std::array::from_fn(|k| a::sub(hit.position[k], sc[k]));
                let span = a::sqrt(a::add(a::add(a::mul(v[0], v[0]), a::mul(v[1], v[1])), a::mul(v[2], v[2])));
                normal = if span > REPAIR_MIN { v.map(|x| a::div(x, span)) } else { [0.0, 0.0, 1.0] };
            }
            let travel = a::sub(hit.distance, skin);
            let nearer = if arms::on("tieTakesLater") { travel <= best } else { travel < best };
            if !nearer || candidate.is_trigger {
                continue;
            }
            let collider_id = candidate.collider_id;
            let body_id = candidate.body_id.unwrap_or(collider_id);
            if hit.distance > 0.0 || (arms::on("penetrationRecorded") && hit.distance < 0.0) {
                let point = std::array::from_fn(|k| a::add(a::mul(direction[k], travel), start[k]));
                result = Some(HitRecord { index: lane.index, start, direction, normal, point, collider_id, body_id,
                    order_free: false });
                best = travel;
                record.returned = Some(i);
                continue;
            }
            if hit.distance == 0.0 {
                if arms::on("touchContinues") {
                    continue;
                }
                record.returned = None;
                finish(record, sweeps, trace);
                return None;
            }
            record.returned = Some(i);
            finish(record, sweeps, trace);
            let point = std::array::from_fn(|k| a::sub(start[k], a::mul(normal[k], travel)));
            return Some(HitRecord { index: lane.index, start, direction, normal: normal.map(a::neg), point, collider_id, body_id,
                order_free: false });
        }
        finish(record, sweeps, trace);
        result
    }

    /// ParticleIntersect when the scene cannot give the broadphase order:
    /// every candidate whose box meets the lane's is swept (the sweeps are
    /// pure, so the ones the engine skips after an early return change
    /// nothing), and the selection runs over every order of the non-trigger
    /// colliders that hit (a miss or a trigger never changes the selection,
    /// wherever it stands). With one such collider there is one answer;
    /// with several, the answer is kept when every order gives the same hit
    /// or the same no-hit, bit for bit in point and normal; otherwise every
    /// distinct answer goes back for the comparison of what each writes.
    #[allow(clippy::too_many_arguments)]
    fn intersect_unordered(&self, lane: &Lane, direction: [f32; 3], (pc, pe): ([f32; 3], [f32; 3]), length: f32,
        candidates: &[Candidate], boxes: &[([f32; 3], [f32; 3])], scene: &mut dyn CollisionScene,
        trace: Option<&mut Trace>) -> Selected {
        let start = lane.query.start;
        let r = lane.query.radius;
        let skin = if arms::on("noSkin") { 0.0 } else { a::mul(r, SKIN) };
        let sphere_radius = a::max_nm(a::sub(r, skin), TINY);
        let distance = a::add(skin, length);
        let mut sweeps = Vec::new();
        let mut hits: Vec<Swept> = Vec::new();
        for (i, candidate) in candidates.iter().enumerate() {
            let (sc, se) = boxes[i];
            if !(0..3).all(|k| a::abs(a::sub(sc[k], pc[k])) <= a::add(pe[k], se[k])) {
                continue;
            }
            let request = SweepRequest { shape: i, particle: lane.index, origin: start, direction, distance, sphere_radius };
            sweeps.push(request);
            let Some(hit) = scene.sweep_sphere(&request) else {
                continue;
            };
            if candidate.is_trigger {
                continue;
            }
            hits.push(Swept { shape: i, normal: repaired_normal(&hit, sc), distance: hit.distance,
                travel: a::sub(hit.distance, skin) });
        }
        let answer = |order: &[usize]| -> Option<(usize, HitRecord)> {
            let (k, penetrating) = select_in_order(order.iter().map(|&k| (k, &hits[k])))?;
            let s = &hits[k];
            let candidate = &candidates[s.shape];
            let collider_id = candidate.collider_id;
            let body_id = candidate.body_id.unwrap_or(collider_id);
            let (normal, point) = if penetrating {
                (s.normal.map(a::neg), std::array::from_fn(|c| a::sub(start[c], a::mul(s.normal[c], s.travel))))
            } else {
                (s.normal, std::array::from_fn(|c| a::add(a::mul(direction[c], s.travel), start[c])))
            };
            Some((s.shape, HitRecord { index: lane.index, start, direction, normal, point, collider_id, body_id,
                order_free: hits.len() > 1 }))
        };
        let listed: Vec<usize> = (0..hits.len()).collect();
        let first = answer(&listed);
        let selected = if hits.len() > MAX_UNORDERED_HITS {
            Selected::Unbounded
        } else if hits.len() > 1 {
            let key = |x: &Option<(usize, HitRecord)>| x.map(|(shape, h)| {
                let id = if arms::on("orderFreeComparesIds") { shape } else { 0 };
                (id, h.normal.map(f32::to_bits), h.point.map(f32::to_bits))
            });
            let expected = key(&first);
            let mut answers: Vec<Option<HitRecord>> = vec![first.map(|(_, hit)| hit)];
            let mut keys = vec![expected];
            each_order(hits.len(), &mut |order| {
                let this = answer(order);
                let k = key(&this);
                if !keys.contains(&k) {
                    keys.push(k);
                    answers.push(this.map(|(_, hit)| hit));
                }
            });
            if answers.len() == 1 { Selected::OrderFree(answers[0]) } else { Selected::Alternatives(answers) }
        } else {
            Selected::Hit(first.map(|(_, hit)| hit))
        };
        if let Some(trace) = trace {
            trace.intersects.push(IntersectRecord {
                particle: lane.index,
                start,
                direction,
                aabb: [pc[0], pc[1], pc[2], pe[0], pe[1], pe[2]],
                length,
                radius: r,
                candidates: candidates.len(),
                returned: first.map(|(shape, _)| shape),
            });
            trace.sweeps.extend(sweeps);
        }
        selected
    }
}

/// What `find` selected over the call's lanes, in lane order.
#[derive(Default)]
struct Found {
    choices: Vec<Choice>,
    order_free: usize,
}

/// One lane's hit, or the distinct answers of the orders of its colliders.
enum Choice {
    Hit(HitRecord),
    Alternatives(Vec<Option<HitRecord>>),
}

/// One lane's selection.
enum Selected {
    Hit(Option<HitRecord>),
    /// Several colliders hit in an unknown order and every order agrees.
    OrderFree(Option<HitRecord>),
    /// Two orders of the colliders that hit give different hits: every
    /// distinct answer, the scene's listed order's first.
    Alternatives(Vec<Option<HitRecord>>),
    /// More colliders hit than the orders evaluated.
    Unbounded,
}

/// A world hit taken into simulation space by the owner inverse (the World
/// space's hit is already there).
fn to_simulation(owner: Option<OwnerPair>, mut hit: HitRecord) -> HitRecord {
    if let Some(owner) = owner {
        let simulated = if arms::on("hostNormalize") {
            host_normalized(owner.world_to_local, hit.point, hit.normal)
        } else {
            world_hit_to_simulation(owner.world_to_local, hit.index as usize, hit.point, hit.normal)
        };
        hit.point = simulated.point;
        hit.normal = simulated.normal;
    }
    hit
}

/// One non-trigger collider's hit, as the selection compares it.
#[derive(Clone, Copy)]
struct Swept {
    shape: usize,
    normal: [f32; 3],
    distance: f32,
    travel: f32,
}

/// A non-finite hit normal rebuilt as ParticleIntersect rebuilds it.
fn repaired_normal(hit: &SweepHit, centre: [f32; 3]) -> [f32; 3] {
    if hit.normal.iter().all(|v| v.is_finite()) {
        return hit.normal;
    }
    let v: [f32; 3] = std::array::from_fn(|k| a::sub(hit.position[k], centre[k]));
    let span = a::sqrt(a::add(a::add(a::mul(v[0], v[0]), a::mul(v[1], v[1])), a::mul(v[2], v[2])));
    if span > REPAIR_MIN { v.map(|x| a::div(x, span)) } else { [0.0, 0.0, 1.0] }
}

/// ParticleIntersect's selection over hits in one order: the key of the
/// selected hit and whether it is a penetrating start, or `None` for no hit
/// or a touching start.
fn select_in_order<'s>(order: impl Iterator<Item = (usize, &'s Swept)>) -> Option<(usize, bool)> {
    let mut best = f32::INFINITY;
    let mut result = None;
    for (k, s) in order {
        if !(s.travel < best) {
            continue;
        }
        if s.distance > 0.0 {
            result = Some((k, false));
            best = s.travel;
            continue;
        }
        if s.distance == 0.0 {
            return None;
        }
        return Some((k, true));
    }
    result
}

/// Calls `visit` with every order of `0..n` (Heap's algorithm).
fn each_order(n: usize, visit: &mut dyn FnMut(&[usize])) {
    let mut order: Vec<usize> = (0..n).collect();
    let mut c = vec![0usize; n];
    visit(&order);
    let mut i = 0;
    while i < n {
        if c[i] < i {
            if i % 2 == 0 {
                order.swap(0, i);
            } else {
                order.swap(c[i], i);
            }
            visit(&order);
            c[i] += 1;
            i = 0;
        } else {
            c[i] = 0;
            i += 1;
        }
    }
}

/// The range word the query stores: the count left after this call's lanes,
/// clamped at zero, truncated to 32 bits and rounded up to a multiple of four
/// with the signed idiom, then sign-extended.
fn range_word(s10: u64, count: u64) -> u64 {
    let left = s10.saturating_sub(count);
    if arms::on("stateUnbounded") {
        return left.div_ceil(4).wrapping_mul(4);
    }
    let w = left as u32 as i32;
    let mut q = w.wrapping_add(3);
    if q < 0 {
        q = w.wrapping_add(6);
    }
    (q & !3) as i64 as u64
}

/// The lane's segment box: centre and half extents plus the radius.
fn lane_box(query: &CollisionQuery) -> ([f32; 3], [f32; 3]) {
    let half: [f32; 3] = std::array::from_fn(|k| a::mul(a::sub(query.end[k], query.start[k]), 0.5));
    (std::array::from_fn(|k| a::add(half[k], query.start[k])),
        std::array::from_fn(|k| a::add(a::abs(half[k]), query.radius)))
}

/// A collider's box: centre and half extents plus the margin.
fn candidate_box(candidate: &Candidate) -> ([f32; 3], [f32; 3]) {
    let (b0, b1) = (candidate.bounds_min, candidate.bounds_max);
    (std::array::from_fn(|k| a::mul(a::add(b0[k], b1[k]), 0.5)),
        std::array::from_fn(|k| a::add(a::mul(a::sub(b1[k], b0[k]), 0.5), MARGIN)))
}

fn pack_words(pack: &[Lane; 4]) -> [u32; 32] {
    let mut words = [0u32; 32];
    for (l, lane) in pack.iter().enumerate() {
        words[l] = lane.index;
        for k in 0..3 {
            words[4 + 4 * k + l] = lane.query.start[k].to_bits();
            words[16 + 4 * k + l] = lane.query.end[k].to_bits();
        }
        words[28 + l] = lane.query.radius.to_bits();
    }
    words
}

/// Arm only: the hit normal renormalized by a host reciprocal square root.
fn host_normalized(inverse: QueryAffine, point: [f32; 3], normal: [f32; 3])
    -> crate::particle::collision_response::SuppliedHit {
    let v = inverse.vector_inverse(normal);
    let squared = a::add(a::add(a::mul(v[0], v[0]), a::mul(v[1], v[1])), a::mul(v[2], v[2]));
    let normal = if !(squared > f32::from_bits(0x0da2_4260)) {
        [0.0, 0.0, 1.0]
    } else {
        let scale = a::div(1.0, a::sqrt(squared));
        v.map(|x| a::mul(x, scale))
    };
    crate::particle::collision_response::SuppliedHit { particle_index: 0, point: inverse.point_inverse(point), normal }
}

#[cfg(test)]
mod tests;
