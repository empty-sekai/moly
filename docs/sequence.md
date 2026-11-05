# Sequences

A sequence document lays out a staged scene on one time axis: camera shots, the cast's moves,
motions, faces and gaze, sounds, lines of dialogue, captions, screen colours and one-shot actions.
The crate `moly-sequence` reads and compiles the document and gives its state at any time;
`moly-sequence-player` plays it on the running game.

A compiled sequence is a function of time: the state at a time is the same however playback reached
it. Only the event track depends on direction: an event fires when forward playback crosses its time.

## The document

A document is JSON. Every object refuses fields it does not know. Positions are `[x, y, z]` in world
space with +Y up; a yaw is degrees about +Y, 0 facing +Z; angles are degrees and times seconds.

```json
{
  "version": 1,
  "name": "greeting",
  "fps": 60,
  "defaults": { "line_gap": 0.3, "line_hold": 0.5, "reveal": 20 },
  "cast": [{ "id": "a", "unit": "21", "at": [0, 0, 2], "yaw": 180 }],
  "markers": [{ "name": "turn", "at": { "line": 1, "edge": "end" } }],
  "tracks": [
    { "kind": "camera", "clips": [
      { "duration": 4, "eye": { "keys": [{ "t": 0, "pos": [0, 1.4, -1] }, { "t": 4, "pos": [0.5, 1.3, -0.6], "ease": "in_out_sine" }] },
        "aim": { "at": { "actor": "a", "part": "head" } }, "fov": 35 }
    ] },
    { "kind": "line", "clips": [{ "speaker": "a", "name": "A", "text": "Hello." }] },
    { "kind": "move", "actor": "a", "clips": [{ "start": { "marker": "turn" }, "duration": 1, "path": [], "face": { "yaw": 90 } }] }
  ]
}
```

