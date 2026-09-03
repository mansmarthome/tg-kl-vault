//! RSStT-inspired HTML rewriter that converts a feed item's HTML description
//! into a Telegram-flavoured HTML string safe to ship with `ParseMode::Html`.
//!
//! Two public entry points:
//!
//! - [`format_description_html`] — walk arbitrary RSS/Atom HTML and emit
//!   Telegram HTML for the supported subset (bold/italic/links/lists/etc.).
//! - [`compose_feed_message`] — assemble the full feed post (header lines +
//!   formatted description) into a single Telegram HTML string ready to send.

use scraper::{Html, element_ref::ElementRef, node::Node};
use teloxide::utils::html as teloxide_html;

/// Maximum length, in characters, of a Bot API `sendMessage` text payload.
/// Telegram counts the *parsed* message (i.e. plain text), not the HTML source.
pub const MESSAGE_TEXT_LIMIT: usize = 4096;

/// Convert a feed item's HTML description into a Telegram HTML string.
///
/// `base_url` is used to resolve relative `href`/`src` attributes (best
/// effort: feeds rarely ship truly relative URLs, but we honour them when
/// given a base). Pass `None` if the feed always provides absolute URLs.
pub fn format_description_html(html: &str, base_url: Option<&str>) -> String {
    let doc = Html::parse_fragment(html);
    let mut out = String::new();
    walk(doc.tree.root(), base_url, &mut out);
    let mut out = insert_inline_spaces(&out);
    // Trim a single trailing newline; if the description ended with a block
    // element (which already emitted a blank line) keep one newline.
    if out.ends_with("\n\n") {
        out.truncate(out.len() - 1);
    } else {
        out.push('\n');
    }
    out
}

/// Insert a single space at every boundary between adjacent inline tokens
/// (e.g. `</b>is` → `</b> is`, and `a<a` → `a <a`). This makes the rendered
/// text readable when feed authors write `<b>x</b>is` or `a<a href="...">link</a>`
/// without whitespace separators. The pass leaves existing whitespace
/// alone.
fn insert_inline_spaces(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 16);
    let chars: Vec<char> = s.chars().collect();
    let len = chars.len();
    let mut i = 0usize;
    while i < len {
        let c = chars[i];
        out.push(c);
        // After a closing tag `</tag>`, if the next char is non-space and not
        // `<` or `>`, insert a space.
        if c == '>'
            && i >= 2
            && chars[i - 1] != '>'
            && chars[i.saturating_sub(1)] != '/'
        {
            // Detect "closing tag": walk back to the matching `<`; if the
            // next char after `<` is `/`, we're closing.
            let mut is_closing = false;
            let mut j = i - 1;
            while j > 0 {
                if chars[j] == '<' {
                    if j + 1 < len && chars[j + 1] == '/' {
                        is_closing = true;
                    }
                    break;
                }
                j -= 1;
            }
            if is_closing {
                if let Some(&next) = chars.get(i + 1) {
                    // Only insert a space when the next char is a word char
                    // (letter/digit/underscore); punctuation like `.` `,`
                    // `!` `?` should sit flush against the tag.
                    if next.is_alphanumeric() || next == '_' {
                        out.push(' ');
                    }
                }
            }
        }
        // Before an opening tag, if the previous char is non-space and not
        // `<` or `>`, insert a space.
        if c == '<'
            && i + 1 < len
            && chars[i + 1] != '/'
            && chars[i + 1] != '!'
        {
            if let Some(prev) = out.chars().rev().next() {
                if !prev.is_whitespace() && prev != '<' && prev != '>' {
                    out.insert(out.len() - 1, ' ');
                }
            }
        }
        i += 1;
    }
    out
}

