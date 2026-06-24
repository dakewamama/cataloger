pub const SUBSCRIPTIONS_PROGRAM_ID: &str =
    "De1egAFMkMWZSN5rYXRj9CAdheBamobVNubTsi9avR44";

pub const EVENT_IX_TAG: [u8; 8] = [0xe4, 0x45, 0xa5, 0x2e, 0x51, 0xcb, 0x9a, 0x1d];

pub const EVENT_PREFIX_LEN: usize = 9;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn event_ix_tag_is_8_bytes() {
        assert_eq!(EVENT_IX_TAG.len(), 8);
    }

    #[test]
    fn program_id_is_44_chars() {
        assert_eq!(SUBSCRIPTIONS_PROGRAM_ID.len(), 44);
    }

    #[test]
    fn event_prefix_len_is_tag_plus_discriminator() {
        assert_eq!(EVENT_PREFIX_LEN, EVENT_IX_TAG.len() + 1);
    }
}