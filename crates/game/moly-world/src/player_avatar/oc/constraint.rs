//! Node constraints of a model file (`VRMC_node_constraint` 1.0).
//!
//! A constraint drives one node (the destination) from another (the source):
//! `rotation` copies the source's rotation change, `roll` copies the part of
//! it about one axis, and `aim` turns the destination to face the source.
//! The formulas are the spec's, over the rest rotations of both nodes in the
//! file: the rest is the node's local rotation as the file gives it, and the
//! source's rotation is its local rotation in the frame being posed.
//!
//! The pipeline has three steps. [`read_constraints`](super::vrm) parses the
//! file and calls [`order`] to sort the constraints so that every constraint
//! runs after those that write its source (and, for aim, its ancestors). At
//! bind time [`resolve`] maps the node names to the scene's entities. Every
//! frame [`apply`] writes the destinations' rotations.

use super::rig::Frame;
use bevy::prelude::*;
use std::collections::HashMap;

/// The constraint kinds, with the axes the kind reads.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ConstraintKind {
    /// The source's rotation change, applied to the destination.
    Rotation,
    /// The part of the source's rotation change about `axis` (a unit axis of
    /// the destination's rest frame).
    Roll { axis: Vec3 },
    /// The destination turns so that `axis` (a unit axis of its own frame)
    /// points at the source.
    Aim { axis: Vec3 },
}

/// A constraint as the file states it: node indices, and the rest rotations
/// read from the file. The names are the copy's node names ([`super::vrm::node_name`]).
#[derive(Clone, Debug)]
pub struct NodeConstraint {
    pub dst: usize,
    pub src: usize,
    pub dst_name: String,
    pub src_name: String,
    pub kind: ConstraintKind,
    pub weight: f32,
    pub dst_rest: Quat,
    pub src_rest: Quat,
}

/// A constraint resolved to the scene's entities.
#[derive(Clone, Debug)]
pub struct BoundConstraint {
    pub kind: ConstraintKind,
    pub weight: f32,
    pub dst: Entity,
    pub src: Entity,
    /// The destination's chain from the model root down, the destination
    /// last. Aim reads the parent's world rotation from it.
    pub dst_chain: Vec<Entity>,
    /// The source's chain from the model root down, the source last. Aim
    /// reads the source's world position from it.
    pub src_chain: Vec<Entity>,
    pub dst_rest: Quat,
    pub src_rest: Quat,
}

/// Whether `ancestor` is a strict ancestor of `node` in the node tree.
/// `parent` is indexed by node; a walk stops after one step per node, so a
/// malformed cyclic tree cannot loop.
fn is_ancestor(parent: &[Option<usize>], ancestor: usize, node: usize) -> bool {
    let mut at = parent.get(node).copied().flatten();
    for _ in 0..parent.len() {
        let Some(up) = at else {
            return false;
        };
        if up == ancestor {
            return true;
        }
        at = parent.get(up).copied().flatten();
    }
    false
}

/// Sorts the constraints so that each one runs after the constraints that
/// write its source, and, for aim, after those whose destination is an
/// ancestor of its source or its destination. Ties keep file order.
///
/// Returns the sorted constraints and the destinations of the ones that
/// could not be placed: those on a cycle and those that depend on one.
pub fn order(constraints: &[NodeConstraint], parent: &[Option<usize>]) -> (Vec<NodeConstraint>, Vec<usize>) {
    let count = constraints.len();
    let mut needs: Vec<Vec<usize>> = vec![Vec::new(); count];
    for (later_index, later) in constraints.iter().enumerate() {
        let aims = matches!(later.kind, ConstraintKind::Aim { .. });
        for (earlier_index, earlier) in constraints.iter().enumerate() {
            let feeds_source = earlier.dst == later.src;
            let feeds_aim = aims
                && (is_ancestor(parent, earlier.dst, later.src)
                    || is_ancestor(parent, earlier.dst, later.dst));
            if feeds_source || feeds_aim {
                needs[later_index].push(earlier_index);
            }
        }
    }
    let mut done = vec![false; count];
    let mut ordered = Vec::with_capacity(count);
    while let Some(next) = (0..count).find(|&j| !done[j] && needs[j].iter().all(|&i| done[i])) {
        done[next] = true;
        ordered.push(constraints[next].clone());
    }
    let blocked = (0..count)
        .filter(|&j| !done[j])
        .map(|j| constraints[j].dst)
        .collect();
    (ordered, blocked)
}

