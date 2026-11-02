//! Compiling a document: every time resolved, every reference checked, every
//! curve built, so that sampling is a pure function of time.

use std::collections::{HashMap, HashSet};

use bevy_math::Vec3;
use serde_json::Value;

use crate::actor::{Actor, Face, Facing, Motion, Move};
use crate::camera::{self, Aim, Curve, Eye, Focus, Place, Shot};
use crate::doc::{self, Envelope, Sequence, Sound, Target, Track};
use crate::lane::{absent_after, camera_after, Entry, Lane, Shape};
use crate::sample::Point;
use crate::spline::Spline;
use crate::time::{At, Edge, LineRef};
use crate::{Compiled, Error};

/// What the host knows while a document compiles.
pub trait Catalog {
    /// The length in seconds of a sound, when the host knows it.
    fn sound_length(&self, sound: &Sound) -> Option<f32>;
    /// The length in seconds of a body motion of a unit, when known.
    fn motion_length(&self, unit: &str, motion: &str) -> Option<f32>;
    /// Whether the host places cameras for a rig of this name.
    fn has_rig(&self, name: &str) -> bool;
}

/// A catalogue that knows nothing: sounds and motions need written
/// durations, and rigs are refused.
pub struct NoCatalog;

impl Catalog for NoCatalog {
    fn sound_length(&self, _: &Sound) -> Option<f32> {
        None
    }
    fn motion_length(&self, _: &str, _: &str) -> Option<f32> {
        None
    }
    fn has_rig(&self, _: &str) -> bool {
        false
    }
}

pub(crate) struct AudioLane {
    pub track: usize,
    pub lane: Lane<AudioClip>,
}

pub(crate) struct AudioClip {
    pub source: Sound,
    pub volume: f32,
    pub speaker: Option<usize>,
}

pub(crate) struct Lines {
    pub track: usize,
    /// In time order, not overlapping.
    pub lines: Vec<Line>,
}

pub(crate) struct Line {
    pub start: f32,
    pub end: f32,
    pub id: Option<String>,
    pub speaker: Option<usize>,
    pub name: String,
    pub text: String,
    pub chars: usize,
    pub voice: Option<Sound>,
    pub voice_length: Option<f32>,
    pub reveal: f32,
}

pub(crate) struct Caption {
    pub text: String,
    pub style: doc::CaptionStyle,
}

// ---- time resolution ----

#[derive(Clone)]
enum Length {
    Fixed(f32),
    End(At),
    /// Ends with the sequence.
    Open,
    Missing(&'static str),
}

struct Item {
    path: String,
    start: Option<At>,
    /// The clip this one follows when it writes no start.
    previous: Option<usize>,
    gap: f32,
    length: Length,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Node {
    Start(usize),
    End(usize),
    Marker(usize),
}

#[derive(Default)]
struct Times {
    items: Vec<Item>,
    markers: Vec<(String, At)>,
    clip_ids: HashMap<String, usize>,
    line_ids: HashMap<String, usize>,
    lines: Vec<usize>,
    marker_names: HashMap<String, usize>,
    done: HashMap<Node, f32>,
    busy: Vec<Node>,
}

impl Times {
    /// The resolved start and end of an item.
    fn span(&self, item: usize) -> (f32, f32) {
        (self.done[&Node::Start(item)], self.done[&Node::End(item)])
    }

    fn entries<P>(&self, pending: Vec<Pending<P>>) -> Vec<Entry<P>> {
        pending
            .into_iter()
            .map(|p| {
                let (start, end) = self.span(p.item);
                Entry { path: self.items[p.item].path.clone(), start, end, shape: p.shape, payload: p.payload }
            })
            .collect()
    }

    fn name(&self, node: Node) -> String {
        match node {
            Node::Start(k) => format!("{}.start", self.items[k].path),
            Node::End(k) => format!("{}.end", self.items[k].path),
            Node::Marker(m) => self.markers[m].0.clone(),
        }
    }