/// Build the HTML body for a feed post.
///
/// Layout (RSStT-style):
/// ```text
/// <b>{source_title}</b>
/// <a href="{item_link}">{item_title}</a> [ | <a href="...">Telegraph</a> ]
/// {tags}
/// {formatted description}
/// ```
///
/// If the assembled message would exceed `MESSAGE_TEXT_LIMIT` characters
/// (Telegram's hard cap on the *parsed* message), the description is
/// truncated and an ellipsis appended. The header lines are never cut.
pub fn compose_feed_message(
    source_title: &str,
    item_title: &str,
    item_link: &str,
    tags: &str,
    description_html: &str,
    telegraph_url: Option<&str>,
    show_source_title: bool,
) -> String {
    let mut out = String::new();

    if show_source_title && !source_title.is_empty() {
        out.push_str("<b>");
        push_escape_text(&mut out, source_title);
        out.push_str("</b>\n");
    }

    if item_link.is_empty() {
        // Telegram rejects `<a href=""></a>` as an unclosed tag, so when the
        // feed item has no link we drop the anchor and just bold/underline
        // the title.
        out.push_str("<b><u>");
        push_escape_text(&mut out, item_title);
        out.push_str("</u></b>");
    } else {
        out.push_str("<b><u><a href=\"");
        push_escape_attr(&mut out, item_link);
        out.push_str("\">");
        push_escape_text(&mut out, item_title);
        out.push_str("</a></u></b>");
    }

    if let Some(url) = telegraph_url.filter(|u| !u.is_empty()) {
        out.push_str(" | <a href=\"");
        push_escape_attr(&mut out, url);
        out.push_str("\">Telegraph</a>");
    }

    out.push('\n');

    if !tags.is_empty() {
        // Tags come from the user and are plain text — escape everything so
        // an accidental `<` or `&` cannot break out of the surrounding HTML.
        push_escape_text(&mut out, tags);
        out.push('\n');
    }

    if !description_html.is_empty() {
        out.push('\n');
        let mut body = format_description_html(description_html, Some(item_link));
        truncate_to_limit(&mut out, &mut body);
        out.push_str(&body);
    }

    out
}

/// If the assembled header + body would exceed `MESSAGE_TEXT_LIMIT`, drop
/// the description's tail until we fit. The `…` is appended to the body so
/// the header is preserved verbatim. The budget is measured in *characters*
/// (matching Telegram's per-message cap) and the cut is rewound to the last
/// safely-closed tag boundary so we never leave an unclosed `<a>` (or any
/// other tag) dangling in the output — Telegram rejects those with
/// "can't find end tag corresponding to start tag".
fn truncate_to_limit(header: &mut String, body: &mut String) {
    let total_chars = header.chars().count() + body.chars().count();
    if total_chars <= MESSAGE_TEXT_LIMIT {
        return;
    }
    // Reserve one char for the trailing `…`.
    let budget = MESSAGE_TEXT_LIMIT
        .saturating_sub(header.chars().count())
        .saturating_sub(1);
    if body.chars().count() <= budget {
        return;
    }
    // Walk the body backwards in chars, keeping the last `budget` chars but
    // snapping the cut to the byte index just after the last `</tag>` (or
    // any other non-tag char) so the truncation never lands inside a tag.
    let keep: String = body
        .chars()
        .rev()
        .take(budget)
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    body.clear();
    body.push_str(&rewind_to_safe_boundary(&keep));
    body.push('…');
}

/// Given a candidate body fragment produced by walking backwards from the
/// end, rewind to a position that does not leave an unclosed HTML tag. The
/// scan is intentionally simple: find the byte offset of the last `</` (or
/// the last `>` if no closing tag is present) and cut just after it. If the
/// fragment contains no `>` at all, we cannot safely emit any of it.
fn rewind_to_safe_boundary(s: &str) -> &str {
    let Some(last_close) = s.rfind("</") else {
        return "";
    };
    let Some(close_end) = s[last_close..].find('>') else {
        return "";
    };
    &s[..last_close + close_end + 1]
}

fn push_escape_text(out: &mut String, value: &str) {
    out.push_str(&teloxide_html::escape(value));
}

fn push_escape_attr(out: &mut String, value: &str) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '"' => out.push_str("&quot;"),
            '<' => out.push_str("&lt;"),
            _ => out.push(ch),
        }
    }
}

