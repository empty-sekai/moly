//! Parent side of the sub-emitter birth and death events on the native birth
//! path.
//!
//! The parent's sub-emitter module runs at two points of every slice: after
//! the existing particles have been simulated and their deaths removed (over
//! the whole pool, the slice dt in every lane), and inside the birth of the
//! slice's newborns, once per new four-lane group after that group's
//! pre-simulation modules and its position and age update and before newborn
//! deaths are removed (each lane its own birth dt, padding lanes included).
//! Each call visits every cached birth edge in slot order and, per edge,
//! every particle of the range; the per-particle carry it writes moves with
//! the particle (it lives in `Side`).
//!
//! The first call is inside the module batch after simulation, which does not
//! test the stopped state, so a stopped system keeps recording for its
//! existing particles.
//!
//! Death events are recorded where the particle is killed, before its slot
//! is overwritten: in the kill pass of the existing particles (inside their
//! simulation, before the module batch after it) and in the newborn kill pass
//! (after the newborn groups' calls). Each killed particle visits every cached
//! death edge in slot order. The kill pass of the existing particles, too,
//! does not test the stopped state.
//!
//! The slot order of two edges of one trigger is the engine's runtime order of
//! the child instances; it is not in the export. With at most two birth edges
//! and death edges to distinct children it is not observable per child: each
//! edge owns its own carry (a death edge has none), every event of a particle
//! reseeds from the same words, and the children receive their own command
//! streams. Authored order is used.
//!
//! Commands go to the edge's child system. When the child is an installed
//! target of the same effect instance the edge is marked delivered and its
//! commands queue, in the order recorded, until the host hands them to the
//! target after every system's frame; otherwise they reach a refusing owner
//! that counts them and applies nothing. None of the parent's own particles
//! reads an event, so the parent stays exact either way.
use super::*;
use moly_law::particle::death_event::{record_death, DeathEmitEdge, DeathParent};
use moly_law::particle::sub_emission::{BirthEdgeLaw, EventOwner, EventParticle, SubEmitterCommand};

/// One cached birth edge: the child node it names, resolved against the
/// effect's systems at admission, and the count law read from that child.
#[derive(Clone, Debug)]
pub(crate) struct BirthEdge {
    pub(crate) target: String,
    pub(crate) law: BirthEdgeLaw,
}

/// One cached death edge: the child node it names, resolved like a birth
/// edge, and the event law read from that child's first burst.
#[derive(Clone, Debug)]
pub(crate) struct DeathEdge {
    pub(crate) target: String,
    pub(crate) law: DeathEmitEdge,
}

/// One cached collision edge: the child node it names, resolved like a birth
/// edge, and the RecordEmit law read from that child's first burst. Its
/// events are recorded by the CollisionModule's own call, not here.
#[derive(Clone, Debug)]
pub(crate) struct CollisionEdge {
    pub(crate) target: String,
    pub(crate) law: moly_law::particle::collision_event::CollisionEmitEdge,
}

/// Every real edge of a sub-emitter parent, resolved at admission: the birth
/// edges, the death edges and the collision edges, each in slot order.
#[derive(Clone, Debug, Default)]
pub(crate) struct EventEdges {
    pub(crate) births: Vec<BirthEdge>,
    pub(crate) deaths: Vec<DeathEdge>,
    pub(crate) collisions: Vec<CollisionEdge>,
}

impl EventEdges {
    /// Every child an edge names.
    pub(crate) fn targets(&self) -> impl Iterator<Item = &str> {
        self.births.iter().map(|edge| edge.target.as_str())
            .chain(self.deaths.iter().map(|edge| edge.target.as_str()))
            .chain(self.collisions.iter().map(|edge| edge.target.as_str()))
    }
}

/// What one edge has recorded and sent to its child owner.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct EdgeTally {
    pub(crate) records: u64,
    pub(crate) commands: u64,
    /// Sum of the command counts: the child births the source would make.
    pub(crate) births: u64,
}

/// One edge in its slot: the edge, whether its child is an installed target
/// that receives the commands, and what it recorded.
#[derive(Clone, Debug)]
pub(crate) struct EdgeSlot {
    pub(crate) edge: BirthEdge,
    pub(crate) delivered: bool,
    pub(crate) tally: EdgeTally,
}

/// One death edge in its slot, as [`EdgeSlot`] holds a birth edge.
#[derive(Clone, Debug)]
pub(crate) struct DeathSlot {
    pub(crate) edge: DeathEdge,
    pub(crate) delivered: bool,
    pub(crate) tally: EdgeTally,
}

