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
deliver the same base font separately. These are integration-build numbers.

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

The implementation is under integration review. Native checks and the stage's
system-access audit have passed on a combined source snapshot. Protocol and
text-publication checks pass, including unchanged resource text after a
translation edit. These results do not establish full behavioral or visual
equivalence.

Real WebGPU runs completed ordinary cross-band playback with nonzero audio
before and after resource prewarming. The embedded text flow has also verified
official Chinese priority, supplemental font rendering, active-playback pinning,
next-playback updates, stale-source fallback and Stop cleanup.

A single local run of the same stage scenario observed first playback at 19.5s
for stage and 20.7s for the game assembly. This is not a benchmark distribution.
WASM memory capacity was 655,032,320 and 616,759,296 bytes respectively;
geometric growth capacity is not live allocation, and no memory reduction is
claimed. Source-aligned weather shader assets now close all selected passes;
their complete weather presentation still needs browser acceptance.

Remaining gates include WebGL2 and broader playback/retry tests, imported-layout
restoration, and controller completion: tutorial callback producers and turn
restoration, off-site NPC lifecycle, and empty conversation selection. Photo
capture supports a local result; album/film services and full
frame-selection presentation are separate incomplete paths. Do not treat a
declared state or an `awaitingProducer` status as a completed product flow.