    fn value(&mut self, node: Node) -> Result<f32, Error> {
        if let Some(value) = self.done.get(&node) {
            return Ok(*value);
        }
        if let Some(first) = self.busy.iter().position(|n| *n == node) {
            let mut chain: Vec<String> = self.busy[first..].iter().map(|n| self.name(*n)).collect();
            chain.push(self.name(node));
            return Err(Error::new(&self.name(node), format!("times refer to each other in a cycle: {}", chain.join(" -> "))));
        }
        self.busy.push(node);
        let value = self.compute(node);
        self.busy.pop();
        let value = value?;
        self.done.insert(node, value);
        Ok(value)
    }

    fn compute(&mut self, node: Node) -> Result<f32, Error> {
        match node {
            Node::Start(k) => {
                let item = &self.items[k];
                let (start, previous, gap, path) = (item.start.clone(), item.previous, item.gap, item.path.clone());
                match (start, previous) {
                    (Some(at), _) => self.at(&at, &format!("{path}.start")),
                    (None, Some(previous)) => Ok(self.value(Node::End(previous))? + gap),
                    (None, None) => Ok(0.0),
                }
            }
            Node::End(k) => {
                let start = self.value(Node::Start(k))?;
                let path = self.items[k].path.clone();
                match self.items[k].length.clone() {
                    Length::Fixed(length) => Ok(start + length),
                    Length::End(at) => {
                        let end = self.at(&at, &format!("{path}.end"))?;
                        if end <= start {
                            return Err(Error::new(&path, format!("the clip ends at {end} s, not after its start at {start} s")));
                        }
                        Ok(end)
                    }
                    Length::Open => Err(Error::new(
                        &path,
                        "the clip has no length of its own, so a time that depends on its end cannot be resolved; give it a duration or an end",
                    )),
                    Length::Missing(message) => Err(Error::new(&path, message)),
                }
            }
            Node::Marker(m) => {
                let (path, at) = self.markers[m].clone();
                self.at(&at, &path)
            }
        }
    }

    fn at(&mut self, at: &At, path: &str) -> Result<f32, Error> {
        let edge = |k: usize, edge: Edge| match edge {
            Edge::Start => Node::Start(k),
            Edge::End => Node::End(k),
        };
        match at {
            At::Seconds(seconds) => Ok(*seconds),
            At::Line { line, edge: e, offset } => {
                let k = match line {
                    LineRef::Number(n) => (*n as usize).checked_sub(1).and_then(|i| self.lines.get(i)).copied(),
                    LineRef::Id(id) => self.line_ids.get(id).copied(),
                }
                .ok_or_else(|| Error::new(path, format!("there is no {line}")))?;
                Ok(self.value(edge(k, *e))? + offset)
            }
            At::Marker { marker, offset } => {
                let m = *self.marker_names.get(marker).ok_or_else(|| Error::new(path, format!("there is no marker {marker:?}")))?;
                Ok(self.value(Node::Marker(m))? + offset)
            }
            At::Clip { clip, edge: e, offset } => {
                let k = *self.clip_ids.get(clip).ok_or_else(|| Error::new(path, format!("there is no clip with id {clip:?}")))?;
                Ok(self.value(edge(k, *e))? + offset)
            }
        }
    }
}

// ---- compiling ----

struct Pending<P> {
    item: usize,
    shape: Shape,
    payload: P,
}

#[derive(Default)]
struct ActorPending {
    moves: Vec<(usize, Move)>,
    motion: Vec<Pending<Motion>>,
    face: Vec<Pending<Face>>,
    look: Vec<Pending<Point>>,
}

struct LinePending {
    item: usize,
    clip: doc::LineClip,
    speaker: Option<usize>,
    voice_length: Option<f32>,
    reveal: f32,
}

struct Builder<'a> {
    catalog: &'a dyn Catalog,
    cast: HashMap<&'a str, usize>,
    times: Times,
}

fn vec3(v: [f32; 3]) -> Vec3 {
    Vec3::from_array(v)
}

impl<'a> Builder<'a> {
    fn actor(&self, id: &str, path: &str) -> Result<usize, Error> {
        self.cast.get(id).copied().ok_or_else(|| Error::new(path, format!("there is no actor {id:?} in the cast")))
    }

