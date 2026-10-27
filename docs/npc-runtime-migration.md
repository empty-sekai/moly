# NPC runtime and embedded stage migration

The source descriptor chooses the NPC controller. Japanese sources with an
application major version of 7 or later use the hierarchical controller;
earlier Japanese and Chinese sources retain their existing controller.
Ordinary multi-character conversations have their own participant, script and
voice identities, separate from furniture conversations. Updating the corpus
alone does not implement the new controller's behavior.

The hierarchical controller coordinates navigation, activities, recruitment,
reservations and cancellation. Shared helpers preserve ownership as a
character moves between conversation, furniture and other activities.
Off-site scenes retain their entities, navigation contexts, detached visuals
and spatial sound positions. Exported Director transform chains are required
when the corresponding source Director is not instantiated.

## Independent stage

`moly-stage` is an independent application with playback, weather, audio,
camera, temporary scene restoration and imported layouts. It shares engine
crates with the game and does not depend on `moly-game` or `moly-devtools`.
It omits full-game inventory, harvest and other game screens. Imported layouts
restore the supplied scene instead of substituting the local account's home.

The browser release builds stage and game for both WebGPU and WebGL2.
Receipts identify each product, renderer and actual source fingerprint.
Compare package sizes only across builds with the same source fingerprint;
allocated WASM memory capacity does not measure live memory use.

## Display text and fonts

Text bindings use stable identities and source hashes. Resource snapshots
contain original bindings; official and community translations have separate
publications. A translation edit therefore does not change the resource
snapshot identity. Playback pins a text session, and the next playback can
discover a new text publication.

For Chinese display, matching official Chinese text takes priority, followed
by a matching community translation and then the original text. Only display
strings change: Japanese scenes, motion, voice and timing remain Japanese.
An unmatched or stale translation cannot replace the source string.

The font catalogue matches coverage to the engine's actual base font hash.
Supplemental open fonts are fetched only when the selected display strings
need them, then verified before atomic installation. Translators use the
existing NextTrans review workflow and do not edit Lua or build font files.
Distributed font packages include their open-source license and hash records.

## Full-game changes

The game restores source road editing, weather selection and list scrolling,
including confirmation, saved state and cancellation. JP 7 bulk harvest adds
the original charge/release interaction, target preferences, hammer timeline
and authoritative settlement. See [Bulk harvest](bulk-harvest.md) and
[Server](server.md) for the resource and protocol requirements.

The photo presenter supports capture animation, Skip, owned frame selection
and a local ZIP containing the original JPEG plus frame identity. It does not
report album-service success or consume film.

Full tutorial progression, mission-service expansion and rank rewards remain
outside this change. Existing tutorial NPC-state adapters remain. These
changes do not establish complete fidelity of every inventory or game screen.

## Acceptance

Release verification must use the final source and its matching resources:

- Run shared rule, protocol, settlement and timeline tests, and audit native
  stage and full-game system access.
- Exercise cross-band playback, cancellation, scene restoration and retry on
  both browser renderers; check sound and resource retirement.
- Check official Chinese priority, supplemental fonts, active-playback text
  pinning, next-playback updates and stale-source fallback. Editing a text
  publication must leave the resource snapshot unchanged.
- Exercise road placement, scrolling, weather save/cancel/restart and bulk
  harvest's success, cancellation, shortages and replay protections.
- Verify production assets and application pointers, then repeat playback in
  the production host. Build success and HTTP availability alone do not
  establish these interaction results.
