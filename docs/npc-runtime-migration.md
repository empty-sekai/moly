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
Actual size and startup comparisons require completed builds of both products.

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

Remaining gates include completed WebGPU/WebGL2 builds, real playback and
locale/stop/retry tests, imported-layout restoration, and measured stage/game
size and startup. A number of controller details still need source-aligned
completion: tutorial callback producers and turn restoration, fixture arrival
fallbacks, scene-entry/exit presentation, off-site NPC lifecycle, and some IK
branches. Photo capture supports a local result; album/film services and full
frame-selection presentation are separate incomplete paths. Do not treat a
declared state or an `awaitingProducer` status as a completed product flow.