    fn point(&self, target: &Target, path: &str) -> Result<Point, Error> {
        Ok(match target {
            Target::Point(at) => Point::Fixed(vec3(*at)),
            Target::Actor(on) => Point::Actor { index: self.actor(&on.actor, path)?, part: on.part, offset: vec3(on.offset) },
        })
    }

    fn register_id(&mut self, id: Option<&str>, item: usize, path: &str) -> Result<(), Error> {
        match id {
            Some(id) if self.times.clip_ids.insert(id.to_owned(), item).is_some() => Err(Error::new(path, format!("the id {id:?} is used twice"))),
            _ => Ok(()),
        }
    }

    /// Adds a clip's times. `natural` is the content length in content
    /// seconds; `open` lets a clip without any length end with the sequence.
    fn clip(&mut self, path: String, envelope: Envelope<'_>, previous: Option<usize>, natural: Option<f32>, open: bool) -> Result<(usize, Shape), Error> {
        let e = envelope;
        if e.speed <= 0.0 {
            return Err(Error::new(&path, "\"speed\" must be above 0"));
        }
        if e.clip_in < 0.0 || e.blend_in < 0.0 || e.blend_out < 0.0 {
            return Err(Error::new(&path, "\"clip_in\", \"blend_in\" and \"blend_out\" cannot be negative"));
        }
        let length = match (e.duration, e.end, natural) {
            (Some(_), Some(_), _) => return Err(Error::new(&path, "write \"duration\" or \"end\", not both")),
            (Some(duration), None, _) if duration <= 0.0 => return Err(Error::new(&path, "\"duration\" must be above 0")),
            (Some(duration), None, _) => Length::Fixed(duration),
            (None, Some(end), _) => Length::End(end.clone()),
            (None, None, Some(natural)) => {
                let length = (natural - e.clip_in) / e.speed;
                if length <= 0.0 {
                    return Err(Error::new(&path, format!("\"clip_in\" {} is at or past the end of the content ({natural} s)", e.clip_in)));
                }
                Length::Fixed(length)
            }
            (None, None, None) if open => Length::Open,
            (None, None, None) => Length::Missing("the clip needs a \"duration\" or an \"end\""),
        };
        let item = self.times.items.len();
        self.times.items.push(Item { path: path.clone(), start: e.start.cloned(), previous, gap: 0.0, length });
        self.register_id(e.id, item, &path)?;
        let shape = Shape {
            clip_in: e.clip_in,
            speed: e.speed,
            blend_in: e.blend_in,
            blend_out: e.blend_out,
            ease_in: e.ease_in,
            ease_out: e.ease_out,
            before: e.before,
            after: e.after,
        };
        Ok((item, shape))
    }

    fn curve(&self, scalar: &doc::Scalar, path: &str, check: impl Fn(f32) -> bool, what: &str) -> Result<Curve, Error> {
        let curve = match scalar {
            doc::Scalar::Value(value) => Curve::Value(*value),
            doc::Scalar::Keys { keys } => Curve::Keys(self.keys(keys.iter().map(|k| (k.t, k.value, k.ease)), path)?),
        };
        let ok = match &curve {
            Curve::Value(value) => check(*value),
            Curve::Keys(keys) => keys.iter().all(|(_, value, _)| check(*value)),
        };
        if !ok {
            return Err(Error::new(path, format!("{what} is out of range")));
        }
        Ok(curve)
    }

