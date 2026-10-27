# Bulk harvest

JP 7 sources provide a bulk hammer action on harvest sites. Hold the button for
the configured charge duration, then release it to execute. Releasing early,
leaving the button, moving the joystick or losing focus cancels the charge.
Reaching a full gauge alone does not execute it.

The action follows the source's pass, material-capacity, treasure-sync, stamina
and tool checks. A full furniture inventory does not globally disable it;
obtaining furniture still checks that inventory's capacity. Target preferences
are saved per site. An absent preference selects every available target, while
an explicitly empty preference selects none.

Targets follow the site's group/target order, then distance from the player.
A target is accepted only when its complete stamina and tool cost fits. An
unaffordable target does not prevent later affordable targets from being
selected. The result retains separate tool and stamina shortage flags.

Ordinary harvest, gathering and bulk harvest use the same persisted server
map. Bulk harvest waits for earlier ordinary requests to settle. The server
checks target identities, before-drop coverage, inventory limits and resource
budgets before committing. Rejected requests debit nothing; replaying an
accepted request cannot grant the same rows again. Existing fallen drops
outside accepted targets remain on the map.

The step timeline binds the player's motion, attached hammer, camera proxy,
sound cues and particle systems. Named Cancel/Finish jumps keep their source
times. Pause holds the sampled pose; Stop ends the director. The camera enters
over 0.5 seconds and normally returns over 0.3 seconds using smoothstep.
Immediate cancellation inherits its last displayed pose.

The resource exporter supplies the timeline, hammer, camera proxy, particle
documents and source UI bindings. These resources are not part of the code
repository. The older offline map generator remains a statistical policy;
this feature does not claim to reproduce the original server's map generation,
timed break schedule or rank rewards.

## Verification

Synthetic tests cover selection order, exact costs, partial affordability,
stamina pools, gating, shared ordinary/bulk settlement, forged and duplicate
requests, replay protection and persisted reload. Timeline tests cover
section pause/stop, named jumps and particle replay behavior.

Release acceptance also requires real interaction with source resources:
early release and pointer exit without debit, a full hold with one settlement,
saved target selection after restart, shortages, no-pass/expired-pass gates,
correct sound and camera restoration, and a second operation in the same
session. A successful build does not establish these interaction results.