fn walk(node: ego_tree::NodeRef<Node>, base_url: Option<&str>, out: &mut String) {
    match node.value() {
        Node::Document | Node::Fragment => {
            for child in node.children() {
                walk(child, base_url, out);
            }
        }
        Node::Doctype(_) | Node::Comment(_) | Node::ProcessingInstruction(_) => {}
        Node::Text(_) => {
            // Already handled by the element path: text nodes inside an
            // element are emitted by `render_children`, and text nodes at
            // the top level (a fragment) are uncommon in feed HTML.
        }
        Node::Element(_) => {
            if let Some(el) = ElementRef::wrap(node) {
                walk_element(&el, base_url, out);
            }
        }
    }
}

fn walk_element(el: &ElementRef<'_>, base_url: Option<&str>, out: &mut String) {
    let tag = el.value().name();
    match tag {
        "br" => {
            out.push('\n');
        }
        "p" | "div" | "section" | "article" | "main" | "header" | "footer"
        | "aside" | "figure" | "figcaption" | "details" | "summary" | "address" => {
            ensure_blank_line(out);
            render_children(el, base_url, out);
            out.push('\n');
            out.push('\n');
        }
        "hr" => {
            ensure_blank_line(out);
            out.push_str("——————\n\n");
        }
        "b" | "strong" => {
            out.push_str("<b>");
            render_children(el, base_url, out);
            out.push_str("</b>");
        }
        "i" | "em" | "cite" | "dfn" => {
            out.push_str("<i>");
            render_children(el, base_url, out);
            out.push_str("</i>");
        }
        "u" | "ins" => {
            out.push_str("<u>");
            render_children(el, base_url, out);
            out.push_str("</u>");
        }
        "s" | "del" | "strike" => {
            out.push_str("<s>");
            render_children(el, base_url, out);
            out.push_str("</s>");
        }
        "code" if !inside_pre(el) => {
            out.push_str("<code>");
            render_children(el, base_url, out);
            out.push_str("</code>");
        }
        "pre" => {
            ensure_blank_line(out);
            out.push_str("<pre>");
            render_children(el, base_url, out);
            out.push_str("</pre>\n\n");
        }
        "blockquote" => {
            ensure_blank_line(out);
            out.push_str("<blockquote>");
            render_children(el, base_url, out);
            out.push_str("</blockquote>\n\n");
        }
        "q" => {
            out.push_str("<i>");
            render_children(el, base_url, out);
            out.push_str("</i>");
        }
        "h1" | "h2" | "h3" | "h4" | "h5" | "h6" => {
            ensure_blank_line(out);
            out.push_str("<u>");
            render_children(el, base_url, out);
            out.push_str("</u>\n\n");
        }
        "a" => render_anchor(el, base_url, out),
        "ul" => render_list(el, base_url, out, ListKind::Unordered, 0),
        "ol" => render_list(el, base_url, out, ListKind::Ordered, 0),
        "img" => {
            if let Some(src) = el.value().attr("src") {
                if let Some(absolute) = resolve_url(base_url, src) {
                    if is_safe_url(&absolute) {
                        out.push_str("<a href=\"");
                        push_escape_attr(out, &absolute);
                        out.push_str("\">[image]</a>");
                    }
                }
            }
        }
        "table" | "tbody" | "thead" | "tfoot" | "tr" | "td" | "th" => {
            // Telegram can't render tables meaningfully; flatten to text.
            render_children(el, base_url, out);
        }
        "script" | "style" | "head" | "noscript" | "iframe" | "object" | "embed" => {
            // Drop entirely along with content.
        }
        _ => {
            // Unknown tag — keep children.
            render_children(el, base_url, out);
        }
    }
}

fn render_children(el: &ElementRef<'_>, base_url: Option<&str>, out: &mut String) {
    for child in el.children() {
        match child.value() {
            Node::Element(_) => {
                if let Some(child_el) = ElementRef::wrap(child) {
                    walk_element(&child_el, base_url, out);
                }
            }
            Node::Text(text) => push_text(out, text),
            _ => {}
        }
    }
}

