//! The talk lens: while a conversation plays, the field camera is placed in
//! front of whoever speaks, every frame, from the way that body faces. A line
//! spoken toward the player is framed from the front, from the player's side;
//! any other line is turned a little toward the listeners' side so the shot
//! is a three-quarter front view. When another cast member stands in the
//! line of sight, the eye turns aside by `swing` steps until the speaker is
//! seen. The framing is computed in the same frame the speaker changes, not
//! polled from outside. Between a line's voice ending and the next line
//! being released the lens already aims at the next speaker; before a talk
//! starts it frames the unit the step named.
//!
//! A wide shot of the whole cast replaces the close-up while a voice line
//! has no single speaker, and for `wide_hold` seconds after a cast member
//! other than the speaker starts an animation in the middle of a line.
//!
//! The eye does not jump between framings: it eases toward each new one at
//! `glide` per second, and while a framing holds it closes in on its target
//! by `push` of the distance.
//!
//! The pose is written on the camera lease ([`moly_features::lease`]) into
//! [`moly_features::camera_hold::CameraHold`], which is the one place it
//! reaches the camera. A placed fixture standing between the eye and a cast
//! member fades out while `clear` is on: its materials take the fixture
//! programs' own Bayer dither with a lowered alpha, and go back once the
//! fixture no longer stands in the way. The fixture a single character is
//! talking about is the subject of the shot and never fades.

use std::collections::HashMap;

use bevy::asset::AssetId;
use bevy::camera::primitives::Aabb;
use bevy::prelude::*;
use moly_features::camera_hold::{CameraHold, HeldPose};
use moly_features::fixture_material::FixtureMaterial;
use moly_features::markers::AvatarRoot;
use moly_features::render_inputs::FixtureRoot;
use moly_features::site_material::SiteMaterial;
use moly_features::site_object_dither::{dither_level, level_alpha, with_state};

/// The lens's framing, read from `camera.talk_lens`.
#[derive(Clone, Debug)]
pub struct TalkLens {
    /// Metres from the speaker to the eye.
    pub distance: f32,
    /// Eye height above the speaker's feet.
    pub height: f32,
    /// Height above the feet the lens looks at.
    pub target: f32,
    pub fov: f32,
    /// Radians the eye is turned from straight ahead of the speaker.
    pub swing: f32,
    /// The unit framed while no conversation plays.
    pub idle_unit: Option<u32>,
    /// Whether the wide shot of the whole cast is used at all.
    pub wide: bool,
    /// Seconds the wide shot holds after a listener's reaction starts.
    pub wide_hold: f32,
    /// Rate per second at which the eye eases toward a new framing; 0 cuts.
    pub glide: f32,
    /// Fraction of the distance the eye closes in while a framing holds.
    pub push: f32,
    /// Whether placed fixtures standing between the eye and the cast fade
    /// out.
    pub clear: bool,
}

/// The lens set by a `camera.talk_lens` step, and the program that set it.
#[derive(Resource)]
pub struct TalkLensState {
    pub lens: TalkLens,
    pub holder: String,
}

/// A listener's animation counts as a reaction when it starts this long
/// after the line began; the ones that start with the line are its posture.
const REACTION_AFTER: f32 = 0.5;

/// Screen aspect the wide shot fits the cast into.
const ASPECT: f32 = 16.0 / 9.0;

/// A speaker faces the player when its facing is within this angle of the
/// direction to the player (cosine); once framed from the front the wider
/// angle keeps it there, so a body settling around the edge does not swap
/// the framing back and forth.
const ADDRESS_COS: f32 = 0.766;
const ADDRESS_KEEP_COS: f32 = 0.643;

/// Seconds a held framing takes to close in by `push`.
const PUSH_SECS: f32 = 6.0;

/// The alpha a blocking fixture fades to.
const CLEAR_ALPHA: f32 = 0.3;
/// Alpha change per second while a fixture fades out or back in.
const FADE_PER_SECOND: f32 = 3.0;
/// Heights above a cast member's feet the eye must see.
const BODY_HEIGHTS: [f32; 2] = [0.35, 0.8];
/// A fixture this close behind the body along the line of sight does not
/// count as blocking it (the body stands at or in front of it).
const BODY_MARGIN: f32 = 0.25;
/// The dither level of alpha 1: every fragment drawn.
const OPAQUE: u32 = 16;

