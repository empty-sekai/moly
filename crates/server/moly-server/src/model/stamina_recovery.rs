//! The boost stamina recovery: `PostUserMysekaiBoostStaminaRecoveryApi`
//! (`UserMysekaiStaminaRequest`), the call the recover dialog's confirmation
//! sends.
//!
//! The request names the recovery type and the resources it spends:
//!
//! - `boost` — live bonus (`resourceId` 0): `quantity` live bonus become
//!   `⌊recoveryBoostStamina · quantity / boostQuantity⌋` boost stamina;
//! - `jewel` — paid crystals (`resourceId` 1): the same conversion, charged
//!   to the crystal wallet;
//! - `boost_item` — boost items (`resourceId` = a `mysekaiBoostItems` id):
//!   each row adds `⌊recoveryValue · quantity · recoveryBoostStamina /
//!   boostQuantity⌋` boost stamina.
//!
//! The boost pool gains what the type restores, up to its master maximum, and
//! the reply is a `SuiteUser` reply carrying the stamina section.
//!
//! Policies (`policies.staminaRecovery`, both optional):
//!
//! - `reply` — `success` or `failure` (the `policies.musicPlayReply` form): a
//!   failure answers `success: false` without changing anything.
//! - `capital` — the resources a recovery may spend without being charged,
//!   `[{resourceType, resourceId, quantity}]`; a resource the policy does not
//!   state is not charged (the source's live bonus and crystals are the
//!   account's wallet, which the user document does not carry), and a stated
//!   one is spent and refused when the quantity exceeds what is left.

use std::collections::BTreeMap;

use bevy::log::info;

use moly_protocol::api::{BoostStaminaRecoveryType, SuiteUserReply, UserMysekaiStaminaRequest};
use moly_protocol::json::SECTION_STAMINA;
use serde_json::{json, Map, Value};

use super::{ResponseKind, ServerModel, Tables};

/// `policies.staminaRecovery`.
pub(crate) const POLICY_KEY: &str = "staminaRecovery";
const POLICY_REPLY: &str = "reply";
const POLICY_CAPITAL: &str = "capital";
const REPLY_SUCCESS: &str = "success";
const REPLY_FAILURE: &str = "failure";

/// The answer a recovery gives by policy.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum RecoveryReplyPolicy {
    #[default]
    Success,
    Failure,
}

/// `policies.staminaRecovery` as the document states it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RecoveryPolicy {
    reply: RecoveryReplyPolicy,
    /// `(resourceType, resourceId) -> quantity left`.
    capital: BTreeMap<(String, i32), i32>,
}

/// `policies.staminaRecovery`, when the document states it.
pub(crate) fn parse_policies(value: Option<&Value>) -> Result<RecoveryPolicy, String> {
    let mut policy = RecoveryPolicy::default();
    let Some(value) = value else {
        return Ok(policy);
    };
    let at = format!("policies.{POLICY_KEY}");
    let row = value.as_object().ok_or_else(|| format!("{at} is not an object"))?;
    for key in row.keys() {
        if key != POLICY_REPLY && key != POLICY_CAPITAL {
            return Err(format!("{at}.{key} is not a field"));
        }
    }
    if let Some(reply) = row.get(POLICY_REPLY) {
        policy.reply = match reply.as_str() {
            Some(REPLY_SUCCESS) => RecoveryReplyPolicy::Success,
            Some(REPLY_FAILURE) => RecoveryReplyPolicy::Failure,
            _ => {
                return Err(format!(
                    "{at}.{POLICY_REPLY} must be {REPLY_SUCCESS:?} or {REPLY_FAILURE:?}"
                ))
            }
        };
    }
    if let Some(capital) = row.get(POLICY_CAPITAL) {
        let rows = capital
            .as_array()
            .ok_or_else(|| format!("{at}.{POLICY_CAPITAL} is not an array"))?;
        for (index, value) in rows.iter().enumerate() {
            let at = format!("{at}.{POLICY_CAPITAL}[{index}]");
            let row = moly_protocol::json::object(value, &at)?;
            let kind = row
                .get("resourceType")
                .and_then(Value::as_str)
                .ok_or_else(|| format!("{at}.resourceType is not a string"))?
                .to_owned();
            let id = moly_protocol::json::int32(row, "resourceId", &at)?;
            let quantity = moly_protocol::json::int32(row, "quantity", &at)?;
            if quantity < 0 {
                return Err(format!("{at}.quantity = {quantity} is negative"));
            }
            policy.capital.insert((kind, id), quantity);
        }
    }
    Ok(policy)
}

/// Writes the policy back into the document.
pub(crate) fn write_policy(policy: &RecoveryPolicy, policies: &mut Map<String, Value>) {
    if policy.reply == RecoveryReplyPolicy::Success && policy.capital.is_empty() {
        return;
    }
    let mut row = Map::new();
    if policy.reply == RecoveryReplyPolicy::Failure {
        row.insert(POLICY_REPLY.into(), json!(REPLY_FAILURE));
    }
    if !policy.capital.is_empty() {
        row.insert(
            POLICY_CAPITAL.into(),
            Value::Array(
                policy
                    .capital
                    .iter()
                    .map(|((kind, id), quantity)| {
                        json!({"resourceType": kind, "resourceId": id, "quantity": quantity})
                    })
                    .collect(),
            ),
        );
    }
    policies.insert(POLICY_KEY.into(), Value::Object(row));
}