/// Resolves constraints to scene entities.
///
/// `names` maps each node's name to its entity, `parents` maps each entity
/// below `root` to its parent, and `root` is the model's scene root. A
/// constraint whose nodes are not in the scene is left out with the reason;
/// the rest keep their order.
pub fn resolve(
    constraints: &[NodeConstraint],
    names: &HashMap<&str, Entity>,
    parents: &HashMap<Entity, Entity>,
    root: Entity,
) -> (Vec<BoundConstraint>, Vec<String>) {
    let chain = |entity: Entity| -> Option<Vec<Entity>> {
        let mut chain = vec![entity];
        let mut at = entity;
        while at != root {
            at = *parents.get(&at)?;
            chain.push(at);
            if chain.len() > parents.len() + 1 {
                return None;
            }
        }
        chain.pop();
        chain.reverse();
        Some(chain)
    };
    let mut bound = Vec::new();
    let mut skipped = Vec::new();
    for constraint in constraints {
        let found = |name: &str| -> Result<Entity, String> {
            names
                .get(name)
                .copied()
                .ok_or_else(|| format!("node {name} is not in the scene"))
        };
        let resolved = found(&constraint.dst_name)
            .and_then(|dst| found(&constraint.src_name).map(|src| (dst, src)))
            .and_then(|(dst, src)| {
                let dst_chain = chain(dst)
                    .ok_or_else(|| "the destination is not below the model root".to_owned())?;
                let src_chain = chain(src)
                    .ok_or_else(|| "the source is not below the model root".to_owned())?;
                Ok((dst, src, dst_chain, src_chain))
            });
        match resolved {
            Ok((dst, src, dst_chain, src_chain)) => bound.push(BoundConstraint {
                kind: constraint.kind,
                weight: constraint.weight,
                dst,
                src,
                dst_chain,
                src_chain,
                dst_rest: constraint.dst_rest,
                src_rest: constraint.src_rest,
            }),
            Err(reason) => skipped.push(format!("node {} ({reason})", constraint.dst)),
        }
    }
    (bound, skipped)
}

/// How many constraints of each kind: rotation, roll, aim.
pub fn tally(constraints: &[BoundConstraint]) -> [usize; 3] {
    let mut counts = [0usize; 3];
    for constraint in constraints {
        let slot = match constraint.kind {
            ConstraintKind::Rotation => 0,
            ConstraintKind::Roll { .. } => 1,
            ConstraintKind::Aim { .. } => 2,
        };
        counts[slot] += 1;
    }
    counts
}

/// A transform as a frame (uniform scale: the mean of the axes).
fn frame_of(transform: &Transform) -> Frame {
    Frame::new(
        transform.translation,
        transform.rotation,
        (transform.scale.x + transform.scale.y + transform.scale.z) / 3.0,
    )
}

/// The frame of the chain's last node in the model root's frame, from the
/// nodes' current local transforms.
fn world(chain: &[Entity], transforms: &Query<&mut Transform>) -> Option<Frame> {
    chain.iter().try_fold(Frame::IDENTITY, |frame, entity| {
        transforms
            .get(*entity)
            .ok()
            .map(|transform| frame.then(&frame_of(transform)))
    })
}

/// The rotation from unit `from` to unit `to`, or `None` when either vector
/// has no direction.
fn from_to(from: Vec3, to: Vec3) -> Option<Quat> {
    let (from, to) = (from.normalize_or_zero(), to.normalize_or_zero());
    (from != Vec3::ZERO && to != Vec3::ZERO).then(|| Quat::from_rotation_arc(from, to))
}