pub fn install(app: &mut App) {
    app.init_resource::<FixtureBounds>()
        .add_systems(
            PostUpdate,
            (measure_fixtures, follow_speaker, clear_view).chain().after(moly_features::talk_camera::advance),
        );
}

/// What the lens is aimed at; a change starts a new push-in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Framing {
    Idle(u32),
    Speaker { unit: u32, front: bool },
    Wide,
}

/// The eye as shown, eased toward the framing the lens asks for.
#[derive(Default)]
struct LensMotion {
    shown: Option<HeldPose>,
    framing: Option<Framing>,
    since: f32,
}

impl LensMotion {
    fn reset(&mut self) {
        *self = Self::default();
    }

    fn step(&mut self, lens: &TalkLens, framing: Framing, wanted: HeldPose, dt: f32) -> HeldPose {
        if self.framing == Some(framing) {
            self.since += dt;
        } else {
            self.framing = Some(framing);
            self.since = 0.0;
        }
        let t = (self.since / PUSH_SECS).clamp(0.0, 1.0);
        let close = lens.push * t * t * (3.0 - 2.0 * t);
        let wanted = HeldPose {
            eye: wanted.target + (wanted.eye - wanted.target) * (1.0 - close),
            ..wanted
        };
        let shown = match self.shown {
            Some(prev) if lens.glide > 0.0 => {
                let k = 1.0 - (-lens.glide * dt).exp();
                HeldPose {
                    eye: prev.eye.lerp(wanted.eye, k),
                    target: prev.target.lerp(wanted.target, k),
                    up: wanted.up,
                    fov_degrees: prev.fov_degrees + (wanted.fov_degrees - prev.fov_degrees) * k,
                }
            }
            _ => wanted,
        };
        self.shown = Some(shown);
        shown
    }
}

/// PostUpdate, after the talk camera: the lens's pose for the frame.
#[allow(clippy::too_many_arguments)]
fn follow_speaker(
    lens: Option<Res<TalkLensState>>,
    talk: Option<Res<moly_features::talk::ActiveTalk>>,
    window: Option<Res<moly_features::talk_window::TalkWindowState>>,
    time: Res<Time>,
    characters: Query<(&moly_features::npc::CharacterUnitId, &GlobalTransform)>,
    bodies: Query<&GlobalTransform>,
    player: Query<&GlobalTransform, With<AvatarRoot>>,
    fixtures: Res<FixtureBounds>,
    mut motion: Local<LensMotion>,
    mut hold: ResMut<CameraHold>,
) {
    let Some(state) = lens else {
        motion.reset();
        return;
    };
    // The lease decides: a lens whose camera was taken no longer writes it.
    if hold.holder.as_deref() != Some(state.holder.as_str()) {
        motion.reset();
        return;
    }
    let lens = &state.lens;
    let player = player.iter().next().map(|at| at.translation().xz());
    let aimed = match talk.as_deref() {
        Some(talk) => aim(lens, talk, window.as_deref(), time.elapsed_secs(), &bodies, &fixtures, player, motion.framing),
        None => lens.idle_unit.and_then(|unit| {
            let (_, body) = characters.iter().find(|(id, _)| id.0 == unit)?;
            let (out, _) = front(lens, body, Vec2::ZERO, None, false)?;
            Some((Framing::Idle(unit), place(lens, body, out)))
        }),
    };
    match aimed {
        Some((framing, wanted)) => {
            hold.pose = Some(motion.step(lens, framing, wanted, time.delta_secs()));
        }
        // Between talks the next one starts from its own framing, not from
        // a glide out of the last one.
        None if talk.is_none() => motion.reset(),
        None => {}
    }
}

