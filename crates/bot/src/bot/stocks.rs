//! Telegram handlers for the stock feature. Thin: parse args → `StockService`
//! → `stock::render` → send. The service is the single source of truth; these
//! functions never touch the DB or a data source directly.
//!
//! Callback data uses the `stk:` colon namespace built by [`cb`] (never inlined
//! at call sites, so the byte-budget test can't be bypassed). The `stk:` branch
//! in `callbacks.rs` sits before the legacy binary decoder, which would happily
//! mangle a `stk:` string.

use teloxide::{
    prelude::*,
    types::{InlineKeyboardButton, InlineKeyboardMarkup, MessageId, ParseMode},
    ApiError, RequestError,
};
use tracing::warn;

use crate::bot::i18n::Lang;
use crate::bot::runtime::{
    chat_lang, no_preview, now_unix, send_force_reply_prompt, to_request_error, BotState,
};
use crate::stock::render::{render_quote_card, render_watchlist};
use crate::stock::{
    classify_session, manual_report_day, market_date_string, parse, render_report_chunks, AddError,
    Market, MarketScope, Parsed, StockService, WatchlistPage, YahooSource,
};

/// The concrete service type shared by the bot and the worker.
pub type StockSvc = StockService<YahooSource>;

/// Wraps a service `anyhow::Error` into a teloxide `RequestError` (the service
/// returns `anyhow`, which — unlike the DB layer's `libsql::Error` — doesn't
/// implement `std::error::Error`, so `runtime::to_request_error` can't take it).
fn req_err(err: anyhow::Error) -> RequestError {
    RequestError::Io(std::sync::Arc::new(std::io::Error::other(err.to_string())))
}

// ─── Callback data builders (never inline `format!` at call sites) ────────────

pub mod cb {
    use crate::stock::MarketScope;

    pub fn list(scope: MarketScope, page: usize) -> String {
        format!("stk:list:{}:{page}", scope.as_wire())
    }
    pub fn del(id: i64, scope: MarketScope, page: usize) -> String {
        format!("stk:del:{id}:{}:{page}", scope.as_wire())
    }
    pub fn delok(id: i64, scope: MarketScope, page: usize) -> String {
        format!("stk:delok:{id}:{}:{page}", scope.as_wire())
    }
    pub fn qadd(symbol: &str) -> String {
        format!("stk:qadd:{symbol}")
    }
    pub fn qai(symbol: &str) -> String {
        format!("stk:qai:{symbol}")
    }
    pub fn ptoggle(market: crate::stock::Market) -> String {
        format!("stk:ptoggle:{}", market.as_wire())
    }
    pub fn ptime(market: crate::stock::Market) -> String {
        format!("stk:ptime:{}", market.as_wire())
    }
}

// ─── Keyboards ───────────────────────────────────────────────────────────────

fn quote_card_keyboard(symbol: &str, ai_available: bool, lang: Lang) -> InlineKeyboardMarkup {
    let mut row = vec![InlineKeyboardButton::callback(lang.stk_add_button(), cb::qadd(symbol))];
    if ai_available {
        row.push(InlineKeyboardButton::callback(lang.stk_ai_button(), cb::qai(symbol)));
    }
    InlineKeyboardMarkup::new(vec![row])
}

