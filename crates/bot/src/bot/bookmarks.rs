//! Bookmark command handlers, the `bm:` callback protocol + dispatch, and the
//! `settings:bm*` delegation. Pure rendering lives in `crate::bookmark::render`.

use teloxide::{
    prelude::*,
    types::{
        ForceReply, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, MessageEntityKind, MessageId,
        ParseMode, ReplyParameters,
    },
    utils::html::escape,
    ApiError, RequestError,
};

use crate::bookmark::render::{self, ListPageData, RenderedBookmark};
use crate::tagging::mcp::McpClient;
use crate::bot::i18n::Lang;
use crate::bot::pagination::{nav_row, Page};
use crate::bot::runtime::{chat_lang, no_preview, to_request_error, BotState};
use crate::config::Config;
use crate::db::bookmarks::{now_unix, NewBookmark};
use crate::db::models::Bookmark;
use crate::db::repo::Repo;
use crate::tagging::taxonomy;
use crate::tagging::url_norm::normalize_url;

const SEARCH_LIMIT: i64 = 10;
const NOTE_MAX_CHARS: usize = 1000;

pub const BM_BTN_PREFIX: &str = "tg-kl-vault:bmbtn:";
pub const BM_AI_PREFIX: &str = "tg-kl-vault:bmai:";
pub const BM_SUM_PREFIX: &str = "tg-kl-vault:bmsum:";

pub const BM_PROMPT: &str = "🔖 請回覆此訊息貼上要收藏的網址（輸入「取消」可中止）";
pub const BMSEARCH_PROMPT: &str = "🔍 請回覆此訊息輸入要搜尋的書籤關鍵字（輸入「取消」可中止）";
pub const BMNOTE_PROMPT: &str = "📝 請回覆此訊息輸入：書籤 ID 備註內容（輸入「取消」可中止）";
pub const BMTAG_PROMPT: &str = "🏷️ 請回覆此訊息輸入：書籤 ID 標籤1 標籤2（輸入「取消」可中止）";
pub const BMDEL_PROMPT: &str = "🗑️ 請回覆此訊息輸入要刪除的書籤 ID（輸入「取消」可中止）";

fn force_reply(placeholder: &str) -> ForceReply {
    ForceReply::new().input_field_placeholder(placeholder.to_owned())
}

fn bm_btn_key(chat_id: i64) -> String {
    format!("{BM_BTN_PREFIX}{chat_id}")
}
fn bm_ai_key(chat_id: i64) -> String {
    format!("{BM_AI_PREFIX}{chat_id}")
}
fn bm_sum_key(chat_id: i64) -> String {
    format!("{BM_SUM_PREFIX}{chat_id}")
}

/// Opt-out options default to on: a missing/`"1"` row is on, `"0"` is off.
async fn option_on(state: &BotState, key: &str) -> bool {
    state.repo.get_option(key).await.ok().flatten().as_deref() != Some("0")
}

/// The 📝 summary button is available when a bridge is configured and the chat
/// hasn't opted out. Used by handlers (which have `BotState`).
pub async fn summary_enabled(state: &BotState, chat_id: i64) -> bool {
    summary_enabled_raw(&state.repo, &state.config, chat_id).await
}

/// Same check for callers without a `BotState` (the tag worker).
pub async fn summary_enabled_raw(repo: &Repo, cfg: &Config, chat_id: i64) -> bool {
    cfg.bookmark.ai.mcp.is_configured()
        && repo.get_option(&bm_sum_key(chat_id)).await.ok().flatten().as_deref() != Some("0")
}

// ─── Scope (list filter, encoded into callback_data) ────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Scope {
    All,
    Tag(usize),
    Untagged,
}

impl Scope {
    pub fn to_wire(self) -> String {
        match self {
            Scope::All => "a".to_owned(),
            Scope::Tag(idx) => format!("t{idx}"),
            Scope::Untagged => "u".to_owned(),
        }
    }

    pub fn parse(s: &str) -> Option<Scope> {
        match s {
            "a" => Some(Scope::All),
            "u" => Some(Scope::Untagged),
            _ => s
                .strip_prefix('t')
                .and_then(|n| n.parse::<usize>().ok())
                .filter(|idx| taxonomy::slug_of(*idx).is_some())
                .map(Scope::Tag),
        }
    }
}

// ─── Callback data builders. NEVER inline `format!` at call sites, or the byte
//     tests below are bypassed. Pure-colon `bm:` namespace; the frozen telebot
//     binary format is untouched. ────────────────────────────────────────────

pub mod cb {
    use super::Scope;