#[derive(Clone, Copy, Debug)]
enum Slot {
    Birth(usize),
    Death(usize),
}

#[derive(Clone, Debug)]
pub(crate) struct BirthEvents {
    slots: Vec<EdgeSlot>,
    deaths: Vec<DeathSlot>,
    /// Commands of delivered edges not yet handed to their target, in the
    /// order recorded: the slot and the command.
    pending: Vec<(Slot, SubEmitterCommand)>,
    /// The first recording refusal. Afterwards the stream records nothing and
    /// its carries stay as they were; the parent keeps simulating.
    pub(crate) broken: Option<String>,
    #[cfg(test)]
    pub(crate) trace: Option<Vec<TraceCall>>,
    #[cfg(test)]
    pub(crate) death_trace: Option<Vec<DeathTraceCall>>,
}

/// One call as the recording saw it (test instrumentation only).
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct TraceCall {
    pub newborn: bool,
    /// First index of the newborn block (0 for the call after simulation).
    pub first: usize,
    /// Pool range as the runtime holds it at the call.
    pub start: usize,
    pub end: usize,
    /// The call's four lane times (a newborn group's birth times, padding
    /// lanes included; the slice dt otherwise).
    pub dt4: [f32; 4],
    pub owner: EventOwner,
    /// The range's particles and carries before the call.
    pub particles: Vec<(EventParticle, [f32; 2])>,
    pub records: Vec<(usize, usize, moly_law::particle::sub_emission::BirthEventRecord)>,
}

/// One kill pass as the death recording saw it (test instrumentation only).
#[cfg(test)]
#[derive(Clone, Debug)]
pub(crate) struct DeathTraceCall {
    pub newborn: bool,
    /// First index of the newborn block (0 for the existing particles).
    pub first: usize,
    pub owner: EventOwner,
    /// The killed particles in removal order, each with its records in slot
    /// order.
    pub deaths: Vec<(DeathParent, Vec<moly_law::particle::death_event::RecordedDeath>)>,
}

impl BirthEvents {
    /// `edges` in slot order; the admission keeps one or two. Only two carry
    /// slots exist per particle, so a longer list records nothing.
    pub(crate) fn new(edges: Vec<BirthEdge>) -> Self {
        Self::with_edges(EventEdges { births: edges, ..EventEdges::default() })
    }

    /// Birth and death edges, each in slot order.
    pub(crate) fn with_edges(edges: EventEdges) -> Self {
        let broken = (edges.births.len() > 2)
            .then(|| format!("{} birth edges: only two own a carry", edges.births.len()));
        let slots = edges.births.into_iter()
            .map(|edge| EdgeSlot { edge, delivered: false, tally: EdgeTally::default() })
            .collect();
        let deaths = edges.deaths.into_iter()
            .map(|edge| DeathSlot { edge, delivered: false, tally: EdgeTally::default() })
            .collect();
        Self {
            slots,
            deaths,
            pending: Vec::new(),
            broken,
            #[cfg(test)]
            trace: None,
            #[cfg(test)]
            death_trace: None,
        }
    }

    pub(crate) fn slots(&self) -> &[EdgeSlot] {
        &self.slots
    }

    pub(crate) fn death_slots(&self) -> &[DeathSlot] {
        &self.deaths
    }

    /// Whether any death edge is cached (the kill passes then hand their
    /// killed particles over).
    pub(crate) fn records_deaths(&self) -> bool {
        !self.deaths.is_empty()
    }

    /// Marks every edge naming `target` delivered; false when no edge names it.
    pub(crate) fn deliver_to(&mut self, target: &str) -> bool {
        let mut found = false;
        for slot in self.slots.iter_mut().filter(|slot| slot.edge.target == target) {
            slot.delivered = true;
            found = true;
        }
        for slot in self.deaths.iter_mut().filter(|slot| slot.edge.target == target) {
            slot.delivered = true;
            found = true;
        }
        found
    }

    /// The queued commands of delivered edges with their target, in the order
    /// recorded; the queue is left empty.
    pub(crate) fn take_commands(&mut self) -> Vec<(String, SubEmitterCommand)> {
        let (births, deaths) = (&self.slots, &self.deaths);
        self.pending.drain(..).map(|(slot, command)| {
            let target = match slot {
                Slot::Birth(slot) => &births[slot].edge.target,
                Slot::Death(slot) => &deaths[slot].edge.target,
            };
            (target.clone(), command)
        }).collect()
    }

