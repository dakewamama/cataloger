use crate::constants::{EVENT_IX_TAG, EVENT_PREFIX_LEN, SUBSCRIPTIONS_PROGRAM_ID};
use crate::types::WebhookPayload;

pub fn extract_events(payload: &WebhookPayload) -> Vec<Vec<u8>> {
    let mut events = Vec::new();

    for transaction in &payload.transactions {
        for inner in &transaction.meta.inner_instructions {
            for instruction in &inner.instructions {
                if instruction.program_id != SUBSCRIPTIONS_PROGRAM_ID {
                    continue;
                }

                let bytes = match bs58::decode(&instruction.data).into_vec() {
                    Ok(b) => b,
                    Err(_) => continue,
                };

                if bytes.len() < EVENT_PREFIX_LEN {
                    continue;
                }

                if bytes[..8] != EVENT_IX_TAG {
                    continue;
                }

                events.push(bytes);
            }
        }
    }

    events
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{InnerInstruction, Instruction, Meta, Transaction, WebhookPayload};

    fn make_payload(program_id: &str, data: &str) -> WebhookPayload {
        WebhookPayload {
            event_type: "TEST".to_string(),
            transactions: vec![Transaction {
                signature: "sig".to_string(),
                meta: Meta {
                    inner_instructions: vec![InnerInstruction {
                        instructions: vec![Instruction {
                            program_id: program_id.to_string(),
                            data: data.to_string(),
                            accounts: vec![],
                        }],
                    }],
                },
            }],
        }
    }

    #[test]
    fn drop_wrong_program_id() {
        let payload = make_payload("wrongprogramid", "somedata");
        assert_eq!(extract_events(&payload).len(), 0);
    }

    #[test]
    fn drops_invalid_base58() {
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, "0OIL");
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
    fn returns_matching_event() {
        let mut bytes = vec![0u8; 9];
        bytes[..8].copy_from_slice(&EVENT_IX_TAG);
        bytes[8] = 0;
        let encoded = bs58::encode(&bytes).into_string();
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &encoded);
        assert_eq!(extract_events(&payload).len(), 1);
    }

    #[test]
    fn returns_correct_bytes() {
        let mut bytes = vec![0u8; 9];
        bytes[..8].copy_from_slice(&EVENT_IX_TAG);
        bytes[8] = 3;
        let encoded = bs58::encode(&bytes).into_string();
        let payload = make_payload(SUBSCRIPTIONS_PROGRAM_ID, &encoded);
        let events = extract_events(&payload);
        assert_eq!(events[0][8], 3);
    }
}