    pub fn add(hash: &str) -> String {
        format!("bm:add:{hash}")
    }
    pub fn sum(hash: &str) -> String {
        format!("bm:sum:{hash}")
    }
    pub fn list(scope: Scope, page: usize) -> String {
        format!("bm:list:{}:{page}", scope.to_wire())
    }
    pub fn view(id: i64, scope: Scope, page: usize) -> String {
        format!("bm:view:{id}:{}:{page}", scope.to_wire())
    }
    pub fn del(id: i64, scope: Scope, page: usize) -> String {
        format!("bm:del:{id}:{}:{page}", scope.to_wire())
    }
    pub fn delok(id: i64, scope: Scope, page: usize) -> String {
        format!("bm:delok:{id}:{}:{page}", scope.to_wire())
    }
    pub fn retag(id: i64, scope: Scope, page: usize) -> String {
        format!("bm:retag:{id}:{}:{page}", scope.to_wire())
    }
    pub fn note(id: i64, scope: Scope, page: usize) -> String {
        format!("bm:note:{id}:{}:{page}", scope.to_wire())
    }
    pub fn tt(id: i64, idx: usize, scope: Scope, page: usize) -> String {
        format!("bm:tt:{id}:{idx}:{}:{page}", scope.to_wire())
    }
    pub fn tags(page: usize) -> String {
        format!("bm:tags:{page}")
    }
    pub fn export() -> String {
        "bm:export".to_owned()
    }
}

// ─── Keyboards ──────────────────────────────────────────────────────────────

/// State of the bookmark button on a pushed item: not-yet-saved (`bm:add`) or
/// already saved (points at the detail page, label carries tags / "saved").
pub enum BmBtn {
    Add(String),
    Saved { id: i64, label: String },
}

/// The inline keyboard for a pushed item: an optional 🔖/saved button and an
/// optional 📝 summary button, sharing one row. Emoji-only labels so no
/// per-chat language lookup is needed on the broadcast path. `None` when both
/// are absent (so callers send no `reply_markup`).
pub fn item_keyboard(bm: Option<BmBtn>, sum_hash: Option<&str>) -> Option<InlineKeyboardMarkup> {
    let mut row: Vec<InlineKeyboardButton> = Vec::new();
    match &bm {
        Some(BmBtn::Add(hash)) => row.push(InlineKeyboardButton::callback("🔖", cb::add(hash))),
        Some(BmBtn::Saved { id, label }) => {
            row.push(InlineKeyboardButton::callback(label.clone(), cb::view(*id, Scope::All, 0)))
        }
        None => {}
    }
    if let Some(hash) = sum_hash {
        row.push(InlineKeyboardButton::callback("📝", cb::sum(hash)));
    }
    (!row.is_empty()).then(|| InlineKeyboardMarkup::new(vec![row]))
}

fn list_keyboard(items: &[Bookmark], page: &Page, scope: Scope, lang: Lang) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    let id_row: Vec<InlineKeyboardButton> = items
        .iter()
        .map(|b| InlineKeyboardButton::callback(format!("{}", b.id), cb::view(b.id, scope, page.index)))
        .collect();
    if !id_row.is_empty() {
        rows.push(id_row);
    }
    let nav = nav_row(page, |i| cb::list(scope, i), lang);
    if !nav.is_empty() {
        rows.push(nav);
    }
    rows.push(vec![
        InlineKeyboardButton::callback(lang.bm_tags_button(), cb::tags(0)),
        InlineKeyboardButton::callback(lang.bm_export_button(), cb::export()),
    ]);
    InlineKeyboardMarkup::new(rows)
}

/// Detail keyboard for a fresh view (scope=All, page=0). Public for the tag
/// worker, which edits a `/bm` reply into the final detail card.
pub fn detail_markup(id: i64, lang: Lang) -> InlineKeyboardMarkup {
    detail_keyboard(id, Scope::All, 0, lang)
}

/// Per-chat AI-tagging option key. Public so the worker can honour the toggle.
pub fn ai_option_key(chat_id: i64) -> String {
    bm_ai_key(chat_id)
}

fn detail_keyboard(id: i64, scope: Scope, page: usize, lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback(lang.bm_tag_button(), cb::retag(id, scope, page)),
            InlineKeyboardButton::callback(lang.bm_note_button(), cb::note(id, scope, page)),
        ],
        vec![InlineKeyboardButton::callback(lang.bm_delete_button(), cb::del(id, scope, page))],
        vec![InlineKeyboardButton::callback(lang.bm_back_to_list(), cb::list(scope, page))],
    ])
}

fn delete_confirm_keyboard(id: i64, scope: Scope, page: usize, lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(lang.bm_confirm_delete(), cb::delok(id, scope, page)),
        InlineKeyboardButton::callback(lang.bm_cancel(), cb::view(id, scope, page)),
    ]])
}

