use serde::Deserialize;

#[derive(Deserialize)]
pub struct WebhookPayload {
    #[serde(rename = "type")]
    pub event_type: String,
    pub transactions: Vec<Transaction>,
}

#[derive(Deserialize)]
pub struct Transaction {
    pub signature: String,
    pub meta: Meta,
}

#[derive(Deserialize)]
pub struct Meta {
    #[serde(rename = "innerInstructions")]
    pub inner_instructions: Vec<InnerInstruction>,
}

#[derive(Deserialize)]
pub struct InnerInstruction {
    pub instructions: Vec<Instruction>,
}

#[derive(Deserialize)]
pub struct Instruction {
    #[serde(rename = "programId")]
    pub program_id: String,
    pub data: String,
    pub accounts: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::SUBSCRIPTIONS_PROGRAM_ID;

    #[test]
    fn deserializes_webhook_payload() {
        let json = format!(r#"{{
            "type": "TRANSFER",
            "transactions": [{{
                "signature": "sig123",
                "meta": {{
                    "innerInstructions": [{{
                        "instructions": [{{
                            "programId": "{}",
                            "data": "somedata",
                            "accounts": []
                        }}]
                    }}]
                }}
            }}]
        }}"#, SUBSCRIPTIONS_PROGRAM_ID);

        let payload: WebhookPayload = serde_json::from_str(&json).unwrap();
        assert_eq!(payload.event_type, "TRANSFER");
        assert_eq!(payload.transactions.len(), 1);
        assert_eq!(payload.transactions[0].signature, "sig123");
        assert_eq!(
            payload.transactions[0].meta.inner_instructions[0].instructions[0].program_id,
            SUBSCRIPTIONS_PROGRAM_ID
        );
    }
}