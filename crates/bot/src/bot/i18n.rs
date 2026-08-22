//! Localized UI strings. Extracted from `runtime.rs` (which had grown past
//! 750 lines).
//!
//! Deliberately "one string, one method": each `match` over `Lang` is
//! exhaustive, so adding a third language turns into a compile error at every
//! string — the property we want, and one a HashMap/JSON catalog would throw
//! away. The `strings!` macro just collapses the boilerplate 4 lines → 1.
//! Parameterized strings stay hand-written below the macro.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lang {
    En,
    ZhTw,
}

impl Lang {
    pub fn from_value(value: Option<&str>) -> Self {
        match value {
            Some("en") => Self::En,
            _ => Self::ZhTw,
        }
    }

    pub fn value(self) -> &'static str {
        match self {
            Self::En => "en",
            Self::ZhTw => "zh-tw",
        }
    }
}

/// Generates one `pub fn name(self) -> &'static str` per entry.
macro_rules! strings {
    ($($name:ident => en: $en:expr, zh: $zh:expr);* $(;)?) => {
        impl Lang {
            $(
                pub fn $name(self) -> &'static str {
                    match self {
                        Self::En => $en,
                        Self::ZhTw => $zh,
                    }
                }
            )*
        }
    };
}

strings! {
    help => en: "Commands:\n/sub Subscribe to an RSS feed\n/unsub Unsubscribe\n/list Show subscriptions\n/set Feed settings\n/settings Bot settings\n/check Fetch all subscriptions now and push new articles\n/feedcheck Check whether the subscribed feeds still work\n/activeall Enable all subscriptions\n/pauseall Pause all subscriptions\n/unsuball Remove all subscriptions\n/bm Bookmark a URL\n/bookmarks Show bookmarks\n/bmsearch Search bookmarks\n/help Help\n/version Bot version",
        zh: "命令：\n/sub 訂閱 RSS 源\n/unsub 取消訂閱\n/list 查看目前訂閱源\n/set 設定訂閱\n/settings Bot 設定\n/check 立刻抓取所有訂閱並推播新文章\n/feedcheck 檢查訂閱的 feed 是否還有效\n/activeall 開啟所有訂閱\n/pauseall 暫停所有訂閱\n/unsuball 取消所有訂閱\n/bm 收藏網址\n/bookmarks 查看書籤\n/bmsearch 搜尋書籤\n/help 幫助\n/version Bot 版本資訊";
    settings_title => en: "Settings", zh: "設定";
    settings_opml_button => en: "OPML import/export", zh: "OPML 匯入/匯出";
    settings_import_button => en: "Import", zh: "匯入";
    settings_export_button => en: "Export", zh: "匯出";
    settings_interval_button => en: "Refresh interval", zh: "更新頻率";
    settings_language_button => en: "Language", zh: "語系";
    settings_back_button => en: "Back", zh: "返回";
    import_hint => en: "Send an OPML file to import subscriptions.", zh: "請直接傳送 OPML 檔案以匯入訂閱。";
    interval_hint => en: "Choose a refresh interval for all subscriptions in this chat.", zh: "請選擇此聊天室所有訂閱的更新頻率。";
    lang_updated => en: "Language updated: English", zh: "語言已更新：繁體中文";

    // ── ForceReply prompts ─────────────────────────────────────────────────
    prompt_cancel_hint => en: "type \"cancel\" to stop", zh: "輸入「取消」可中止";
    prompt_cancelled => en: "Cancelled.", zh: "已取消。";
    prompt_cancel_button => en: "Cancel", zh: "取消";
    prompt_cancel_control => en: "Tap to cancel this prompt.", zh: "點選以取消這次輸入。";
    sub_prompt => en: "🔗 Reply with the RSS URL to subscribe (send only the URL; type \"cancel\" to stop)", zh: "🔗 請回覆此訊息貼上要訂閱的 RSS 網址（直接傳網址即可；輸入「取消」可中止）";
    sub_placeholder => en: "https://example.com/feed", zh: "https://example.com/feed";
    setfeedtag_prompt => en: "🏷️ Reply with: source ID tag1 tag2 (up to three tags; type \"cancel\" to stop)", zh: "🏷️ 請回覆此訊息輸入：來源 ID 標籤1 標籤2（最多三個標籤；輸入「取消」可中止）";
    setfeedtag_placeholder => en: "12 AI optics", zh: "12 AI 光通訊";

    // ── Bookmarks ──────────────────────────────────────────────────────────
    bm_settings_button => en: "🔖 Bookmarks", zh: "🔖 書籤";
    bm_settings_export => en: "⬇ Export bookmarks", zh: "⬇ 匯出書籤";
    bm_untitled => en: "Untitled", zh: "未命名";
    bm_no_tags => en: "untagged", zh: "未分類";
    bm_untagged_label => en: "untagged", zh: "未分類";
    bm_empty => en: "No bookmarks yet. Send /bm <url>, or tap 🔖 under any pushed item.", zh: "還沒有任何書籤。傳送 /bm <網址>，或點推播訊息下方的 🔖。";
    bm_search_empty => en: "No bookmarks matched.", zh: "沒有符合的書籤。";
    bm_search_usage => en: "Usage: /bmsearch <keyword>", zh: "用法：/bmsearch <關鍵字>";
    bm_prev => en: "◀ Prev", zh: "◀ 上一頁";
    bm_next => en: "Next ▶", zh: "下一頁 ▶";
    bm_tags_button => en: "🏷 Tags", zh: "🏷 標籤";
    bm_export_button => en: "⬇ Export", zh: "⬇ 匯出";
    bm_tag_button => en: "🏷 Tags", zh: "🏷 標籤";
    bm_note_button => en: "📝 Note", zh: "📝 備註";
    bm_delete_button => en: "🗑 Delete", zh: "🗑 刪除";
    bm_back_to_list => en: "◀ Back to list", zh: "◀ 返回列表";
    bm_tag_pending => en: "⏳ Tagging…", zh: "⏳ 標籤處理中…";
    bm_delete_confirm_prompt => en: "Delete this bookmark?", zh: "確定要刪除這則書籤嗎？";
    bm_confirm_delete => en: "Confirm delete", zh: "確認刪除";
    bm_cancel => en: "Cancel", zh: "取消";
    bm_deleted => en: "Deleted", zh: "已刪除";
    bm_saved_toast => en: "🔖 Saved", zh: "🔖 已收藏";
    bm_saved_button => en: "🔖 Saved", zh: "🔖 已收藏";
    bm_expired => en: "This item has expired — use /bm <url> instead.", zh: "此項目已過期，請改用 /bm <網址>";
    bm_not_found => en: "Bookmark not found.", zh: "找不到該書籤。";
    bm_no_permission => en: "You can't do that here.", zh: "你沒有權限這麼做。";
    bm_bad_action => en: "Unknown action.", zh: "未知的操作。";
    bm_usage => en: "Usage: /bm <url> (or reply to a message with a link).", zh: "用法：/bm <網址>（或回覆一則含連結的訊息）。";
    bm_invalid_url => en: "That doesn't look like a valid http(s) URL.", zh: "這看起來不是有效的 http(s) 網址。";
    bm_prompt => en: "🔖 Reply with the URL to bookmark (type \"cancel\" to stop)", zh: "🔖 請回覆此訊息貼上要收藏的網址（輸入「取消」可中止）";
    bm_placeholder => en: "https://example.com/article", zh: "https://example.com/article";
    bm_search_prompt => en: "🔍 Reply with bookmark search keywords (type \"cancel\" to stop)", zh: "🔍 請回覆此訊息輸入要搜尋的書籤關鍵字（輸入「取消」可中止）";
    bm_search_placeholder => en: "keyword", zh: "關鍵字";
    bm_note_prompt => en: "📝 Reply with: bookmark ID note text (type \"cancel\" to stop)", zh: "📝 請回覆此訊息輸入：書籤 ID 備註內容（輸入「取消」可中止）";
    bm_note_placeholder => en: "123 useful for later research", zh: "123 這篇很適合之後研究";
    bm_tag_prompt => en: "🏷️ Reply with: bookmark ID tag1 tag2 (type \"cancel\" to stop)", zh: "🏷️ 請回覆此訊息輸入：書籤 ID 標籤1 標籤2（輸入「取消」可中止）";
    bm_tag_placeholder => en: "123 AI optics", zh: "123 AI 光通訊";
    bm_delete_prompt => en: "🗑️ Reply with the bookmark ID to delete (type \"cancel\" to stop)", zh: "🗑️ 請回覆此訊息輸入要刪除的書籤 ID（輸入「取消」可中止）";
    bm_delete_placeholder => en: "123", zh: "123";
    bm_note_usage => en: "Usage: /bmnote <id> <text>", zh: "用法：/bmnote <id> <文字>";
    bm_note_saved => en: "Note saved.", zh: "備註已儲存。";
    bm_tag_usage => en: "Usage: /bmtag <id> <slug…>", zh: "用法：/bmtag <id> <分類…>";
    bm_tag_saved => en: "Tags updated.", zh: "標籤已更新。";
    bm_tag_index_header => en: "🏷 <b>Tags</b>", zh: "🏷 <b>標籤</b>";
    bm_toggle_hint => en: "Tap to toggle tags:", zh: "點選以切換標籤：";
    bm_added => en: "🔖 Bookmarked.", zh: "🔖 已收藏。";
    bm_summarizing => en: "📝 Summarizing…", zh: "📝 摘要產生中…";
    bm_summary_failed => en: "Summary failed.", zh: "摘要失敗。";
    bm_summary_unavailable => en: "AI summary is not configured.", zh: "尚未設定 AI 摘要。";
    bm_summary_heading => en: "📝 <b>Summary</b>", zh: "📝 <b>摘要</b>";

    // ── Stock tracking (render-facing) ─────────────────────────────────────
    stk_vol => en: "Vol", zh: "量";
    stk_week52 => en: "52w", zh: "52週";
    stk_macd_hist => en: "hist", zh: "柱";
    stk_insufficient => en: "Not enough history for indicators yet.", zh: "歷史資料不足，暫無技術指標。";
    stk_indicators_unavailable => en: "Indicators unavailable (history not updated).", zh: "技術指標暫不可用（歷史資料未更新）。";
    stk_stale => en: "⚠️ cached data", zh: "⚠️ 快取資料";
    stk_delayed => en: "⏰ delayed", zh: "⏰ 延遲送出";
    stk_empty => en: "No stocks tracked yet. Use /stockadd <symbol>.", zh: "還沒有追蹤任何股票。使用 /stockadd <代號>。";
    stk_ai_disclaimer => en: "For reference only; not investment advice.", zh: "以上僅供參考，不構成投資建議。";

    // ── Stock tracking (commands / callbacks / settings) ───────────────────
    stk_settings_button => en: "📈 Stocks", zh: "📈 股票";
    stk_add_button => en: "➕ Track", zh: "➕ 加入追蹤";
    stk_added => en: "➕ Added to your watchlist.", zh: "➕ 已加入自選股。";
    stk_already => en: "Already in your watchlist.", zh: "已在自選股清單中。";
    stk_unknown_symbol => en: "Symbol not found. Check the code and try again.", zh: "找不到這個代號，請確認後再試。";
    stk_upstream => en: "Data source is unavailable right now. Try again later.", zh: "資料來源暫時無法使用，請稍後再試。";
    stk_removed => en: "🗑 Removed from your watchlist.", zh: "🗑 已從自選股移除。";
    stk_not_found => en: "Not in your watchlist.", zh: "不在你的自選股清單中。";
    stk_no_permission => en: "You can't do that here.", zh: "你沒有權限這麼做。";
    stk_bad_action => en: "Unknown action.", zh: "未知的操作。";
    stk_stock_usage => en: "Usage: /stock <symbol> (e.g. 2330, AAPL).", zh: "用法：/stock <代號>（例如 2330、AAPL）。";
    stk_add_usage => en: "Usage: /stockadd <symbol> (e.g. 2330, 6488, AAPL).", zh: "用法：/stockadd <代號>（例如 2330、6488、AAPL）。";
    stk_del_usage => en: "Usage: /stockdel <id or symbol>.", zh: "用法：/stockdel <編號或代號>。";
    stk_delete_confirm => en: "Remove this stock from the watchlist?", zh: "確定要從自選股移除這檔股票嗎？";
    stk_confirm_delete => en: "Confirm remove", zh: "確認移除";
    stk_prompt => en: "📈 Reply with a stock symbol to look up (type \"cancel\" to stop)", zh: "📈 請回覆此訊息輸入要查詢的股票代號（輸入「取消」可中止）";
    stk_placeholder => en: "2330 or AAPL", zh: "2330 或 AAPL";
    stk_add_prompt => en: "➕ Reply with a stock symbol to track (type \"cancel\" to stop)", zh: "➕ 請回覆此訊息輸入要追蹤的股票代號（輸入「取消」可中止）";
    stk_del_prompt => en: "🗑️ Reply with the id or symbol to remove (type \"cancel\" to stop)", zh: "🗑️ 請回覆此訊息輸入要移除的編號或代號（輸入「取消」可中止）";
    stk_push_time_tw_prompt => en: "🇹🇼 Reply with the TW close-push time HH:MM, or \"off\" (type \"cancel\" to stop)", zh: "🇹🇼 請回覆此訊息輸入台股收盤推播時間 HH:MM，或「off」關閉（輸入「取消」可中止）";
    stk_push_time_us_prompt => en: "🇺🇸 Reply with the US close-push time HH:MM, or \"off\" (type \"cancel\" to stop)", zh: "🇺🇸 請回覆此訊息輸入美股收盤推播時間 HH:MM，或「off」關閉（輸入「取消」可中止）";
    stk_push_time_placeholder => en: "14:00 or off", zh: "14:00 或 off";
    stk_push_title => en: "📈 <b>Close-push settings</b>\nTap a market to toggle, or set a time.", zh: "📈 <b>收盤推播設定</b>\n點市場可切換開關，或設定推播時間。";
    stk_push_saved => en: "Saved.", zh: "已儲存。";
    stk_push_bad_time => en: "Use HH:MM (00:00–23:59) or \"off\".", zh: "請輸入 HH:MM（00:00–23:59）或「off」。";
    stk_push_time_default => en: "after close", zh: "收盤後";
    stk_report_working => en: "📈 Generating today's close report…", zh: "📈 開始產生今日收盤報告…";
    stk_report_already => en: "Today's report has already been sent.", zh: "今日報告已經送出過了。";
    stk_ai_button => en: "🤖 AI read", zh: "🤖 AI 解讀";
    stk_ai_working => en: "🤖 Generating AI commentary…", zh: "🤖 產生 AI 解讀中…";
    stk_ai_unavailable => en: "AI commentary is not configured.", zh: "尚未設定 AI 解讀。";
    stk_ai_failed => en: "AI commentary failed.", zh: "AI 解讀失敗。";
    stk_ai_heading => en: "🤖 <b>AI commentary</b>", zh: "🤖 <b>AI 解讀</b>";
}