fn retag_keyboard(id: i64, selected: &[String], scope: Scope, page: usize, lang: Lang) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for chunk in taxonomy::TAGS.chunks(3) {
        let row: Vec<InlineKeyboardButton> = chunk
            .iter()
            .map(|cat| {
                let idx = taxonomy::idx_of(cat.slug).unwrap_or(0);
                let checked = selected.iter().any(|s| s == cat.slug);
                let label = if checked { format!("✅ {}", cat.slug) } else { cat.slug.to_owned() };
                InlineKeyboardButton::callback(label, cb::tt(id, idx, scope, page))
            })
            .collect();
        rows.push(row);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        lang.bm_back_to_list(),
        cb::view(id, scope, page),
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn tags_index_keyboard(counts: &[(String, i64)], untagged: i64, lang: Lang) -> InlineKeyboardMarkup {
    let mut buttons: Vec<InlineKeyboardButton> = Vec::new();
    for (tag, n) in counts {
        if *n <= 0 {
            continue;
        }
        if let Some(idx) = taxonomy::idx_of(tag) {
            buttons.push(InlineKeyboardButton::callback(
                format!("#{tag} ({n})"),
                cb::list(Scope::Tag(idx), 0),
            ));
        }
    }
    if untagged > 0 {
        buttons.push(InlineKeyboardButton::callback(
            format!("{} ({untagged})", lang.bm_untagged_label()),
            cb::list(Scope::Untagged, 0),
        ));
    }
    let mut rows: Vec<Vec<InlineKeyboardButton>> = buttons.chunks(3).map(<[_]>::to_vec).collect();
    rows.push(vec![InlineKeyboardButton::callback(
        lang.bm_back_to_list(),
        cb::list(Scope::All, 0),
    )]);
    InlineKeyboardMarkup::new(rows)
}

/// `sum_on` is `Some(state)` only when an MCP bridge is configured; `None`
/// hides the summary-button toggle entirely.
pub fn settings_bm_keyboard(
    lang: Lang,
    btn_on: bool,
    ai_on: bool,
    sum_on: Option<bool>,
) -> InlineKeyboardMarkup {
    let mut rows = vec![
        vec![InlineKeyboardButton::callback(lang.bm_settings_btn_toggle(btn_on), "settings:bm:btn")],
        vec![InlineKeyboardButton::callback(lang.bm_settings_ai_toggle(ai_on), "settings:bm:ai")],
    ];
    if let Some(on) = sum_on {
        rows.push(vec![InlineKeyboardButton::callback(
            lang.bm_settings_summary_toggle(on),
            "settings:bm:sum",
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(lang.bm_settings_export(), "settings:bm:export")]);
    rows.push(vec![InlineKeyboardButton::callback(lang.settings_back_button(), "settings:back")]);
    InlineKeyboardMarkup::new(rows)
}

// ─── URL extraction from a message (entities first, naive scan last) ─────────

fn pick_url<'a>(entities: impl Iterator<Item = (&'a MessageEntityKind, &'a str)>) -> Option<String> {
    for (kind, text) in entities {
        match kind {
            MessageEntityKind::TextLink { url } => return Some(url.as_str().to_owned()),
            MessageEntityKind::Url => return Some(text.to_owned()),
            _ => {}
        }
    }
    None
}

fn naive_url_scan(text: &str) -> Option<String> {
    let start = text.find("https://").or_else(|| text.find("http://"))?;
    let rest = &text[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_owned())
}

/// Extracts the first URL from a message. Scans entities (the bot renders its
/// own links as `<a href>`, so the URL lives in a `TextLink`, not the visible
/// text), falling back to a naive `http(s)://` scan of text/caption.
pub fn extract_url(msg: &Message) -> Option<String> {
    if let Some(entities) = msg.parse_entities() {
        if let Some(url) = pick_url(entities.iter().map(|e| (e.kind(), e.text()))) {
            return Some(url);
        }
    }
    if let Some(entities) = msg.parse_caption_entities() {
        if let Some(url) = pick_url(entities.iter().map(|e| (e.kind(), e.text()))) {
            return Some(url);
        }
    }
    naive_url_scan(msg.text().or_else(|| msg.caption())?)
}

// ─── Authorization (see design step 8) ──────────────────────────────────────

/// The callback_data carries no owner/chat id; ownership is always the message
/// entity's chat. Read/add is open to any member; destructive ops require the
/// creator or a chat admin/owner.
async fn bookmark_auth(bot: &Bot, chat_id: ChatId, query: &CallbackQuery, created_by: i64) -> bool {
    let user_id = query.from.id.0 as i64;
    if chat_id.0 == user_id || user_id == created_by {
        return true;
    }
    // Group/supergroup: one round-trip, well within answerCallbackQuery's ~15s.
    match bot.get_chat_member(chat_id, query.from.id).await {
        Ok(member) => member.is_privileged(),
        Err(_) => false,
    }
}

fn user_allowed(state: &BotState, user_id: i64) -> bool {
    // `allowed_users` is parsed but never enforced elsewhere; enforce it here so
    // strangers can't burn the Gemini quota. Empty list = open to everyone.
    state.config.allowed_users.is_empty() || state.config.allowed_users.contains(&user_id)
}

// ─── Edit helpers ────────────────────────────────────────────────────────────

fn is_benign_edit_error(err: &RequestError) -> bool {
    matches!(
        err,
        RequestError::Api(ApiError::MessageNotModified)
            | RequestError::Api(ApiError::MessageToEditNotFound)
            | RequestError::Api(ApiError::MessageCantBeEdited)
    )
}

/// Edits a bookmark message in place: HTML + no link preview (or paging
/// re-triggers the preview card), swallowing benign edit errors.
async fn edit_page(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    text: &str,
    markup: InlineKeyboardMarkup,
) -> ResponseResult<()> {
    let result = bot
        .edit_message_text(chat_id, message_id, text)
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview())
        .reply_markup(markup)
        .await;
    if let Err(err) = result {
        if !is_benign_edit_error(&err) {
            return Err(err);
        }
    }
    Ok(())
}

async fn toast(bot: &Bot, query: &CallbackQuery, text: &str) -> ResponseResult<()> {
    bot.answer_callback_query(query.id.clone()).text(text).await?;
    Ok(())
}

async fn ack(bot: &Bot, query: &CallbackQuery) -> ResponseResult<()> {
    bot.answer_callback_query(query.id.clone()).await?;
    Ok(())
}

// ─── View builders (async: hit the DB, then render) ─────────────────────────

async fn tag_slugs_for(state: &BotState, id: i64) -> Vec<String> {
    state
        .repo
        .tags_for_bookmarks(&[id])
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| t.tag)
        .collect()
}

