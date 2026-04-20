use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ToolCallRow {
    pub id: String,
    pub session_id: String,
    pub agent: String,
    pub tool_name: String,
    pub tool_args: serde_json::Value,
    pub tool_result_summary: String,
    pub success: bool,
    pub user_correction_flag: bool,
    pub created_at: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_call_row_roundtrips_via_serde_json() {
        let row = ToolCallRow {
            id: "01KPK000".to_string(),
            session_id: "01KPG000".to_string(),
            agent: "claude-code".to_string(),
            tool_name: "Read".to_string(),
            tool_args: serde_json::json!({"file_path": "/tmp/a"}),
            tool_result_summary: "ok".to_string(),
            success: true,
            user_correction_flag: false,
            created_at: "2026-04-19 12:34:56".to_string(),
        };
        let s = serde_json::to_string(&row).unwrap();
        let back: ToolCallRow = serde_json::from_str(&s).unwrap();
        assert_eq!(row, back);
    }
}