/// The framing for the line now playing and the pose it asks for.
fn aim(
    lens: &TalkLens,
    talk: &moly_features::talk::ActiveTalk,
    window: Option<&moly_features::talk_window::TalkWindowState>,
    now: f32,
    bodies: &Query<&GlobalTransform>,
    fixtures: &FixtureBounds,
    player: Option<Vec2>,
    current: Option<Framing>,
) -> Option<(Framing, HeldPose)> {
    let participants = talk.participants();
    let release_pending = window.is_some_and(moly_features::talk_window::TalkWindowState::release_pending);
    let next = release_pending.then(|| talk.upcoming_line_speaker()).flatten();
    let &(unit, entity) = next
        .or_else(|| talk.line_speaker())
        .or_else(|| talk.voice_speaker())
        .and_then(|unit| participants.iter().find(|(who, _)| *who == unit))
        .or_else(|| participants.first())?;
    let body = bodies.get(entity).ok()?;
    let feet = body.translation();
    let mut listeners = Vec2::ZERO;
    let mut count = 0.0;
    for (who, other) in participants {
        if *who == unit {
            continue;
        }
        if let Ok(at) = bodies.get(*other) {
            listeners += at.translation().xz();
            count += 1.0;
        }
    }
    let toward = if count > 0.0 { (listeners / count - feet.xz()).normalize_or_zero() } else { Vec2::ZERO };
    let was_front = current == Some(Framing::Speaker { unit, front: true });
    let (out, addressed) = front(lens, body, toward, player, was_front)?;
    let group_line = if release_pending { talk.upcoming_line_is_group() } else { talk.line_is_group() };
    let reacting =
        !release_pending && talk.late_reaction(now, REACTION_AFTER).is_some_and(|(_, ago)| ago < lens.wide_hold);
    if lens.wide && (group_line || reacting) {
        let members: Vec<Vec3> = participants
            .iter()
            .filter_map(|(_, member)| bodies.get(*member).ok().map(GlobalTransform::translation))
            .collect();
        if let Some(wide) = place_wide(lens, &members, out) {
            return Some((Framing::Wide, wide));
        }
    }
    // A single character talking about a fixture: the fixture is half of the
    // shot.
    if participants.len() == 1 {
        let prop = talk.fixture_instances().iter().find_map(|(_, fixture)| fixtures.of(*fixture));
        if let Some(pose) = prop.and_then(|prop| place_with_prop(lens, feet, out, prop)) {
            return Some((Framing::Speaker { unit, front: addressed }, pose));
        }
    }
    let others: Vec<Vec3> = participants
        .iter()
        .filter(|(who, _)| *who != unit)
        .filter_map(|(_, other)| bodies.get(*other).ok().map(GlobalTransform::translation))
        .collect();
    let out = unblocked(lens, feet, out, &others);
    Some((Framing::Speaker { unit, front: addressed }, place(lens, body, out)))
}

/// Height above the speaker's feet the lens aims at in a shot with a fixture.
const PROP_AIM: f32 = 0.1;

/// The speaker and a fixture's bounds in one frame: seen from between the
/// speaker's front (`side`) and the side of the line from the speaker to the
/// fixture, far enough back for the wider of the two and the fixture's
/// height, aimed low between them.
fn place_with_prop(lens: &TalkLens, feet: Vec3, side: Vec2, (min, max): (Vec3, Vec3)) -> Option<HeldPose> {
    let body = feet.xz();
    let prop = Vec2::new(min.x + max.x, min.z + max.z) / 2.0;
    let axis = (prop - body).normalize_or_zero();
    if axis == Vec2::ZERO {
        return None;
    }
    let mut across = axis.perp();
    if across.dot(side) < 0.0 {
        across = -across;
    }
    let out = (across + side).normalize_or(across);
    let centre = (body + prop) / 2.0;
    let corners = [
        Vec2::new(min.x, min.z),
        Vec2::new(min.x, max.z),
        Vec2::new(max.x, min.z),
        Vec2::new(max.x, max.z),
    ];
    let radius = corners
        .iter()
        .chain(std::iter::once(&body))
        .map(|point| point.distance(centre))
        .fold(0.0, f32::max);
    let top = (max.y - feet.y).max(1.0);
    let tan_half = (lens.fov.to_radians() / 2.0).tan();
    let width_fit = (2.0 * radius + 1.0) / (2.0 * tan_half * ASPECT);
    // The lens aims low so the pair stands in the upper part of the frame,
    // clear of the dialogue box, and stands back far enough for the top of
    // the fixture to fit in the upper half.
    let height_fit = top * 1.15 / tan_half;
    let distance = width_fit.max(height_fit).max(lens.distance);
    let aim = feet.y + PROP_AIM;
    Some(HeldPose {
        eye: Vec3::new(
            centre.x + out.x * distance,
            aim + (lens.height - lens.target).max(0.0),
            centre.y + out.y * distance,
        ),
        target: Vec3::new(centre.x, aim, centre.y),
        up: Vec3::Y,
        fov_degrees: lens.fov,
    })
}