async fn build_list_view(
    state: &BotState,
    chat_id: i64,
    scope: Scope,
    requested_page: usize,
    lang: Lang,
) -> crate::db::DbResult<(String, InlineKeyboardMarkup)> {
    let per_page = state.config.bookmark.ai.page_size.max(1) as usize;
    let total = match scope {
        Scope::All => state.repo.count_bookmarks(chat_id).await?,
        Scope::Tag(idx) => {
            let slug = taxonomy::slug_of(idx).unwrap_or("other");
            state.repo.count_bookmarks_by_tag(chat_id, slug).await?
        }
        Scope::Untagged => state.repo.count_untagged(chat_id).await?,
    };
    let page = Page::clamped(requested_page, per_page, total as usize);
    let items = match scope {
        Scope::All => state.repo.bookmarks_page(chat_id, page.offset(), page.limit()).await?,
        Scope::Tag(idx) => {
            let slug = taxonomy::slug_of(idx).unwrap_or("other");
            state.repo.bookmarks_page_by_tag(chat_id, slug, page.offset(), page.limit()).await?
        }
        Scope::Untagged => state.repo.bookmarks_page_untagged(chat_id, page.offset(), page.limit()).await?,
    };

    let ids: Vec<i64> = items.iter().map(|b| b.id).collect();
    let all_tags = state.repo.tags_for_bookmarks(&ids).await?;
    let rendered: Vec<RenderedBookmark> = items
        .iter()
        .map(|b| RenderedBookmark {
            bookmark: b,
            tags: Box::leak(
                all_tags
                    .iter()
                    .filter(|t| t.bookmark_id == b.id)
                    .map(|t| t.tag.clone())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        })
        .collect();

    let text = render::render_list_page(&ListPageData {
        lang,
        total,
        human_page: page.human_index(),
        total_pages: page.total_pages(),
        items: &rendered,
    });
    let markup = list_keyboard(&items, &page, scope, lang);
    Ok((text, markup))
}

async fn build_detail_view(
    state: &BotState,
    bm: &Bookmark,
    scope: Scope,
    page: usize,
    lang: Lang,
) -> (String, InlineKeyboardMarkup) {
    let tags = tag_slugs_for(state, bm.id).await;
    let text = render::render_detail(bm, &tags, lang);
    let markup = detail_keyboard(bm.id, scope, page, lang);
    (text, markup)
}

// ─── Command handlers ────────────────────────────────────────────────────────

pub async fn handle_bm(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let user_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(chat_id.0);
    let lang = chat_lang(&state.repo, chat_id.0).await;

    if !user_allowed(state, user_id) {
        bot.send_message(chat_id, lang.bm_no_permission()).await?;
        return Ok(());
    }

    let raw = if !payload.is_empty() {
        payload.to_owned()
    } else if let Some(url) = msg.reply_to_message().and_then(extract_url) {
        url
    } else {
        bot.send_message(chat_id, BM_PROMPT)
            .reply_markup(force_reply("https://example.com/article"))
            .await?;
        return Ok(());
    };

    let url = match normalize_url(&raw) {
        Ok(url) => url,
        Err(_) => {
            bot.send_message(chat_id, format!("{} 請重新貼上網址，或輸入「取消」。", lang.bm_invalid_url()))
                .reply_markup(force_reply("https://example.com/article"))
                .await?;
            return Ok(());
        }
    };

    let new = NewBookmark {
        chat_id: chat_id.0,
        created_by: user_id,
        url: &url,
        title: "",
        note: "",
        source_title: "",
        content_hash_id: None,
        telegraph_url: None,
        notify_kind: 0,
        tag_next_attempt_at: now_unix() + 3,
    };
    let outcome = state.repo.upsert_bookmark(&new).await.map_err(to_request_error)?;
    let id = outcome.id;

    let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
        return Ok(());
    };
    let (text, markup) = build_detail_view(state, &bm, Scope::All, 0, lang).await;
    let sent = bot
        .send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview())
        .reply_markup(markup)
        .await?;

    // Record the message the worker should edit. If the worker already
    // finished (raced our insert→reply), re-render the final text now.
    let state_after = state
        .repo
        .set_bookmark_notify(id, sent.id.0 as i64)
        .await
        .map_err(to_request_error)?;
    if state_after == Some(1) {
        if let Ok(Some(bm)) = state.repo.get_bookmark(chat_id.0, id).await {
            let (text, markup) = build_detail_view(state, &bm, Scope::All, 0, lang).await;
            edit_page(bot, chat_id, sent.id, &text, markup).await?;
        }
    }
    Ok(())
}

pub async fn handle_bookmarks(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let (text, markup) = build_list_view(state, chat_id.0, Scope::All, 0, lang)
        .await
        .map_err(to_request_error)?;
    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview())
        .reply_markup(markup)
        .await?;
    Ok(())
}

