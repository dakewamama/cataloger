use crate::constants::{EVENT_IX_TAG, EVENT_PREFIX_LEN};
use crate::events::{
    CatalystEvent, FixedTransfer, RecurringTransfer, SubscriptionCancelled,
    SubscriptionCreated, SubscriptionResumed, SubscriptionTransfer,
};

const ADDRESS_LEN: usize = 32;

fn read_address(buf: &[u8], offset: usize) -> String {
    bs58::encode(&buf[offset..offset + ADDRESS_LEN]).into_string()
}

fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap())
}

fn read_i64(buf: &[u8], offset: usize) -> i64 {
    i64::from_le_bytes(buf[offset..offset + 8].try_into().unwrap())
}

pub fn decode_event(raw: &[u8]) -> Option<CatalystEvent> {
    if raw.len() < EVENT_PREFIX_LEN {
        return None;
    }
    if raw[..8] != EVENT_IX_TAG {
        return None;
    }

    let discriminator = raw[8];
    let p = &raw[EVENT_PREFIX_LEN..];

    match discriminator {
        0 => Some(CatalystEvent::SubscriptionCreated(SubscriptionCreated {
            plan: read_address(p, 0),
            subscriber: read_address(p, 32),
            mint: read_address(p, 64),
            created_ts: read_i64(p, 96),
        })),
        1 => Some(CatalystEvent::SubscriptionCancelled(SubscriptionCancelled {
            plan: read_address(p, 0),
            subscriber: read_address(p, 32),
            expires_at_ts: read_i64(p, 64),
        })),
        2 => Some(CatalystEvent::SubscriptionTransfer(SubscriptionTransfer {
            subscription: read_address(p, 0),
            plan: read_address(p, 32),
            delegator: read_address(p, 64),
            mint: read_address(p, 96),
            amount: read_u64(p, 128),
            period_start_ts: read_i64(p, 136),
            period_end_ts: read_i64(p, 144),
            amount_pulled_in_period: read_u64(p, 152),
            receiver: read_address(p, 160),
        })),
        3 => Some(CatalystEvent::FixedTransfer(FixedTransfer {
            delegation: read_address(p, 0),
            delegator: read_address(p, 32),
            delegatee: read_address(p, 64),
            mint: read_address(p, 96),
            amount: read_u64(p, 128),
            remaining_amount: read_u64(p, 136),
            receiver: read_address(p, 144),
        })),
        4 => Some(CatalystEvent::RecurringTransfer(RecurringTransfer {
            delegation: read_address(p, 0),
            delegator: read_address(p, 32),
            delegatee: read_address(p, 64),
            mint: read_address(p, 96),
            amount: read_u64(p, 128),
            period_start_ts: read_i64(p, 136),
            period_end_ts: read_i64(p, 144),
            amount_pulled_in_period: read_u64(p, 152),
            receiver: read_address(p, 160),
        })),
        5 => Some(CatalystEvent::SubscriptionResumed(SubscriptionResumed {
            plan: read_address(p, 0),
            subscriber: read_address(p, 32),
            resumed_ts: read_i64(p, 64),
        })),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::constants::EVENT_IX_TAG;

    fn make_raw(discriminator: u8, payload: &[u8]) -> Vec<u8> {
        let mut raw = Vec::new();
        raw.extend_from_slice(&EVENT_IX_TAG);
        raw.push(discriminator);
        raw.extend_from_slice(payload);
        raw
    }

    fn address_bytes(val: u8) -> [u8; 32] {
        [val; 32]
    }

    #[test]
    fn decodes_subscription_created() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&address_bytes(1)); // plan
        payload.extend_from_slice(&address_bytes(2)); // subscriber
        payload.extend_from_slice(&address_bytes(3)); // mint
        payload.extend_from_slice(&42i64.to_le_bytes()); // created_ts

        let raw = make_raw(0, &payload);
        let event = decode_event(&raw).unwrap();

        match event {
            CatalystEvent::SubscriptionCreated(e) => {
                assert_eq!(e.created_ts, 42);
            }
            _ => panic!("wrong event type"),
        }
    }

    #[test]
    fn returns_none_for_wrong_tag() {
        let raw = vec![0u8; 20];
        assert!(decode_event(&raw).is_none());
    }

    #[test]
    fn returns_none_for_unknown_discriminator() {
        let mut raw = EVENT_IX_TAG.to_vec();
        raw.push(99);
        raw.extend_from_slice(&[0u8; 32]);
        assert!(decode_event(&raw).is_none());
    }

    #[test]
    fn returns_none_for_short_payload() {
        assert!(decode_event(&[0u8; 4]).is_none());
    }
}