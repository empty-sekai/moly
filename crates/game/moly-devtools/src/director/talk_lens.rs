//! The talk lens: while a conversation plays, the free camera is placed in
//! front of whoever speaks, every frame, from the way that body faces,
//! turned a little toward the listeners' side so the shot is a three-quarter
//! front view. The pose is computed in the same frame the speaker changes,
//! not polled from outside. Between a line's voice ending and the next line
//! being released the lens already stands on the next speaker; before a talk
//! starts it frames the unit the step named.
//!
//! A wide shot of the whole cast replaces the close-up while a voice line
//! has no single speaker, and for `wide_hold` seconds after a cast member
//! other than the speaker starts an animation in the middle of a line.

use bevy::prelude::*;

use crate::camera::{FreeCameraPose, FreeCameraPoseValue};

/// The lens's framing, set by `camera.talk_lens`; removed by
/// `camera.talk_lens_off`.
#[derive(Resource, Clone, Copy, Debug)]
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
}

/// A listener's animation counts as a reaction when it starts this long
/// after the line began; the ones that start with the line are its posture.
const REACTION_AFTER: f32 = 0.5;

/// Screen aspect the wide shot fits the cast into.
const ASPECT: f32 = 16.0 / 9.0;

pub fn follow_speaker(
    lens: Option<Res<TalkLens>>,
    talk: Option<Res<crate::talk::ActiveTalk>>,
    window: Option<Res<crate::talk_window::TalkWindowState>>,
    time: Res<Time>,
    characters: Query<(&crate::npc::CharacterUnitId, &GlobalTransform)>,
    bodies: Query<&GlobalTransform>,
    mut pose: ResMut<FreeCameraPose>,
) {
    let Some(lens) = lens else { return };
    let Some(talk) = talk else {
        if let Some(unit) = lens.idle_unit {
            if let Some((_, body)) = characters.iter().find(|(id, _)| id.0 == unit) {
                place(&lens, body, Vec2::ZERO, &mut pose);
            }
        }
        return;
    };
    let participants = talk.participants();
    let release_pending = window
        .as_deref()
        .is_some_and(crate::talk_window::TalkWindowState::release_pending);
    let next = release_pending.then(|| talk.upcoming_voice_speaker()).flatten();
    let Some(&(unit, entity)) = next
        .or_else(|| talk.voice_speaker())
        .or_else(|| talk.speaker())
        .and_then(|unit| participants.iter().find(|(who, _)| *who == unit))
        .or_else(|| participants.first())
    else {
        return;
    };
    let Ok(body) = bodies.get(entity) else { return };
    let feet = body.translation();
    let mut listeners = Vec2::ZERO;
    let mut count = 0.0;
    for (who, other) in participants {
        if *who == unit {
            continue;
        }
        if let Ok(at) = bodies.get(*other) {
            listeners += Vec2::new(at.translation().x, at.translation().z);
            count += 1.0;
        }
    }
    let toward = if count > 0.0 {
        (listeners / count - Vec2::new(feet.x, feet.z)).normalize_or_zero()
    } else {
        Vec2::ZERO
    };
    let group_line = if release_pending { talk.upcoming_voice_is_group() } else { talk.voice_group() };
    let reacting = !release_pending
        && talk
            .late_reaction(time.elapsed_secs(), REACTION_AFTER)
            .is_some_and(|(_, ago)| ago < lens.wide_hold);
    if lens.wide && (group_line || reacting) {
        let members: Vec<Vec3> = participants
            .iter()
            .filter_map(|(_, member)| bodies.get(*member).ok().map(GlobalTransform::translation))
            .collect();
        if let Some(out) = front(&lens, body, toward) {
            if place_wide(&lens, &members, out, &mut pose) {
                return;
            }
        }
    }
    place(&lens, body, toward, &mut pose);
}

/// The eye in front of the body, `toward` being the direction of the
/// listeners (zero when unknown).
fn place(lens: &TalkLens, body: &GlobalTransform, toward: Vec2, pose: &mut FreeCameraPose) {
    let Some(out) = front(lens, body, toward) else { return };
    let feet = body.translation();
    *pose = FreeCameraPose(Some(FreeCameraPoseValue {
        eye: Vec3::new(
            feet.x + out.x * lens.distance,
            feet.y + lens.height,
            feet.z + out.y * lens.distance,
        ),
        target: Vec3::new(feet.x, feet.y + lens.target, feet.z),
        up: Vec3::Y,
        fov_degrees: lens.fov,
    }));
}

/// The horizontal direction from the body to the eye: ahead of the body,
/// swung toward the listeners' side.
fn front(lens: &TalkLens, body: &GlobalTransform, toward: Vec2) -> Option<Vec2> {
    let facing = (body.compute_transform().rotation * Vec3::Z).xz().normalize_or_zero();
    let ahead = if facing == Vec2::ZERO { toward } else { facing };
    if ahead == Vec2::ZERO {
        return None;
    }
    let side = if ahead.perp_dot(toward) < 0.0 { -1.0 } else { 1.0 };
    let (sin, cos) = (lens.swing * side).sin_cos();
    Some(Vec2::new(ahead.x * cos - ahead.y * sin, ahead.x * sin + ahead.y * cos))
}

/// The whole cast in one frame, seen from the side of the line between the
/// two members farthest apart that the close-ups use (`side` is the
/// close-up's eye direction). False when fewer than two members stand apart.
fn place_wide(lens: &TalkLens, members: &[Vec3], side: Vec2, pose: &mut FreeCameraPose) -> bool {
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
        return false;
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
    *pose = FreeCameraPose(Some(FreeCameraPoseValue {
        eye: Vec3::new(centre.x + out.x * distance, feet + lens.height, centre.z + out.y * distance),
        target: Vec3::new(centre.x, feet + lens.target * 0.75, centre.z),
        up: Vec3::Y,
        fov_degrees: lens.fov,
    }));
    true
}