/// The spec's rotation and roll formulas: the destination's local rotation
/// from its rest, the source's rest and current local rotations, and the
/// weight. `kind` must be `Rotation` or `Roll`.
pub fn rotation_target(
    kind: ConstraintKind,
    rest: Quat,
    src_rest: Quat,
    src_quat: Quat,
    weight: f32,
) -> Quat {
    let src_delta = src_rest.inverse() * src_quat;
    let target = match kind {
        ConstraintKind::Roll { axis } => {
            let src_delta_in_parent = src_rest * src_delta * src_rest.inverse();
            let src_delta_in_dst = rest.inverse() * src_delta_in_parent * rest;
            let to_vec = src_delta_in_dst * axis;
            match from_to(axis, to_vec) {
                Some(from_to) => rest.slerp(rest * from_to.inverse() * src_delta_in_dst, weight),
                None => rest,
            }
        }
        _ => rest.slerp(rest * src_delta, weight),
    };
    target.normalize()
}

/// The spec's aim formula: the destination's local rotation from its rest,
/// given its parent's world rotation, the destination's and the source's
/// world positions (in the same frame), and the weight.
pub fn aim_target(
    axis: Vec3,
    rest: Quat,
    parent_quat: Quat,
    dst_pos: Vec3,
    src_pos: Vec3,
    weight: f32,
) -> Quat {
    let from_vec = (parent_quat * rest) * axis;
    let target = match from_to(from_vec, src_pos - dst_pos) {
        Some(from_to) => rest.slerp(parent_quat.inverse() * from_to * parent_quat * rest, weight),
        None => rest,
    };
    target.normalize()
}

/// The destination's local rotation this frame, or `None` when a node
/// cannot be read.
fn solve(bound: &BoundConstraint, transforms: &Query<&mut Transform>) -> Option<Quat> {
    match bound.kind {
        ConstraintKind::Rotation | ConstraintKind::Roll { .. } => {
            let src_quat = transforms.get(bound.src).ok()?.rotation;
            Some(rotation_target(
                bound.kind,
                bound.dst_rest,
                bound.src_rest,
                src_quat,
                bound.weight,
            ))
        }
        ConstraintKind::Aim { axis } => {
            let parent_quat = world(&bound.dst_chain[..bound.dst_chain.len() - 1], transforms)?.rot;
            let dst_pos = world(&bound.dst_chain, transforms)?.pos;
            let src_pos = world(&bound.src_chain, transforms)?.pos;
            Some(aim_target(
                axis,
                bound.dst_rest,
                parent_quat,
                dst_pos,
                src_pos,
                bound.weight,
            ))
        }
    }
}

