use serde::Deserialize;

pub type WebhookPayload = Vec<RawTransaction>;

#[derive(Deserialize)]
pub struct RawTransaction {
    pub slot: u64,
    pub meta: Meta,
    pub transaction: TransactionInner,
}

#[derive(Deserialize)]
pub struct TransactionInner {
    pub message: Message,
    pub signatures: Vec<String>,
}

#[derive(Deserialize)]
pub struct Message {
    #[serde(rename = "accountKeys")]
    pub account_keys: Vec<String>,
}

#[derive(Deserialize)]
pub struct Meta {
    #[serde(default)]
    pub err: Option<serde_json::Value>,
    #[serde(rename = "innerInstructions", default)]
    pub inner_instructions: Vec<InnerInstruction>,
}

#[derive(Deserialize)]
pub struct InnerInstruction {
    pub instructions: Vec<Instruction>,
}

#[derive(Deserialize)]
pub struct Instruction {
    #[serde(rename = "programIdIndex")]
    pub program_id_index: u32,
    pub data: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deserializes_real_helius_raw_payload_shape() {
        let json = r#"[{
            "slot": 123456,
            "meta": {
                "innerInstructions": [{
                    "index": 0,
                    "instructions": [{
                        "programIdIndex": 12,
                        "accounts": [0, 2],
                        "data": "somedata"
                    }]
                }]
            },
            "transaction": {
                "message": {
                    "accountKeys": ["acct0", "acct1", "acct2"]
                },
                "signatures": ["sig123"]
            }
        }]"#;

        let payload: WebhookPayload = serde_json::from_str(json).unwrap();
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0].slot, 123456);
        assert_eq!(payload[0].transaction.signatures[0], "sig123");
        assert_eq!(
            payload[0].meta.inner_instructions[0].instructions[0].program_id_index,
            12
        );
    }
}