pub async fn handle_bmsearch(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let query = payload.trim();
    if query.is_empty() || query.chars().count() > 100 {
        bot.send_message(chat_id, BMSEARCH_PROMPT)
            .reply_markup(force_reply("關鍵字"))
            .await?;
        return Ok(());
    }
    let hits = state
        .repo
        .search_bookmarks(chat_id.0, query, SEARCH_LIMIT)
        .await
        .map_err(to_request_error)?;
    if hits.is_empty() {
        bot.send_message(chat_id, lang.bm_search_empty()).await?;
        return Ok(());
    }
    let ids: Vec<i64> = hits.iter().map(|b| b.id).collect();
    let all_tags = state.repo.tags_for_bookmarks(&ids).await.unwrap_or_default();
    let rendered: Vec<RenderedBookmark> = hits
        .iter()
        .map(|b| RenderedBookmark {
            bookmark: b,
            tags: Box::leak(
                all_tags
                    .iter()
                    .filter(|t| t.bookmark_id == b.id)
                    .map(|t| t.tag.clone())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        })
        .collect();
    let text = render::render_list_page(&ListPageData {
        lang,
        total: hits.len() as i64,
        human_page: 1,
        total_pages: 1,
        items: &rendered,
    });
    bot.send_message(chat_id, text)
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview())
        .await?;
    Ok(())
}

pub async fn handle_bmnote(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let Some((id_str, text)) = payload.trim().split_once(char::is_whitespace) else {
        bot.send_message(chat_id, BMNOTE_PROMPT)
            .reply_markup(force_reply("123 這篇很適合之後研究"))
            .await?;
        return Ok(());
    };
    let Ok(id) = id_str.parse::<i64>() else {
        bot.send_message(chat_id, BMNOTE_PROMPT)
            .reply_markup(force_reply("123 這篇很適合之後研究"))
            .await?;
        return Ok(());
    };
    let note: String = text.trim().chars().take(NOTE_MAX_CHARS).collect();
    let ok = state
        .repo
        .set_bookmark_note(chat_id.0, id, &note)
        .await
        .map_err(to_request_error)?;
    bot.send_message(chat_id, if ok { lang.bm_note_saved() } else { lang.bm_not_found() })
        .await?;
    Ok(())
}

pub async fn handle_bmtag(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let mut parts = payload.split_whitespace();
    let Some(id) = parts.next().and_then(|s| s.parse::<i64>().ok()) else {
        bot.send_message(chat_id, BMTAG_PROMPT)
            .reply_markup(force_reply("123 AI 光通訊"))
            .await?;
        return Ok(());
    };
    let slugs: Vec<&str> = parts.filter_map(taxonomy::normalize).collect();
    let ok = state
        .repo
        .set_bookmark_tags_manual(chat_id.0, id, &slugs)
        .await
        .map_err(to_request_error)?;
    bot.send_message(chat_id, if ok { lang.bm_tag_saved() } else { lang.bm_not_found() })
        .await?;
    Ok(())
}

pub async fn handle_bmdel(bot: &Bot, msg: &Message, state: &BotState, payload: &str) -> ResponseResult<()> {
    let chat_id = msg.chat.id;
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let Ok(id) = payload.trim().parse::<i64>() else {
        bot.send_message(chat_id, BMDEL_PROMPT)
            .reply_markup(force_reply("123"))
            .await?;
        return Ok(());
    };
    let ok = state.repo.delete_bookmark(chat_id.0, id).await.map_err(to_request_error)?;
    bot.send_message(chat_id, if ok { lang.bm_deleted() } else { lang.bm_not_found() })
        .await?;
    Ok(())
}

// ─── Callback dispatch ───────────────────────────────────────────────────────

