/// telebot v3 prefixes callback data with a literal form-feed, a unique button
/// name, then `|`, then a hex-encoded protobuf payload. Keep this wire format
/// byte-compatible so old inline buttons still work after cutover.
pub const TELEBOT_CALLBACK_PREFIX: char = '\u{000c}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attachment {
    pub user_id: i64,
    pub source_id: u32,
}

/// Encode `Attachment { int64 user_id = 1; uint32 source_id = 2; }` as proto3
/// and lowercase hex. Negative `int64` values are encoded as their two's-
/// complement `u64` varint, matching proto3 `int64` (not `sint64`).
pub fn encode_attachment_hex(attachment: Attachment) -> String {
    let mut bytes = Vec::with_capacity(14);
    if attachment.user_id != 0 {
        bytes.push(0x08);
        encode_varint(attachment.user_id as u64, &mut bytes);
    }
    if attachment.source_id != 0 {
        bytes.push(0x10);
        encode_varint(u64::from(attachment.source_id), &mut bytes);
    }
    encode_hex(&bytes)
}

pub fn encode_telebot_callback(unique: &str, attachment: Attachment) -> String {
    format!("{TELEBOT_CALLBACK_PREFIX}{unique}|{}", encode_attachment_hex(attachment))
}

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn attachment_hex_matches_go_protobuf_vectors() {
        let vectors = [
            (Attachment { user_id: 123_456_789, source_id: 42 }, "08959aef3a102a"),
            (
                Attachment { user_id: -1_001_234_567_890, source_id: 7 },
                "08aeda938eeee2ffffff011007",
            ),
            (Attachment { user_id: 0, source_id: 0 }, ""),
        ];

        for (attachment, expected) in vectors {
            assert_eq!(encode_attachment_hex(attachment), expected, "{attachment:?}");
        }
    }

    #[test]
    fn telebot_callback_prefix_is_form_feed_unique_pipe_payload() {
        assert_eq!(
            encode_telebot_callback(
                "set_feed_item_btn",
                Attachment { user_id: 123_456_789, source_id: 42 },
            ),
            "\u{000c}set_feed_item_btn|08959aef3a102a",
        );
    }
}