fn watchlist_keyboard(page: &WatchlistPage, lang: Lang) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    // One 🗑-per-id row: tapping an id opens the remove confirmation.
    let id_row: Vec<InlineKeyboardButton> = page
        .items
        .iter()
        .map(|w| {
            InlineKeyboardButton::callback(
                format!("🗑 {}", w.id),
                cb::del(w.id, page.scope, page.page_index),
            )
        })
        .collect();
    if !id_row.is_empty() {
        rows.push(id_row);
    }
    // Scope filter row.
    rows.push(
        [MarketScope::All, MarketScope::Tw, MarketScope::Us]
            .into_iter()
            .map(|s| {
                let label = if s == page.scope {
                    format!("• {}", lang.stk_scope_name(s))
                } else {
                    lang.stk_scope_name(s).to_owned()
                };
                InlineKeyboardButton::callback(label, cb::list(s, 0))
            })
            .collect(),
    );
    // Pagination row.
    let pages = page.total.div_ceil(page.per_page).max(1);
    let mut nav = Vec::new();
    if page.page_index > 0 {
        nav.push(InlineKeyboardButton::callback(
            lang.bm_prev(),
            cb::list(page.scope, page.page_index - 1),
        ));
    }
    if page.page_index + 1 < pages {
        nav.push(InlineKeyboardButton::callback(
            lang.bm_next(),
            cb::list(page.scope, page.page_index + 1),
        ));
    }
    if !nav.is_empty() {
        rows.push(nav);
    }
    InlineKeyboardMarkup::new(rows)
}

fn delete_confirm_keyboard(id: i64, scope: MarketScope, page: usize, lang: Lang) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback(lang.stk_confirm_delete(), cb::delok(id, scope, page)),
        InlineKeyboardButton::callback(lang.prompt_cancel_button(), cb::list(scope, page)),
    ]])
}

/// The close-push settings panel (title + keyboard), shared by `/stockpush`
/// with no args and the `settings:stk` submenu.
pub async fn push_panel(
    state: &BotState,
    chat_id: i64,
    lang: Lang,
) -> anyhow::Result<(String, InlineKeyboardMarkup)> {
    let settings = state.stock.push_settings(chat_id).await?;
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    for s in &settings {
        let market = Market::from_wire(&s.market).unwrap_or(Market::Tw);
        let time = match s.push_minute {
            Some(m) => format!("{:02}:{:02}", m / 60, m % 60),
            None => lang.stk_push_time_default().to_owned(),
        };
        rows.push(vec![
            InlineKeyboardButton::callback(
                lang.stk_push_toggle(market, s.enabled != 0),
                cb::ptoggle(market),
            ),
            InlineKeyboardButton::callback(
                lang.stk_push_time_button(market, &time),
                cb::ptime(market),
            ),
        ]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        lang.settings_back_button(),
        "settings:back",
    )]);
    Ok((lang.stk_push_title().to_owned(), InlineKeyboardMarkup::new(rows)))
}

// ─── Command handlers ────────────────────────────────────────────────────────

fn actor(msg: &Message) -> i64 {
    msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(msg.chat.id.0)
}

/// `/stock <symbol>` — quote card with indicators. Does not touch the list.
pub async fn handle_stock(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    if payload.is_empty() {
        return send_force_reply_prompt(bot, msg.chat.id, lang, lang.stk_prompt(), lang.stk_placeholder()).await;
    }
    let sym = match state.stock.resolve(payload).await {
        Ok(sym) => sym,
        Err(err) => {
            let text = if matches!(err, crate::stock::SourceError::NotFound) {
                lang.stk_unknown_symbol()
            } else {
                lang.stk_upstream()
            };
            bot.send_message(msg.chat.id, text).await?;
            return Ok(());
        }
    };
    let view = match state.stock.snapshot(&sym, now_unix()).await {
        Ok(view) => view,
        Err(err) => {
            warn!(symbol = %sym.canonical, error = %err, "stock snapshot failed");
            bot.send_message(msg.chat.id, lang.stk_upstream()).await?;
            return Ok(());
        }
    };
    let ai_available = state.config.bookmark.ai.mcp.is_configured();
    bot.send_message(msg.chat.id, render_quote_card(&view, lang))
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview())
        .reply_markup(quote_card_keyboard(&sym.canonical, ai_available, lang))
        .await?;
    Ok(())
}

/// `/stocks` — the paged watchlist.
pub async fn handle_stocks(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    let page = state
        .stock
        .list_page(msg.chat.id.0, MarketScope::All, 0)
        .await
        .map_err(req_err)?;
    send_watchlist(bot, msg.chat.id, &page, lang).await
}

