//! GridLayoutGroup's measurement and two-axis placement, without child metrics.

use super::{Sizes, clamp, integer, padding};
use bevy::math::Vec2;
use serde_json::Value;

#[derive(Clone, Copy)]
pub(super) struct Grid {
    pub(super) cell: Vec2,
    spacing: Vec2,
    padding: [i32; 4],
    alignment: i32,
    corner: i32,
    axis: usize,
    constraint: i32,
    count: i32,
}

impl Grid {
    pub(super) fn parse(fields: &Value) -> Result<Self, String> {
        let grid = Self {
            cell: vector(fields, "m_CellSize")?,
            spacing: vector(fields, "m_Spacing")?,
            padding: padding(fields)?,
            alignment: integer(fields, "m_ChildAlignment")?,
            corner: integer(fields, "m_StartCorner")?,
            axis: integer(fields, "m_StartAxis")? as usize,
            constraint: integer(fields, "m_Constraint")?,
            count: integer(fields, "m_ConstraintCount")?,
        };
        if !(0..=8).contains(&grid.alignment) || !(0..=3).contains(&grid.corner)
            || grid.axis > 1 || !(0..=2).contains(&grid.constraint) || grid.count < 1
        {
            return Err("invalid GridLayoutGroup mode or constraint count".into());
        }
        Ok(grid)
    }

    fn padding_sum(self, axis: usize) -> f32 {
        self.padding[axis * 2] as f32 + self.padding[axis * 2 + 1] as f32
    }

    fn cells_fitting(self, axis: usize, size: Vec2) -> i32 {
        let value = ((size[axis] - self.padding_sum(axis) + self.spacing[axis] + 0.001)
            / (self.cell[axis] + self.spacing[axis])).floor();
        // The managed conversion returns int.MinValue outside the signed range,
        // then Max(1, ...) applies. Rust's saturating float cast differs for +inf.
        let count = if !value.is_finite() || value < i32::MIN as f32 || value >= 2147483648. {
            i32::MIN
        } else { value as i32 };
        count.max(1)
    }

    pub(super) fn metrics(self, axis: usize, children: usize, size: Vec2) -> Sizes {
        let n = children as f32;
        let (min, preferred) = if axis == 0 {
            match self.constraint {
                1 => (self.count as f32, self.count as f32),
                2 => { let columns = (n / self.count as f32 - 0.001).ceil(); (columns, columns) }
                _ => (1., n.sqrt().ceil()),
            }
        } else {
            let rows = match self.constraint {
                1 => (n / self.count as f32 - 0.001).ceil(),
                2 => self.count as f32,
                _ => (n / self.cells_fitting(0, size) as f32).ceil(),
            };
            (rows, rows)
        };
        let extent = |count| self.padding_sum(axis)
            + (self.cell[axis] + self.spacing[axis]) * count - self.spacing[axis];
        Sizes { min: extent(min), preferred: extent(preferred), flexible: -1. }
    }

    pub(super) fn positions(self, children: usize, size: Vec2) -> Vec<Vec2> {
        let n = children as i32;
        let mut cells = [1, 1];
        if self.constraint != 0 {
            let fixed = if self.constraint == 1 { 0 } else { 1 };
            cells[fixed] = self.count;
            if n > self.count { cells[1 - fixed] = (n + self.count - 1) / self.count; }
        } else {
            for (axis, count) in cells.iter_mut().enumerate() {
                *count = if self.cell[axis] + self.spacing[axis] <= 0. {
                    i32::MAX
                } else { self.cells_fitting(axis, size) };
            }
        }
        let per_main = cells[self.axis];
        let bound = |value: i32, maximum: i32| clamp(value as f32, 1., maximum as f32) as i32;
        let cross = 1 - self.axis;
        let fixed_cross = self.constraint == if self.axis == 0 { 2 } else { 1 };
        let mut actual = cells;
        actual[self.axis] = bound(per_main, n);
        actual[cross] = if fixed_cross { cells[cross].min(n) }
            else { bound(cells[cross], (n as f32 / per_main as f32).ceil() as i32) };
        let mut start = Vec2::ZERO;
        for axis in 0..2 {
            let required = actual[axis] as f32 * self.cell[axis]
                + (actual[axis] - 1) as f32 * self.spacing[axis];
            let align = if axis == 0 { self.alignment % 3 } else { self.alignment / 3 };
            start[axis] = self.padding[axis * 2] as f32
                + (size[axis] - self.padding_sum(axis) - required) * align as f32 * 0.5;
        }
        // The source redistributes the final cells when a fixed cross-axis
        // constraint would otherwise leave one of its promised rows/columns empty.
        let used = (n as f32 / per_main as f32).ceil() as i32;
        let mut moved = 0;
        if n > self.count && used < self.count {
            moved = self.count - used;
            moved += (moved as f32 / (per_main as f32 - 1.)).floor() as i32;
            if n % per_main == 1 { moved += 1; }
        }
        (0..n).map(|index| {
            let mut cell = [0, 0];
            if fixed_cross && n - index <= moved {
                cell[cross] = self.count - (n - index);
            } else {
                cell[self.axis] = index % per_main;
                cell[cross] = index / per_main;
            }
            if self.corner % 2 == 1 { cell[0] = actual[0] - 1 - cell[0]; }
            if self.corner / 2 == 1 { cell[1] = actual[1] - 1 - cell[1]; }
            start + (self.cell + self.spacing) * Vec2::new(cell[0] as f32, cell[1] as f32)
        }).collect()
    }
}

fn vector(fields: &Value, name: &str) -> Result<Vec2, String> {
    let values = fields[name].as_array().filter(|values| values.len() == 2)
        .ok_or_else(|| format!("layout field {name} must have two components"))?;
    let number = |index: usize| values[index].as_f64().map(|n| n as f32)
        .filter(|n| n.is_finite()).ok_or_else(|| format!("invalid layout vector {name}"));
    Ok(Vec2::new(number(0)?, number(1)?))
}
