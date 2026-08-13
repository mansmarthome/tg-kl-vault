use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

use crate::bot::runtime::Lang;

use crate::bot::callback::{encode_telebot_callback, Attachment, Button};

/// One inline button per source, `[id] title`, used by both `/set` (feed
/// picker) and `/unsub` with no URL argument (matches Go's `Set.Handle` /
/// `RemoveSubscription.removeForChat`).
pub fn feed_item_list_keyboard(
    button: Button,
    user_id: i64,
    sources: &[(i64, String)],
) -> InlineKeyboardMarkup {
    let rows = sources
        .iter()
        .map(|(source_id, title)| {
            let attachment = Attachment {
                user_id,
                source_id: *source_id as u32,
            };
            vec![InlineKeyboardButton::callback(
                format!("[{source_id}] {title}"),
                encode_telebot_callback(button, attachment),
            )]
        })
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(rows)
}

/// The two-row toggle keyboard shown under `feedSettingTmpl` (Go's
/// `genFeedSetBtn`). All four buttons reuse the same attachment hex payload;
/// only the button `unique` differs.
pub fn feed_setting_keyboard(
    attachment: Attachment,
    source_error_count: i64,
    error_threshold: i64,
    enable_notification: Option<i64>,
    enable_telegraph: Option<i64>,
) -> InlineKeyboardMarkup {
    let toggle_update_text = if source_error_count >= error_threshold {
        "重啟更新"
    } else {
        "暫停更新"
    };
    let toggle_notice_text = if enable_notification == Some(1) {
        "關閉通知"
    } else {
        "開啟通知"
    };
    let toggle_telegraph_text = if enable_telegraph == Some(1) {
        "關閉 Telegraph 轉碼"
    } else {
        "開啟 Telegraph 轉碼"
    };

    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(
                toggle_update_text,
                encode_telebot_callback(Button::SetToggleUpdate, attachment),
            ),
            InlineKeyboardButton::callback(
                toggle_notice_text,
                encode_telebot_callback(Button::SetToggleNotice, attachment),
            ),
        ],
        vec![
            InlineKeyboardButton::callback(
                toggle_telegraph_text,
                encode_telebot_callback(Button::SetToggleTelegraph, attachment),
            ),
            InlineKeyboardButton::callback(
                "標籤設定",
                encode_telebot_callback(Button::SetSetSubTag, attachment),
            ),
        ],
    ])
}

/// Go's `RemoveAllSubscription.Handle` sends confirm/cancel buttons with no
/// attachment payload at all (`Data` is left unset), so the callback handler
/// authorizes off the callback sender directly rather than an embedded id.
pub fn unsuball_confirm_keyboard() -> InlineKeyboardMarkup {
    let empty = Attachment {
        user_id: 0,
        source_id: 0,
    };
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(
            "確認",
            encode_telebot_callback(Button::UnsubAllConfirm, empty),
        ),
        InlineKeyboardButton::callback(
            "取消",
            encode_telebot_callback(Button::UnsubAllCancel, empty),
        ),
    ]])
}

pub fn settings_keyboard(lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            lang.settings_opml_button(),
            "settings:opml",
        )],
        vec![InlineKeyboardButton::callback(
            lang.settings_interval_button(),
            "settings:interval",
        )],
        vec![InlineKeyboardButton::callback(
            lang.settings_language_button(),
            "settings:language",
        )],
        vec![InlineKeyboardButton::callback(
            lang.bm_settings_button(),
            "settings:bm",
        )],
    ])
}

pub fn settings_opml_keyboard(lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(lang.settings_import_button(), "settings:opml:import"),
            InlineKeyboardButton::callback(lang.settings_export_button(), "settings:opml:export"),
        ],
        vec![InlineKeyboardButton::callback(
            lang.settings_back_button(),
            "settings:back",
        )],
    ])
}

pub fn settings_language_keyboard(lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("English", "settings:language:en"),
            InlineKeyboardButton::callback("繁體中文", "settings:language:zh-tw"),
        ],
        vec![InlineKeyboardButton::callback(
            lang.settings_back_button(),
            "settings:back",
        )],
    ])
}

pub fn settings_interval_keyboard(lang: Lang) -> InlineKeyboardMarkup {
    let labels = [5, 10, 30, 60, 120]
        .into_iter()
        .map(|minutes| {
            InlineKeyboardButton::callback(
                format!("{minutes} min"),
                format!("settings:interval:{minutes}"),
            )
        })
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(vec![
        labels,
        vec![InlineKeyboardButton::callback(
            lang.settings_back_button(),
            "settings:back",
        )],
    ])
}
