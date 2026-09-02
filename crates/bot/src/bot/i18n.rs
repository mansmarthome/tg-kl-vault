//! Localized UI strings.
//!
//! Deliberately "one string, one method": each `match` over `Lang` is
//! exhaustive, so adding a fourth language turns into a compile error at every
//! string — the property we want, and one a HashMap/JSON catalog would throw
//! away. The `strings!` macro just collapses the boilerplate 4 lines → 1.
//! Parameterized strings stay hand-written below the macro.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
    Ru,
}

impl Lang {
    pub fn from_value(value: Option<&str>) -> Self {
        match value {
            Some("zh-tw") | Some("zh") => Self::ZhTw,
            Some("ru") => Self::Ru,
            // missing / unknown / "en" → English (new default)
            _ => Self::En,
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhTw => "zh-tw",
            Self::Ru => "ru",
        }
    }
}

/// Generates one `pub fn name(self) -> &'static str` per entry.
macro_rules! strings {
    ($($name:ident => en: $en:expr, zh: $zh:expr, ru: $ru:expr);* $(;)?) => {
        impl Lang {
            $(
                pub fn $name(self) -> &'static str {
                    match self {
                        Self::En => $en,
                        Self::ZhTw => $zh,
                        Self::Ru => $ru,
                    }
                }
            )*
        }
    };
}

