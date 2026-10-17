//! The harvest tool master (`mysekaiTools`).

use serde_json::Value;

/// The `mysekaiTools` fields the server reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ToolMaster {
    pub id: i32,
    pub tool_type: String,
    pub level: i32,
    pub max_durability: i32,
}

/// `mysekaiTools`, id order.
pub fn parse_tool_master(rows: &[Value]) -> Result<Vec<ToolMaster>, String> {
    rows.iter()
        .map(|row| {
            Ok(ToolMaster {
                id: crate::int32(row, "id")?,
                tool_type: row["mysekaiToolType"]
                    .as_str()
                    .ok_or_else(|| format!("mysekaiToolType of {row} is not a string"))?
                    .to_owned(),
                level: crate::int32(row, "toolLevel")?,
                max_durability: crate::int32(row, "maxDurability")?,
            })
        })
        .collect()
}