/// Another cast member standing closer than this to the line of sight hides
/// the speaker.
const BODY_RADIUS: f32 = 0.35;
/// Heights above the feet where a cast member's body and the speaker's face
/// are taken.
const BODY_CENTRE: f32 = 0.5;
const FACE: f32 = 0.6;

/// `out`, or the nearest turn of it either way by `swing` steps whose line
/// of sight to the speaker passes no other cast member; `out` itself when
/// every candidate is blocked.
fn unblocked(lens: &TalkLens, feet: Vec3, out: Vec2, others: &[Vec3]) -> Vec2 {
    let turn = lens.swing.max(0.35);
    let candidates = [0.0, turn, -turn, 2.0 * turn, -2.0 * turn];
    for angle in candidates {
        let (sin, cos) = angle.sin_cos();
        let candidate = Vec2::new(out.x * cos - out.y * sin, out.x * sin + out.y * cos);
        let eye = Vec3::new(
            feet.x + candidate.x * lens.distance,
            feet.y + lens.height,
            feet.z + candidate.y * lens.distance,
        );
        let face = feet + Vec3::Y * FACE;
        if !others.iter().any(|other| hides(eye, face, *other + Vec3::Y * BODY_CENTRE)) {
            return candidate;
        }
    }
    out
}

/// Whether a body centred at `body` stands on the segment from `eye` to `face`
/// (closer than `BODY_RADIUS`, short of the face).
fn hides(eye: Vec3, face: Vec3, body: Vec3) -> bool {
    let along = face - eye;
    let length_squared = along.length_squared();
    if length_squared < 1e-6 {
        return false;
    }
    let t = ((body - eye).dot(along) / length_squared).clamp(0.0, 1.0);
    t < 0.95 && (eye + along * t).distance(body) < BODY_RADIUS
}

/// The eye in front of the body, `out` being the horizontal direction from
/// the body to the eye.
fn place(lens: &TalkLens, body: &GlobalTransform, out: Vec2) -> HeldPose {
    let feet = body.translation();
    HeldPose {
        eye: Vec3::new(feet.x + out.x * lens.distance, feet.y + lens.height, feet.z + out.y * lens.distance),
        target: Vec3::new(feet.x, feet.y + lens.target, feet.z),
        up: Vec3::Y,
        fov_degrees: lens.fov,
    }
}

/// The horizontal direction from the body to the eye, and whether the body
/// faces the player. Facing the player, the eye stands on the player's side,
/// straight ahead of the body; otherwise ahead of the body, swung toward the
/// listeners' side (`toward`, zero when unknown).
fn front(lens: &TalkLens, body: &GlobalTransform, toward: Vec2, player: Option<Vec2>, was_front: bool) -> Option<(Vec2, bool)> {
    let facing = (body.compute_transform().rotation * Vec3::Z).xz().normalize_or_zero();
    if let Some(player) = player {
        let to_player = (player - body.translation().xz()).normalize_or_zero();
        let keep = if was_front { ADDRESS_KEEP_COS } else { ADDRESS_COS };
        if facing != Vec2::ZERO && to_player != Vec2::ZERO && facing.dot(to_player) >= keep {
            return Some((to_player, true));
        }
    }
    let ahead = if facing == Vec2::ZERO { toward } else { facing };
    if ahead == Vec2::ZERO {
        return None;
    }
    let side = if ahead.perp_dot(toward) < 0.0 { -1.0 } else { 1.0 };
    let (sin, cos) = (lens.swing * side).sin_cos();
    Some((Vec2::new(ahead.x * cos - ahead.y * sin, ahead.x * sin + ahead.y * cos), false))
}

