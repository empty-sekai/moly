//! The boost stamina recovery masters and the arithmetic the recovery
//! dialogs put on them.
//!
//! - `mysekaiStaminaRecovery`: one row, `boostQuantity` (the live bonus one
//!   recovery unit spends) and `recoveryBoostStamina` (the boost stamina it
//!   restores).
//! - `mysekaiBoostItems`: each boost item's `recoveryValue`.
//!
//! The rules are the dialogs' own, with the client's `int` arithmetic:
//! `ConvertLiveBonusToBoostStamina` (one count restores `⌊recoveryBoostStamina
//! · count / boostQuantity⌋`), `GetSelectedItemSumValue` (a boost item row is
//! worth `⌊recoveryValue · quantity · recoveryBoostStamina / boostQuantity⌋`),
//! `OnGetItemSelectNumMax` (how much of an item the remaining room allows) and
//! `GetSliderViewData` (the live-bonus slider's upper bound).

/// `MasterMysekaiStaminaRecovery`'s two columns.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RecoveryRow {
    pub boost_quantity: i32,
    pub recovery_boost_stamina: i32,
}

/// `MasterBoostItem`'s recovery column: an item restores `recovery_value`
/// boost stamina.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoostItemRow {
    pub id: i32,
    pub recovery_value: i32,
}

/// `ConvertLiveBonusToBoostStamina`: one count restores
/// `⌊recoveryBoostStamina · count / boostQuantity⌋` (0 with a zero
/// `boostQuantity`).
pub fn converted_boost(row: RecoveryRow, count: i32) -> i32 {
    if row.boost_quantity == 0 {
        0
    } else {
        row.recovery_boost_stamina.wrapping_mul(count) / row.boost_quantity
    }
}

/// `GetSelectedItemSumValue`'s value of one item row: `⌊recoveryValue ·
/// quantity · recoveryBoostStamina / boostQuantity⌋`, or 0 with a zero
/// `boostQuantity`.
pub fn boost_item_value(recovery_value: i32, quantity: i32, row: RecoveryRow) -> i32 {
    if row.boost_quantity == 0 {
        0
    } else {
        recovery_value
            .wrapping_mul(quantity)
            .wrapping_mul(row.recovery_boost_stamina)
            / row.boost_quantity
    }
}

/// The pools `item_select_max` divides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ItemRoom {
    pub boost_max: i32,
    pub boost: i32,
    pub floor: i32,
}

/// `OnGetItemSelectNumMax`: `Math.Max(price, Math.Min(ceiling,
/// (boostMax − (boost + others)) / recoveryValue))`; the room is computed in
/// `int` and divided in single precision, a zero `recoveryValue` leaves the
/// ceiling.
pub fn item_select_max(item: BoostItemRow, room: ItemRoom, others: i32, ceiling: i32) -> i32 {
    let bound = if item.recovery_value == 0 {
        ceiling
    } else {
        let unused = room.boost_max.wrapping_sub(room.boost.wrapping_add(others));
        (unused as f32 / item.recovery_value as f32) as i32
    };
    bound.min(ceiling).max(room.floor)
}

/// `GetSliderViewData`'s upper bound for the live-bonus conversion:
/// `Math.Max(Math.Min(liveBonus, (boostMax − boost) · boostQuantity /
/// recoveryBoostStamina), 1)`, the product in single precision (0 with a zero
/// `recoveryBoostStamina`).
pub fn conversion_slider_max(live_bonus: i32, boost: i32, boost_max: i32, row: RecoveryRow) -> i32 {
    let bound = if row.recovery_boost_stamina == 0 {
        0
    } else {
        (boost_max.saturating_sub(boost) as f32 * row.boost_quantity as f32
            / row.recovery_boost_stamina as f32) as i32
    };
    live_bonus.min(bound).max(1)
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

    fn item(value: i32) -> BoostItemRow {
        BoostItemRow {
            id: value,
            recovery_value: value,
        }
    }

    #[test]
    fn one_live_bonus_restores_the_rows_yield() {
        assert_eq!(converted_boost(row(), 3), 300);
        assert_eq!(
            converted_boost(
                RecoveryRow {
                    boost_quantity: 2,
                    recovery_boost_stamina: 100
                },
                3
            ),
            150
        );
        assert_eq!(
            converted_boost(
                RecoveryRow {
                    boost_quantity: 0,
                    recovery_boost_stamina: 100
                },
                3
            ),
            0
        );
    }

    #[test]
    fn a_boost_item_row_is_worth_its_value_at_the_rate() {
        assert_eq!(boost_item_value(5, 2, row()), 1000);
        assert_eq!(
            boost_item_value(
                5,
                2,
                RecoveryRow {
                    boost_quantity: 2,
                    recovery_boost_stamina: 100
                }
            ),
            500
        );
    }

    #[test]
    fn the_item_ceiling_leaves_the_room_and_the_other_rows() {
        let room = ItemRoom {
            boost_max: 10000,
            boost: 100,
            floor: 1,
        };
        // (10000 - 100) / 5 = 1980, under the held 3000.
        assert_eq!(item_select_max(item(5), room, 0, 3000), 1980);
        // The other rows' values take their share of the room.
        assert_eq!(item_select_max(item(5), room, 500, 3000), 1880);
        // A full pool leaves one.
        assert_eq!(
            item_select_max(
                item(5),
                ItemRoom {
                    boost_max: 100,
                    boost: 100,
                    floor: 1
                },
                0,
                3000
            ),
            1
        );
    }

    #[test]
    fn the_live_bonus_slider_is_bounded_by_the_room_and_the_wallet() {
        // (10000 - 100) * 1 / 100 = 99, the wallet has 500.
        assert_eq!(conversion_slider_max(500, 100, 10000, row()), 99);
        assert_eq!(conversion_slider_max(10, 100, 10000, row()), 10);
        // A full pool leaves one.
        assert_eq!(conversion_slider_max(500, 10000, 10000, row()), 1);
    }
}