/// Dispatches `bm:*` callbacks. `rest` is the data after the `bm:` prefix.
/// Slice matching structurally avoids the prefix-ordering trap.
pub async fn handle_bm_callback(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    rest: &str,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let parts: Vec<&str> = rest.split(':').collect();
    match parts.as_slice() {
        ["add", hash] => handle_add(bot, query, state, hash, chat_id, message_id, lang).await,
        ["sum", hash] => handle_summary(bot, query, state, hash, chat_id, message_id, lang).await,
        ["list", scope, page] => {
            let (Some(scope), Ok(page)) = (Scope::parse(scope), page.parse::<usize>()) else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let (text, markup) = build_list_view(state, chat_id.0, scope, page, lang)
                .await
                .map_err(to_request_error)?;
            edit_page(bot, chat_id, message_id, &text, markup).await?;
            ack(bot, query).await
        }
        ["view", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), Scope::parse(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
                return toast(bot, query, lang.bm_not_found()).await;
            };
            let (text, markup) = build_detail_view(state, &bm, scope, page, lang).await;
            edit_page(bot, chat_id, message_id, &text, markup).await?;
            ack(bot, query).await
        }
        ["del", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), Scope::parse(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
                return toast(bot, query, lang.bm_not_found()).await;
            };
            if !bookmark_auth(bot, chat_id, query, bm.created_by).await {
                return toast(bot, query, lang.bm_no_permission()).await;
            }
            edit_page(
                bot,
                chat_id,
                message_id,
                lang.bm_delete_confirm_prompt(),
                delete_confirm_keyboard(id, scope, page, lang),
            )
            .await?;
            ack(bot, query).await
        }
        ["delok", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), Scope::parse(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
                return toast(bot, query, lang.bm_not_found()).await;
            };
            if !bookmark_auth(bot, chat_id, query, bm.created_by).await {
                return toast(bot, query, lang.bm_no_permission()).await;
            }
            state.repo.delete_bookmark(chat_id.0, id).await.map_err(to_request_error)?;
            toast(bot, query, lang.bm_deleted()).await?;
            let (text, markup) = build_list_view(state, chat_id.0, scope, page, lang)
                .await
                .map_err(to_request_error)?;
            edit_page(bot, chat_id, message_id, &text, markup).await
        }
        ["retag", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), Scope::parse(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
                return toast(bot, query, lang.bm_not_found()).await;
            };
            if !bookmark_auth(bot, chat_id, query, bm.created_by).await {
                return toast(bot, query, lang.bm_no_permission()).await;
            }
            let selected = tag_slugs_for(state, id).await;
            edit_page(
                bot,
                chat_id,
                message_id,
                lang.bm_toggle_hint(),
                retag_keyboard(id, &selected, scope, page, lang),
            )
            .await?;
            ack(bot, query).await
        }
        ["tt", id, idx, scope, page] => {
            let (Ok(id), Ok(idx), Some(scope), Ok(page)) = (
                id.parse::<i64>(),
                idx.parse::<usize>(),
                Scope::parse(scope),
                page.parse::<usize>(),
            ) else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(slug) = taxonomy::slug_of(idx) else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let Some(bm) = state.repo.get_bookmark(chat_id.0, id).await.map_err(to_request_error)? else {
                return toast(bot, query, lang.bm_not_found()).await;
            };
            if !bookmark_auth(bot, chat_id, query, bm.created_by).await {
                return toast(bot, query, lang.bm_no_permission()).await;
            }
            state.repo.toggle_bookmark_tag(chat_id.0, id, slug).await.map_err(to_request_error)?;
            let selected = tag_slugs_for(state, id).await;
            edit_page(
                bot,
                chat_id,
                message_id,
                lang.bm_toggle_hint(),
                retag_keyboard(id, &selected, scope, page, lang),
            )
            .await?;
            ack(bot, query).await
        }
        ["note", id, _scope, _page] => {
            let Ok(id) = id.parse::<i64>() else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            // Free text can't be avoided; point the user at /bmnote.
            toast(bot, query, &format!("/bmnote {id} …")).await
        }
        ["tags", page] => {
            let Ok(_page) = page.parse::<usize>() else {
                return toast(bot, query, lang.bm_bad_action()).await;
            };
            let counts = state.repo.tag_counts(chat_id.0).await.map_err(to_request_error)?;
            let untagged = state.repo.count_untagged(chat_id.0).await.map_err(to_request_error)?;
            edit_page(
                bot,
                chat_id,
                message_id,
                lang.bm_tag_index_header(),
                tags_index_keyboard(&counts, untagged, lang),
            )
            .await?;
            ack(bot, query).await
        }
        ["export"] => {
            export_bookmarks(bot, state, chat_id, lang).await?;
            ack(bot, query).await
        }
        _ => toast(bot, query, lang.bm_bad_action()).await,
    }
}

async fn handle_add(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    hash: &str,
    chat_id: ChatId,
    message_id: MessageId,
    lang: Lang,
) -> ResponseResult<()> {
    let user_id = query.from.id.0 as i64;
    if !user_allowed(state, user_id) {
        return toast(bot, query, lang.bm_no_permission()).await;
    }
    let Some(content) = state.repo.content_by_hash(hash).await.map_err(to_request_error)? else {
        // prune_contents may have removed it; be honest and leave the button.
        return toast(bot, query, lang.bm_expired()).await;
    };
    let raw_link = content.raw_link.clone().unwrap_or_default();
    let Ok(url) = normalize_url(&raw_link) else {
        return toast(bot, query, lang.bm_expired()).await;
    };
    let source_title = match content.source_id {
        Some(sid) => state
            .repo
            .get_source(sid)
            .await
            .ok()
            .flatten()
            .and_then(|s| s.title)
            .unwrap_or_default(),
        None => String::new(),
    };

    let new = NewBookmark {
        chat_id: chat_id.0,
        created_by: user_id,
        url: &url,
        title: content.title.as_deref().unwrap_or(""),
        note: "",
        source_title: &source_title,
        content_hash_id: Some(hash),
        telegraph_url: content.telegraph_url.as_deref(),
        notify_kind: 1,
        tag_next_attempt_at: now_unix() + 3,
    };
    let outcome = state.repo.upsert_bookmark(&new).await.map_err(to_request_error)?;
    let id = outcome.id;

    // The button lives on this pushed message; point the worker at it.
    state.repo.set_bookmark_notify(id, message_id.0 as i64).await.map_err(to_request_error)?;

    toast(bot, query, lang.bm_saved_toast()).await?;
    // Relabel to "saved → view", keeping the 📝 summary button if enabled. The
    // worker relabels again to the tags once tagging finishes. Correctness
    // never depends on the label; the DB row is the source of truth.
    let sum = summary_enabled(state, chat_id.0).await.then_some(hash);
    if let Some(markup) = item_keyboard(
        Some(BmBtn::Saved { id, label: lang.bm_saved_button().to_owned() }),
        sum,
    ) {
        let _ = bot.edit_message_reply_markup(chat_id, message_id).reply_markup(markup).await;
    }
    Ok(())
}