/// The whole cast in one frame, seen from the side of the line between the
/// two members farthest apart that the close-ups use (`side` is the
/// close-up's eye direction). None when fewer than two members stand apart.
fn place_wide(lens: &TalkLens, members: &[Vec3], side: Vec2) -> Option<HeldPose> {
    let mut axis = Vec2::ZERO;
    for (i, a) in members.iter().enumerate() {
        for b in &members[i + 1..] {
            let d = Vec2::new(b.x - a.x, b.z - a.z);
            if d.length() > axis.length() {
                axis = d;
            }
        }
    }
    let axis = axis.normalize_or_zero();
    if axis == Vec2::ZERO {
        return None;
    }
    let mut out = axis.perp();
    if out.dot(side) < 0.0 {
        out = -out;
    }
    let count = members.len() as f32;
    let centre = members.iter().fold(Vec3::ZERO, |sum, m| sum + *m) / count;
    let radius = members
        .iter()
        .map(|m| Vec2::new(m.x - centre.x, m.z - centre.z).length())
        .fold(0.0, f32::max);
    let tan_half_width = (lens.fov.to_radians() / 2.0).tan() * ASPECT;
    let distance = ((2.0 * radius + 1.6) / (2.0 * tan_half_width)).max(lens.distance * 1.5);
    let feet = members.iter().map(|m| m.y).fold(f32::MAX, f32::min);
    Some(HeldPose {
        eye: Vec3::new(centre.x + out.x * distance, feet + lens.height, centre.z + out.y * distance),
        target: Vec3::new(centre.x, feet + lens.target * 0.75, centre.z),
        up: Vec3::Y,
        fov_degrees: lens.fov,
    })
}

/// Each placed fixture's world bounds (the union of its meshes' bounds) and
/// its meshes, measured while the talk lens is on.
#[derive(Resource, Default)]
pub struct FixtureBounds {
    bounds: HashMap<Entity, (Vec3, Vec3)>,
    meshes: HashMap<Entity, Vec<Entity>>,
    owner: HashMap<Entity, Option<Entity>>,
}

impl FixtureBounds {
    /// The world bounds (min, max) of a placed fixture's root.
    pub fn of(&self, root: Entity) -> Option<(Vec3, Vec3)> {
        self.bounds.get(&root).copied()
    }
}

/// The fixtures faded so far, the materials their meshes had, and the shared
/// material copies per base and dither level.
#[derive(Default)]
pub struct LensClearState {
    alpha: HashMap<Entity, f32>,
    fixture_base: HashMap<Entity, Handle<FixtureMaterial>>,
    site_base: HashMap<Entity, Handle<SiteMaterial>>,
    fixture_copies: HashMap<(AssetId<FixtureMaterial>, u32), Handle<FixtureMaterial>>,
    site_copies: HashMap<(AssetId<SiteMaterial>, u32), Handle<SiteMaterial>>,
}

/// PostUpdate: measures the placed fixtures while the talk lens is on (and
/// while a faded fixture still has to come back).
pub fn measure_fixtures(
    lens: Option<Res<TalkLensState>>,
    meshes: Query<(Entity, &GlobalTransform, &Aabb), Or<(With<MeshMaterial3d<FixtureMaterial>>, With<MeshMaterial3d<SiteMaterial>>)>>,
    parents: Query<&ChildOf>,
    roots: Query<(), With<FixtureRoot>>,
    mut fixtures: ResMut<FixtureBounds>,
) {
    if lens.is_none() && fixtures.bounds.is_empty() {
        return;
    }
    let fixtures = &mut *fixtures;
    fixtures.bounds.clear();
    fixtures.meshes.clear();
    if lens.is_none() {
        fixtures.owner.clear();
        return;
    }
    fixtures.owner.retain(|mesh, _| meshes.contains(*mesh));
    for (mesh, transform, aabb) in &meshes {
        let owner = *fixtures
            .owner
            .entry(mesh)
            .or_insert_with(|| fixture_root(mesh, &parents, &roots));
        let Some(root) = owner else { continue };
        let (min, max) = world_bounds(transform, aabb);
        let entry = fixtures.bounds.entry(root).or_insert((min, max));
        entry.0 = entry.0.min(min);
        entry.1 = entry.1.max(max);
        fixtures.meshes.entry(root).or_default().push(mesh);
    }
}

