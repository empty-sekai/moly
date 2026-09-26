//! Developer instrument: drives the gate and cut-scene entries of
//! [`super::gate_entries`] and [`super::dispose`] from a script, so a run can
//! exercise each entry before the gate lane calls it. Only with developer
//! tools installed and `MOLY_NPC_ENTRY_PROBE` set.
//!
//! The script is `;`-separated steps `SECONDS:OP[:UNIT[:ARG]]`, each run once
//! when the app clock passes `SECONDS`:
//! - `create:UNIT`: the invite's CreateNPC at the gate's first end locator
//!   with the invite rotation, then its two waits and its calls after them;
//! - `dispose:UNIT`, `dispose_all`;
//! - `hide_cancel:UNIT`, `hide:UNIT`, `show:UNIT`, `cancel:UNIT`,
//!   `force_update:UNIT`;
//! - `takeover:UNIT` (the cut-scene presenter's takeover), `restore:UNIT`
//!   (its RestoreStates), `on_start:UNIT` (the go-home start callback with
//!   that target);
//! - `move:UNIT:SITE`: the change-site controller's MoveNPC to that site type
//!   value (0 home, 1 the first floor, ...), whose floor entry on the
//!   player's site knocks, opens the room door and waits for its open clip.
//!
//! Every step writes one `[npc-entry-probe]` line.

use bevy::prelude::*;

use super::{dispose, gate_entries};

const PROBE: &str = "MOLY_NPC_ENTRY_PROBE";

struct Step {
    at: f64,
    op: String,
    unit: Option<u32>,
    arg: Option<i32>,
    done: bool,
}

/// An invite create waiting for its two waits.
struct Invite {
    unit: u32,
    entity: Entity,
    since: f64,
    exists_at: Option<f64>,
}

#[derive(Resource, Default)]
pub(crate) struct EntryProbe {
    parsed: bool,
    steps: Vec<Step>,
    invites: Vec<Invite>,
}

fn parse(script: &str) -> Vec<Step> {
    script
        .split(';')
        .filter(|step| !step.trim().is_empty())
        .filter_map(|step| {
            let mut parts = step.trim().split(':');
            let at = parts.next()?.trim().parse::<f64>().ok()?;
            let op = parts.next()?.trim().to_owned();
            let unit = parts
                .next()
                .and_then(|unit| unit.trim().parse::<u32>().ok());
            let arg = parts.next().and_then(|arg| arg.trim().parse::<i32>().ok());
            Some(Step {
                at,
                op,
                unit,
                arg,
                done: false,
            })
        })
        .collect()
}

fn gate(world: &mut World) -> Option<Entity> {
    world
        .query::<(
            Entity,
            &crate::fixture_activity_state::FixtureActivityIdentity,
        )>()
        .iter(world)
        .find(|(_, identity)| identity.model_package.contains("_gate_"))
        .map(|(entity, _)| entity)
}

