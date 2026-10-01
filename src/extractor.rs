use crate::constants::{EVENT_IX_TAG, EVENT_PREFIX_LEN, SUBSCRIPTIONS_PROGRAM_ID};
use crate::types::WebhookPayload;

pub struct ExtractedEvent {
    pub signature: String,
    pub slot: u64,
    pub bytes: Vec<u8>,
}

pub fn extract_events(payload: &WebhookPayload) -> Vec<ExtractedEvent> {
    let mut events = Vec::new();

    for tx in payload {
        if tx.meta.err.is_some() {
            continue;
        }
        let signature = tx
            .transaction
            .signatures
            .first()
            .cloned()
            .unwrap_or_default();

        let account_keys = &tx.transaction.message.account_keys;

        for inner in &tx.meta.inner_instructions {
            for instruction in &inner.instructions {
                let program_id = match account_keys.get(instruction.program_id_index as usize) {
                    Some(id) => id,
                    None => continue,
                };

                if program_id != SUBSCRIPTIONS_PROGRAM_ID {
                    continue;
                }

                let bytes: Vec<u8> = match bs58::decode(&instruction.data).into_vec() {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                if bytes.len() < EVENT_PREFIX_LEN {
                    continue;
                }

                if bytes[..8] != EVENT_IX_TAG {
                    continue;
                }

                events.push(ExtractedEvent {
                    signature: signature.clone(),
                    slot: tx.slot,
                    bytes,
                });
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{
        InnerInstruction, Instruction, Message, Meta, RawTransaction, TransactionInner,
    };

    fn make_payload(program_id: &str, data: &str) -> WebhookPayload {
        vec![RawTransaction {
            slot: 1,
            meta: Meta {
                err: None,
                inner_instructions: vec![InnerInstruction {
                    instructions: vec![Instruction {
                        program_id_index: 0,
                        data: data.to_string(),
                    }],
                }],
            },
            transaction: TransactionInner {
                message: Message {
                    account_keys: vec![program_id.to_string()],
                },
                signatures: vec!["sig".to_string()],
            },
        }]
    }

    #[test]
    fn drops_wrong_program_id() {
        let payload = make_payload("wrongprogramid", "somedata");
        assert_eq!(extract_events(&payload).len(), 0);
    }

    #[test]
    fn drops_invalid_base58() {
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, "0OIl");
        assert_eq!(extract_events(&payload).len(), 0);
    }

    #[test]
    fn drops_payload_too_short() {
        let short = bs58::encode(vec![0u8; 8]).into_string();
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &short);
        assert_eq!(extract_events(&payload).len(), 0);
    }

    #[test]
    fn drops_wrong_tag() {
        let bytes = vec![0u8; 9];
        let encoded = bs58::encode(&bytes).into_string();
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &encoded);
        assert_eq!(extract_events(&payload).len(), 0);
    }

    #[test]
    fn returns_matching_event_with_signature() {
        let mut bytes = vec![0u8; 9];
        bytes[..8].copy_from_slice(&EVENT_IX_TAG);
        bytes[8] = 0;
        let encoded = bs58::encode(&bytes).into_string();
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &encoded);
        let events = extract_events(&payload);
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].signature, "sig");
        assert_eq!(events[0].slot, 1);
    }

    #[test]
    fn drops_events_from_failed_transactions() {
        let mut bytes = vec![0u8; 9];
        bytes[..8].copy_from_slice(&EVENT_IX_TAG);
        bytes[8] = 0;
        let encoded = bs58::encode(&bytes).into_string();

        let mut payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &encoded);
        payload[0].meta.err = Some(serde_json::json!({"InstructionError": [0, "Custom"]}));

        assert_eq!(extract_events(&payload).len(), 0);
    }
}