async fn send_watchlist(
    bot: &Bot,
    chat_id: ChatId,
    page: &WatchlistPage,
    lang: Lang,
) -> ResponseResult<()> {
    let mut send = bot
        .send_message(chat_id, render_watchlist(page, lang))
        .parse_mode(ParseMode::Html)
        .link_preview_options(no_preview());
    if page.total > 0 {
        send = send.reply_markup(watchlist_keyboard(page, lang));
    }
    send.await?;
    Ok(())
}

/// `/stockadd <symbol>` — add to the watchlist, four distinct outcomes.
pub async fn handle_stockadd(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    if payload.is_empty() {
        return send_force_reply_prompt(bot, msg.chat.id, lang, lang.stk_add_prompt(), lang.stk_placeholder()).await;
    }
    let reply = add_reply(state, msg.chat.id.0, actor(msg), payload, lang).await;
    bot.send_message(msg.chat.id, reply).await?;
    Ok(())
}

async fn add_reply(state: &BotState, chat_id: i64, by: i64, raw: &str, lang: Lang) -> String {
    match state.stock.add(chat_id, by, raw).await {
        Ok(o) if o.existed => lang.stk_already().to_owned(),
        Ok(_) => lang.stk_added().to_owned(),
        Err(AddError::NotFound) => lang.stk_unknown_symbol().to_owned(),
        Err(AddError::LimitReachedChat(m)) => lang.stk_limit_chat(m),
        Err(AddError::LimitReachedGlobal(m)) => lang.stk_limit_global(m),
        Err(AddError::Upstream) => lang.stk_upstream().to_owned(),
    }
}

/// `/stockdel <id|symbol>` — remove from the watchlist (creator/admin only).
pub async fn handle_stockdel(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    if payload.is_empty() {
        return send_force_reply_prompt(bot, msg.chat.id, lang, lang.stk_del_prompt(), lang.stk_placeholder()).await;
    }
    let chat_id = msg.chat.id.0;
    let target = resolve_del_target(state, chat_id, payload).await.map_err(req_err)?;
    let Some(item) = target else {
        bot.send_message(msg.chat.id, lang.stk_not_found()).await?;
        return Ok(());
    };
    if !command_auth(bot, msg.chat.id, actor(msg), item.created_by).await {
        bot.send_message(msg.chat.id, lang.stk_no_permission()).await?;
        return Ok(());
    }
    state.stock.remove(chat_id, item.id).await.map_err(req_err)?;
    bot.send_message(msg.chat.id, lang.stk_removed()).await?;
    Ok(())
}

/// Finds the watch row for a `/stockdel` argument: a bare number is an id,
/// otherwise it is a symbol matched without any upstream probe (we only look at
/// the shapes the chat could plausibly hold).
async fn resolve_del_target(
    state: &BotState,
    chat_id: i64,
    raw: &str,
) -> anyhow::Result<Option<crate::db::models::WatchItem>> {
    if let Ok(id) = raw.trim().parse::<i64>() {
        return state.stock.get_watch(chat_id, id).await;
    }
    let candidates = match parse(raw) {
        Parsed::Resolved(sym) => vec![sym.canonical],
        Parsed::TaiwanAmbiguous { local_code } => {
            vec![format!("{local_code}.TW"), format!("{local_code}.TWO")]
        }
        Parsed::Invalid(_) => vec![],
    };
    for canonical in candidates {
        if let Some(item) = state.stock.get_watch_by_symbol(chat_id, &canonical).await? {
            return Ok(Some(item));
        }
    }
    Ok(None)
}

