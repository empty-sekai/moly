//! The client's copy of the owned harvest tools.

use bevy::prelude::*;
use serde_json::Value;

use moly_protocol::json::tools_value as rows_value;
use moly_protocol::rows::UserMysekaiTool;
/// `UserDataManager.UserMysekaiTools`, set by the server model only.
#[derive(Resource, Debug, Clone, Default)]
pub struct ClientMysekaiTools {
    rows: Vec<UserMysekaiTool>,
    /// Applications that carried the section (0: none yet).
    pub revision: u64,
}

impl ClientMysekaiTools {
    pub fn rows(&self) -> &[UserMysekaiTool] {
        &self.rows
    }

    /// `UpdateUserMysekaiTool`: the carried list replaces the copy.
    pub fn apply(&mut self, rows: Vec<UserMysekaiTool>) {
        self.rows = rows;
        self.revision += 1;
    }

    /// The quantity of a tool (0 without a row), as the craft screen shows it.
    pub fn quantity(&self, tool_id: i32) -> i32 {
        self.rows
            .iter()
            .find(|row| row.mysekai_tool_id == tool_id)
            .map_or(0, |row| row.quantity)
    }

    pub fn view(&self) -> Value {
        rows_value(&self.rows)
    }
}
