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
}

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
}

/// One selected hit, in simulation coordinates after the owner inverse.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HitRecord {
    pub index: u32,
    pub start: [f32; 3],
    pub direction: [f32; 3],
    pub normal: [f32; 3],
    pub point: [f32; 3],
    pub collider_id: i32,
    pub body_id: i32,
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
        let mut hits = self.find(&packs, input.dt, to, scene, trace.as_deref_mut());
        if let Some(reason) = scene.refusal() {
            return Err(Refused::Scene(reason));
        }
        if let Some(owner) = owner {
            for hit in &mut hits {
                let simulated = if arms::on("hostNormalize") {
                    host_normalized(owner.world_to_local, hit.point, hit.normal)
                } else {
                    world_hit_to_simulation(owner.world_to_local, hit.index as usize, hit.point, hit.normal)
                };
                hit.point = simulated.point;
                hit.normal = simulated.normal;
            }
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
        mut trace: Option<&mut Trace>) -> Vec<HitRecord> {
        if self.max_shapes < 1 {
            return Vec::new();
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
        candidates.truncate(self.max_shapes as usize);
        if candidates.is_empty() {
            return Vec::new();
        }
        if let Some(trace) = trace.as_deref_mut() {
            trace.bounds_calls = candidates.len();
        }
        let boxes: Vec<_> = candidates.iter().map(candidate_box).collect();
        let mut hits = Vec::new();
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
                if let Some(hit) = self.intersect(lane, direction, lanes[l], length, &candidates, &boxes,
                    scene, trace.as_deref_mut()) {
                    hits.push(hit);
                }
            }
        }
        hits
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
            if !(travel < best) || candidate.is_trigger {
                continue;
            }
            let collider_id = candidate.collider_id;
            let body_id = candidate.body_id.unwrap_or(collider_id);
            if hit.distance > 0.0 {
                let point = std::array::from_fn(|k| a::add(a::mul(direction[k], travel), start[k]));
                result = Some(HitRecord { index: lane.index, start, direction, normal, point, collider_id, body_id });
                best = travel;
                record.returned = Some(i);
                continue;
            }
            if hit.distance == 0.0 {
                record.returned = None;
                finish(record, sweeps, trace);
                return None;
            }
            record.returned = Some(i);
            finish(record, sweeps, trace);
            let point = std::array::from_fn(|k| a::sub(start[k], a::mul(normal[k], travel)));
            return Some(HitRecord { index: lane.index, start, direction, normal: normal.map(a::neg), point, collider_id, body_id });
        }
        finish(record, sweeps, trace);
        result
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
