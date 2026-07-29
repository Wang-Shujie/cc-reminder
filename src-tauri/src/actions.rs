use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ActionCapability {
    Approve,
    Reject,
    Reply,
}

pub fn new_v1_action_fields() -> (Uuid, Option<String>, Vec<ActionCapability>) {
    (Uuid::now_v7(), None, Vec::new())
}

#[cfg(test)]
mod tests {
    use super::new_v1_action_fields;

    #[test]
    fn v1_actions_are_correlated_but_not_actionable() {
        let (correlation_id, action_id, capabilities) = new_v1_action_fields();

        assert!(!correlation_id.is_nil());
        assert_eq!(correlation_id.get_version_num(), 7);
        assert_eq!(action_id, None);
        assert!(capabilities.is_empty());
    }
}