fn push_text(out: &mut String, text: &str) {
    // Collapse runs of whitespace within the text node to a single space,
    // then HTML-escape the result so a stray `<` or `&` from a feed cannot
    // break out of the surrounding markup.
    let mut first = true;
    let mut prev_space = false;
    let mut buf = String::with_capacity(text.len());
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !first && !prev_space {
                buf.push(' ');
            }
            prev_space = true;
        } else {
            buf.push(ch);
            prev_space = false;
            first = false;
        }
    }
    push_escape_text(out, &buf);
}

fn inside_pre(el: &ElementRef<'_>) -> bool {
    for ancestor in el.ancestors() {
        if let Some(anc) = ancestor.value().as_element() {
            if anc.name() == "pre" {
                return true;
            }
        }
    }
    false
}

fn render_anchor(el: &ElementRef<'_>, base_url: Option<&str>, out: &mut String) {
    let href = el.value().attr("href").unwrap_or("");
    let absolute = resolve_url(base_url, href);

    let safe = match absolute.as_deref() {
        Some(u) if is_safe_url(u) => u.to_owned(),
        _ => {
            // Non-http URL (or empty): render the inner text without a link.
            render_children(el, base_url, out);
            return;
        }
    };

    out.push_str("<a href=\"");
    push_escape_attr(out, &safe);
    out.push_str("\">");
    render_children(el, base_url, out);
    out.push_str("</a>");
}

#[derive(Copy, Clone, PartialEq, Eq)]
enum ListKind {
    Unordered,
    Ordered,
}

fn render_list(
    el: &ElementRef<'_>,
    base_url: Option<&str>,
    out: &mut String,
    kind: ListKind,
    parent_indent: usize,
) {
    ensure_blank_line(out);
    let mut ordered_counter = 0usize;
    for child in el.children() {
        let Some(child_el) = ElementRef::wrap(child) else {
            // Skip stray text/whitespace between <li> elements.
            continue;
        };
        if child_el.value().name() != "li" {
            continue;
        }
        match kind {
            ListKind::Unordered => render_list_item(&child_el, base_url, out, ListKind::Unordered, 0, parent_indent),
            ListKind::Ordered => {
                ordered_counter += 1;
                render_list_item(&child_el, base_url, out, ListKind::Ordered, ordered_counter, parent_indent);
            }
        }
    }
    out.push('\n');
}

fn render_list_item(
    el: &ElementRef<'_>,
    base_url: Option<&str>,
    out: &mut String,
    kind: ListKind,
    n: usize,
    parent_indent: usize,
) {
    for _ in 0..parent_indent {
        out.push_str("    ");
    }
    match kind {
        ListKind::Unordered => out.push_str("<b>•</b> "),
        ListKind::Ordered => {
            out.push_str("<b>");
            out.push_str(&n.to_string());
            out.push_str(".</b> ");
        }
    }
    for child in el.children() {
        match child.value() {
            Node::Element(e) if e.name() == "ul" => {
                if let Some(nested) = ElementRef::wrap(child) {
                    out.push('\n');
                    render_list(&nested, base_url, out, ListKind::Unordered, parent_indent + 1);
                }
            }
            Node::Element(e) if e.name() == "ol" => {
                if let Some(nested) = ElementRef::wrap(child) {
                    out.push('\n');
                    render_list(&nested, base_url, out, ListKind::Ordered, parent_indent + 1);
                }
            }
            Node::Element(_) => {
                if let Some(child_el) = ElementRef::wrap(child) {
                    walk_element(&child_el, base_url, out);
                }
            }
            Node::Text(text) => push_text(out, text),
            _ => {}
        }
    }
    if !out.ends_with('\n') {
        out.push('\n');
    }
}

fn resolve_url(base_url: Option<&str>, value: &str) -> Option<String> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(base) = base_url {
        if let Ok(base) = reqwest::Url::parse(base) {
            if let Ok(joined) = base.join(trimmed) {
                return Some(joined.to_string());
            }
        }
    }
    if reqwest::Url::parse(trimmed).is_ok() {
        Some(trimmed.to_owned())
    } else {
        None
    }
}