/// 📝 button: summarize the article via the MCP agent. Because a summary can be
/// slow (async agent turn), we answer the callback immediately and do the work
/// in a spawned task that replies when ready — never blocking the ~15s callback
/// budget.
async fn handle_summary(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    hash: &str,
    chat_id: ChatId,
    message_id: MessageId,
    lang: Lang,
) -> ResponseResult<()> {
    if !state.config.bookmark.ai.mcp.is_configured() {
        return toast(bot, query, lang.bm_summary_unavailable()).await;
    }
    if !user_allowed(state, query.from.id.0 as i64) {
        return toast(bot, query, lang.bm_no_permission()).await;
    }
    let Some(content) = state.repo.content_by_hash(hash).await.map_err(to_request_error)? else {
        return toast(bot, query, lang.bm_expired()).await;
    };
    let Some(url) = content.raw_link.filter(|s| !s.is_empty()) else {
        return toast(bot, query, lang.bm_expired()).await;
    };

    toast(bot, query, lang.bm_summarizing()).await?;

    // Reuse the fetcher's HTTP client (proxy/timeout policy) for the MCP calls.
    let client = McpClient::new(state.fetcher.client().clone(), state.config.bookmark.ai.mcp.clone());
    let bot = bot.clone();
    tokio::spawn(async move {
        let language = match lang {
            Lang::ZhTw => "Traditional Chinese",
            Lang::En => "English",
        };
        let prompt = format!(
            "Fetch the article at {url} and write a concise summary of about 5 short \
             sentences in {language}. Output only the summary text — no preamble, no markdown.",
        );
        let reply = match client.run(&prompt).await {
            Ok(text) if !text.trim().is_empty() => {
                let body: String = text.trim().chars().take(3500).collect();
                format!("{}\n\n{}", lang.bm_summary_heading(), escape(&body))
            }
            Ok(_) | Err(_) => lang.bm_summary_failed().to_owned(),
        };
        let _ = bot
            .send_message(chat_id, reply)
            .parse_mode(ParseMode::Html)
            .link_preview_options(no_preview())
            .reply_parameters(ReplyParameters::new(message_id))
            .await;
    });
    Ok(())
}

// ─── Settings submenu (delegated from callbacks.rs `settings:` handler) ──────

pub async fn handle_settings_bm(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    action: &str,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    match action {
        "bm" => {
            render_settings_bm(bot, state, chat_id, message_id, lang).await?;
            ack(bot, query).await
        }
        "bm:btn" => {
            toggle_option(state, &bm_btn_key(chat_id.0)).await.map_err(to_request_error)?;
            render_settings_bm(bot, state, chat_id, message_id, lang).await?;
            ack(bot, query).await
        }
        "bm:ai" => {
            toggle_option(state, &bm_ai_key(chat_id.0)).await.map_err(to_request_error)?;
            render_settings_bm(bot, state, chat_id, message_id, lang).await?;
            ack(bot, query).await
        }
        "bm:sum" => {
            toggle_option(state, &bm_sum_key(chat_id.0)).await.map_err(to_request_error)?;
            render_settings_bm(bot, state, chat_id, message_id, lang).await?;
            ack(bot, query).await
        }
        "bm:export" => {
            export_bookmarks(bot, state, chat_id, lang).await?;
            ack(bot, query).await
        }
        _ => toast(bot, query, lang.bm_bad_action()).await,
    }
}

async fn render_settings_bm(
    bot: &Bot,
    state: &BotState,
    chat_id: ChatId,
    message_id: MessageId,
    lang: Lang,
) -> ResponseResult<()> {
    let btn_on = option_on(state, &bm_btn_key(chat_id.0)).await;
    let ai_on = option_on(state, &bm_ai_key(chat_id.0)).await;
    let sum_on = if state.config.bookmark.ai.mcp.is_configured() {
        Some(option_on(state, &bm_sum_key(chat_id.0)).await)
    } else {
        None
    };
    let result = bot
        .edit_message_text(chat_id, message_id, lang.bm_settings_button())
        .reply_markup(settings_bm_keyboard(lang, btn_on, ai_on, sum_on))
        .await;
    if let Err(err) = result {
        if !is_benign_edit_error(&err) {
            return Err(err);
        }
    }
    Ok(())
}

async fn toggle_option(state: &BotState, key: &str) -> crate::db::DbResult<()> {
    let on = option_on(state, key).await;
    state.repo.set_option(key, if on { "0" } else { "1" }).await
}

