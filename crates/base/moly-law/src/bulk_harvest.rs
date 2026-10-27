//! Engine-independent bulk harvest selection and resource budgets.
//! Callers supply the current site's available fixture and tool rows.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct FixtureKey {
    pub fixture_id: i32,
    pub position_x: i32,
    pub position_z: i32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Candidate {
    pub key: FixtureKey,
    pub target_id: i32,
    pub ordinal: usize,
    pub distance_squared: f32,
    pub hp: i32,
    pub last_attack_stamina: i32,
    pub required_tool_id: Option<i32>,
    pub spawned: bool,
    pub view_exists: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    pub id: i32,
    pub quantity: i32,
    pub durability: i32,
    pub max_durability: i32,
    pub attack_power: i32,
}

impl Tool {
    pub fn remaining_hits(self) -> i64 {
        if self.quantity < 1 || self.durability < 1 || self.max_durability < 1 {
            0
        } else {
            i64::from(self.durability)
                + i64::from(self.max_durability) * i64::from(self.quantity - 1)
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    pub normal: i32,
    pub enhance: i32,
    pub boost: i32,
}

impl Budget {
    pub fn total(self) -> i64 {
        i64::from(self.normal.max(0))
            + i64::from(self.enhance.max(0))
            + i64::from(self.boost.max(0))
    }

    /// CreateStaminaUsedList: boost, enhance, normal; never overdrafts.
    pub fn consume(self, amount: i64) -> Option<Self> {
        if amount < 0
            || amount > self.total()
            || self.normal < 0
            || self.enhance < 0
            || self.boost < 0
        {
            return None;
        }
        let mut out = self;
        let mut left = amount;
        for pool in [&mut out.boost, &mut out.enhance, &mut out.normal] {
            let debit = left.min(i64::from(*pool));
            *pool -= debit as i32;
            left -= debit;
        }
        Some(out)
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub targets: Vec<FixtureKey>,
    pub target_ids: Vec<i32>,
    pub consumed_stamina: i64,
    pub tool_hits: BTreeMap<i32, i64>,
    pub stamina_after: Budget,
    pub exists_tool_shortage_excluded_target: bool,
    pub exists_stamina_shortage_excluded_target: bool,
}

/// Site filtering and target ordinal assignment precede this function.
/// `checked=None` means there is no saved preference; `Some([])` selects none.
pub fn calculate(
    candidates: &[Candidate],
    checked: Option<&[i32]>,
    tools: &[Tool],
    budget: Budget,
) -> Plan {
    let mut ordered: Vec<_> = candidates
        .iter()
        .filter(|row| {
            row.spawned
                && row.view_exists
                && row.hp >= 0
                && row.last_attack_stamina >= 0
                && checked.is_none_or(|ids| ids.contains(&row.target_id))
        })
        .collect();
    ordered.sort_by(|a, b| {
        a.ordinal
            .cmp(&b.ordinal)
            .then_with(|| a.distance_squared.total_cmp(&b.distance_squared))
    });
    let mut remaining: BTreeMap<i32, i64> = tools
        .iter()
        .map(|tool| (tool.id, tool.remaining_hits()))
        .collect();
    let mut seen = BTreeSet::new();
    let mut stamina = budget.total();
    let mut plan = Plan {
        stamina_after: budget,
        ..Plan::default()
    };
    for row in ordered {
        if !seen.insert(row.key) {
            continue;
        }
        let hits = match row.required_tool_id {
            Some(id) => {
                let Some(tool) = tools
                    .iter()
                    .find(|tool| tool.id == id && tool.quantity > 0 && tool.attack_power > 0)
                else {
                    plan.exists_tool_shortage_excluded_target = true;
                    continue;
                };
                let attack = i64::from(tool.attack_power);
                // The extra finishing hit is explicit in the original.
                let hits = (i64::from(row.hp) + attack - 1) / attack + 1;
                if remaining.get(&id).copied().unwrap_or(0) < hits {
                    plan.exists_tool_shortage_excluded_target = true;
                    continue;
                }
                hits
            }
            None => 0,
        };
        let cost = i64::from(row.hp) + i64::from(row.last_attack_stamina);
        if cost > stamina {
            plan.exists_stamina_shortage_excluded_target = true;
            continue;
        }
        stamina -= cost;
        plan.consumed_stamina += cost;
        if let Some(id) = row.required_tool_id {
            *remaining.entry(id).or_default() -= hits;
            *plan.tool_hits.entry(id).or_default() += hits;
        }
        plan.targets.push(row.key);
        if !plan.target_ids.contains(&row.target_id) {
            plan.target_ids.push(row.target_id);
        }
    }
    plan.stamina_after = budget.consume(plan.consumed_stamina).unwrap_or(budget);
    plan
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Gates {
    pub pass_valid: bool,
    pub possession_limit: bool,
    pub treasure_box_not_synced: bool,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[repr(i32)]
pub enum UnavailableReason {
    #[default]
    None = 0,
    PassExpired = 1,
    PossessionLimit = 2,
    TreasureBoxNotSynced = 3,
    StaminaShortage = 4,
    ToolShortage = 5,
    NoTarget = 6,
}

pub fn judge_available(
    gates: Gates,
    candidates: &[Candidate],
    checked: Option<&[i32]>,
    tools: &[Tool],
    budget: Budget,
) -> UnavailableReason {
    use UnavailableReason::{
        NoTarget, PassExpired, PossessionLimit, StaminaShortage, ToolShortage, TreasureBoxNotSynced,
    };
    if !gates.pass_valid {
        return PassExpired;
    }
    if gates.possession_limit {
        return PossessionLimit;
    }
    if gates.treasure_box_not_synced {
        return TreasureBoxNotSynced;
    }
    if budget.total() < 1 {
        return StaminaShortage;
    }
    if !calculate(candidates, checked, tools, budget)
        .targets
        .is_empty()
    {
        return UnavailableReason::None;
    }
    if !calculate(
        candidates,
        checked,
        tools,
        Budget {
            normal: i32::MAX,
            enhance: i32::MAX,
            boost: i32::MAX,
        },
    )
    .targets
    .is_empty()
    {
        return StaminaShortage;
    }
    if candidates.iter().any(|row| {
        row.spawned && row.view_exists && checked.is_none_or(|ids| ids.contains(&row.target_id))
    }) {
        ToolShortage
    } else {
        NoTarget
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn target(id: i32, ordinal: usize, distance: f32, hp: i32, tool: Option<i32>) -> Candidate {
        Candidate {
            key: FixtureKey {
                fixture_id: id,
                position_x: id,
                position_z: 0,
            },
            target_id: id,
            ordinal,
            distance_squared: distance,
            hp,
            last_attack_stamina: 10,
            required_tool_id: tool,
            spawned: true,
            view_exists: true,
        }
    }
    fn tool() -> Tool {
        Tool {
            id: 10,
            quantity: 2,
            durability: 1,
            max_durability: 4,
            attack_power: 40,
        }
    }
    #[test]
    fn target_priority_then_distance_and_explicit_empty_selection() {
        let rows = [
            target(1, 1, 0., 0, None),
            target(2, 0, 90000., 0, None),
            target(3, 0, 1., 0, None),
        ];
        let budget = Budget {
            normal: 100,
            ..Budget::default()
        };
        assert_eq!(calculate(&rows, None, &[], budget).target_ids, [3, 2, 1]);
        assert!(calculate(&rows, Some(&[]), &[], budget).targets.is_empty());
        assert_eq!(calculate(&rows, Some(&[2]), &[], budget).target_ids, [2]);
    }
    #[test]
    fn finishing_hit_exact_tool_and_partial_budget_continue() {
        let rows = [
            target(1, 0, 0., 80, Some(10)),
            target(2, 1, 0., 80, Some(10)),
            target(3, 2, 0., 0, None),
            target(4, 3, 0., 80, Some(5)),
        ];
        let plan = calculate(
            &rows,
            None,
            &[tool()],
            Budget {
                normal: 100,
                ..Budget::default()
            },
        );
        assert_eq!(tool().remaining_hits(), 5);
        assert_eq!(plan.target_ids, [1, 3]);
        assert_eq!(plan.tool_hits[&10], 3);
        assert!(plan.exists_tool_shortage_excluded_target);
        assert_eq!(plan.consumed_stamina, 100);
    }
    #[test]
    fn unaffordable_fixture_does_not_prevent_later_cheaper_one() {
        let plan = calculate(
            &[target(1, 0, 0., 100, None), target(2, 1, 0., 0, None)],
            None,
            &[],
            Budget {
                normal: 20,
                ..Budget::default()
            },
        );
        assert_eq!(plan.target_ids, [2]);
        assert!(plan.exists_stamina_shortage_excluded_target);
    }
    #[test]
    fn pool_debit_and_gate_priority() {
        let b = Budget {
            normal: 50,
            enhance: 30,
            boost: 20,
        };
        assert_eq!(
            b.consume(60),
            Some(Budget {
                normal: 40,
                enhance: 0,
                boost: 0
            })
        );
        assert_eq!(b.consume(101), None);
        let mut gates = Gates {
            pass_valid: false,
            possession_limit: true,
            treasure_box_not_synced: true,
        };
        assert_eq!(
            judge_available(gates, &[], None, &[], b),
            UnavailableReason::PassExpired
        );
        gates.pass_valid = true;
        assert_eq!(
            judge_available(gates, &[], None, &[], b),
            UnavailableReason::PossessionLimit
        );
        gates.possession_limit = false;
        assert_eq!(
            judge_available(gates, &[], None, &[], b),
            UnavailableReason::TreasureBoxNotSynced
        );
        gates.treasure_box_not_synced = false;
        assert_eq!(
            judge_available(gates, &[target(1, 0, 0., 100, None)], None, &[], b),
            UnavailableReason::StaminaShortage
        );
        assert_eq!(
            judge_available(gates, &[target(1, 0, 0., 0, Some(10))], None, &[], b),
            UnavailableReason::ToolShortage
        );
    }
}
