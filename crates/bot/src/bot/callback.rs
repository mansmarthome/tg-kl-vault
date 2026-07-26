use std::{error::Error, fmt};

/// telebot v3 prefixes callback data with a literal form-feed, a unique button
/// name, then `|`, then a hex-encoded protobuf payload. Keep this wire format
/// byte-compatible so old inline buttons still work after cutover.
pub const TELEBOT_CALLBACK_PREFIX: char = '\u{000c}';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attachment {
    pub user_id: i64,
    pub source_id: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Button {
    SetFeedItem,
    SetToggleNotice,
    SetToggleTelegraph,
    SetToggleUpdate,
    SetSetSubTag,
    UnsubFeedItem,
    UnsubAllConfirm,
    UnsubAllCancel,
}

impl Button {
    pub fn unique(self) -> &'static str {
        match self {
            Self::SetFeedItem => "set_feed_item_btn",
            Self::SetToggleNotice => "set_toggle_notice_btn",
            Self::SetToggleTelegraph => "set_toggle_telegraph_btn",
            Self::SetToggleUpdate => "set_toggle_update_btn",
            Self::SetSetSubTag => "set_set_sub_tag_btn",
            Self::UnsubFeedItem => "unsub_feed_item_btn",
            Self::UnsubAllConfirm => "unsub_all_confirm_btn",
            Self::UnsubAllCancel => "unsub_all_cancel_btn",
        }
    }

    pub fn from_unique(unique: &str) -> Option<Self> {
        Some(match unique {
            "set_feed_item_btn" => Self::SetFeedItem,
            "set_toggle_notice_btn" => Self::SetToggleNotice,
            "set_toggle_telegraph_btn" => Self::SetToggleTelegraph,
            "set_toggle_update_btn" => Self::SetToggleUpdate,
            "set_set_sub_tag_btn" => Self::SetSetSubTag,
            "unsub_feed_item_btn" => Self::UnsubFeedItem,
            "unsub_all_confirm_btn" => Self::UnsubAllConfirm,
            "unsub_all_cancel_btn" => Self::UnsubAllCancel,
            _ => return None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Callback {
    pub button: Button,
    pub attachment: Attachment,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CallbackError {
    MissingPrefix,
    MissingSeparator,
    UnknownButton(String),
    InvalidHex,
    InvalidProtobuf,
    VarintOverflow,
}

impl fmt::Display for CallbackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingPrefix => write!(f, "callback data missing telebot prefix"),
            Self::MissingSeparator => write!(f, "callback data missing separator"),
            Self::UnknownButton(unique) => write!(f, "unknown callback button: {unique}"),
            Self::InvalidHex => write!(f, "invalid callback hex payload"),
            Self::InvalidProtobuf => write!(f, "invalid callback protobuf payload"),
            Self::VarintOverflow => write!(f, "callback protobuf varint overflow"),
        }
    }
}

impl Error for CallbackError {}

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

pub fn decode_attachment_hex(hex: &str) -> Result<Attachment, CallbackError> {
    let bytes = decode_hex(hex)?;
    decode_attachment_proto(&bytes)
}

pub fn encode_telebot_callback(button: Button, attachment: Attachment) -> String {
    format!(
        "{TELEBOT_CALLBACK_PREFIX}{}|{}",
        button.unique(),
        encode_attachment_hex(attachment)
    )
}

pub fn decode_telebot_callback(data: &str) -> Result<Callback, CallbackError> {
    let rest = data.strip_prefix(TELEBOT_CALLBACK_PREFIX).ok_or(CallbackError::MissingPrefix)?;
    let (unique, hex) = rest.split_once('|').ok_or(CallbackError::MissingSeparator)?;
    let button = Button::from_unique(unique).ok_or_else(|| CallbackError::UnknownButton(unique.to_owned()))?;
    let attachment = decode_attachment_hex(hex)?;
    Ok(Callback { button, attachment })
}

fn encode_varint(mut value: u64, out: &mut Vec<u8>) {
    while value >= 0x80 {
        out.push((value as u8 & 0x7f) | 0x80);
        value >>= 7;
    }
    out.push(value as u8);
}

fn decode_attachment_proto(bytes: &[u8]) -> Result<Attachment, CallbackError> {
    let mut idx = 0usize;
    let mut attachment = Attachment { user_id: 0, source_id: 0 };

    while idx < bytes.len() {
        let tag = bytes[idx];
        idx += 1;
        match tag {
            0x08 => {
                let raw = decode_varint(bytes, &mut idx)?;
                attachment.user_id = raw as i64;
            }
            0x10 => {
                let raw = decode_varint(bytes, &mut idx)?;
                attachment.source_id = u32::try_from(raw).map_err(|_| CallbackError::InvalidProtobuf)?;
            }
            _ => return Err(CallbackError::InvalidProtobuf),
        }
    }

    Ok(attachment)
}

fn decode_varint(bytes: &[u8], idx: &mut usize) -> Result<u64, CallbackError> {
    let mut value = 0u64;
    for shift in (0..=63).step_by(7) {
        let Some(&byte) = bytes.get(*idx) else {
            return Err(CallbackError::InvalidProtobuf);
        };
        *idx += 1;
        value |= u64::from(byte & 0x7f) << shift;
        if byte & 0x80 == 0 {
            return Ok(value);
        }
    }
    Err(CallbackError::VarintOverflow)
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

fn decode_hex(hex: &str) -> Result<Vec<u8>, CallbackError> {
    let bytes = hex.as_bytes();
    if !bytes.len().is_multiple_of(2) {
        return Err(CallbackError::InvalidHex);
    }
    let mut out = Vec::with_capacity(bytes.len() / 2);
    for pair in bytes.chunks_exact(2) {
        let hi = hex_nibble(pair[0])?;
        let lo = hex_nibble(pair[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_nibble(b: u8) -> Result<u8, CallbackError> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(CallbackError::InvalidHex),
    }
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
            assert_eq!(decode_attachment_hex(expected).unwrap(), attachment, "{expected}");
        }
    }

    #[test]
    fn telebot_callback_prefix_is_form_feed_unique_pipe_payload() {
        let encoded = encode_telebot_callback(
            Button::SetFeedItem,
            Attachment { user_id: 123_456_789, source_id: 42 },
        );
        assert_eq!(encoded, "\u{000c}set_feed_item_btn|08959aef3a102a");
        assert_eq!(
            decode_telebot_callback(&encoded).unwrap(),
            Callback {
                button: Button::SetFeedItem,
                attachment: Attachment { user_id: 123_456_789, source_id: 42 },
            }
        );
    }

    #[test]
    fn all_button_unique_values_match_overview() {
        let buttons = [
            (Button::SetFeedItem, "set_feed_item_btn"),
            (Button::SetToggleNotice, "set_toggle_notice_btn"),
            (Button::SetToggleTelegraph, "set_toggle_telegraph_btn"),
            (Button::SetToggleUpdate, "set_toggle_update_btn"),
            (Button::SetSetSubTag, "set_set_sub_tag_btn"),
            (Button::UnsubFeedItem, "unsub_feed_item_btn"),
            (Button::UnsubAllConfirm, "unsub_all_confirm_btn"),
            (Button::UnsubAllCancel, "unsub_all_cancel_btn"),
        ];
        for (button, unique) in buttons {
            assert_eq!(button.unique(), unique);
            assert_eq!(Button::from_unique(unique), Some(button));
        }
    }
}