async fn export_bookmarks(bot: &Bot, state: &BotState, chat_id: ChatId, lang: Lang) -> ResponseResult<()> {
    let items = state.repo.bookmarks_for_export(chat_id.0).await.map_err(to_request_error)?;
    if items.is_empty() {
        bot.send_message(chat_id, lang.bm_empty()).await?;
        return Ok(());
    }
    let ids: Vec<i64> = items.iter().map(|b| b.id).collect();
    let all_tags = state.repo.tags_for_bookmarks(&ids).await.unwrap_or_default();
    let rendered: Vec<RenderedBookmark> = items
        .iter()
        .map(|b| RenderedBookmark {
            bookmark: b,
            tags: Box::leak(
                all_tags
                    .iter()
                    .filter(|t| t.bookmark_id == b.id)
                    .map(|t| t.tag.clone())
                    .collect::<Vec<_>>()
                    .into_boxed_slice(),
            ),
        })
        .collect();
    let markdown = render::render_export_markdown(&rendered, lang);
    let file_name = format!("bookmarks_{}.md", now_unix());
    let document = InputFile::memory(markdown.into_bytes()).file_name(file_name);
    bot.send_document(chat_id, document).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_round_trips() {
        for scope in [Scope::All, Scope::Untagged, Scope::Tag(0), Scope::Tag(19)] {
            assert_eq!(Scope::parse(&scope.to_wire()), Some(scope));
        }
        assert_eq!(Scope::parse("t999"), None); // out of taxonomy range
        assert_eq!(Scope::parse("zzz"), None);
    }

    #[test]
    fn item_keyboard_combines_bookmark_and_summary_buttons() {
        // Both buttons → one row of two.
        let both = item_keyboard(Some(BmBtn::Add("abc".into())), Some("abc")).unwrap();
        assert_eq!(both.inline_keyboard[0].len(), 2);
        // Bookmark only.
        let one = item_keyboard(Some(BmBtn::Add("abc".into())), None).unwrap();
        assert_eq!(one.inline_keyboard[0].len(), 1);
        // Summary only.
        let sum = item_keyboard(None, Some("abc")).unwrap();
        assert_eq!(sum.inline_keyboard[0].len(), 1);
        // Neither → no keyboard at all.
        assert!(item_keyboard(None, None).is_none());
    }

    #[test]
    fn settings_summary_row_hidden_when_mcp_unconfigured() {
        let with = settings_bm_keyboard(Lang::ZhTw, true, true, Some(true));
        let without = settings_bm_keyboard(Lang::ZhTw, true, true, None);
        assert_eq!(with.inline_keyboard.len(), without.inline_keyboard.len() + 1);
    }

    #[test]
    fn all_worst_case_payloads_fit_64_ascii_bytes() {
        let big = i64::MAX; // 19 digits
        let idx = taxonomy::TAGS.len() - 1; // widest index
        let scope = Scope::Tag(idx);
        let page = 9999usize;
        let payloads = vec![
            cb::add("ffffffff"),
            cb::sum("ffffffff"),
            cb::list(scope, page),
            cb::view(big, scope, page),
            cb::del(big, scope, page),
            cb::delok(big, scope, page),
            cb::retag(big, scope, page),
            cb::note(big, scope, page),
            cb::tt(big, idx, scope, page),
            cb::tags(page),
            cb::export(),
        ];
        for p in payloads {
            assert!(p.len() <= 64, "payload too long ({}): {p}", p.len());
            assert!(p.is_ascii(), "payload not ascii: {p}");
            assert!(p.starts_with("bm:"), "missing namespace: {p}");
        }
    }

    #[test]
    fn pick_url_prefers_first_entity_and_handles_both_kinds() {
        let tl_url = reqwest::Url::parse("https://real.example/x").unwrap();
        // TextLink only.
        let only_textlink = [(MessageEntityKind::TextLink { url: tl_url.clone() }, "Visible Title")];
        assert_eq!(
            pick_url(only_textlink.iter().map(|(k, t)| (k, *t))),
            Some("https://real.example/x".to_owned())
        );
        // Plain Url only.
        let only_url = [(MessageEntityKind::Url, "https://plain.example/y")];
        assert_eq!(
            pick_url(only_url.iter().map(|(k, t)| (k, *t))),
            Some("https://plain.example/y".to_owned())
        );
        // Both: first by iteration order (offset) wins.
        let both = [
            (MessageEntityKind::Url, "https://first.example/a"),
            (MessageEntityKind::TextLink { url: tl_url }, "second"),
        ];
        assert_eq!(
            pick_url(both.iter().map(|(k, t)| (k, *t))),
            Some("https://first.example/a".to_owned())
        );
        // Neither.
        let none: Vec<(MessageEntityKind, &str)> = vec![(MessageEntityKind::Bold, "x")];
        assert_eq!(pick_url(none.iter().map(|(k, t)| (k, *t))), None);
    }

    #[test]
    fn naive_scan_avoids_trailing_paren() {
        // The entity path handles the classic trailing-`)` bug; the naive path
        // stops at whitespace.
        assert_eq!(
            naive_url_scan("see https://x.test/a and more"),
            Some("https://x.test/a".to_owned())
        );
        assert_eq!(naive_url_scan("no url here"), None);
    }

    #[test]
    fn benign_edit_errors_classified() {
        assert!(is_benign_edit_error(&RequestError::Api(ApiError::MessageNotModified)));
        assert!(is_benign_edit_error(&RequestError::Api(ApiError::MessageToEditNotFound)));
        assert!(!is_benign_edit_error(&RequestError::Api(ApiError::ChatNotFound)));
    }
}
