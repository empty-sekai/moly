# NPC runtime and embedded stage migration

The source descriptor chooses the NPC controller. Japanese sources with an
application major version of 7 or later use the hierarchical controller;
earlier Japanese and Chinese sources retain their existing controller. The
ordinary multi-character talk corpus is separate from furniture conversations
and includes its own participant, script and voice identities.

The new controller coordinates upper navigation, lower activity, recruitment,
reservations and cancellation. Shared approach and animation helpers preserve
ownership when a character moves between conversation, furniture, event and
tutorial states. Full-game photo controls and tutorial chapter input have
separate feature owners; the embedded stage does not install those game screens.

`moly-stage` is an independent browser application with the existing playback,
weather, audio, camera, temporary scene restoration and imported-layout paths.
It shares implementation crates with the game and has no dependency on
`moly-game` or `moly-devtools`. Build receipts identify each product and backend.
Matched WebGPU builds currently measure 122,040,178 raw WASM bytes for the
stage and 145,153,835 for the full game (15.9% smaller). Brotli quality 11
measures 10,493,359 and 12,362,768 bytes respectively (15.1% smaller); both
deliver the same base font separately. Corresponding WebGL2 builds are
123,395,058 and 146,842,777 bytes. All four receipts carry source fingerprint
`f82976c8e54d970d27251cc8a145eb65e8de3d87688692e33fc5d58de61b5627`.
These previous integration builds predate the latest site-residency,
avatar tint, empty-lottery and photo-presentation changes.

Display text uses stable IDs and source hashes. Resource snapshots contain
original bindings, while official and community translations have independent
publications. A playback pins its text session; new playback can discover a
new publication. Official Chinese text takes priority over matching human
translations, then original text. This changes display strings only.

The independent font catalogue matches coverage to the engine's actual base
font hash. Supplemental open fonts are fetched only when the selected display
strings require them, then verified before atomic installation. Editors use
the same coverage in NextTrans; they do not build or upload font files.

## Validation status

The implementation is under integration review. Native checks of stage, app
and server-host pass after removing the unplanned tutorial expansion. The
stage's 636-system access audit passed on an earlier integration snapshot and
must be repeated after the latest changes. Protocol and
text-publication checks pass, including unchanged resource text after a
translation edit. These results do not establish full behavioral or visual
equivalence.

Real WebGPU and WebGL2 runs completed ordinary cross-band playback with nonzero audio
before and after resource prewarming. The embedded text flow has also verified
official Chinese priority, supplemental font rendering, active-playback pinning,
next-playback updates, stale-source fallback and Stop cleanup.

A single local run of the same stage scenario observed first playback at 19.5s
for stage and 20.7s for the game assembly. This is not a benchmark distribution.
WASM memory capacity was 655,032,320 and 616,759,296 bytes respectively;
geometric growth capacity is not live allocation, and no memory reduction is
claimed. Six weather selections completed transitions with live particle
emitters and no renderer errors in a WebGPU run. This does not establish
complete weather equivalence.

Current source retains real off-site scene and fixture entities, navigation
contexts, detached visual ownership and captured spatial sound positions.
Exported Director transform chains are required when the source Director is
not instantiated. Imported layouts restore their own supplied layout instead
of substituting the local account's home gate or house. Japanese 7.x empty
conversation selection returns zero without a random draw; earlier source
versions retain their original behavior. The avatar tint now multiplies the
sampled color by the skin color before alpha interpolation.

The photo presenter implements capture animation, Skip, owned frame selection
and a local ZIP containing the original JPEG plus frame identity. It does not
report album-service success or consume film. These changes are native-check
verified; current-build browser and desktop interaction acceptance remains open.

Remaining gates include rebuilding both products and renderers from the final
source, repeating system-access and playback/retry checks, verifying the
imported-layout repair, off-site identity/cancellation/return behavior, avatar
rendering and the full photo pointer/download flow. Matching final artifact
measurements are also pending. A declared state is not proof of a working flow.

Full tutorial progression, mission service expansion, navigation, rank-up and
reward UI are outside this change's scope. Existing tutorial NPC-state adapters
remain. The next UI work should investigate fidelity defects in the existing
inventory and full-game screens, including preview, filtering, recycling,
layout, input and dialog lifetimes; it is not completed by this migration.