    fn keys<T>(&self, keys: impl Iterator<Item = (f32, T, crate::ease::Ease)>, path: &str) -> Result<Vec<(f32, T, crate::ease::Ease)>, Error> {
        let keys: Vec<_> = keys.collect();
        if keys.is_empty() {
            return Err(Error::new(path, "keys need at least one key"));
        }
        if keys.windows(2).any(|pair| pair[1].0 <= pair[0].0) {
            return Err(Error::new(path, "key times must increase"));
        }
        Ok(keys)
    }

    fn shot(&self, clip: &doc::CameraClip, path: &str) -> Result<Shot, Error> {
        let place = match (&clip.rig, &clip.eye) {
            (Some(_), Some(_)) => return Err(Error::new(path, "a shot is placed by \"eye\" or by \"rig\", not both")),
            (Some(_), None) if clip.aim.is_some() => return Err(Error::new(path, "a rig shot aims itself; remove \"aim\"")),
            (Some(rig), None) => {
                if !self.catalog.has_rig(&rig.name) {
                    return Err(Error::new(path, format!("the host has no camera rig {:?}", rig.name)));
                }
                Place::Rig { name: rig.name.clone(), params: rig.params.clone() }
            }
            (None, None) => return Err(Error::new(path, "a shot needs an \"eye\" or a \"rig\"")),
            (None, Some(eye)) => {
                let eye = match eye {
                    doc::Eye::At(at) => Eye::At(vec3(*at)),
                    doc::Eye::Keys(keys) => Eye::Keys(self.keys(keys.iter().map(|k| (k.t, vec3(k.pos), k.ease)), path)?),
                    doc::Eye::Path(p) => {
                        let needed = if p.closed { 3 } else { 2 };
                        if p.points.len() < needed {
                            return Err(Error::new(path, format!("a path eye needs at least {needed} points")));
                        }
                        let points: Vec<Vec3> = p.points.iter().copied().map(vec3).collect();
                        Eye::Path { spline: Spline::new(&points, p.closed), ease: p.ease }
                    }
                    doc::Eye::Orbit(o) => {
                        if o.radius < 0.0 {
                            return Err(Error::new(path, "an orbit radius cannot be negative"));
                        }
                        Eye::Orbit { around: self.point(&o.around, path)?, radius: o.radius, height: o.height, from: o.from, to: o.to, ease: o.ease }
                    }
                    doc::Eye::Follow(f) => {
                        if f.lag < 0.0 {
                            return Err(Error::new(path, "\"lag\" cannot be negative"));
                        }
                        Eye::Follow { actor: self.actor(&f.actor, path)?, offset: vec3(f.offset), lag: f.lag }
                    }
                };
                let aim = match &clip.aim {
                    Some(aim) => {
                        if aim.lag < 0.0 {
                            return Err(Error::new(path, "\"lag\" cannot be negative"));
                        }
                        match (&aim.at, aim.dir) {
                            (Some(target), None) => Aim::At { target: self.point(target, path)?, lag: aim.lag },
                            (None, Some([yaw, pitch])) => Aim::Dir(camera::forward(yaw, pitch)),
                            _ => return Err(Error::new(path, "an aim has exactly one of \"at\" and \"dir\"")),
                        }
                    }
                    None => match &eye {
                        Eye::Path { .. } => Aim::Travel,
                        Eye::Orbit { around, .. } => Aim::At { target: around.clone(), lag: 0.0 },
                        _ => return Err(Error::new(path, "this shot needs an \"aim\"; only a path or an orbit aims itself")),
                    },
                };
                Place::Eye { eye, aim }
            }
        };
        let fov = clip.fov.as_ref().map(|fov| self.curve(fov, path, |v| v > 0.0 && v < 180.0, "\"fov\"")).transpose()?;
        let roll = clip.roll.as_ref().map(|roll| self.curve(roll, path, |_| true, "\"roll\"")).transpose()?;
        if clip.near.is_some_and(|near| near <= 0.0) {
            return Err(Error::new(path, "\"near\" must be above 0"));
        }
        if matches!((clip.near, clip.far), (Some(near), Some(far)) if far <= near) {
            return Err(Error::new(path, "\"far\" must be beyond \"near\""));
        }
        if clip.aperture.is_some_and(|aperture| aperture <= 0.0) {
            return Err(Error::new(path, "\"aperture\" must be above 0"));
        }
        if clip.shake.is_some_and(|shake| shake.frequency <= 0.0) {
            return Err(Error::new(path, "a shake \"frequency\" must be above 0"));
        }
        let focus = match &clip.focus {
            Some(doc::Focus::Distance(distance)) if *distance <= 0.0 => return Err(Error::new(path, "a focus distance must be above 0")),
            Some(doc::Focus::Distance(distance)) => Some(Focus::Distance(*distance)),
            Some(doc::Focus::At { at }) => Some(Focus::At(self.point(at, path)?)),
            None => None,
        };
        Ok(Shot { place, roll, fov, near: clip.near, far: clip.far, focus, aperture: clip.aperture, shake: clip.shake })
    }
}

/// Compiles a document against what the host knows.
pub fn compile(doc: &Sequence, catalog: &dyn Catalog) -> Result<Compiled, Error> {
    if doc.version != 1 {
        return Err(Error::new("version", format!("version {} is not known; the format version is 1", doc.version)));
    }
    if doc.fps.is_some_and(|fps| fps <= 0.0) {
        return Err(Error::new("fps", "\"fps\" must be above 0"));
    }
    let defaults = doc.defaults;
    if defaults.line_gap < 0.0 || defaults.line_hold < 0.0 || defaults.reveal <= 0.0 {
        return Err(Error::new("defaults", "\"line_gap\" and \"line_hold\" cannot be negative and \"reveal\" must be above 0"));
    }
    let mut cast = HashMap::new();
    for (index, member) in doc.cast.iter().enumerate() {
        if cast.insert(member.id.as_str(), index).is_some() {
            return Err(Error::new(&format!("cast[{index}]"), format!("the actor id {:?} is used twice", member.id)));
        }
    }
    let mut b = Builder { catalog, cast, times: Times::default() };
    for (index, marker) in doc.markers.iter().enumerate() {
        let path = format!("markers[{index}]");
        if b.times.marker_names.insert(marker.name.clone(), index).is_some() {
            return Err(Error::new(&path, format!("the marker {:?} is named twice", marker.name)));
        }
        b.times.markers.push((path, marker.at.clone()));
    }

    let mut camera: Vec<Pending<Shot>> = Vec::new();
    let mut actors: Vec<ActorPending> = doc.cast.iter().map(|_| ActorPending::default()).collect();
    let mut audio: Vec<(usize, Vec<Pending<AudioClip>>)> = Vec::new();
    let mut lines: Option<(usize, Vec<LinePending>)> = None;
    let mut captions: Vec<Vec<Pending<Caption>>> = Vec::new();
    let mut screen: Vec<Pending<[f32; 4]>> = Vec::new();
    let mut events: Vec<(String, At, Value)> = Vec::new();
    let mut seen_camera = false;
    let mut seen_screen = false;
    let mut seen_actor: HashSet<(&'static str, usize)> = HashSet::new();

    for (ti, track) in doc.tracks.iter().enumerate() {
        let track_path = format!("tracks[{ti}]");
        let mut once_per_actor = |kind: &'static str, actor: usize| {
            if seen_actor.insert((kind, actor)) {
                Ok(())
            } else {
                Err(Error::new(&track_path, format!("an actor has one {kind} track; {:?} has two", doc.cast[actor].id)))
            }
        };
        let mut previous: Option<usize> = None;
        match track {
            Track::Camera(t) => {
                if std::mem::replace(&mut seen_camera, true) {
                    return Err(Error::new(&track_path, "a sequence has one camera track"));
                }
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    let shot = b.shot(clip, &path)?;
                    let (item, shape) = b.clip(path, clip.envelope(), previous, None, true)?;
                    previous = Some(item);
                    camera.push(Pending { item, shape, payload: shot });
                }
            }
            Track::Motion(t) => {
                let actor = b.actor(&t.actor, &track_path)?;
                once_per_actor("motion", actor)?;
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    if !(0.0..=1.0).contains(&clip.weight) {
                        return Err(Error::new(&path, "\"weight\" is between 0 and 1"));
                    }
                    let natural = match clip.mode {
                        doc::MotionMode::Once => catalog.motion_length(&doc.cast[actor].unit, &clip.motion),
                        doc::MotionMode::Loop => None,
                    };
                    let (item, shape) = b.clip(path, clip.envelope(), previous, natural, true)?;
                    previous = Some(item);
                    actors[actor].motion.push(Pending { item, shape, payload: Motion { name: clip.motion.clone(), mode: clip.mode, weight: clip.weight } });
                }
            }
            Track::Move(t) => {
                let actor = b.actor(&t.actor, &track_path)?;
                once_per_actor("move", actor)?;
                let mut stand = vec3(doc.cast[actor].at);
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    let e = clip.envelope();
                    if e.clip_in != 0.0 || e.speed != 1.0 || e.blend_in != 0.0 || e.blend_out != 0.0 || e.before.is_some() || e.after.is_some() {
                        return Err(Error::new(&path, "a move does not blend, extend or change speed; it ends where its path ends"));
                    }
                    if clip.turn < 0.0 {
                        return Err(Error::new(&path, "\"turn\" cannot be negative"));
                    }
                    let mut points = Vec::with_capacity(clip.path.len() + 1);
                    points.push(stand);
                    points.extend(clip.path.iter().copied().map(vec3));
                    let spline = Spline::new(&points, false);
                    let natural = match clip.pace {
                        Some(pace) if pace <= 0.0 => return Err(Error::new(&path, "\"pace\" must be above 0")),
                        Some(_) if spline.length() <= 1e-4 => return Err(Error::new(&path, "the path has no length to pace; give the move a duration")),
                        Some(pace) => Some(spline.length() / pace),
                        None => None,
                    };
                    stand = *points.last().unwrap_or(&stand);
                    let facing = match &clip.face {
                        doc::Facing::Path => Facing::Path,
                        doc::Facing::Keep => Facing::Keep,
                        doc::Facing::Yaw(yaw) => Facing::Yaw(*yaw),
                        doc::Facing::Toward(target) => Facing::Toward(b.point(target, &path)?),
                    };
                    let (item, _) = b.clip(path, e, previous, natural, false)?;
                    previous = Some(item);
                    let m = Move { start: 0.0, end: 0.0, spline, facing, gait: clip.gait.clone(), ease: clip.ease, turn: clip.turn, yaw_from: 0.0, yaw_end: 0.0 };
                    actors[actor].moves.push((item, m));
                }
            }
            Track::Face(t) => {
                let actor = b.actor(&t.actor, &track_path)?;
                once_per_actor("face", actor)?;
                for (ci, clip) in t.clips.iter().enumerate() {
                    let (item, shape) = b.clip(format!("{track_path}.clips[{ci}]"), clip.envelope(), previous, None, true)?;
                    previous = Some(item);
                    actors[actor].face.push(Pending { item, shape, payload: Face { eye: clip.eye.clone(), mouth: clip.mouth.clone() } });
                }
            }
            Track::Look(t) => {
                let actor = b.actor(&t.actor, &track_path)?;
                once_per_actor("look", actor)?;
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    let target = b.point(&clip.at, &path)?;
                    let (item, shape) = b.clip(path, clip.envelope(), previous, None, true)?;
                    previous = Some(item);
                    actors[actor].look.push(Pending { item, shape, payload: target });
                }
            }
            Track::Audio(t) => {
                let mut pending = Vec::with_capacity(t.clips.len());
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    if clip.volume < 0.0 {
                        return Err(Error::new(&path, "\"volume\" cannot be negative"));
                    }
                    let speaker = clip.speaker.as_deref().map(|id| b.actor(id, &path)).transpose()?;
                    let natural = catalog.sound_length(&clip.source);
                    let (item, shape) = b.clip(path, clip.envelope(), previous, natural, true)?;
                    previous = Some(item);
                    pending.push(Pending { item, shape, payload: AudioClip { source: clip.source.clone(), volume: clip.volume, speaker } });
                }
                audio.push((ti, pending));
            }
            Track::Line(t) => {
                if lines.is_some() {
                    return Err(Error::new(&track_path, "a sequence has one line track"));
                }
                let mut pending = Vec::with_capacity(t.clips.len());
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    let speaker = clip.speaker.as_deref().map(|id| b.actor(id, &path)).transpose()?;
                    let reveal = clip.reveal.unwrap_or(defaults.reveal);
                    if reveal <= 0.0 {
                        return Err(Error::new(&path, "\"reveal\" must be above 0"));
                    }
                    let voice = clip.voice.clone().map(Sound::Voice);
                    let voice_length = voice.as_ref().and_then(|voice| catalog.sound_length(voice));
                    let length = match (clip.duration, &voice, voice_length) {
                        (Some(duration), _, _) if duration <= 0.0 => return Err(Error::new(&path, "\"duration\" must be above 0")),
                        (Some(duration), _, _) => Length::Fixed(duration),
                        (None, Some(_), Some(length)) => Length::Fixed(length + defaults.line_hold),
                        (None, Some(_), None) => Length::Missing("the length of the line's voice is not known; give the line a \"duration\""),
                        (None, None, _) => {
                            let length = clip.text.chars().count() as f32 / reveal + defaults.line_hold;
                            if length <= 0.0 {
                                return Err(Error::new(&path, "an empty line without a hold has no length; give it a \"duration\""));
                            }
                            Length::Fixed(length)
                        }
                    };
                    let item = b.times.items.len();
                    b.times.items.push(Item { path: path.clone(), start: clip.start.clone(), previous, gap: defaults.line_gap, length });
                    b.register_id(clip.id.as_deref(), item, &path)?;
                    if let Some(id) = &clip.id {
                        b.times.line_ids.insert(id.clone(), item);
                    }
                    b.times.lines.push(item);
                    previous = Some(item);
                    pending.push(LinePending { item, clip: clip.clone(), speaker, voice_length, reveal });
                }
                lines = Some((ti, pending));
            }
            Track::Caption(t) => {
                let mut pending = Vec::with_capacity(t.clips.len());
                for (ci, clip) in t.clips.iter().enumerate() {
                    let (item, shape) = b.clip(format!("{track_path}.clips[{ci}]"), clip.envelope(), previous, None, true)?;
                    previous = Some(item);
                    pending.push(Pending { item, shape, payload: Caption { text: clip.text.clone(), style: clip.style } });
                }
                captions.push(pending);
            }
            Track::Screen(t) => {
                if std::mem::replace(&mut seen_screen, true) {
                    return Err(Error::new(&track_path, "a sequence has one screen track"));
                }
                for (ci, clip) in t.clips.iter().enumerate() {
                    let path = format!("{track_path}.clips[{ci}]");
                    if !clip.fade.iter().all(|c| (0.0..=1.0).contains(c)) {
                        return Err(Error::new(&path, "\"fade\" components are between 0 and 1"));
                    }
                    let (item, shape) = b.clip(path, clip.envelope(), previous, None, true)?;
                    previous = Some(item);
                    screen.push(Pending { item, shape, payload: clip.fade });
                }
            }
            Track::Event(t) => {
                for (ei, event) in t.events.iter().enumerate() {
                    let path = format!("{track_path}.events[{ei}]");
                    if !event.action.get("do").is_some_and(Value::is_string) {
                        return Err(Error::new(&path, "an action is an object naming what it does in \"do\""));
                    }
                    events.push((path, event.at.clone(), event.action.clone()));
                }
            }
        }
    }

    // Resolve every time; clips that end with the sequence come last.
    let times = &mut b.times;
    let mut latest = 0.0f32;
    for k in 0..times.items.len() {
        latest = latest.max(times.value(Node::Start(k))?);
        if !matches!(times.items[k].length, Length::Open) {
            latest = latest.max(times.value(Node::End(k))?);
        }
    }
    for m in 0..times.markers.len() {
        latest = latest.max(times.value(Node::Marker(m))?);
    }
    let mut resolved_events = Vec::with_capacity(events.len());
    for (path, at, action) in events {
        let at = times.at(&at, &path)?;
        latest = latest.max(at);
        resolved_events.push((at, action));
    }
    resolved_events.sort_by(|a, b| a.0.total_cmp(&b.0));
    for k in 0..times.items.len() {
        if matches!(times.items[k].length, Length::Open) {
            let start = times.done[&Node::Start(k)];
            times.done.insert(Node::End(k), latest.max(start));
        }
    }
    let times = &*times;

    let camera = Lane::new(times.entries(camera), camera_after)?;
    let screen = Lane::new(times.entries(screen), absent_after)?;
    let audio = audio
        .into_iter()
        .map(|(track, pending)| Ok(AudioLane { track, lane: Lane::new(times.entries(pending), absent_after)? }))
        .collect::<Result<Vec<_>, Error>>()?;
    let captions = captions.into_iter().map(|pending| Lane::new(times.entries(pending), absent_after)).collect::<Result<Vec<_>, Error>>()?;

    let mut built = Vec::with_capacity(actors.len());
    for (index, pending) in actors.into_iter().enumerate() {
        let member = &doc.cast[index];
        let mut moves: Vec<Move> = Vec::with_capacity(pending.moves.len());
        for (item, mut m) in pending.moves {
            (m.start, m.end) = times.span(item);
            if let Some(previous) = moves.last().filter(|previous| m.start < previous.end) {
                return Err(Error::new(
                    &times.items[item].path,
                    format!("the move starts at {} s, before the previous move of {:?} ends at {} s", m.start, member.id, previous.end),
                ));
            }
            moves.push(m);
        }
        built.push(Actor {
            id: member.id.clone(),
            unit: member.unit.clone(),
            at: vec3(member.at),
            yaw: member.yaw,
            moves,
            motion: Lane::new(times.entries(pending.motion), absent_after)?,
            face: Lane::new(times.entries(pending.face), absent_after)?,
            look: Lane::new(times.entries(pending.look), absent_after)?,
        });
    }

    let lines = match lines {
        None => None,
        Some((track, pending)) => {
            let mut out: Vec<Line> = Vec::with_capacity(pending.len());
            for p in pending {
                let (start, end) = times.span(p.item);
                if let Some(previous) = out.last().filter(|previous| start < previous.end) {
                    return Err(Error::new(
                        &times.items[p.item].path,
                        format!("the line starts at {start} s, before the previous line ends at {} s", previous.end),
                    ));
                }
                out.push(Line {
                    start,
                    end,
                    id: p.clip.id,
                    speaker: p.speaker,
                    name: p.clip.name,
                    chars: p.clip.text.chars().count(),
                    text: p.clip.text,
                    voice: p.clip.voice.map(Sound::Voice),
                    voice_length: p.voice_length,
                    reveal: p.reveal,
                });
            }
            Some(Lines { track, lines: out })
        }
    };

    let markers = doc.markers.iter().enumerate().map(|(m, marker)| (marker.name.clone(), times.done[&Node::Marker(m)])).collect();
    let mut compiled = Compiled {
        name: doc.name.clone(),
        fps: doc.fps,
        duration: latest,
        markers,
        camera,
        actors: built,
        audio,
        lines,
        captions,
        screen,
        events: resolved_events,
    };
    compiled.settle_facings();
    Ok(compiled)
}