| Field | Meaning |
|---|---|
| `version` | The format version; only `1` exists. |
| `name` | The sequence's name. |
| `fps` | The frame rate the sequence is meant to be rendered at, when it has one. |
| `defaults` | `line_gap` (seconds between one line's end and the next one's start, 0.3), `line_hold` (seconds a line stays up after its voice or text reveal, 0.5), `reveal` (characters per second, 20). |
| `cast` | Actors: `id` (the name tracks use), `unit` (the character's unit number, as text), `at` and `yaw` (where it stands before anything moves it). |
| `markers` | Named times: `name`, `at`. |
| `tracks` | Lanes of clips, each with a `kind` and an optional `name`. |

### Times

A time is a number of seconds from the start, or an object relative to something else in the
document, plus an optional `offset` in seconds:

- `{ "line": 2 }` or `{ "line": "id" }`: a line, by its 1-based position in the line track or its `id`.
- `{ "marker": "turn" }`: a marker.
- `{ "clip": "id" }`: a clip with that `id`.

`edge` (`start`, the default, or `end`) picks the end of a line or clip. Relative times are
resolved when the document is compiled; a reference that does not resolve, or a cycle, is refused.

### Clips

Every clip except a line shares these fields:

| Field | Meaning |
|---|---|
| `id` | The name other times refer to this clip by. |
| `start` | When it starts; absent means when the previous clip of the track ends, or 0 for the first. |
| `duration` or `end` | How long it lasts, or when it ends. A motion or sound clip without either lasts its content's own length. |
| `clip_in` | Seconds into the clip's content at its start. |
| `speed` | Content seconds per sequence second (1). |
| `blend_in`, `blend_out` | Seconds over which the clip's weight rises from 0 at its start and falls to 0 at its end, eased by `ease_in` and `ease_out`. |
| `before`, `after` | How the clip extends past an edge: `none` (absent), `hold` (keeps its edge state) or `loop` (after its end only). |

### Eases

`linear`, `step`, `smooth`, `in_<family>`, `out_<family>` and `in_out_<family>` for the families
`sine`, `quad`, `cubic`, `quart`, `quint`, `expo`, `circ` and `back`, or a CSS cubic Bézier
`{ "bezier": [x1, y1, x2, y2] }` with `x1` and `x2` in 0..=1. An unknown ease is refused with the
list of eases.

### Track kinds

| `kind` | Clip fields | Meaning |
|---|---|---|
| `camera` | `eye`, `aim`, `roll`, `fov`, `near`, `far`, `focus`, `aperture`, `shake`, `rig` | A shot. `eye` is one of `{ "at": [x, y, z] }`, `{ "keys": [{ "t", "pos", "ease" }] }` (clip-local times, straight segments), `{ "path": { "points", "closed", "ease" } }` (a smooth curve travelled at constant speed), `{ "orbit": { "around", "radius", "height", "from", "to", "ease" } }` or `{ "follow": { "actor", "offset", "lag" } }` (metres right, up and forward of the actor). `aim` is `at` (a point or `{ "actor", "part", "offset" }`, with `part` one of `head`, `chest`, `waist`, `feet`) or `dir` (`[yaw, pitch]`), with an optional `lag`; a path eye without an aim looks along its travel. `roll` and `fov` are a number or `{ "keys": [{ "t", "value", "ease" }] }`. `focus` is a distance or `{ "at": target }`; `aperture` an f-number; `shake` is `{ "amplitude", "rotation", "frequency", "seed" }`, a repeatable tremble. Lens values left out are the live camera's. The game has no camera rigs, so a `rig` shot is refused. |
| `motion` | `actor`; clips `motion`, `mode` (`once` or `loop`), `weight` | A body motion, named exactly as in the shared motion library. |
| `move` | `actor`; clips `path`, `face`, `gait`, `ease`, `pace`, `turn` | A walk from where the actor stands through the points of `path`; an empty path turns it in place. `face` is `path` (along the travel), `keep`, `{ "yaw": deg }` or `{ "toward": target }`. `pace` (metres per second) gives the duration when none is written; `turn` is the seconds a turn takes (0.3). The game animates a moving actor with its own walk motion and does not read `gait`. |
| `face` | `actor`; clips `eye`, `mouth` | Facial expression names. |
| `look` | `actor`; clips `at` | The head and eyes turn toward a target. |
| `audio` | clips `source`, `volume`, `speaker` | `source` is `{ "voice": { "cue", "bundle" } }`, `{ "bgm": { "cue", "bundle" } }` or `{ "se": "name" }`. A `file` source is refused by the game. |
| `line` | clips `id`, `start`, `duration`, `speaker`, `name`, `text`, `voice`, `reveal` | A line in the talk window. Absent `start` is `line_gap` after the previous line; absent `duration` is the voice length (or the reveal time) plus `line_hold`. |
| `caption` | clips `text`, `style` | Text over the picture: `subtitle` (default), `location`, `title` or `fullscreen`. |
| `screen` | clips `fade` | A colour `[r, g, b, a]` in 0..=1 laid over the whole picture. |
| `event` | `events`: `{ "at", "action" }` | A one-shot action: `action` is a director step object naming its action in `do` (see [Plugins](plugins.md#director-steps)). |

A document either compiles completely or is refused with the place in the document, such as
`tracks[2].clips[0]`, and the reason.

## Playing in the game

Loading a file reads, parses and compiles it against what the game has loaded (sound and motion
lengths). The load is refused, with the reason, when:

- the file cannot be read, does not parse or does not compile;
- an audio clip names a `file` sound;
- a line's name or text uses a character the balloon font atlas does not hold;
- a cast member's unit is not on the current site with a body.

While it plays, the player holds the camera and takes the cast from their AI. Playback advances by
the game clock, so `moly.time`'s scale, pause and steps, and a recording's fixed frame step, drive it
as they drive everything else. A pause or the end holds the frame as laid until a play or a stop.
Stopping gives the camera and the cast back. A loaded file that is edited is read again: the new
document is entered at the played time, and a file that no longer compiles is refused and the
previous document kept.

A holder of the same or a higher rank (a director script, a person's input) that takes the camera
stops the player.

## Control methods

A native run with `dev.control_port` (`MOLY_CONTROL_PORT`) set answers these JSON-RPC methods.
Unknown parameters are refused. A method that changes the player answers `{ "reply", "status" }`.

| Method | Params | What it does |
|---|---|---|
| `moly.sequence.load` | `path` | Loads a sequence file (a path relative to the game's working directory, or absolute). |
| `moly.sequence.play` | `from` (seconds, optional) | Plays from `from`, or on from the played time. |
| `moly.sequence.pause` | | Pauses, holding the frame. |
| `moly.sequence.seek` | exactly one of `t` (seconds) and `marker` | Lays the frame at that time. An unknown marker is refused with the list of markers. |
| `moly.sequence.speed` | `scale` (0 or more) | Sets the playback speed. |
| `moly.sequence.stop` | | Stops and gives everything back. |
| `moly.sequence.frame` | `on` (default `true`) | While paused, lets the free camera move to frame the shot; `on: false` gives the camera back to the sequence. |
| `moly.sequence.key` | `at` (seconds, optional; the played time by default) | Writes the live camera as one key into the camera clip that holds `at`: an eye key at the clip-local time (a key within 1 ms is replaced and keeps its ease), the look as a `dir` unless the clip looks at a target, and the field of view. Saves the file as 2-space JSON, which reloads it. Refused when no camera clip holds the time, or the clip's eye is `at`, `path`, `orbit` or `follow`, or it is a rig shot. |
| `moly.sequence.status` | | Answers `name`, `path`, `state`, `t`, `duration`, `speed`, `markers`, `cast` (each `id`, `unit`, `held`), `framing`, `ended` and `last_error`. |
| `moly.sequence.schema` | | Answers the methods and the JSON Schema of their params. |

## Director steps

`sequence.play` (`path`, `from`) and `sequence.stop` run the player from a director script, a plugin
or a sequence's own event track; see [Plugins](plugins.md#sequences).
