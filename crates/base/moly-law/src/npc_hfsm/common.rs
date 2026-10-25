//! Ordinary multi-character gathering. Geometry preserves member order while
//! assigning circle slots by the members' angles around the selected centre.

pub const ARRIVAL_DISTANCE: f32 = 0.25;
pub const FINAL_APPROACH_SPEED: f32 = 0.3;
pub const FINAL_APPROACH_EPSILON: f32 = 0.01;
pub const RESERVATION_DEADLINE: f32 = 30.0;
pub const GATHER_STALL_SECONDS: f32 = 15.0;
pub const RECRUIT_RETRY_SECONDS: f32 = 10.0;
pub const SHARED_PLAN_GRACE_SECONDS: f32 = 1.0;

pub fn distance(a: [f32; 3], b: [f32; 3]) -> f32 {
    let [x, y, z] = [a[0] - b[0], a[1] - b[1], a[2] - b[2]];
    (z * z + (x * x + y * y)).sqrt()
}

pub fn already_together(positions: &[[f32; 3]]) -> bool {
    positions.iter().enumerate().all(|(i, a)| positions.iter().skip(i + 1)
        .all(|b| distance(*a, *b) <= ARRIVAL_DISTANCE))
}

pub fn circle_positions(positions: &[[f32; 3]], center: [f32; 3]) -> Option<Vec<[f32; 3]>> {
    if !(2..=5).contains(&positions.len()) { return None; }
    let angles: Vec<f32> = positions.iter().map(|p| (p[2] - center[2]).atan2(p[0] - center[0])).collect();
    let mut order: Vec<usize> = (0..positions.len()).collect();
    order.sort_by(|a, b| angles[*a].total_cmp(&angles[*b]));
    // These literals intentionally retain the authored precision.
    let count = positions.len() as f32;
    let radius = 1.0 / (2.0 * (3.1416 / count).sin());
    let base = angles[order[0]];
    let mut output = positions.to_vec();
    for (slot, member) in order.into_iter().enumerate() {
        let angle = base + slot as f32 * 6.2832 / count;
        output[member] = [center[0] + radius * angle.cos(), center[1], center[2] + radius * angle.sin()];
    }
    Some(output)
}

pub fn approach_step(from: [f32; 3], target: [f32; 3], dt: f32) -> [f32; 3] {
    let length = distance(from, target);
    let step = dt * FINAL_APPROACH_SPEED;
    if length == 0.0 || (step >= 0.0 && length <= step) { return target; }
    std::array::from_fn(|i| from[i] + step * ((target[i] - from[i]) / length))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommonPhase { Gathering, Tweeting, WaitingTalkEnd, Complete }

#[derive(Debug, Clone, Copy)]
pub struct GatheringClock {
    pub elapsed: f32,
    pub last_progress: f32,
    pub deadline: f32,
}

impl GatheringClock {
    pub fn new(now: f32) -> Self {
        Self { elapsed: 0.0, last_progress: 0.0, deadline: now + RESERVATION_DEADLINE }
    }
    pub fn advance(&mut self, dt: f32, now: f32, pending: bool, moving: bool) -> bool {
        self.elapsed += dt;
        if pending && now >= self.deadline { return false; }
        if pending || moving { self.progress(); }
        self.elapsed - self.last_progress < GATHER_STALL_SECONDS
    }
    pub fn progress(&mut self) { self.last_progress = self.elapsed; }
    pub fn can_retry(&self) -> bool { self.elapsed < RECRUIT_RETRY_SECONDS }
}

/// Reservations are unique per member. Validity is tied to the live main
/// plan, rather than to the talk id (the same conversation can run again).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReservationKind { Common, MultiFixture, WhileDoingWait }

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReservationIdentity {
    pub kind: ReservationKind,
    pub plan: u64,
    pub main: u32,
}

pub fn valid_reservation(
    reservation: ReservationIdentity,
    active: Option<(u64, u32, bool)>,
    pending_upper: Option<(u64, u32)>,
) -> bool {
    match active {
        Some((plan, main, accepting)) if plan == reservation.plan && main == reservation.main => accepting,
        _ => pending_upper == Some((reservation.plan, reservation.main)),
    }
}