/// `/stockpush [tw|us] [HH:MM|off]` — no args shows the panel.
pub async fn handle_stockpush(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    let chat_id = msg.chat.id.0;
    if payload.is_empty() {
        let (text, markup) = push_panel(state, chat_id, lang).await.map_err(req_err)?;
        bot.send_message(msg.chat.id, text)
            .parse_mode(ParseMode::Html)
            .reply_markup(markup)
            .await?;
        return Ok(());
    }
    let mut it = payload.split_whitespace();
    let (Some(market_tok), Some(value_tok)) = (it.next(), it.next()) else {
        bot.send_message(msg.chat.id, "用法：/stockpush [tw|us] [HH:MM|off]").await?;
        return Ok(());
    };
    let Some(market) = Market::from_wire(&market_tok.to_ascii_lowercase()) else {
        bot.send_message(msg.chat.id, "用法：/stockpush [tw|us] [HH:MM|off]").await?;
        return Ok(());
    };
    let text = match parse_push_value(value_tok) {
        Some((enabled, minute)) => {
            state.stock.set_push(chat_id, market, enabled, minute).await.map_err(req_err)?;
            lang.stk_push_saved()
        }
        None => lang.stk_push_bad_time(),
    };
    bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

/// Parses the `/stockpush` value: `off` / `on` / `HH:MM`. Returns
/// `(enabled, push_minute)`.
fn parse_push_value(value: &str) -> Option<(bool, Option<i64>)> {
    match value.to_ascii_lowercase().as_str() {
        "off" => Some((false, None)),
        "on" => Some((true, None)),
        other => parse_hhmm(other).map(|m| (true, Some(m))),
    }
}

fn parse_hhmm(s: &str) -> Option<i64> {
    let (h, m) = s.split_once(':')?;
    let h: i64 = h.parse().ok()?;
    let m: i64 = m.parse().ok()?;
    if (0..24).contains(&h) && (0..60).contains(&m) {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// `/stockreport` — produce today's close report now (shares the ledger with
/// the worker, so a manual run suppresses that evening's automatic push).
pub async fn handle_stockreport(bot: &Bot, msg: &Message, state: &BotState) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    let chat_id = msg.chat.id.0;
    let now = now_unix();
    let repo = state.stock.repo();

    bot.send_message(msg.chat.id, lang.stk_report_working()).await?;

    let mut produced_any = false;
    for market in [Market::Tw, Market::Us] {
        let scope = if market == Market::Tw { MarketScope::Tw } else { MarketScope::Us };
        let page = state.stock.list_page(chat_id, scope, 0).await.map_err(req_err)?;
        if page.total == 0 {
            continue;
        }
        produced_any = true;

        let Some(probe) = probe_symbol(state, market) else {
            continue;
        };
        let meta = match state.stock.fetch_session_meta(&probe).await {
            Ok(meta) => meta,
            Err(err) => {
                warn!(?market, error = %err, "stockreport probe failed");
                bot.send_message(msg.chat.id, lang.stk_upstream()).await?;
                continue;
            }
        };
        let Some(trading_day) = manual_report_day(classify_session(now, meta)) else {
            bot.send_message(msg.chat.id, lang.stk_report_not_closed(market)).await?;
            continue;
        };
        let trade_date = market_date_string(trading_day);

        // Share the worker's ledger: claim once (at-most-once), which also
        // suppresses tonight's automatic push for the same trading day.
        if !repo
            .claim_report(chat_id, market.as_wire(), &trade_date, now, 1800, 3)
            .await
            .map_err(to_request_error)?
        {
            bot.send_message(msg.chat.id, lang.stk_report_already()).await?;
            continue;
        }
        let entries = state
            .stock
            .report_entries(chat_id, market, &trade_date, now)
            .await
            .map_err(req_err)?;
        for chunk in render_report_chunks(market, &trade_date, &entries, false, lang) {
            bot.send_message(msg.chat.id, chunk)
                .parse_mode(ParseMode::Html)
                .link_preview_options(no_preview())
                .await?;
        }
        repo.mark_report_sent(chat_id, market.as_wire(), &trade_date).await.map_err(to_request_error)?;
    }

    if !produced_any {
        bot.send_message(msg.chat.id, lang.stk_empty()).await?;
    }
    Ok(())
}

/// Builds the probe `Symbol` for a market from config (no network).
fn probe_symbol(state: &BotState, market: Market) -> Option<crate::stock::Symbol> {
    let raw = match market {
        Market::Tw => &state.config.stock.tw_probe_symbol,
        Market::Us => &state.config.stock.us_probe_symbol,
    };
    match parse(raw) {
        Parsed::Resolved(sym) => Some(sym),
        _ => None,
    }
}

/// Command-context authorization for destructive ops: the actor must be the
/// creator, the private-chat owner, or a group admin. Mirrors `bookmark_auth`.
async fn command_auth(bot: &Bot, chat_id: ChatId, actor: i64, created_by: i64) -> bool {
    if chat_id.0 == actor || actor == created_by {
        return true;
    }
    match bot.get_chat_member(chat_id, teloxide::types::UserId(actor as u64)).await {
        Ok(member) => member.is_privileged(),
        Err(_) => false,
    }
}

// ─── Callback dispatch (`stk:` namespace) ────────────────────────────────────

async fn toast(bot: &Bot, query: &CallbackQuery, text: &str) -> ResponseResult<()> {
    bot.answer_callback_query(query.id.clone()).text(text).await?;
    Ok(())
}

async fn ack(bot: &Bot, query: &CallbackQuery) -> ResponseResult<()> {
    bot.answer_callback_query(query.id.clone()).await?;
    Ok(())
}

/// Entry from `callbacks.rs` for `stk:` callback data.
pub async fn handle_stock_callback(
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
        ["list", scope, page] => {
            let (Some(scope), Ok(page)) = (MarketScope::from_wire(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.stk_bad_action()).await;
            };
            let view = state
                .stock
                .list_page(chat_id.0, scope, page)
                .await
                .map_err(req_err)?;
            edit_watchlist(bot, chat_id, message_id, &view, lang).await?;
            ack(bot, query).await
        }
        ["del", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), MarketScope::from_wire(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.stk_bad_action()).await;
            };
            let Some(item) = state.stock.get_watch(chat_id.0, id).await.map_err(req_err)?
            else {
                return toast(bot, query, lang.stk_not_found()).await;
            };
            if !callback_auth(bot, chat_id, query, item.created_by).await {
                return toast(bot, query, lang.stk_no_permission()).await;
            }
            edit_text(bot, chat_id, message_id, lang.stk_delete_confirm(), delete_confirm_keyboard(id, scope, page, lang)).await?;
            ack(bot, query).await
        }
        ["delok", id, scope, page] => {
            let (Ok(id), Some(scope), Ok(page)) =
                (id.parse::<i64>(), MarketScope::from_wire(scope), page.parse::<usize>())
            else {
                return toast(bot, query, lang.stk_bad_action()).await;
            };
            let Some(item) = state.stock.get_watch(chat_id.0, id).await.map_err(req_err)?
            else {
                return toast(bot, query, lang.stk_not_found()).await;
            };
            if !callback_auth(bot, chat_id, query, item.created_by).await {
                return toast(bot, query, lang.stk_no_permission()).await;
            }
            state.stock.remove(chat_id.0, id).await.map_err(req_err)?;
            let view = state
                .stock
                .list_page(chat_id.0, scope, page)
                .await
                .map_err(req_err)?;
            edit_watchlist(bot, chat_id, message_id, &view, lang).await?;
            toast(bot, query, lang.stk_removed()).await
        }
        ["qadd", symbol] => {
            let by = query.from.id.0 as i64;
            let reply = add_reply(state, chat_id.0, by, symbol, lang).await;
            toast(bot, query, &reply).await
        }
        ["qai", symbol] => handle_ai(bot, query, state, symbol, chat_id, message_id, lang).await,
        ["ptoggle", market] => {
            let Some(market) = Market::from_wire(market) else {
                return toast(bot, query, lang.stk_bad_action()).await;
            };
            let [tw, us] = state.stock.push_settings(chat_id.0).await.map_err(req_err)?;
            let current = if market == Market::Tw { &tw } else { &us };
            state
                .stock
                .set_push(chat_id.0, market, current.enabled == 0, current.push_minute)
                .await
                .map_err(req_err)?;
            let (text, markup) = push_panel(state, chat_id.0, lang).await.map_err(req_err)?;
            edit_text(bot, chat_id, message_id, &text, markup).await?;
            ack(bot, query).await
        }
        ["ptime", market] => {
            let Some(market) = Market::from_wire(market) else {
                return toast(bot, query, lang.stk_bad_action()).await;
            };
            let prompt = match market {
                Market::Tw => lang.stk_push_time_tw_prompt(),
                Market::Us => lang.stk_push_time_us_prompt(),
            };
            ack(bot, query).await?;
            send_force_reply_prompt(bot, chat_id, lang, prompt, lang.stk_push_time_placeholder()).await
        }
        _ => toast(bot, query, lang.stk_bad_action()).await,
    }
}

