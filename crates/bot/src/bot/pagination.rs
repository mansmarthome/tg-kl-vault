//! Pure pagination math + a nav-row builder. No async, no `Bot` — fully unit
//! testable, which matters in a codebase where runtime.rs/callbacks.rs/
//! keyboard.rs have no tests.

use teloxide::types::{InlineKeyboardButton, InlineKeyboardButtonKind};

use super::i18n::Lang;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Page {
    pub index: usize,
    pub per_page: usize,
    pub total: usize,
}

impl Page {
    /// Clamps a requested page index to a real page. Clamping **at read time**
    /// is the one rule that kills every stale-page problem at once: a stale
    /// `bm:list:a:47` lands on the last real page; deleting the last item on
    /// the last page falls back a page; `total == 0` renders an empty state.
    pub fn clamped(requested: usize, per_page: usize, total: usize) -> Self {
        let per_page = per_page.max(1);
        let last = Self::total_pages_for(per_page, total).saturating_sub(1);
        Self { index: requested.min(last), per_page, total }
    }

    fn total_pages_for(per_page: usize, total: usize) -> usize {
        if total == 0 {
            1
        } else {
            total.div_ceil(per_page)
        }
    }

    pub fn total_pages(&self) -> usize {
        Self::total_pages_for(self.per_page, self.total)
    }

    pub fn offset(&self) -> i64 {
        (self.index * self.per_page) as i64
    }

    pub fn limit(&self) -> i64 {
        self.per_page as i64
    }

    pub fn has_prev(&self) -> bool {
        self.index > 0
    }

    pub fn has_next(&self) -> bool {
        self.index + 1 < self.total_pages()
    }

    /// 1-based page number for display.
    pub fn human_index(&self) -> usize {
        self.index + 1
    }
}

/// Builds the prev/next row, rendering only arrows that lead somewhere. `cb`
/// maps a target page index to its callback_data.
pub fn nav_row(
    page: &Page,
    cb: impl Fn(usize) -> String,
    lang: Lang,
) -> Vec<InlineKeyboardButton> {
    let mut row = Vec::new();
    if page.has_prev() {
        row.push(InlineKeyboardButton::new(
            lang.bm_prev(),
            InlineKeyboardButtonKind::CallbackData(cb(page.index - 1)),
        ));
    }
    if page.has_next() {
        row.push(InlineKeyboardButton::new(
            lang.bm_next(),
            InlineKeyboardButtonKind::CallbackData(cb(page.index + 1)),
        ));
    }
    row
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clamps_stale_page_to_last() {
        let p = Page::clamped(47, 5, 34); // 34 items, 5/page => 7 pages (0..6)
        assert_eq!(p.index, 6);
        assert_eq!(p.total_pages(), 7);
        assert_eq!(p.human_index(), 7);
        assert!(p.has_prev());
        assert!(!p.has_next());
    }

    #[test]
    fn empty_total_is_single_empty_page() {
        let p = Page::clamped(3, 5, 0);
        assert_eq!(p.index, 0);
        assert_eq!(p.total_pages(), 1);
        assert!(!p.has_prev());
        assert!(!p.has_next());
    }

    #[test]
    fn offset_and_limit() {
        let p = Page::clamped(2, 5, 34);
        assert_eq!(p.offset(), 10);
        assert_eq!(p.limit(), 5);
    }

    #[test]
    fn nav_row_only_shows_reachable_arrows() {
        let lang = Lang::ZhTw;
        let first = Page::clamped(0, 5, 34);
        assert_eq!(nav_row(&first, |i| format!("p{i}"), lang).len(), 1); // next only
        let mid = Page::clamped(3, 5, 34);
        assert_eq!(nav_row(&mid, |i| format!("p{i}"), lang).len(), 2); // both
        let only = Page::clamped(0, 5, 3);
        assert_eq!(nav_row(&only, |i| format!("p{i}"), lang).len(), 0); // single page
    }
}