fn run_step(
    world: &mut World,
    probe: &mut EntryProbe,
    op: &str,
    unit: Option<u32>,
    arg: Option<i32>,
    t: f64,
) {
    let npc = unit.and_then(|unit| gate_entries::find_npc(world, unit));
    let word = match (op, unit, npc) {
        ("dispose_all", _, _) => format!("{:?}", dispose::dispose_npc_all(world, "probe")),
        ("create", Some(unit), _) => {
            let pose = gate(world)
                .ok_or_else(|| "no gate fixture".to_owned())
                .and_then(|gate| gate_entries::gate_first_end_loc(world, gate));
            match pose.and_then(|position| {
                gate_entries::create_npc(world, unit, position, gate_entries::invite_rotation())
            }) {
                Ok(entity) => {
                    probe.invites.push(Invite {
                        unit,
                        entity,
                        since: t,
                        exists_at: None,
                    });
                    format!("created {entity:?}")
                }
                Err(reason) => format!("refused: {reason}"),
            }
        }
        ("on_start", Some(unit), _) => {
            format!("{:?}", gate_entries::on_start_cut_scene(world, unit))
        }
        (_, Some(unit), None) => format!("unit {unit} is not present"),
        ("dispose", _, Some(npc)) => format!("{:?}", dispose::dispose_npc(world, npc, "probe")),
        ("hide_cancel", _, Some(npc)) => {
            format!(
                "cancel {}",
                gate_entries::hide_and_cancel_objective(world, npc)
            )
        }
        ("hide", _, Some(npc)) => {
            gate_entries::hide(world, npc);
            "hidden".to_owned()
        }
        ("show", _, Some(npc)) => {
            gate_entries::show(world, npc);
            "shown".to_owned()
        }
        ("cancel", _, Some(npc)) => {
            format!(
                "cancel {}",
                gate_entries::try_cancel_current_objective(world, npc)
            )
        }
        ("force_update", _, Some(npc)) => {
            format!(
                "cancel {}",
                gate_entries::force_update_objective(world, npc)
            )
        }
        ("takeover", _, Some(npc)) => {
            format!(
                "cancel {}",
                gate_entries::take_over_for_cut_scene(world, npc)
            )
        }
        ("restore", _, Some(npc)) => {
            gate_entries::restore_state(world, npc);
            "idle".to_owned()
        }
        ("move", Some(unit), Some(npc)) => match arg {
            Some(site) => {
                let frame = world
                    .get_resource::<bevy::diagnostic::FrameCount>()
                    .map_or(0, |count| count.0);
                format!(
                    "MoveNPC({site}) took {}",
                    crate::npc::change_site_state::move_npc(world, npc, unit, site, frame)
                )
            }
            None => "no site".to_owned(),
        },
        _ => "unknown step".to_owned(),
    };
    info!("[npc-entry-probe] t={t:.3} {op} {unit:?} {arg:?}: {word}");
}

/// One frame of the probe (called from the gate's appearance system).
pub(crate) fn step(world: &mut World) {
    if !crate::dev_tools::installed() {
        return;
    }
    world.init_resource::<EntryProbe>();
    let mut probe = std::mem::take(&mut *world.resource_mut::<EntryProbe>());
    if !probe.parsed {
        probe.parsed = true;
        if let Ok(script) = std::env::var(PROBE) {
            probe.steps = parse(&script);
            info!(
                "[npc-entry-probe] {PROBE}={script:?}: {} steps",
                probe.steps.len()
            );
        }
    }
    let t = world.resource::<Time>().elapsed_secs_f64();
    let mut index = 0;
    while index < probe.steps.len() {
        if !probe.steps[index].done && t >= probe.steps[index].at {
            probe.steps[index].done = true;
            let op = probe.steps[index].op.clone();
            let unit = probe.steps[index].unit;
            let arg = probe.steps[index].arg;
            run_step(world, &mut probe, &op, unit, arg, t);
        }
        index += 1;
    }
    let mut waiting = Vec::new();
    for mut invite in std::mem::take(&mut probe.invites) {
        if world.get_entity(invite.entity).is_err() {
            info!(
                "[npc-entry-probe] t={t:.3} invite unit {}: the created NPC is gone",
                invite.unit
            );
            continue;
        }
        let (exists, decided) = gate_entries::invite_waits(world, invite.entity);
        if exists && invite.exists_at.is_none() {
            invite.exists_at = Some(t);
            info!(
                "[npc-entry-probe] t={t:.3} invite unit {}: IsExistNPCAll true after {:.3} s",
                invite.unit,
                t - invite.since
            );
        }
        if decided {
            let objective = world
                .get::<crate::npc_objective::ObjectiveMind>(invite.entity)
                .and_then(|mind| mind.current);
            let (cancelled, updated) = gate_entries::invite_show(world, invite.entity);
            info!("[npc-entry-probe] t={t:.3} invite unit {}: current objective {objective:?}; TryCancelCurrentObjective {cancelled}, ForceUpdateObjective {updated}, Show", invite.unit);
        } else {
            waiting.push(invite);
        }
    }
    probe.invites = waiting;
    *world.resource_mut::<EntryProbe>() = probe;
}