/// 🤖 button: AI commentary for one symbol. Like the bookmark 📝 handler, we
/// answer the callback immediately and do the slow agent turn in a spawned task
/// that replies when ready — never blocking the ~15s callback budget.
async fn handle_ai(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    symbol: &str,
    chat_id: ChatId,
    message_id: MessageId,
    lang: Lang,
) -> ResponseResult<()> {
    if !state.config.bookmark.ai.mcp.is_configured() {
        return toast(bot, query, lang.stk_ai_unavailable()).await;
    }
    let Parsed::Resolved(sym) = parse(symbol) else {
        return toast(bot, query, lang.stk_bad_action()).await;
    };

    toast(bot, query, lang.stk_ai_working()).await?;

    let client = crate::tagging::mcp::McpClient::new(
        state.fetcher.client().clone(),
        state.config.bookmark.ai.mcp.clone(),
    );
    let stock = state.stock.clone();
    let now = now_unix();
    let bot = bot.clone();
    tokio::spawn(async move {
        use crate::stock::commentary::{build_single_prompt, sanitize, SymbolBrief};
        let reply = match stock.snapshot(&sym, now).await {
            Ok(view) => {
                let name = view
                    .meta
                    .as_ref()
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| sym.local_code.clone());
                let change_pct = match (view.snapshot.last_close, view.snapshot.prev_close) {
                    (Some(l), Some(p)) if p != 0.0 => Some((l - p) / p * 100.0),
                    _ => None,
                };
                let brief = SymbolBrief {
                    code: &sym.local_code,
                    name: &name,
                    close: view.snapshot.last_close,
                    change_pct,
                    signals: &view.signals,
                };
                match client.run(&build_single_prompt(&brief, lang)).await {
                    Ok(text) if !text.trim().is_empty() => {
                        let body = sanitize(&text);
                        format!(
                            "{}\n\n{}\n\n{}",
                            lang.stk_ai_heading(),
                            teloxide::utils::html::escape(&body),
                            lang.stk_ai_disclaimer()
                        )
                    }
                    _ => lang.stk_ai_failed().to_owned(),
                }
            }
            Err(_) => lang.stk_ai_failed().to_owned(),
        };
        let _ = bot
            .send_message(chat_id, reply)
            .parse_mode(ParseMode::Html)
            .link_preview_options(no_preview())
            .reply_parameters(teloxide::types::ReplyParameters::new(message_id))
            .await;
    });
    Ok(())
}