/// A refusal of the recovery.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Refusal {
    pub(crate) code: &'static str,
    pub(crate) reason: String,
}

impl ServerModel {
    /// One boost stamina recovery. `Err` refuses without a change; a
    /// successful answer records the stamina section.
    pub(crate) fn boost_stamina_recovery(
        &mut self,
        request: &UserMysekaiStaminaRequest,
        masters: &Tables<'_>,
    ) -> Result<SuiteUserReply, Refusal> {
        let refusal = |code: &'static str, reason: String| Refusal { code, reason };
        if self.doc.stamina_recovery.reply == RecoveryReplyPolicy::Failure {
            return Ok(SuiteUserReply {
                success: false,
                refusal: Some(format!("policies.{POLICY_KEY}.{POLICY_REPLY} = {REPLY_FAILURE}")),
                updated: Default::default(),
            });
        }
        let recovery = masters
            .recovery
            .ok_or_else(|| refusal("stamina_masters_missing", "mysekaiStaminaRecovery is not seated".to_owned()))?;
        let max = masters
            .stamina_max()
            .ok_or_else(|| refusal("stamina_masters_missing", "mysekaiStaminas is not seated".to_owned()))?;
        let Some(stamina) = self.doc.stamina else {
            return Err(refusal("stamina_not_seated", "userMysekaiStamina is not seated".to_owned()));
        };
        let consumes = &request.user_mysekai_stamina_recovery_consume_resources;
        if consumes.is_empty() {
            return Err(refusal("recovery_refused", "the request spends nothing".to_owned()));
        }
        let mut gained: i32 = 0;
        let mut rows: Vec<(String, i32, i32)> = Vec::new();
        for consume in consumes {
            if consume.quantity < 0 {
                return Err(refusal(
                    "recovery_refused",
                    format!("resource {} quantity {} is negative", consume.resource_id, consume.quantity),
                ));
            }
        }
        match request.mysekai_boost_stamina_recovery_type {
            BoostStaminaRecoveryType::Boost | BoostStaminaRecoveryType::Jewel => {
                let expected = if request.mysekai_boost_stamina_recovery_type == BoostStaminaRecoveryType::Jewel {
                    1
                } else {
                    0
                };
                for consume in consumes {
                    if consume.resource_id != expected {
                        return Err(refusal(
                            "recovery_refused",
                            format!(
                                "{} spends resource {}, not {expected}",
                                request.mysekai_boost_stamina_recovery_type.word(),
                                consume.resource_id
                            ),
                        ));
                    }
                    gained = gained.wrapping_add(moly_masters::converted_boost(recovery, consume.quantity));
                    rows.push((request.mysekai_boost_stamina_recovery_type.word().to_owned(), consume.resource_id, consume.quantity));
                }
            }
            BoostStaminaRecoveryType::BoostItem => {
                let items = masters
                    .boost_items
                    .as_deref()
                    .ok_or_else(|| refusal("stamina_masters_missing", "mysekaiBoostItems is not seated".to_owned()))?;
                for consume in consumes {
                    let item = items.iter().find(|item| item.id == consume.resource_id).ok_or_else(|| {
                        refusal(
                            "recovery_refused",
                            format!("boost item {} is not a master boost item", consume.resource_id),
                        )
                    })?;
                    gained = gained.wrapping_add(moly_masters::boost_item_value(
                        item.recovery_value,
                        consume.quantity,
                        recovery,
                    ));
                    rows.push(("boost_item".to_owned(), consume.resource_id, consume.quantity));
                }
            }
        }
        if gained <= 0 {
            return Err(refusal("recovery_refused", "the request restores no boost stamina".to_owned()));
        }
        if stamina.boost >= max.boost {
            return Err(refusal(
                "boost_stamina_full",
                format!("userMysekaiStamina.boostStamina {} is at its master maximum {}", stamina.boost, max.boost),
            ));
        }
        for (kind, id, quantity) in &rows {
            if let Some(left) = self.doc.stamina_recovery.capital.get_mut(&(kind.clone(), *id)) {
                if *quantity > *left {
                    return Err(refusal(
                        "recovery_refused",
                        format!("{kind} {id}: quantity {quantity} is above the {left} left"),
                    ));
                }
            }
        }
        for (kind, id, quantity) in &rows {
            if let Some(left) = self.doc.stamina_recovery.capital.get_mut(&(kind.clone(), *id)) {
                *left -= quantity;
            }
        }
        let boost = stamina.boost.saturating_add(gained).min(max.boost);
        self.doc.stamina = Some(moly_protocol::rows::Stamina { boost, ..stamina });
        self.commit();
        info!(
            "[server] PostUserMysekaiBoostStaminaRecoveryApi: {} {:?} restores {gained}; boost {} -> {boost} (maximum {})",
            request.mysekai_boost_stamina_recovery_type.word(),
            consumes,
            stamina.boost,
            max.boost
        );
        self.respond(ResponseKind::BoostStaminaRecovery, false, &[SECTION_STAMINA], masters);
        let updated = self
            .responses
            .last()
            .expect("the response just recorded")
            .sections
            .inventory
            .clone();
        Ok(SuiteUserReply {
            success: true,
            refusal: None,
            updated,
        })
    }
}