fn is_safe_url(url: &str) -> bool {
    matches!(url.split_once(':').map(|(s, _)| s), Some(scheme) if scheme.eq_ignore_ascii_case("http") || scheme.eq_ignore_ascii_case("https"))
}

fn ensure_blank_line(out: &mut String) {
    if out.is_empty() {
        return;
    }
    if out.ends_with("\n\n") {
        return;
    }
    if out.ends_with('\n') {
        out.push('\n');
    } else {
        out.push_str("\n\n");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn normalize(s: &str) -> String {
        // Collapse runs of whitespace for comparison, but keep newlines.
        let mut out = String::with_capacity(s.len());
        let mut prev_space = false;
        for ch in s.chars() {
            if ch == ' ' || ch == '\t' {
                if !prev_space {
                    out.push(' ');
                }
                prev_space = true;
            } else {
                out.push(ch);
                prev_space = false;
            }
        }
        out
    }

    #[test]
    fn preserves_bold_and_link() {
        let html = "<p>Hello <b>world</b> from <a href=\"https://example.com\">a link</a></p>";
        let out = format_description_html(html, None);
        assert!(out.contains("<b>world</b>"), "got: {out:?}");
        assert!(out.contains("<a href=\"https://example.com\">a link</a>"), "got: {out:?}");
    }

    #[test]
    fn renders_unordered_list_with_bullet() {
        let html = "<ul><li>alpha</li><li>beta</li></ul>";
        let out = format_description_html(html, None);
        assert!(out.contains("<b>•</b> alpha"), "got: {out:?}");
        assert!(out.contains("<b>•</b> beta"), "got: {out:?}");
    }

    #[test]
    fn renders_ordered_list_with_incrementing_numbers() {
        let html = "<ol><li>first</li><li>second</li></ol>";
        let out = format_description_html(html, None);
        assert!(out.contains("<b>1.</b> first"), "got: {out:?}");
        assert!(out.contains("<b>2.</b> second"), "got: {out:?}");
    }

    #[test]
    fn nested_list_is_indented() {
        let html = "<ul><li>outer<ul><li>inner</li></ul></li></ul>";
        let out = format_description_html(html, None);
        assert!(out.contains("<b>•</b> outer"), "got: {out:?}");
        assert!(out.contains("    <b>•</b> inner"), "got: {out:?}");
    }

    #[test]
    fn strips_script_blocks() {
        let html = "<p>before</p><script>alert(1)</script><p>after</p>";
        let out = format_description_html(html, None);
        assert!(!out.contains("alert"), "got: {out:?}");
        assert!(out.contains("before"), "got: {out:?}");
        assert!(out.contains("after"), "got: {out:?}");
    }

    #[test]
    fn resolves_relative_links_against_base() {
        let html = "<a href=\"/post/1\">read</a>";
        let out = format_description_html(html, Some("https://example.com/feed"));
        assert!(
            out.contains("<a href=\"https://example.com/post/1\">read</a>"),
            "got: {out:?}"
        );
    }

    #[test]
    fn strips_non_http_links() {
        let html = "<a href=\"javascript:alert(1)\">click</a>";
        let out = format_description_html(html, None);
        assert!(!out.contains("javascript:"), "got: {out:?}");
        assert!(out.contains("click"), "got: {out:?}");
    }

    #[test]
    fn compose_header_escapes_user_title() {
        let html = compose_feed_message(
            "源<b>danger</b>",
            "a&b",
            "https://example.com/post\"x",
            "#tag",
            "",
            None,
            true,
        );
        assert!(
            html.contains("<b>源&lt;b&gt;danger&lt;/b&gt;</b>"),
            "got: {html:?}"
        );
        assert!(
            html.contains("<a href=\"https://example.com/post&quot;x\">a&amp;b</a>"),
            "got: {html:?}"
        );
        assert!(html.contains("#tag"), "got: {html:?}");
    }

    #[test]
    fn compose_appends_telegraph_link_when_enabled() {
        let html = compose_feed_message(
            "源",
            "title",
            "https://example.com/p",
            "",
            "",
            Some("https://telegra.ph/foo"),
            true,
        );
        assert!(
            html.contains(" | <a href=\"https://telegra.ph/foo\">Telegraph</a>"),
            "got: {html:?}"
        );
    }

    #[test]
    fn compose_with_empty_description_is_header_only() {
        let html = compose_feed_message("源", "title", "https://example.com/p", "", "", None, true);
        let expected = "<b>源</b>\n<b><u><a href=\"https://example.com/p\">title</a></u></b>\n";
        assert_eq!(html, expected);
    }

    #[test]
    fn compose_skips_source_title_when_empty() {
        let html = compose_feed_message("", "title", "https://example.com/p", "", "", None, true);
        let expected = "<b><u><a href=\"https://example.com/p\">title</a></u></b>\n";
        assert_eq!(html, expected);
        assert!(!html.starts_with("<b></b>"), "got: {html:?}");
    }

    #[test]
    fn compose_skips_source_title_when_flag_is_false() {
        let html = compose_feed_message("源", "title", "https://example.com/p", "", "", None, false);
        let expected = "<b><u><a href=\"https://example.com/p\">title</a></u></b>\n";
        assert_eq!(html, expected);
        assert!(!html.contains("<b>源</b>"), "title leaked: {html:?}");
    }

    #[test]
    fn compose_drops_anchor_when_item_link_is_empty() {
        // Telegram rejects `<a href=""></a>` as an unclosed tag, so the
        // header must not emit an anchor when the feed item has no link.
        let html = compose_feed_message("源", "title", "", "", "", None, true);
        let expected = "<b>源</b>\n<b><u>title</u></b>\n";
        assert_eq!(html, expected);
        assert!(!html.contains("<a"), "anchor leaked: {html:?}");
    }

    #[test]
    fn compose_with_empty_source_and_link_keeps_plain_title() {
        let html = compose_feed_message("", "title", "", "", "", None, true);
        let expected = "<b><u>title</u></b>\n";
        assert_eq!(html, expected);
    }

    #[test]
    fn empty_anchor_in_description_is_dropped() {
        // A stray `<a href="" target="_blank"></a>` inside the description
        // must not leak as an unclosed anchor into the Telegram payload.
        let desc = "Августовский дайджест: Умный дом в новом свете (<a href=\"\" target=\"_blank\"></a>)";
        let html = compose_feed_message(
            "源",
            "title",
            "https://example.com/p",
            "",
            desc,
            None,
            true,
        );
        assert!(!html.contains("href=\"\""), "empty href leaked: {html:?}");
        assert!(!html.contains("<a></a>"), "empty anchor leaked: {html:?}");
        assert!(html.contains("Августовский дайджест"), "text lost: {html:?}");
    }

    #[test]
    fn compose_includes_formatted_description() {
        let html = compose_feed_message(
            "源",
            "title",
            "https://example.com/p",
            "",
            "<p>Hello <b>world</b></p>",
            None,
            true,
        );
        assert!(html.contains("<b>world</b>"), "got: {html:?}");
        assert!(html.contains("Hello"), "got: {html:?}");
    }

    #[test]
    fn br_becomes_newline() {
        let html = format_description_html("line one<br>line two", None);
        assert!(html.contains("line one\nline two"), "got: {html:?}");
    }

    #[test]
    fn headless_run_does_not_panic_on_real_snippet() {
        let html = r#"<div class="content">
            <p>The <strong>TLDR</strong> is <em>simple</em>.</p>
            <h2>Details</h2>
            <ul>
                <li>one</li>
                <li>two with a <a href="https://example.com">link</a></li>
            </ul>
            <pre><code>let x = 1;</code></pre>
            <blockquote>quoted text</blockquote>
            <script>nope()</script>
        </div>"#;
        let out = format_description_html(html, Some("https://example.com/post"));
        let n = normalize(&out);
        assert!(n.contains("<b>TLDR</b>"), "got: {out:?}");
        assert!(n.contains("<i>simple</i>"), "got: {out:?}");
        // h1-h6 render as underlined (not bold) so they stand out without
        // being mistaken for inline emphasis.
        assert!(n.contains("<u>Details</u>"), "got: {out:?}");
        assert!(n.contains("<b>•</b> one"), "got: {out:?}");
        assert!(
            n.contains("<b>•</b> two with a <a href=\"https://example.com/\">link</a>"),
            "got: {out:?}"
        );
        assert!(n.contains("<pre>let x = 1;</pre>"), "got: {out:?}");
        assert!(n.contains("<blockquote>quoted text</blockquote>"), "got: {out:?}");
        assert!(!n.contains("nope()"), "got: {out:?}");
    }

    #[test]
    fn escapes_special_characters_in_text_nodes() {
        // `<`, `>`, `&` in feed content must be HTML-escaped, not leaked
        // into the Telegram HTML, or the whole message will fail to parse.
        let out = format_description_html("<p>1 < 2 &amp; 3 > 0</p>", None);
        assert!(out.contains("1 &lt; 2 &amp; 3 &gt; 0"), "got: {out:?}");
        assert!(!out.contains("1 < 2"), "raw `<` leaked: {out:?}");
    }

    #[test]
    fn ordered_list_uses_li_counter_not_sibling_index() {
        // Whitespace text nodes between <li> elements should not bump the
        // counter; the second <li> must still be "1.", not "3.".
        let html = "<ol>\n  <li>a</li>\n  <li>b</li>\n</ol>";
        let out = format_description_html(html, None);
        assert!(out.contains("<b>1.</b> a"), "got: {out:?}");
        assert!(out.contains("<b>2.</b> b"), "got: {out:?}");
        assert!(!out.contains("<b>3."), "got: {out:?}");
    }

    #[test]
    fn image_becomes_html_link_not_markdown() {
        let html = "<img src=\"https://example.com/cat.png\" alt=\"cat\">";
        let out = format_description_html(html, None);
        assert!(
            out.contains("<a href=\"https://example.com/cat.png\">[image]</a>"),
            "got: {out:?}"
        );
        assert!(!out.contains("]("), "markdown-style image leaked: {out:?}");
    }

    #[test]
    fn compose_truncates_long_descriptions_keeping_header_intact() {
        let big = "a".repeat(MESSAGE_TEXT_LIMIT + 500);
        let html = compose_feed_message("源", "title", "https://x", "", &big, None, true);
        assert!(html.chars().count() <= MESSAGE_TEXT_LIMIT, "len={}", html.chars().count());
        // Header lines are preserved.
        assert!(html.contains("<b>源</b>"), "header lost");
        assert!(html.contains("title"), "title lost");
        // Body got truncated with an ellipsis.
        assert!(html.contains('…'), "expected ellipsis: {html:?}");
    }

    #[test]
    fn compose_truncation_does_not_leave_unclosed_anchor() {
        // Telegram rejects any unclosed HTML tag with
        // "can't find end tag corresponding to start tag". A naive byte
        // truncate can chop inside `<a href="…">…</a>`; the rewind must
        // snap back to the last `</…>` so the cut never lands inside a tag.
        let link = "<a href=\"https://example.com/very/long/path/that/forces/truncation\">click me</a>";
        let big = format!("{} {}", link, "x".repeat(MESSAGE_TEXT_LIMIT));
        let html = compose_feed_message("源", "title", "https://x", "", &big, None, true);
        let opens = html.matches("<a ").count();
        let closes = html.matches("</a>").count();
        assert_eq!(opens, closes, "unbalanced anchors: {html:?}");
        assert!(html.contains('…'), "expected ellipsis: {html:?}");
    }

    #[test]
    fn inline_space_between_closing_tag_and_word_works() {
        // Common feed quirk: no whitespace between an inline tag and the
        // following word. Our post-pass should still render it readably.
        let out = format_description_html("<p>foo <b>bar</b>baz</p>", None);
        assert!(out.contains("</b> baz") || out.contains("</b>\nbaz"), "got: {out:?}");
    }

    #[test]
    fn inline_space_not_inserted_before_punctuation() {
        let out = format_description_html("<p>foo <b>bar</b>.</p>", None);
        assert!(out.contains("</b>."), "got: {out:?}");
        assert!(!out.contains("</b> ."), "got: {out:?}");
    }
}