/// Handles a completed push-time ForceReply (from `runtime.rs`).
pub async fn handle_push_time_reply(
    bot: &Bot,
    msg: &Message,
    state: &BotState,
    market: Market,
    payload: &str,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, msg.chat.id.0).await;
    let text = match parse_push_value(payload) {
        Some((enabled, minute)) => {
            state.stock.set_push(msg.chat.id.0, market, enabled, minute).await.map_err(req_err)?;
            lang.stk_push_saved()
        }
        None => lang.stk_push_bad_time(),
    };
    bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

/// `settings:stk` submenu entry — edits the settings message into the push panel.
pub async fn handle_settings_stk(
    bot: &Bot,
    query: &CallbackQuery,
    state: &BotState,
    chat_id: ChatId,
    message_id: MessageId,
) -> ResponseResult<()> {
    let lang = chat_lang(&state.repo, chat_id.0).await;
    let (text, markup) = push_panel(state, chat_id.0, lang).await.map_err(req_err)?;
    edit_text(bot, chat_id, message_id, &text, markup).await?;
    ack(bot, query).await
}

async fn edit_watchlist(
    bot: &Bot,
    chat_id: ChatId,
    message_id: MessageId,
    page: &WatchlistPage,
    lang: Lang,
) -> ResponseResult<()> {
    let markup = if page.total > 0 {
        watchlist_keyboard(page, lang)
    } else {
        InlineKeyboardMarkup::new(Vec::<Vec<InlineKeyboardButton>>::new())
    };
    edit_text(bot, chat_id, message_id, &render_watchlist(page, lang), markup).await
}