/// Writes each constraint's destination rotation, in the order given. The
/// constraints must already be sorted ([`order`]). A constraint whose source
/// cannot be read leaves its destination as it is.
pub fn apply(constraints: &[BoundConstraint], transforms: &mut Query<&mut Transform>) {
    for bound in constraints {
        let Some(rotation) = solve(bound, transforms) else {
            continue;
        };
        if let Ok(mut transform) = transforms.get_mut(bound.dst) {
            transform.rotation = rotation;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f32::consts::FRAC_PI_2;

    fn node(dst: usize, src: usize, kind: ConstraintKind) -> NodeConstraint {
        NodeConstraint {
            dst,
            src,
            dst_name: format!("oc.n{dst}"),
            src_name: format!("oc.n{src}"),
            kind,
            weight: 1.0,
            dst_rest: Quat::IDENTITY,
            src_rest: Quat::IDENTITY,
        }
    }

    #[test]
    fn order_puts_a_constraint_after_the_one_writing_its_source() {
        // Node 3 copies node 2, and node 2 is itself copied from node 1.
        let parent = vec![None, None, None, None];
        let constraints = vec![
            node(3, 2, ConstraintKind::Rotation),
            node(2, 1, ConstraintKind::Rotation),
        ];
        let (ordered, blocked) = order(&constraints, &parent);
        assert!(blocked.is_empty());
        let dsts: Vec<usize> = ordered.iter().map(|c| c.dst).collect();
        assert_eq!(dsts, vec![2, 3]);
    }

    #[test]
    fn order_reports_a_two_cycle_and_keeps_the_rest() {
        let parent = vec![None, None, None];
        let constraints = vec![
            node(1, 2, ConstraintKind::Rotation),
            node(2, 1, ConstraintKind::Rotation),
            node(0, 1, ConstraintKind::Rotation),
        ];
        let (ordered, blocked) = order(&constraints, &parent);
        assert!(ordered.is_empty());
        let mut blocked = blocked;
        blocked.sort_unstable();
        // Node 0 depends on the cycle, so it is reported too.
        assert_eq!(blocked, vec![0, 1, 2]);
    }

    #[test]
    fn aim_waits_for_writers_of_its_ancestors() {
        // Node 1 and node 2 are children of node 0. Node 0 copies node 3, and
        // the aim on node 2 reads node 1 as its source: the write to node 0,
        // an ancestor of both, must run first.
        let parent = vec![None, Some(0), Some(0), None];
        let aim = ConstraintKind::Aim { axis: Vec3::Y };
        let constraints = vec![
            node(2, 1, aim),
            node(0, 3, ConstraintKind::Rotation),
        ];
        let (ordered, blocked) = order(&constraints, &parent);
        assert!(blocked.is_empty());
        let dsts: Vec<usize> = ordered.iter().map(|c| c.dst).collect();
        assert_eq!(dsts, vec![0, 2]);
    }

    #[test]
    fn rotation_copies_the_source_delta_scaled_by_weight() {
        let rest = Quat::from_rotation_x(0.3);
        let src_rest = Quat::IDENTITY;
        let src_quat = Quat::from_rotation_z(FRAC_PI_2);
        let full = rotation_target(ConstraintKind::Rotation, rest, src_rest, src_quat, 1.0);
        let expected = rest * Quat::from_rotation_z(FRAC_PI_2);
        assert!(full.angle_between(expected) < 1e-5);
        let none = rotation_target(ConstraintKind::Rotation, rest, src_rest, src_quat, 0.0);
        assert!(none.angle_between(rest) < 1e-5);
        let half = rotation_target(ConstraintKind::Rotation, rest, src_rest, src_quat, 0.5);
        let expected_half = rest.slerp(expected, 0.5);
        assert!(half.angle_between(expected_half) < 1e-5);
    }

    #[test]
    fn roll_about_the_source_axis_follows_the_twist() {
        // A source turned 90 degrees about Z, rolled about Z: the destination
        // turns by the same 90 degrees about its own Z.
        let rest = Quat::IDENTITY;
        let turned = Quat::from_rotation_z(FRAC_PI_2);
        let rolled = rotation_target(
            ConstraintKind::Roll { axis: Vec3::Z },
            rest,
            Quat::IDENTITY,
            turned,
            1.0,
        );
        assert!(rolled.angle_between(turned) < 1e-5);
        let unrolled = rotation_target(
            ConstraintKind::Roll { axis: Vec3::Z },
            rest,
            Quat::IDENTITY,
            turned,
            0.0,
        );
        assert!(unrolled.angle_between(rest) < 1e-5);
    }

    #[test]
    fn aim_turns_the_axis_toward_the_source() {
        // The parent is unrotated and the destination's rest is identity, so
        // the aim axis is +X: a source on +Y turns it by 90 degrees about Z.
        let aimed = aim_target(
            Vec3::X,
            Quat::IDENTITY,
            Quat::IDENTITY,
            Vec3::ZERO,
            Vec3::Y,
            1.0,
        );
        assert!(aimed.angle_between(Quat::from_rotation_z(FRAC_PI_2)) < 1e-5);
        let degenerate = aim_target(Vec3::X, Quat::IDENTITY, Quat::IDENTITY, Vec3::ZERO, Vec3::ZERO, 1.0);
        assert!(degenerate.angle_between(Quat::IDENTITY) < 1e-5);
    }
}