strings! {
    // ── Bot-level / commands ──────────────────────────────────────────────
    start_message => en: "Hello! Welcome to flowerss.",
        zh: "你好，歡迎使用 flowerss。",
        ru: "Привет! Добро пожаловать в flowerss.";
    version_message => en: "tg-kl-vault compatible with flowerss-bot, version dev, commit none, built at unknown",
        zh: "tg-kl-vault 兼容 flowerss-bot，版本 dev，提交 none，構建於 unknown",
        ru: "tg-kl-vault совместим с flowerss-bot, версия dev, коммит none, собрано unknown";

    // ── Help (full command list) ──────────────────────────────────────────
    help => en: "Commands:\n/sub Subscribe to an RSS feed\n/unsub Unsubscribe\n/list Show subscriptions\n/set Feed settings\n/settings Bot settings\n/check Check current subscriptions\n/activeall Enable all subscriptions\n/pauseall Pause all subscriptions\n/unsuball Remove all subscriptions\n/help Help\n/version Bot version",
        zh: "命令：\n/sub 訂閱 RSS 源\n/unsub 取消訂閱\n/list 查看目前訂閱源\n/set 設定訂閱\n/settings Bot 設定\n/check 檢查目前訂閱\n/activeall 開啟所有訂閱\n/pauseall 暫停所有訂閱\n/unsuball 取消所有訂閱\n/help 幫助\n/version Bot 版本資訊",
        ru: "Команды:\n/sub Подписаться на RSS\n/unsub Отписаться\n/list Показать подписки\n/set Настройки подписки\n/settings Настройки бота\n/check Проверить подписки\n/activeall Включить все подписки\n/pauseall Приостановить все подписки\n/unsuball Удалить все подписки\n/help Помощь\n/version Версия бота";

    // ── Settings menu ─────────────────────────────────────────────────────
    settings_title => en: "Settings", zh: "設定", ru: "Настройки";
    settings_opml_button => en: "OPML import/export", zh: "OPML 匯入/匯出", ru: "Импорт/экспорт OPML";
    settings_import_button => en: "Import", zh: "匯入", ru: "Импорт";
    settings_export_button => en: "Export", zh: "匯出", ru: "Экспорт";
    settings_interval_button => en: "Refresh interval", zh: "更新頻率", ru: "Интервал обновления";
    settings_language_button => en: "Language", zh: "語系", ru: "Язык";
    settings_back_button => en: "Back", zh: "返回", ru: "Назад";
    settings_language_english_label => en: "English", zh: "English", ru: "English";
    settings_language_zh_label => en: "繁體中文", zh: "繁體中文", ru: "繁體中文";
    settings_language_ru_label => en: "Русский", zh: "Русский", ru: "Русский";

    import_hint => en: "Send an OPML file to import subscriptions.",
        zh: "請直接傳送 OPML 檔案以匯入訂閱。",
        ru: "Отправьте OPML-файл для импорта подписок.";
    interval_hint => en: "Choose a refresh interval for all subscriptions in this chat.",
        zh: "請選擇此聊天室所有訂閱的更新頻率。",
        ru: "Выберите интервал обновления для всех подписок в этом чате.";
    lang_updated_en => en: "Language updated: English",
        zh: "語言已更新：English",
        ru: "Язык обновлён: English";
    lang_updated_zh => en: "Language updated: 繁體中文",
        zh: "語言已更新：繁體中文",
        ru: "Язык обновлён: 繁體中文";
    lang_updated_ru => en: "Language updated: Русский",
        zh: "語言已更新：Русский",
        ru: "Язык обновлён: Русский";

    // ── Subscribe / unsubscribe ───────────────────────────────────────────
    sub_missing_url => en: "Please provide an RSS URL after the command, e.g. /sub https://example.com/feed/",
        zh: "請在命令後帶上需要訂閱的 RSS URL，例如：/sub https://justinpot.com/feed/",
        ru: "Укажите RSS-ссылку после команды, например: /sub https://example.com/feed/";
    sub_failed => en: "Subscribe failed",
        zh: "訂閱失敗",
        ru: "Не удалось подписаться";
    sub_succeeded => en: "[[{id}]] [{title}]({link}) Subscribed",
        zh: "[[{}]][{}]({}) 訂閱成功",
        ru: "[[{id}]] [{title}]({link}) Подписка оформлена";
    sub_already => en: "Already subscribed to this feed.",
        zh: "已訂閱該源，請勿重複訂閱",
        ru: "Вы уже подписаны на этот источник.";

    unsub_no_subs => en: "No subscriptions",
        zh: "沒有訂閱",
        ru: "Нет подписок";
    unsub_choose => en: "Choose a feed to unsubscribe from:",
        zh: "請選擇你要退訂的源",
        ru: "Выберите источник для отписки:";
    unsub_not_subscribed => en: "Not subscribed to this RSS feed.",
        zh: "未訂閱該 RSS 源",
        ru: "Вы не подписаны на этот RSS-источник.";
    unsub_succeeded => en: "[{title}]({link}) Unsubscribed",
        zh: "[{}]({}) 退訂成功！",
        ru: "[{title}]({link}) Отписка выполнена";
    unsub_failed => en: "Unsubscribe failed",
        zh: "退訂失敗",
        ru: "Не удалось отписаться";
    unsub_error => en: "Unsubscribe error!",
        zh: "退訂錯誤！",
        ru: "Ошибка отписки!";
    unsub_succeeded_plain => en: "Unsubscribed",
        zh: "退訂成功",
        ru: "Отписка выполнена";
    unsub_failed_plain => en: "Unsubscribe failed",
        zh: "退訂失敗",
        ru: "Не удалось отписаться";
    unsub_cancelled => en: "Cancelled",
        zh: "操作取消",
        ru: "Операция отменена";
    unsuball_confirm_prompt => en: "Unsubscribe from all feeds?",
        zh: "是否退訂當前用戶的所有訂閱？",
        ru: "Отписаться от всех источников?";
    unsuball_confirm_label => en: "Confirm", zh: "確認", ru: "Подтвердить";
    unsuball_cancel_label => en: "Cancel", zh: "取消", ru: "Отмена";

    // ── /set (per-feed settings) ──────────────────────────────────────────
    set_no_subs => en: "No subscriptions yet.",
        zh: "當前沒有訂閱",
        ru: "Пока нет подписок.";
    set_choose => en: "Choose a feed to configure:",
        zh: "請選擇你要設置的源",
        ru: "Выберите источник для настройки:";
    set_unauthorized => en: "Failed to load subscription info.",
        zh: "獲取訂閱信息失敗",
        ru: "Не удалось получить информацию о подписке.";
    set_source_not_found => en: "Subscription not found.",
        zh: "找不到該訂閱源",
        ru: "Подписка не найдена.";
    set_user_not_subscribed => en: "You are not subscribed to this RSS feed.",
        zh: "用戶未訂閱該 rss",
        ru: "Вы не подписаны на этот RSS-источник.";

    set_toggle_update => en: "Resume updates",
        zh: "重啟更新",
        ru: "Возобновить обновления";
    set_toggle_pause => en: "Pause updates",
        zh: "暫停更新",
        ru: "Приостановить обновления";
    set_toggle_notice_off => en: "Disable notifications",
        zh: "關閉通知",
        ru: "Отключить уведомления";
    set_toggle_notice_on => en: "Enable notifications",
        zh: "開啟通知",
        ru: "Включить уведомления";
    set_toggle_telegraph_off => en: "Disable Telegraph transcoding",
        zh: "關閉 Telegraph 轉碼",
        ru: "Отключить Telegraph-транскодирование";
    set_toggle_telegraph_on => en: "Enable Telegraph transcoding",
        zh: "開啟 Telegraph 轉碼",
        ru: "Включить Telegraph-транскодирование";
    set_tag_button => en: "Tags",
        zh: "標籤設置",
        ru: "Теги";
    set_modified_toast => en: "Updated",
        zh: "修改成功",
        ru: "Обновлено";
    set_tag_prompt => en: "Use `/setfeedtag {source_id} tags` to set tags for this subscription. Tags are separated by spaces (up to 3).\nFor example: `/setfeedtag {source_id} tech apple`",
        zh: "請使用 `/setfeedtag {source_id} tags` 命令為該訂閱設置標籤，tags 為需要設置的標籤，以空格分隔。（最多設置三個標籤）\n例如：`/setfeedtag {source_id} 科技 蘋果`",
        ru: "Используйте `/setfeedtag {source_id} tags`, чтобы задать теги подписки. Теги разделяются пробелами (до 3 шт.).\nНапример: `/setfeedtag {source_id} tech apple`";
    set_tag_no_permission => en: "You don't have permission to do that.",
        zh: "無權限",
        ru: "У вас нет прав на это действие.";

    setfeedtag_usage => en: "/setfeedtag [sourceID] [tag1] [tag2] — set subscription tags (up to 3, space-separated)",
        zh: "/setfeedtag [sourceID] [tag1] [tag2] 設置訂閱標籤（最多設置三個 Tag，以空格分割）",
        ru: "/setfeedtag [sourceID] [тег1] [тег2] — задать теги подписки (до 3, через пробел)";
    setfeedtag_succeeded => en: "Tags updated.",
        zh: "訂閱標籤設置成功！",
        ru: "Теги подписки обновлены.";
    setfeedtag_failed => en: "Failed to set tags.",
        zh: "訂閱標籤設置失敗！",
        ru: "Не удалось задать теги подписки.";

    // ── /check (manual check) ─────────────────────────────────────────────
    check_started => en: "Checking {n} subscription(s)…",
        zh: "已開始檢查當前訂閱，共 {n} 個源",
        ru: "Проверяю {n} подписку(ок)…";
    check_done => en: "Check done: {new} new, {unchanged} unchanged, {errors} failed",
        zh: "檢查完成：新增 {new} 篇，{unchanged} 個源無更新，{errors} 個源失敗",
        ru: "Проверка завершена: новых {new}, без изменений {unchanged}, ошибок {errors}";

    // ── /activeall, /pauseall ─────────────────────────────────────────────
    activeall_succeeded => en: "All subscriptions enabled",
        zh: "訂閱已全部開啟",
        ru: "Все подписки включены";
    activeall_failed => en: "Failed to enable subscriptions",
        zh: "激活失敗",
        ru: "Не удалось включить подписки";
    pauseall_succeeded => en: "All subscriptions paused",
        zh: "訂閱已全部暫停",
        ru: "Все подписки приостановлены";
    pauseall_failed => en: "Failed to pause subscriptions",
        zh: "暫停失敗",
        ru: "Не удалось приостановить подписки";
    system_error => en: "System error",
        zh: "系統錯誤",
        ru: "Системная ошибка";
    system_error_exclaim => en: "System error!",
        zh: "系統錯誤！",
        ru: "Системная ошибка!";

    // ── /list (subscriptions) ─────────────────────────────────────────────
    list_empty => en: "No subscriptions yet.",
        zh: "訂閱列表為空",
        ru: "Список подписок пуст.";
    list_header_row => en: "[[{id}]] [{title}]({link})\n",
        zh: "[[{}]] [{}]({})\n",
        ru: "[[{id}]] [{title}]({link})\n";

    // ── OPML import / export ─────────────────────────────────────────────
    opml_wrong_file => en: "Please send a valid OPML file.",
        zh: "請發送正確的 OPML 文件",
        ru: "Пожалуйста, отправьте корректный OPML-файл.";
    opml_download_failed => en: "Failed to download the file.",
        zh: "獲取文件失敗",
        ru: "Не удалось скачать файл.";
    opml_export_empty => en: "No subscriptions to export.",
        zh: "訂閱列表為空",
        ru: "Нет подписок для экспорта.";
    opml_export_failed => en: "Export failed",
        zh: "導出失敗",
        ru: "Не удалось экспортировать";

    opml_import_header => en: "<b>Imported: {ok}, failed: {fail}</b>\n",
        zh: "<b>導入成功：{ok}，導入失敗：{fail}</b>\n",
        ru: "<b>Импортировано: {ok}, ошибок: {fail}</b>\n";
    opml_import_success_header => en: "<b>Successfully imported:</b>\n",
        zh: "<b>以下訂閱源導入成功:</b>\n",
        ru: "<b>Успешно импортированы:</b>\n";
    opml_import_failed_header => en: "<b>Failed to import:</b>\n",
        zh: "<b>以下訂閱源導入失敗:</b>\n",
        ru: "<b>Не удалось импортировать:</b>\n";

    // ── Generic toast for parse/IO failure paths ─────────────────────────
    toast_error => en: "error", zh: "error", ru: "error";
}