impl Lang {
    pub fn interval_updated(self, count: u64) -> String {
        match self {
            Self::En => format!("Updated {count} subscriptions"),
            Self::ZhTw => format!("已更新 {count} 個訂閱"),
        }
    }

    pub fn sub_failed_retry(self, err: &str) -> String {
        match self {
            Self::En => format!("{err}; subscription failed. Reply with the RSS URL again, or type \"cancel\"."),
            Self::ZhTw => format!("{err}，訂閱失敗。請重新貼上 RSS 網址，或輸入「取消」。"),
        }
    }

    pub fn bm_invalid_url_retry(self) -> String {
        match self {
            Self::En => format!("{} Reply with the URL again, or type \"cancel\".", self.bm_invalid_url()),
            Self::ZhTw => format!("{} 請重新貼上網址，或輸入「取消」。", self.bm_invalid_url()),
        }
    }

    /// Header line for a bookmark list page. Uses `<b>` (safe, static) so the
    /// renderer can emit it verbatim.
    pub fn bm_list_header(self, total: i64, page: usize, pages: usize) -> String {
        match self {
            Self::En => format!("🔖 <b>Bookmarks</b> · {total} total · page {page}/{pages}"),
            Self::ZhTw => format!("🔖 <b>書籤</b> · 共 {total} 筆 · 第 {page}/{pages} 頁"),
        }
    }

