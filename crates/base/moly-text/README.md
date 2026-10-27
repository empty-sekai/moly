# moly-text

Display text resolution shared by the game and the embedded stage. It has no
engine dependency and does not execute scripts or fetch network resources.

`SourceCatalog` reads `moly-text-source/1`; `Overlay` reads
`moly-text-overlay/1`. A `TextSession` pins a source catalogue, locale and
official/community overlay revisions for one playback. Matching official
text takes priority, followed by human translation, then the original.

Bindings identify `(script, current step, text kind)` in the source catalogue.
Stable text IDs identify translations. Every replacement also requires a
matching source hash and the original display text. A source update can reuse
unchanged translations without permitting changed lines to receive stale text.
The source region remains independent of the display locale.

The host must admit the source catalogue against the chosen resource revision
and fetch overlays from its configured publication service. Parsing an overlay
does not authenticate its publisher or establish that its `source` label is
true. Published documents are immutable; draft preview uses a separate path.

The caller receives display text only: leave control-flow labels, actor IDs,
animation commands, voice cues and playback timing in the original script.
The caller also owns glyph coverage and text layout. This library alone does
not install runtime UI or authorize publication.

Inspect caller-provided documents with `cargo run -p moly-text --example inspect
-- SOURCE OVERLAY LOCALE`.