impl Lang {
    pub fn interval_updated(self, count: u64) -> String {
        match self {
            Self::En => format!("Updated {count} subscriptions"),
            Self::ZhTw => format!("已更新 {count} 個訂閱"),
            Self::Ru => format!("Обновлено подписок: {count}"),
        }
    }

    /// Header for the per-feed setting panel, e.g.
    /// "\n订阅<b>设置</b>\n[id] 7\n...". Uses `<b>` (safe, static) so the
    /// renderer can emit it verbatim.
    pub fn feed_setting(self, data: &crate::bot::render::FeedSettingData<'_>) -> String {
        let status = if data.source_error_count >= data.error_threshold {
            self.feed_status_paused()
        } else {
            self.feed_status_active()
        };
        let notice = match data.enable_notification {
            Some(0) => self.set_notice_off(),
            Some(1) => self.set_notice_on(),
            _ => "",
        };
        let telegraph = match data.enable_telegraph {
            Some(0) => self.set_telegraph_off(),
            Some(1) => self.set_telegraph_on(),
            _ => "",
        };
        let tag = if data.tag.is_empty() { self.feed_tag_none() } else { data.tag };
        match self {
            Self::En => format!(
                "\nFeed <b>settings</b>\n[id] {id}\n[Title] {title}\n[Link] {link}\n[Status] {status}\n[Interval] {interval} min\n[Notify] {notice}\n[Telegraph] {telegraph}\n[Tags] {tag}\n",
                id = data.source_id,
                title = data.source_title,
                link = data.source_link,
                status = status,
                interval = data.interval,
                notice = notice,
                telegraph = telegraph,
                tag = tag,
            ),
            Self::ZhTw => format!(
                "\n訂閱<b>設置</b>\n[id] {}\n[標題] {}\n[Link] {}\n[抓取更新] {}\n[抓取頻率] {}分鐘\n[通知] {}\n[Telegraph] {}\n[Tag] {}\n",
                data.source_id, data.source_title, data.source_link, status, data.interval, notice, telegraph, tag
            ),
            Self::Ru => format!(
                "\nПараметры <b>подписки</b>\n[id] {id}\n[Заголовок] {title}\n[Ссылка] {link}\n[Статус] {status}\n[Интервал] {interval} мин\n[Уведомления] {notice}\n[Telegraph] {telegraph}\n[Теги] {tag}\n",
                id = data.source_id,
                title = data.source_title,
                link = data.source_link,
                status = status,
                interval = data.interval,
                notice = notice,
                telegraph = telegraph,
                tag = tag,
            ),
        }
    }