    /// The call after simulation: the whole pool, the slice dt in every lane.
    pub(super) fn record_existing(&mut self, system: &mut Runtime, dt: f32, accumulated: f32,
        emission_word: u32, ctx: &Context) {
        let owner = event_owner(system, accumulated, emission_word, ctx);
        let end = system.pool.len();
        self.record(system, 0, 0, end, [dt; 4], owner, false);
    }

    /// The call of one newborn group: pool indices `start..end` (at most
    /// four), `first` the first newborn index and `dts[i - first]` the birth
    /// dt of index i; the newborn block holds whole four-lane groups.
    pub(super) fn record_newborn(&mut self, system: &mut Runtime, first: usize, start: usize, end: usize,
        dts: &[f32], accumulated: f32, emission_word: u32, ctx: &Context) {
        let owner = event_owner(system, accumulated, emission_word, ctx);
        let dt4 = std::array::from_fn(|lane| dts[start - first + lane]);
        self.record(system, first, start, end, dt4, owner, true);
    }

    /// The death events of one kill pass: `deaths` in removal order, each as
    /// it was read before its slot was overwritten; `first` is the first
    /// newborn index for the newborn pass (0 otherwise). The parent's pending
    /// time and emission word are the ones its sub-emitter calls of the same
    /// point read.
    pub(super) fn record_deaths(&mut self, system: &Runtime, deaths: &[DeathParent], newborn: bool, first: usize,
        accumulated: f32, emission_word: u32, ctx: &Context) {
        if self.broken.is_some() || self.deaths.is_empty() {
            return;
        }
        let owner = event_owner(system, accumulated, emission_word, ctx);
        #[cfg(test)]
        let mut call = self.death_trace.as_ref().map(|_| DeathTraceCall { newborn, first, owner, deaths: Vec::new() });
        #[cfg(not(test))]
        let _ = (newborn, first);
        for dying in deaths {
            #[cfg(test)]
            let mut records = Vec::new();
            for (slot, death_slot) in self.deaths.iter_mut().enumerate() {
                let recorded = record_death(&death_slot.edge.law, slot, dying, &owner);
                let tally = &mut death_slot.tally;
                tally.records += 1;
                if let Some(commands) = &recorded.commands {
                    tally.commands += commands.len() as u64;
                    tally.births += commands.iter().map(|c| c.count).sum::<u64>();
                    if death_slot.delivered {
                        self.pending.extend(commands.iter().map(|&command| (Slot::Death(slot), command)));
                    }
                }
                #[cfg(test)]
                records.push(recorded);
            }
            #[cfg(test)]
            if let Some(call) = call.as_mut() {
                call.deaths.push((*dying, records));
            }
        }
        #[cfg(test)]
        if let (Some(trace), Some(call)) = (self.death_trace.as_mut(), call) {
            trace.push(call);
        }
    }

    fn record(&mut self, system: &mut Runtime, first: usize, start: usize, end: usize, dt4: [f32; 4],
        owner: EventOwner, newborn: bool) {
        if self.broken.is_some() {
            return;
        }
        let dt = |index: usize| if newborn { dt4[index - start] } else { dt4[0] };
        // Runtime X is the reflection of source X; the sign flip is exact both ways.
        let source = |v: [f32; 3]| [-v[0], v[1], v[2]];
        let read = |system: &Runtime, index: usize| {
            let particle = &system.pool[index];
            let side = &system.side[index];
            EventParticle {
                seed: side.seed,
                age_percent: particle.age_percent,
                inverse_lifetime: particle.inverse_lifetime,
                position: source(particle.position),
                velocity: source(side.total_velocity),
            }
        };
        #[cfg(test)]
        let mut call = self.trace.as_ref().map(|_| TraceCall {
            newborn, first, start, end, dt4,
            owner,
            particles: (start..end).map(|i| (read(&*system, i), system.side[i].emit_carry)).collect(),
            records: Vec::new(),
        });
        #[cfg(not(test))]
        let _ = first;
        'edges: for (slot, edge_slot) in self.slots.iter_mut().enumerate() {
            let edge = &edge_slot.edge;
            for index in start..end {
                let particle = read(&*system, index);
                let mut carry = system.side[index].emit_carry[slot];
                match edge.law.record(&particle, &mut carry, dt(index), &owner) {
                    Ok(None) => {}
                    Ok(Some(record)) => {
                        system.side[index].emit_carry[slot] = carry;
                        let tally = &mut edge_slot.tally;
                        tally.records += 1;
                        if let Some(commands) = &record.commands {
                            tally.commands += commands.len() as u64;
                            tally.births += commands.iter().map(|c| c.count).sum::<u64>();
                            if edge_slot.delivered {
                                self.pending.extend(commands.iter().map(|&command| (Slot::Birth(slot), command)));
                            }
                        }
                        #[cfg(test)]
                        if let Some(call) = call.as_mut() {
                            call.records.push((slot, index, record));
                        }
                    }
                    Err(refused) => {
                        error!(effect=%system.effect, node=%system.node, target=%edge.target, ?refused,
                            "sub-emitter birth event refused; the event stream stops, the parent keeps simulating");
                        self.broken = Some(format!("{}: {refused:?}", edge.target));
                        break 'edges;
                    }
                }
            }
        }
        #[cfg(test)]
        if let (Some(trace), Some(call)) = (self.trace.as_mut(), call) {
            trace.push(call);
        }
    }
}