async fn edit_text(
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
        let benign = matches!(
            err,
            RequestError::Api(ApiError::MessageNotModified)
                | RequestError::Api(ApiError::MessageToEditNotFound)
                | RequestError::Api(ApiError::MessageCantBeEdited)
        );
        if !benign {
            return Err(err);
        }
    }
    Ok(())
}

async fn callback_auth(bot: &Bot, chat_id: ChatId, query: &CallbackQuery, created_by: i64) -> bool {
    let user_id = query.from.id.0 as i64;
    if chat_id.0 == user_id || user_id == created_by {
        return true;
    }
    match bot.get_chat_member(chat_id, query.from.id).await {
        Ok(member) => member.is_privileged(),
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::stock::{Board, Symbol};

    fn card_symbol() -> Symbol {
        Symbol {
            canonical: "2330.TW".into(),
            market: Market::Tw,
            board: Board::Twse,
            local_code: "2330".into(),
        }
    }

    #[test]
    fn all_worst_case_stock_payloads_fit_64_ascii_bytes() {
        // Symbols are whitelisted to <=12 bytes, ids fit in i64; assert every
        // builder's worst case is short, ASCII, and stk:-prefixed.
        let sym = "123456.TWO"; // longest plausible canonical
        let payloads = vec![
            cb::list(MarketScope::All, 999),
            cb::del(i64::MAX, MarketScope::Us, 999),
            cb::delok(i64::MAX, MarketScope::Tw, 999),
            cb::qadd(sym),
            cb::ptoggle(Market::Tw),
            cb::ptime(Market::Us),
        ];
        for p in payloads {
            assert!(p.len() <= 64, "callback too long: {p} ({} bytes)", p.len());
            assert!(p.is_ascii(), "non-ascii callback: {p}");
            assert!(p.starts_with("stk:"), "missing namespace: {p}");
        }
    }

    #[test]
    fn push_value_parses_off_on_and_time() {
        assert_eq!(parse_push_value("off"), Some((false, None)));
        assert_eq!(parse_push_value("on"), Some((true, None)));
        assert_eq!(parse_push_value("14:00"), Some((true, Some(840))));
        assert_eq!(parse_push_value("09:30"), Some((true, Some(570))));
        assert_eq!(parse_push_value("25:00"), None);
        assert_eq!(parse_push_value("nonsense"), None);
    }

    #[test]
    fn quote_card_keyboard_uses_the_add_namespace() {
        let kb = quote_card_keyboard(&card_symbol().canonical, false, Lang::ZhTw);
        let data = format!("{:?}", kb);
        assert!(data.contains("stk:qadd:2330.TW"));
        assert!(!data.contains("stk:qai"), "no AI button when unconfigured");
        // With AI available the 🤖 button appears.
        let kb2 = quote_card_keyboard(&card_symbol().canonical, true, Lang::ZhTw);
        assert!(format!("{kb2:?}").contains("stk:qai:2330.TW"));
    }
}
