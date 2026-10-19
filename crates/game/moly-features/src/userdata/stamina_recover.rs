//! The boost stamina recovery request: `PostUserMysekaiBoostStaminaRecoveryApi`
//! (`UserMysekaiStaminaRequest`), sent by the recover dialog's confirmation.
//!
//! The reply is a `SuiteUser` reply whose stamina section the client copies
//! take before the answer is read, so the recover dialog's after-views, the
//! field HUD's gauge and the menu cell all read the recovered pools from the
//! copies, never from the reply.

use moly_masters::{BoostItemRow, RecoveryRow};
use moly_protocol::api::{
    BoostStaminaRecoveryType, StaminaRecoveryConsume, UserMysekaiStaminaRequest,
};
use moly_protocol::rows::Stamina;

use super::client::ClientUserData;

/// The client's stamina pools, as `HarvestUtility.GetStaminaData(0)` reads
/// them: the copy, else the harvest model's.
pub fn stamina(user: Option<&ClientUserData>, model: Option<Stamina>) -> Option<Stamina> {
    user.and_then(|copy| copy.stamina).or(model)
}

/// The request of one recovery: the type and the `(resource id, quantity)`
/// rows the dialog built.
pub fn request(
    recovery_type: BoostStaminaRecoveryType,
    consumes: Vec<(i32, i32)>,
) -> UserMysekaiStaminaRequest {
    UserMysekaiStaminaRequest {
        mysekai_boost_stamina_recovery_type: recovery_type,
        user_mysekai_stamina_recovery_consume_resources: consumes
            .into_iter()
            .map(|(resource_id, quantity)| StaminaRecoveryConsume {
                resource_id,
                quantity,
            })
            .collect(),
    }
}

/// The boost stamina a request restores under the recovery master row: the
/// server's own rule, read here for the confirmation and the log line.
pub fn recovered(
    request: &UserMysekaiStaminaRequest,
    row: RecoveryRow,
    items: &[BoostItemRow],
) -> i32 {
    let quantity = |id: i32| {
        request
            .user_mysekai_stamina_recovery_consume_resources
            .iter()
            .find(|consume| consume.resource_id == id)
            .map_or(0, |consume| consume.quantity)
    };
    match request.mysekai_boost_stamina_recovery_type {
        BoostStaminaRecoveryType::Boost => moly_masters::converted_boost(row, quantity(0)),
        BoostStaminaRecoveryType::Jewel => moly_masters::converted_boost(row, quantity(1)),
        BoostStaminaRecoveryType::BoostItem => request
            .user_mysekai_stamina_recovery_consume_resources
            .iter()
            .filter(|consume| consume.quantity > 0)
            .map(|consume| {
                let value = items
                    .iter()
                    .find(|item| item.id == consume.resource_id)
                    .map_or(0, |item| item.recovery_value);
                moly_masters::boost_item_value(value, consume.quantity, row)
            })
            .fold(0i32, i32::wrapping_add),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row() -> RecoveryRow {
        RecoveryRow {
            boost_quantity: 1,
            recovery_boost_stamina: 100,
        }
    }

    #[test]
    fn a_live_bonus_request_restores_the_converted_count() {
        let request = request(BoostStaminaRecoveryType::Boost, vec![(0, 3)]);
        assert_eq!(recovered(&request, row(), &[]), 300);
    }

    #[test]
    fn a_boost_item_request_sums_the_rows_values() {
        let request = request(BoostStaminaRecoveryType::BoostItem, vec![(7, 2), (9, 1)]);
        let items = [
            BoostItemRow {
                id: 7,
                recovery_value: 5,
            },
            BoostItemRow {
                id: 9,
                recovery_value: 60,
            },
        ];
        assert_eq!(recovered(&request, row(), &items), 2 * 5 * 100 + 60 * 100);
    }

    #[test]
    fn a_jewel_request_restores_the_crystal_count() {
        let request = request(BoostStaminaRecoveryType::Jewel, vec![(0, 50), (1, 4)]);
        assert_eq!(recovered(&request, row(), &[]), 400);
    }
}
