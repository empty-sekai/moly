# Method

moly implements behaviour compatible with the original game client in code written for this repository. This page describes how the behaviour is learned and reimplemented, which parts must match the original and which are moly's own, and how the match is checked.

## Understanding first, then our own implementation

- **Learning the behaviour.** The client's behaviour is learned by observing the game and by reading decompiled client code. Decompilation serves two purposes only: understanding what the client does, and reading its numeric values (constants, table keys, durations, ranges, weights).
- **No mechanical copying.** Decompiled code is not copied, transliterated or used as a template. Each rule is implemented anew in Rust from an understanding of the client's behaviour, in moly's own structure, types and names.
- **Naming what is reproduced.** Where a rule reproduces a specific part of the client, its comment names the original class and method and describes the behaviour in its own words, so a reader can tell which original behaviour the code covers.

## Inputs

- **Game data.** Scenes, models, motions, UI layouts, effects and master tables come from each user's own copy of the game, exported by the extraction tools into a resource root that the game and the server read. The repository contains no game data; tests use synthetic inputs.
- **Server behaviour.** Values that the original server decides are produced by `moly-server` from the player document and the master tables, as described in [Server](server.md). They are named where they are produced, so they can be told apart from client rules.

## What must match and what is moly's own

These must match the original client value for value:

- numbers: constants, formulas, rounding, clamping, integer overflow and the order of floating-point operations where it changes the result;
- timing: frame order, delays, durations and the frame on which a change becomes visible;
- ordering: sort keys, tie-breaking and iteration order where they decide a result;
- randomness: how many draws are taken, in which order, and from which range.

These are moly's own and follow the needs of this repository:

- crate and module boundaries, see [Architecture](architecture.md);
- types, names and error handling;
- scheduling within a frame where the original order does not change a result, and concurrency.

When a case cannot be supported yet, the code refuses it with a message that names the case instead of falling back to a default value.

## How matching is checked

- **Rules on sampled inputs.** The rules in `moly-law` are plain functions without the engine. Their tests run each rule on sampled inputs, including edge cases, and compare every output with values worked out for those inputs.
- **Replayed sequences.** Rules that run over many frames (particle emission clocks, blink cycles) are checked by replaying recorded sequences of inputs and comparing each step's state, not only the final one.
- **Scripted draws.** Random rules are checked with scripted draw values, so the number and order of draws is part of the expected output, including the values on each side of a branch boundary.
- **The running game.** The native game logs the calls it makes in the original's terms (for example, which dialog opened and with which arguments) and can be driven step by step through director scripts and observed through the control port, see [MCP server](mcp.md). This checks that the rules are wired into the game, which the rule tests alone do not show.

Tests that need game data read it from paths given in environment variables and are not part of continuous integration.

## What the repository does not contain

- game resources of any kind, or data derived from them;
- decompiled code or code taken from the original client;
- connections to the original game's servers.

## Keeping it matched

- A change to a rule keeps or extends its value tests; a test that the change breaks is updated only when the expected values are shown to be wrong.
- Comments state a function's inputs, outputs, invariants and edge cases in the code's own words.
- Refactoring (moving code between crates, renaming, splitting modules) does not change values or timing. Each such change is compared against the previous code before it is committed.
