# Spatial coordinate contract

Contract identifier: `moly-rh-y-up-reflect-x-v1`.

All newly published runtime geometry, node transforms, animation transforms,
attachment poses and navigation geometry use one right-handed, Y-up frame.
Gameplay forward is +Z; a camera's optical -Z axis is a camera convention, not a
different world frame. One source Unity unit remains one runtime unit. No
physical unit is inferred from an asset's apparent size.

## Source boundary

Let `S = diag(-1, 1, 1, 1)`. Unity source data crosses the boundary once:

| Source value | Runtime value |
| --- | --- |
| Position, direction, normal `(x,y,z)` | `(-x,y,z)` |
| Quaternion `(x,y,z,w)` | `(x,-y,-z,w)` |
| Local or world matrix `M` | `S M S` |
| Scale `(x,y,z)` | unchanged |
| Triangle `(i0,i1,i2)` | `(i0,i2,i1)` |
| Inverse bind matrix `B` | `S B S` |
| TRS animation values and cubic derivatives | same component signs as the corresponding TRS value |

A point transforms as `p_runtime = S p_source`. With column vectors and
`world = parent * local`, conjugation preserves composition. Normals use the
inverse transpose for subsequent nonuniform instance scaling. Spatial
reflection reverses tangent XYZ and tangent handedness W. Any independent UV
reflection reverses W again.

UV origin, texture colour encoding and clip-space depth are separately declared
contracts. They must not trigger another spatial reflection. Existing fixture
UVs remain `unity-bottom-left`; their source material shader retains that UV
interpretation. Site and actor UV producers retain their existing declared UV
conversion.

## Placement and identity

Raw masterdata, serialized source fields, integer direction enums and Unity
curve evidence remain raw, explicitly named source data. The source law layer
implements source arithmetic. An adapter converts a computed source pose once;
no caller negates it again because a model looks reversed.

The source grid cell covers `[x*tile, (x+1)*tile]`; reflecting a cell maps its
index to `-x-1`, not `-x`. Even-sized footprints must be reflected by their
occupied interval and then reconstructed, preserving the source pivot law.
The local editor store persists canonical grid values without converting them
again. Any export to the source housing wire format must apply the inverse
mapping exactly once. UI labels and enum names do not determine rotation signs.

Source `FixtureView.SetPosition` replaces the view root's local position and
`ForceSetRotation` replaces its rotation. The exported authored root pose is
evidence, not an additional placement offset. In the runtime wrapper hierarchy,
the wrapper owns the placement pose, while the exact source FixtureView node
has its placement-controlled translation and rotation reset; its authored
scale and all other child transforms are preserved. StartLoc/EndLoc world
poses, render geometry, collider geometry and navigation input derive from this
same resulting hierarchy.

The export boundary rejects a nested FixtureView until a specific nested-root
placement law is implemented. Attachment view metadata carries its preserved
local scale, so the analytic locator adapter applies the same scale as the
render/collider hierarchy. It never restores the replaced authored T/R.

## Domain ownership

| Domain | Stored/consumed frame | Conversion owner |
| --- | --- | --- |
| Site/furniture/actor GLB nodes, meshes, skins, animations | canonical node-local | extractor |
| Furniture attachments, HouseView door poses | canonical node-local with source identity | extractor |
| Collider meshes/centers and site obstacle centers | canonical node-local, extents unsigned | extractor |
| Player housing wire cells, directions, wall sides | source grid at input; canonical after import | `player_data::mirror_fixture_layout` |
| Source master rules, raw Euler curves, particle simulation laws | explicitly source-labelled evidence | named runtime adapter at application |
| Particle custom mesh/shader inputs | source program ABI inside adapter; canonical world outside | particle/shader boundary |
| Camera-facing sprites and UI | declared presentation-plane layout, not world coordinates | UI/billboard mount |

Raw source typetrees, shader uniforms and opaque native NavMesh bytes are not
relabeled as canonical. A GLB's spatial marker applies to glTF spatial fields,
not every number in arbitrary extras. A presentation-plane convention does
not authorize reflecting the parent site or character again.

Each site is currently instantiated in its own site-local world at origin zero.
If multiple sites later coexist, a site-origin parent must transform every
domain together. Furniture must never receive a separate origin correction.

### Animation and preview placement

The actor wrapper's world pose and its animated Root/Hips pose are separate
owners in the same coordinate frame. A completed source Director must stop and
release its owned bone transforms before sparse shared talk clips take over.
The activity then applies its already-selected source EndLoc (StartLoc when no
EndLoc is authored), synchronizes navigation position/forward, and resumes idle
unless an admitted talk clip already owns the animator. Talk-enabled live loops
remain active. This is an ownership transition, never a 180-degree basis fix.

The independent content preview is a product placement policy, not a source
furniture transform. A fixture-centered observer does not follow a temporary
NPC approach pose. For pre-actions that admit talk only after completion, the
retained action plan supplies the future talk station before the approach
starts. Navigation and actor clearance filter the observer candidates; the
player is not repositioned at the dialogue boundary. Original placement
snapshots remain the sole restoration source.

CN horse talk 7453 demonstrates both distinctions: the canonical furniture
center is `(0.125, 0, 0.125)` and the selected world locator is
`(0.625, 0, -0.775)`. Its unit-30 ending clip writes a 180-degree Root rotation,
whereas the shared notice clip writes Hips. Retaining that ending pose below an
otherwise correct wrapper LookAt reverses the visible actor. Separately,
anchoring the observer on the temporary NPC position left the player about
2.35 units from the final source station instead of the staged 0.95 units.

## Import and publication

New GLB assets and scene roots, fixture indexes, attachment documents and
collision graphs declare the contract identifier. Unknown or mixed contracts
fail validation before publication. Coordinate conversion is never inferred
from a matrix determinant, texture name, model name or visual appearance.
Already published legacy snapshots keep their matching old runtime; they are
not silently relabelled as the new contract.

Physics collider inputs retain source object/component identities, hierarchy,
activity, layer, shape, convex/cooking and modifier fields. A layout footprint
is an editor occupancy rule, not a collider. Any remaining limitation of the
navigation backend must be exposed separately from coordinate correctness.

Navigation voxel settings are selected from `source.json`'s actual source
region, not a donor weather region or URL. The audited CN branch uses 0.05 for
site type 4 only; JP uses 0.05 for types 4 through 8. Other site types use 0.01.

## Verification obligations

- An asymmetric mesh and attachment agree in all four source directions.
- Source transform composition followed by S equals converted composition.
- Mesh normals, winding, tangent handedness, skins and animation derivatives
  agree after conversion, including negative and nonuniform authored scales.
- Nonzero authored FixtureView root offsets are replaced at placement.
- Render, collider, attachment, occupancy and scene origin are compared in the
  same world frame; adding a compensating 180-degree turn cannot pass.
- Imported grid layouts round-trip without changing source cells/directions.

Native evidence: CN FixtureController.SetGridPosition (RVA `0x5A0D508`) calls
FixtureView.SetPosition (RVA `0x5A35004`); ForceSetRotation (RVA `0x5A34AB0`)
writes Euler Y=`90*direction`. NPCAvatarMoveExecutor reads StartLoc world
position and rotation directly. NavMeshField configures PhysicsColliders
(`NavMeshCollectGeometry=1`), not the editor's layout rectangles.