/// The dying particle as the death event reads it, before its slot is
/// overwritten: its seed, position and persistent and animated velocity, and
/// its stored size, age and inverse lifetime (read by an edge that inherits
/// the size). Runtime X is the reflection of source X.
pub(super) fn dying(index: usize, particle: &Particle, side: &Side) -> DeathParent {
    let source = |v: [f32; 3]| [-v[0], v[1], v[2]];
    DeathParent {
        index,
        seed: side.seed,
        position: source(particle.position),
        velocity: source(particle.velocity),
        animated: source(side.animated),
        size: side.size,
        age_percent: particle.age_percent,
        inverse_lifetime: particle.inverse_lifetime,
    }
}

/// The parent state the recording reads at a call: its local-to-world matrix
/// in source axes (the owner the Shape store also uses), its space, the time
/// it still has to simulate and the first word of its emission stream.
fn event_owner(system: &Runtime, accumulated: f32, emission_word: u32, ctx: &Context) -> EventOwner {
    EventOwner {
        local_to_world: birth::source_owner_matrix(&compose_to_world(system, ctx)),
        world_space: system.emitter.simulation_space == SimulationSpace::World,
        accumulated_time: accumulated,
        emission_word,
    }
}

/// One newborn lane of a child Emit as the target's own sub-emitter call reads
/// it before the command commits: the particle in source axes and its carries.
#[derive(Clone, Copy, Debug)]
pub(super) struct StagedLane {
    pub(super) particle: EventParticle,
    pub(super) carry: [f32; 2],
}

impl BirthEvents {
    /// The newborn call of one four-lane group inside a child Emit of this
    /// target: lanes `start..end` of the staged newborn block, `dt4` the
    /// group's four birth times (padding lanes included), every cached birth
    /// edge in slot order and, per edge, every lane of the range; the carries
    /// are written back into the staged lanes. The caller records into a copy
    /// and keeps it only when the whole command succeeds.
    pub(super) fn record_staged(&mut self, lanes: &mut [StagedLane], start: usize, end: usize, dt4: [f32; 4],
        owner: EventOwner) {
        if self.broken.is_some() {
            return;
        }
        'edges: for (slot, edge_slot) in self.slots.iter_mut().enumerate() {
            for index in start..end {
                let lane = &mut lanes[index];
                let mut carry = lane.carry[slot];
                match edge_slot.edge.law.record(&lane.particle, &mut carry, dt4[index - start], &owner) {
                    Ok(None) => {}
                    Ok(Some(record)) => {
                        lane.carry[slot] = carry;
                        let tally = &mut edge_slot.tally;
                        tally.records += 1;
                        if let Some(commands) = &record.commands {
                            tally.commands += commands.len() as u64;
                            tally.births += commands.iter().map(|c| c.count).sum::<u64>();
                            if edge_slot.delivered {
                                self.pending.extend(commands.iter().map(|&command| (Slot::Birth(slot), command)));
                            }
                        }
                    }
                    Err(refused) => {
                        self.broken = Some(format!("{}: {refused:?}", edge_slot.edge.target));
                        break 'edges;
                    }
                }
            }
        }
    }

    /// Whether any delivered command waits for its target.
    pub(crate) fn has_pending(&self) -> bool {
        !self.pending.is_empty()
    }
}