    pub fn feed_status_paused(self) -> &'static str {
        match self {
            Self::En => "paused",
            Self::ZhTw => "暫停",
            Self::Ru => "приостановлено",
        }
    }
    pub fn feed_status_active(self) -> &'static str {
        match self {
            Self::En => "fetching",
            Self::ZhTw => "抓取中",
            Self::Ru => "загрузка",
        }
    }
    pub fn set_notice_on(self) -> &'static str {
        match self {
            Self::En => "on",
            Self::ZhTw => "開啟",
            Self::Ru => "вкл.",
        }
    }
    pub fn set_notice_off(self) -> &'static str {
        match self {
            Self::En => "off",
            Self::ZhTw => "關閉",
            Self::Ru => "выкл.",
        }
    }
    pub fn set_telegraph_on(self) -> &'static str {
        match self {
            Self::En => "on",
            Self::ZhTw => "開啟",
            Self::Ru => "вкл.",
        }
    }
    pub fn set_telegraph_off(self) -> &'static str {
        match self {
            Self::En => "off",
            Self::ZhTw => "關閉",
            Self::Ru => "выкл.",
        }
    }
    pub fn feed_tag_none(self) -> &'static str {
        match self {
            Self::En => "none",
            Self::ZhTw => "無",
            Self::Ru => "нет",
        }
    }

    /// Pick the right `lang_updated_*` variant for the new chat language.
    pub fn lang_updated(self, lang: Lang) -> &'static str {
        match lang {
            Lang::En => self.lang_updated_en(),
            Lang::ZhTw => self.lang_updated_zh(),
            Lang::Ru => self.lang_updated_ru(),
        }
    }

    /// Localized label for the `[N] title` line on a feed list keyboard.
    pub fn feed_item_button(self, source_id: i64, title: &str) -> String {
        match self {
            Self::En => format!("[{source_id}] {title}"),
            Self::ZhTw => format!("[{source_id}] {title}"),
            Self::Ru => format!("[{source_id}] {title}"),
        }
    }

    /// Localized sub/unsub success line for Markdown messages.
    pub fn sub_succeeded_md(self, id: i64, title: &str, link: &str) -> String {
        self.sub_succeeded()
            .replace("{id}", &id.to_string())
            .replace("{title}", title)
            .replace("{link}", link)
    }

    /// Localized unsub success line for Markdown messages.
    pub fn unsub_succeeded_md(self, title: &str, link: &str) -> String {
        self.unsub_succeeded()
            .replace("{title}", title)
            .replace("{link}", link)
    }

    /// Localized unsub success for HTML edit_message_text (used by callback).
    pub fn unsub_succeeded_html(self, source_id: i64, link: &str, title: &str) -> String {
        match self {
            Self::En => format!(
                "[{source_id}] <a href=\"{link}\">{title}</a> Unsubscribed",
            ),
            Self::ZhTw => format!(
                "[{source_id}] <a href=\"{link}\">{title}</a> 退訂成功",
            ),
            Self::Ru => format!(
                "[{source_id}] <a href=\"{link}\">{title}</a> Отписка выполнена",
            ),
        }
    }

    pub fn check_started_msg(self, n: usize) -> String {
        self.check_started().replace("{n}", &n.to_string())
    }

    pub fn check_done_msg(self, new: usize, unchanged: usize, errors: usize) -> String {
        self.check_done()
            .replace("{new}", &new.to_string())
            .replace("{unchanged}", &unchanged.to_string())
            .replace("{errors}", &errors.to_string())
    }

    pub fn opml_import_header_msg(self, ok: usize, fail: usize) -> String {
        self.opml_import_header()
            .replace("{ok}", &ok.to_string())
            .replace("{fail}", &fail.to_string())
    }

    pub fn list_header(self, n: usize) -> String {
        match self {
            Self::En => format!("Subscribed to {n} feed(s):\n"),
            Self::ZhTw => format!("共訂閱 {n} 個源，訂閱列表\n"),
            Self::Ru => format!("Подписки ({n} шт.):\n"),
        }
    }

    /// Command descriptions for `set_my_commands` registration.
    pub fn command_descriptions(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Self::En => &[
                ("start", "Start using the bot"),
                ("sub", "Subscribe to an RSS feed"),
                ("unsub", "Unsubscribe"),
                ("list", "Show current subscriptions"),
                ("set", "Configure a subscription"),
                ("settings", "Bot settings"),
                ("check", "Check current subscriptions"),
                ("setfeedtag", "Set tags for a subscription"),
                ("unsuball", "Remove all subscriptions"),
                ("activeall", "Enable all subscriptions"),
                ("pauseall", "Pause all subscriptions"),
                ("ping", ""),
                ("help", "Help"),
                ("version", "Bot version"),
            ],
            Self::ZhTw => &[
                ("start", "開始使用"),
                ("sub", "訂閱 RSS 源"),
                ("unsub", "退訂 RSS 源"),
                ("list", "已訂閱的 RSS 源"),
                ("set", "設置訂閱"),
                ("settings", "設置"),
                ("check", "檢查當前訂閱"),
                ("setfeedtag", "設置 rss 訂閱標籤"),
                ("unsuball", "取消所有訂閱"),
                ("activeall", "開啟抓取訂閱更新"),
                ("pauseall", "停止抓取所有訂閱更新"),
                ("ping", ""),
                ("help", "幫助"),
                ("version", "Bot 版本資訊"),
            ],
            Self::Ru => &[
                ("start", "Начать работу с ботом"),
                ("sub", "Подписаться на RSS"),
                ("unsub", "Отписаться"),
                ("list", "Показать подписки"),
                ("set", "Настроить подписку"),
                ("settings", "Настройки бота"),
                ("check", "Проверить подписки"),
                ("setfeedtag", "Задать теги подписки"),
                ("unsuball", "Удалить все подписки"),
                ("activeall", "Включить все подписки"),
                ("pauseall", "Приостановить все подписки"),
                ("ping", ""),
                ("help", "Помощь"),
                ("version", "Версия бота"),
            ],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_value_defaults_to_english() {
        assert_eq!(Lang::from_value(None), Lang::En);
        assert_eq!(Lang::from_value(Some("de")), Lang::En);
        assert_eq!(Lang::from_value(Some("")), Lang::En);
        assert_eq!(Lang::from_value(Some("en")), Lang::En);
        assert_eq!(Lang::from_value(Some("zh-tw")), Lang::ZhTw);
        assert_eq!(Lang::from_value(Some("zh")), Lang::ZhTw);
        assert_eq!(Lang::from_value(Some("ru")), Lang::Ru);
    }

    #[test]
    fn value_round_trips() {
        for lang in [Lang::En, Lang::ZhTw, Lang::Ru] {
            assert_eq!(Lang::from_value(Some(lang.value())), lang);
        }
    }

    #[test]
    fn all_languages_have_non_empty_strings() {
        for lang in [Lang::En, Lang::ZhTw, Lang::Ru] {
            assert!(!lang.help().is_empty(), "help empty for {lang:?}");
            assert!(!lang.settings_title().is_empty());
            assert!(!lang.start_message().is_empty());
            assert!(!lang.version_message().is_empty());
            assert!(!lang.sub_missing_url().is_empty());
            assert!(!lang.unsub_error().is_empty());
            assert!(!lang.set_modified_toast().is_empty());
            assert!(!lang.check_started_msg(3).is_empty());
            assert!(!lang.check_done_msg(1, 2, 0).is_empty());
            assert!(!lang.list_header(5).is_empty());
            assert!(!lang.opml_import_header_msg(3, 1).is_empty());
            assert_eq!(lang.feed_item_button(7, "x"), "[7] x");
            assert!(!lang.command_descriptions().is_empty());
        }
    }

    #[test]
    fn lang_updated_picks_correct_variant() {
        assert!(Lang::En.lang_updated(Lang::En).contains("English"));
        assert!(Lang::Ru.lang_updated(Lang::Ru).contains("Русский"));
    }
}