/// PostUpdate, after the lens has placed the eye: the fixtures between the
/// eye and the cast fade.
#[allow(clippy::too_many_arguments)]
pub fn clear_view(
    lens: Option<Res<TalkLensState>>,
    talk: Option<Res<moly_features::talk::ActiveTalk>>,
    hold: Res<CameraHold>,
    time: Res<Time>,
    bodies: Query<&GlobalTransform>,
    fixtures: Res<FixtureBounds>,
    mut fixture_handles: Query<&mut MeshMaterial3d<FixtureMaterial>>,
    mut site_handles: Query<&mut MeshMaterial3d<SiteMaterial>>,
    mut fixture_materials: ResMut<Assets<FixtureMaterial>>,
    mut site_materials: ResMut<Assets<SiteMaterial>>,
    mut state: Local<LensClearState>,
) {
    let looking = lens.as_ref().is_some_and(|lens| lens.lens.clear && hold.holder.as_deref() == Some(lens.holder.as_str()));
    let eye = hold.pose.map(|pose| pose.eye);
    let targets: Vec<Vec3> = match (looking, eye, talk.as_deref()) {
        (true, Some(_), Some(talk)) => talk
            .participants()
            .iter()
            .filter_map(|(_, body)| bodies.get(*body).ok().map(GlobalTransform::translation))
            .flat_map(|feet| BODY_HEIGHTS.map(|height| feet + Vec3::Y * height))
            .collect(),
        _ => Vec::new(),
    };
    if targets.is_empty() && state.alpha.is_empty() {
        return;
    }
    let bounds = &fixtures.bounds;
    let members = &fixtures.meshes;
    if bounds.is_empty() {
        // The lens is off: every faded mesh takes its own material back now.
        for (mesh, base) in state.fixture_base.drain() {
            if let Ok(mut handle) = fixture_handles.get_mut(mesh) {
                handle.0 = base;
            }
        }
        for (mesh, base) in state.site_base.drain() {
            if let Ok(mut handle) = site_handles.get_mut(mesh) {
                handle.0 = base;
            }
        }
        state.alpha.clear();
        state.fixture_copies.clear();
        state.site_copies.clear();
        return;
    }

    // A single character talking about a fixture: that fixture is what the
    // shot shows, so it never fades.
    let subject: Vec<Entity> = match talk.as_deref() {
        Some(talk) if talk.participants().len() == 1 => {
            talk.fixture_instances().iter().map(|(_, fixture)| *fixture).collect()
        }
        _ => Vec::new(),
    };
    let step = FADE_PER_SECOND * time.delta_secs();
    let mut levels: Vec<(Entity, u32)> = Vec::new();
    for (&root, &(min, max)) in bounds {
        let blocking =
            !subject.contains(&root) && eye.is_some_and(|eye| targets.iter().any(|&body| blocks(eye, body, min, max)));
        let current = state.alpha.get(&root).copied().unwrap_or(1.0);
        let wanted = if blocking { CLEAR_ALPHA } else { 1.0 };
        let alpha = if current < wanted { (current + step).min(wanted) } else { (current - step).max(wanted) };
        if alpha >= 1.0 {
            if state.alpha.remove(&root).is_some() {
                levels.push((root, OPAQUE));
            }
        } else {
            state.alpha.insert(root, alpha);
            levels.push((root, dither_level(alpha)));
        }
    }
    // A fixture that left the scene is no longer tracked; its meshes went
    // with it.
    state.alpha.retain(|root, _| bounds.contains_key(root));

    for (root, level) in levels {
        for &mesh in members.get(&root).into_iter().flatten() {
            if let Ok(mut handle) = fixture_handles.get_mut(mesh) {
                set_fixture(&mut state, mesh, &mut handle, level, &mut fixture_materials);
            }
            if let Ok(mut handle) = site_handles.get_mut(mesh) {
                set_site(&mut state, mesh, &mut handle, level, &mut site_materials);
            }
        }
    }
    if state.alpha.is_empty() {
        state.fixture_base.clear();
        state.site_base.clear();
        state.fixture_copies.clear();
        state.site_copies.clear();
    }
}