    /// Settings toggle label, e.g. "🔖 Push bookmark button: on".
    pub fn bm_settings_btn_toggle(self, on: bool) -> String {
        let state = self.on_off(on);
        match self {
            Self::En => format!("🔖 Push bookmark button: {state}"),
            Self::ZhTw => format!("🔖 推送書籤按鈕：{state}"),
        }
    }

    pub fn bm_settings_ai_toggle(self, on: bool) -> String {
        let state = self.on_off(on);
        match self {
            Self::En => format!("🤖 AI auto-tagging: {state}"),
            Self::ZhTw => format!("🤖 AI 自動標籤：{state}"),
        }
    }

    pub fn bm_settings_summary_toggle(self, on: bool) -> String {
        let state = self.on_off(on);
        match self {
            Self::En => format!("📝 Summary button: {state}"),
            Self::ZhTw => format!("📝 摘要按鈕：{state}"),
        }
    }

    fn on_off(self, on: bool) -> &'static str {
        match (self, on) {
            (Self::En, true) => "on",
            (Self::En, false) => "off",
            (Self::ZhTw, true) => "開",
            (Self::ZhTw, false) => "關",
        }
    }

    pub fn stk_market_name(self, market: crate::stock::Market) -> &'static str {
        use crate::stock::Market::{Tw, Us};
        match (self, market) {
            (Self::En, Tw) => "TW",
            (Self::En, Us) => "US",
            (Self::ZhTw, Tw) => "台股",
            (Self::ZhTw, Us) => "美股",
        }
    }

    pub fn stk_scope_name(self, scope: crate::stock::MarketScope) -> &'static str {
        use crate::stock::MarketScope::{All, Tw, Us};
        match (self, scope) {
            (Self::En, All) => "All",
            (Self::ZhTw, All) => "全部",
            (_, Tw) => self.stk_market_name(crate::stock::Market::Tw),
            (_, Us) => self.stk_market_name(crate::stock::Market::Us),
        }
    }

    /// A detected signal as a short, emoji-prefixed label. Kept here (not in
    /// render.rs) so both languages stay side by side per the one-string-one-
    /// method convention.
    pub fn stk_signal(self, sig: crate::stock::Signal) -> &'static str {
        use crate::stock::Signal::*;
        match (self, sig) {
            (Self::En, MaGoldenCross) => "⭐ MA golden cross",
            (Self::En, MaDeadCross) => "⚠️ MA death cross",
            (Self::En, KdGoldenCross) => "⭐ KD golden cross",
            (Self::En, KdDeadCross) => "⚠️ KD death cross",
            (Self::En, MacdGoldenCross) => "⭐ MACD golden cross",
            (Self::En, MacdDeadCross) => "⚠️ MACD death cross",
            (Self::En, BollBreakUpper) => "📈 broke above upper Bollinger",
            (Self::En, BollBreakLower) => "📉 broke below lower Bollinger",
            (Self::En, RsiOverbought) => "🔴 RSI overbought (>70)",
            (Self::En, RsiOversold) => "🟢 RSI oversold (<30)",
            (Self::ZhTw, MaGoldenCross) => "⭐ 均線黃金交叉",
            (Self::ZhTw, MaDeadCross) => "⚠️ 均線死亡交叉",
            (Self::ZhTw, KdGoldenCross) => "⭐ KD 黃金交叉",
            (Self::ZhTw, KdDeadCross) => "⚠️ KD 死亡交叉",
            (Self::ZhTw, MacdGoldenCross) => "⭐ MACD 黃金交叉",
            (Self::ZhTw, MacdDeadCross) => "⚠️ MACD 死亡交叉",
            (Self::ZhTw, BollBreakUpper) => "📈 突破布林上軌",
            (Self::ZhTw, BollBreakLower) => "📉 跌破布林下軌",
            (Self::ZhTw, RsiOverbought) => "🔴 RSI 超買 (>70)",
            (Self::ZhTw, RsiOversold) => "🟢 RSI 超賣 (<30)",
        }
    }

    /// Daily close-report header, e.g. "📈 台股收盤報告 · 2026-08-21 · 5 檔".
    pub fn stk_report_header(self, market: crate::stock::Market, date: &str, count: usize) -> String {
        let name = self.stk_market_name(market);
        match self {
            Self::En => format!("📈 {name} close report · {date} · {count} symbols"),
            Self::ZhTw => format!("📈 {name}收盤報告 · {date} · {count} 檔"),
        }
    }

    pub fn stk_report_overflow(self, more: usize) -> String {
        match self {
            Self::En => format!("…and {more} more, see /stocks"),
            Self::ZhTw => format!("…及其他 {more} 檔，見 /stocks"),
        }
    }

    pub fn stk_report_not_closed(self, market: crate::stock::Market) -> String {
        let name = self.stk_market_name(market);
        match self {
            Self::En => format!("The {name} market hasn't closed yet — try again after the close."),
            Self::ZhTw => format!("{name}尚未收盤，請於收盤後再試。"),
        }
    }

    pub fn stk_limit_chat(self, max: u32) -> String {
        match self {
            Self::En => format!("Watchlist is full ({max} max). Remove one first."),
            Self::ZhTw => format!("自選股已達上限（{max} 檔），請先移除一檔。"),
        }
    }

    pub fn stk_limit_global(self, max: u32) -> String {
        match self {
            Self::En => format!("The global symbol limit ({max}) is reached; try again later."),
            Self::ZhTw => format!("已達全域追蹤上限（{max} 檔），請稍後再試。"),
        }
    }

    /// Per-market enable toggle button, e.g. "🇹🇼 台股：開".
    pub fn stk_push_toggle(self, market: crate::stock::Market, on: bool) -> String {
        let name = self.stk_market_name(market);
        let state = self.on_off(on);
        format!("{} {name}：{state}", self.stk_market_flag(market))
    }

    /// Per-market time button, e.g. "🇹🇼 時間：14:00".
    pub fn stk_push_time_button(self, market: crate::stock::Market, time: &str) -> String {
        let flag = self.stk_market_flag(market);
        match self {
            Self::En => format!("{flag} time: {time}"),
            Self::ZhTw => format!("{flag} 時間：{time}"),
        }
    }

    fn stk_market_flag(self, market: crate::stock::Market) -> &'static str {
        match market {
            crate::stock::Market::Tw => "🇹🇼",
            crate::stock::Market::Us => "🇺🇸",
        }
    }

    /// Watchlist page header line.
    pub fn stk_list_header(
        self,
        scope: crate::stock::MarketScope,
        total: usize,
        page: usize,
        pages: usize,
    ) -> String {
        let scope_name = self.stk_scope_name(scope);
        match self {
            Self::En => {
                format!("📈 <b>Watchlist</b> ({scope_name}) · {total} total · page {page}/{pages}")
            }
            Self::ZhTw => {
                format!("📈 <b>自選股</b>（{scope_name}）· 共 {total} 檔 · 第 {page}/{pages} 頁")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_value_defaults_to_zhtw() {
        assert_eq!(Lang::from_value(None), Lang::ZhTw);
        assert_eq!(Lang::from_value(Some("de")), Lang::ZhTw);
        assert_eq!(Lang::from_value(Some("en")), Lang::En);
    }

    #[test]
    fn value_round_trips() {
        for lang in [Lang::En, Lang::ZhTw] {
            assert_eq!(Lang::from_value(Some(lang.value())), lang);
        }
    }

    #[test]
    fn bookmark_strings_are_non_empty_in_both_languages() {
        for lang in [Lang::En, Lang::ZhTw] {
            for s in [
                lang.bm_settings_button(),
                lang.bm_untitled(),
                lang.bm_no_tags(),
                lang.bm_empty(),
                lang.bm_prev(),
                lang.bm_next(),
                lang.bm_delete_button(),
                lang.bm_tag_pending(),
                lang.bm_saved_toast(),
                lang.bm_expired(),
                lang.bm_no_permission(),
                lang.bm_bad_action(),
                lang.bm_usage(),
                lang.bm_tag_index_header(),
            ] {
                assert!(!s.is_empty(), "empty bookmark string for {lang:?}");
            }
            assert!(!lang.bm_list_header(3, 1, 1).is_empty());
            assert!(lang.bm_settings_btn_toggle(true).contains(lang.on_off(true)));
        }
    }
}
