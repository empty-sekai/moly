# Ragdoll

The crate `moly-ragdoll` drops a character out of its animation and lets it fall as a body of rigid
parts, simulated with [avian3d](https://github.com/Jondolf/avian). When the body has come to rest,
the character gets up: its pose blends from where the parts lie back into its standing animation.

## The body

- A part is built for every bone the skeleton describes, sized from the bones' world positions, so
  characters of any size fall alike. The head is a sphere centred between the head bone and the
  crown; the shoulders, pelvis and chest are laid out in the body's own frame, so a character
  facing any way falls the same.
- Each part starts at the speed its bone had in the animation. An optional push lands on a named
  bone.
- Elbows and knees hinge one way; the other joints turn within a swing cone and a twist range.
- The fallen bones are written back after the animation and before transform propagation, so hair
  chains and shadows follow the fallen pose, and the head's look-at leaves a fallen head alone.

Speeds and pushes are given per metre of the character's height: a value is multiplied by the
height before it is compared with a speed of the simulation or applied as one.

## Coming to rest and getting up

A body is at rest once every part has moved slower than `settle_speed` for `settle_seconds`, or
when it has lain for `max_seconds`. It then gets up: over `recover_seconds` its pose blends into
its standing animation where its pelvis came to rest, and it is placed on the navigation surface.

Who holds the character afterwards depends on who held it before the fall:

- A character that its AI was driving is taken from the AI for the fall and given back to it once
  it has got up.
- A character that a script or plugin had taken (`npc.take`, `npc.ragdoll`) stays taken.

## Control methods

A native run with `dev.control_port` (`MOLY_CONTROL_PORT`) set answers these JSON-RPC methods (see
[Configuration](configuration.md) and [MCP server](mcp.md)). Unknown parameters are refused.

| Method | Params | What it does |
|---|---|---|
| `moly.ragdoll.drop` | `unit` (the character's unit number), `impulse` (`[x, y, z]`, optional), `at` (a bone name, optional; the pelvis by default) | Drops the character. Answers what it did, or a refusal: the character is falling already, has no such bone, or its skeleton has no `Hips` bone. |
| `moly.ragdoll.recover` | `unit` | Makes a falling character get up now. Refused when it is not falling or is already getting up. |
| `moly.ragdoll.status` | | Answers `ragdolls`: for each falling character its `unit`, `phase` (`simulating` or `recovering`), `elapsed`, `height`, the `pelvis` position, the fastest part's speed, the rest speed, the state that holds its body, and every part's role, bone, position, rotation and speed. |
| `moly.ragdoll.settings` | any of the settings below | With no params, answers the settings. With params, checks every value against its range and applies them together; a value out of range is refused with its key and nothing changes. |
| `moly.ragdoll.schema` | | Answers the methods and the JSON Schema of their params, including the settings' ranges. |

## Settings

| Key | Range | Default | What it changes |
|---|---|---|---|
| `gravity_scale` | 0–5 | 1.0 | How strongly gravity pulls the parts, as a multiple of the world's gravity. |
| `linear_damping` | 0–10 | 0.05 | How quickly a part's speed bleeds away besides the contacts. |
| `angular_damping` | 0–10 | 0.5 | How quickly a part's spin bleeds away besides the contacts. |
| `joint_compliance` | 0–0.01 | 0.0 | How soft the joint limits are: 0 holds them rigidly. Not scaled by height. |
| `mass_scale` | 0.1–10 | 1.0 | The weight of every part, as a multiple of its volume. |
| `settle_speed` | 0.001–2 | 0.15 | The speed, in metres per second per metre of height, below which every part counts as at rest. |
| `settle_seconds` | 0–10 | 0.5 | How long every part has to stay at rest before the character gets up. |
| `max_seconds` | 0.1–60 | 8.0 | The longest a character lies before it gets up regardless. |
| `recover_seconds` | 0–5 | 0.35 | How long the pose blends from the fallen parts back to the standing animation. |

## Director step

`npc.ragdoll` (`unit`, `impulse`, `at`) takes the character for the run and drops it, with the same
fields and refusals as `moly.ragdoll.drop`; see [Plugins](plugins.md#characters).