/// The placed fixture a mesh belongs to, if any.
fn fixture_root(mesh: Entity, parents: &Query<&ChildOf>, roots: &Query<(), With<FixtureRoot>>) -> Option<Entity> {
    let mut at = mesh;
    loop {
        if roots.contains(at) {
            return Some(at);
        }
        at = parents.get(at).ok()?.parent();
    }
}

fn world_bounds(transform: &GlobalTransform, aabb: &Aabb) -> (Vec3, Vec3) {
    let centre = Vec3::from(aabb.center);
    let half = Vec3::from(aabb.half_extents);
    let mut min = Vec3::splat(f32::MAX);
    let mut max = Vec3::splat(f32::MIN);
    for corner in 0..8 {
        let sign = Vec3::new(
            if corner & 1 == 0 { -1.0 } else { 1.0 },
            if corner & 2 == 0 { -1.0 } else { 1.0 },
            if corner & 4 == 0 { -1.0 } else { 1.0 },
        );
        let point = transform.transform_point(centre + half * sign);
        min = min.min(point);
        max = max.max(point);
    }
    (min, max)
}

/// Whether the segment from `eye` to `body` passes through the box short of
/// the body.
fn blocks(eye: Vec3, body: Vec3, min: Vec3, max: Vec3) -> bool {
    let along = body - eye;
    let length = along.length();
    if length <= BODY_MARGIN {
        return false;
    }
    let mut enter = 0.0_f32;
    let mut leave = length;
    let direction = along / length;
    for axis in 0..3 {
        let (origin, step, low, high) = (eye[axis], direction[axis], min[axis], max[axis]);
        if step.abs() < 1e-6 {
            if origin < low || origin > high {
                return false;
            }
            continue;
        }
        let (a, b) = ((low - origin) / step, (high - origin) / step);
        enter = enter.max(a.min(b));
        leave = leave.min(a.max(b));
        if enter > leave {
            return false;
        }
    }
    enter < length - BODY_MARGIN
}

fn set_fixture(
    state: &mut LensClearState,
    mesh: Entity,
    handle: &mut MeshMaterial3d<FixtureMaterial>,
    level: u32,
    materials: &mut Assets<FixtureMaterial>,
) {
    if level >= OPAQUE {
        if let Some(base) = state.fixture_base.remove(&mesh) {
            handle.0 = base;
        }
        return;
    }
    let base = state.fixture_base.entry(mesh).or_insert_with(|| handle.0.clone()).clone();
    let key = (base.id(), level);
    let copy = match state.fixture_copies.get(&key) {
        Some(copy) => copy.clone(),
        None => {
            let Some(mut material) = materials.get(&base).cloned() else { return };
            material.key.dither = true;
            material.params.dither_alpha = level_alpha(level);
            let copy = materials.add(material);
            state.fixture_copies.insert(key, copy.clone());
            copy
        }
    };
    if handle.0 != copy {
        handle.0 = copy;
    }
}

fn set_site(
    state: &mut LensClearState,
    mesh: Entity,
    handle: &mut MeshMaterial3d<SiteMaterial>,
    level: u32,
    materials: &mut Assets<SiteMaterial>,
) {
    if level >= OPAQUE {
        if let Some(base) = state.site_base.remove(&mesh) {
            handle.0 = base;
        }
        return;
    }
    let base = state.site_base.entry(mesh).or_insert_with(|| handle.0.clone()).clone();
    let key = (base.id(), level);
    let copy = match state.site_copies.get(&key) {
        Some(copy) => copy.clone(),
        None => {
            let Some(material) = materials.get(&base) else { return };
            // A family without the dither axes keeps its material.
            let Ok(Some(material)) = with_state(material, true, level_alpha(level)) else { return };
            let copy = materials.add(material);
            state.site_copies.insert(key, copy.clone());
            copy
        }
    };
    if handle.0 != copy {
        handle.0 = copy;
    }
}
