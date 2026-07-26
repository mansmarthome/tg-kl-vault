//! HTML → Telegraph node conversion. **Pure**: no I/O, no async, no reqwest.
//!
//! Pipeline:
//!   1. `walk`      — tree → nodes. Rules 1 (unwrap), 2 (headings), 3 (attrs), 5 (whitespace).
//!   2. `normalize` — Rule 4 structural fixes: hoist images, relocate stray li/figcaption.
//!   3. `truncate`  — Rule 6 size cap.
//!
//! TODO(agent): `scraper`'s API shifts between minor versions. If this does not
//! compile, check docs.rs for the current shape of `scraper::Node`,
//! `scraper::node::{Text, Element}`, and `ego_tree::NodeRef` and adjust — the
//! algorithm below is what matters, not these exact accessors.

use std::collections::BTreeMap;

use ego_tree::NodeRef;
use scraper::{Html, Node as DomNode};
use url::Url;

use crate::limits::{attr_allowed, is_discard, is_hoist_barrier, is_void, map_tag};
use crate::node::Node;

#[derive(Debug, Clone)]
pub struct ConvertOptions {
    /// Used to absolutise relative `href`/`src`. Set this to the article URL.
    pub base_url: Option<Url>,
    /// Cap on the serialised JSON size of the result.
    pub max_bytes: usize,
    /// DoS guard on pathological input.
    pub max_nodes: usize,
}

impl Default for ConvertOptions {
    fn default() -> Self {
        Self { base_url: None, max_bytes: 64 * 1024, max_nodes: 10_000 }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ConvertResult {
    pub nodes: Vec<Node>,
    pub truncated: bool,
    /// Unwrapped/discarded tag -> occurrence count. **Log this in production.**
    /// When a site's articles come out empty this is how you learn the body was
    /// wrapped in `<table>`.
    pub dropped_tags: BTreeMap<String, usize>,
}

/// Infallible by design: malformed input yields best-effort output, never an error.
pub fn html_to_nodes(html: &str, opts: &ConvertOptions) -> ConvertResult {
    let dom = Html::parse_fragment(html);

    let mut w = Walker { opts, dropped: BTreeMap::new(), budget: opts.max_nodes };
    let mut nodes = Vec::new();
    // `parse_fragment` synthesises <html>/<body>. Neither is in `map_tag`, so
    // Rule 1 unwraps them for free — no special-casing needed.
    for child in dom.tree.root().children() {
        w.walk(child, &mut nodes);
    }
    let dropped = w.dropped;

    let nodes = normalize(nodes);
    let (nodes, truncated) = truncate(nodes, opts.max_bytes);

    ConvertResult { nodes, truncated, dropped_tags: dropped }
}

// ---------------------------------------------------------------- phase 1

struct Walker<'a> {
    opts: &'a ConvertOptions,
    dropped: BTreeMap<String, usize>,
    budget: usize,
}

impl<'a> Walker<'a> {
    fn note_dropped(&mut self, tag: &str) {
        *self.dropped.entry(tag.to_owned()).or_insert(0) += 1;
    }

    fn walk(&mut self, dom: NodeRef<'_, DomNode>, out: &mut Vec<Node>) {
        if self.budget == 0 {
            return;
        }
        self.budget -= 1;

        match dom.value() {
            DomNode::Text(t) => push_text(out, t),
            DomNode::Element(el) => {
                let tag = el.name();

                if is_discard(tag) {
                    self.note_dropped(tag);
                    return;
                }

                match map_tag(tag) {
                    // Rule 1: unwrap — element vanishes, children survive.
                    None => {
                        self.note_dropped(tag);
                        for c in dom.children() {
                            self.walk(c, out);
                        }
                    }
                    Some(mapped) => {
                        let mut children = Vec::new();
                        for c in dom.children() {
                            self.walk(c, &mut children);
                        }
                        trim_edges(&mut children);
                        if let Some(n) = self.build(mapped, el, children) {
                            out.push(n);
                        }
                    }
                }
            }
            // Comments, doctype, processing instructions.
            _ => {}
        }
    }

    fn build(
        &mut self,
        tag: &'static str,
        el: &scraper::node::Element,
        children: Vec<Node>,
    ) -> Option<Node> {
        let mut attrs = BTreeMap::new();

        for (name, value) in el.attrs() {
            if !attr_allowed(tag, name) {
                continue;
            }
            match resolve_url(value, self.opts.base_url.as_ref()) {
                Some(u) => {
                    attrs.insert(name.to_owned(), u);
                }
                None => continue,
            }
        }

        // Lazy-loaded images: real src hides in data-src / data-original.
        if tag == "img" && !attrs.contains_key("src") {
            for alt in ["data-src", "data-original", "data-lazy-src"] {
                if let Some(v) = el.attr(alt) {
                    if let Some(u) = resolve_url(v, self.opts.base_url.as_ref()) {
                        attrs.insert("src".into(), u);
                        break;
                    }
                }
            }
        }

        match tag {
            // An image with no usable source renders as a dead box.
            "img" if !attrs.contains_key("src") => {
                self.note_dropped("img[no-src]");
                return None;
            }
            // A link with no usable href still has readable text: unwrap it.
            "a" if !attrs.contains_key("href") => {
                return if children.is_empty() {
                    None
                } else {
                    Some(Node::element("span_unwrap_marker", children))
                };
            }
            _ => {}
        }

        // Rule 5: drop elements that ended up empty and carry no meaning alone.
        if children.is_empty() && !is_void(tag) {
            return None;
        }

        Some(Node::element_with(tag, attrs, children))
    }
}

/// Collapse whitespace runs, merging into a preceding text node.
fn push_text(out: &mut Vec<Node>, raw: &str) {
    let mut s = String::with_capacity(raw.len());
    let mut prev_ws = false;
    for ch in raw.chars() {
        if ch.is_whitespace() {
            if !prev_ws {
                s.push(' ');
            }
            prev_ws = true;
        } else {
            s.push(ch);
            prev_ws = false;
        }
    }
    if s.is_empty() {
        return;
    }
    if let Some(Node::Text(last)) = out.last_mut() {
        last.push_str(&s);
    } else {
        out.push(Node::Text(s));
    }
}

/// Drop whitespace-only text at the edges of an element's children, while
/// preserving significant inner whitespace (`<b>a</b> <i>b</i>`).
fn trim_edges(children: &mut Vec<Node>) {
    while children.first().is_some_and(Node::is_blank_text) {
        children.remove(0);
    }
    while children.last().is_some_and(Node::is_blank_text) {
        children.pop();
    }
}

fn resolve_url(raw: &str, base: Option<&Url>) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }
    let parsed = match Url::parse(raw) {
        Ok(u) => u,
        Err(url::ParseError::RelativeUrlWithoutBase) => base?.join(raw).ok()?,
        Err(_) => return None,
    };
    // Reject javascript:, data:, file:, ...
    matches!(parsed.scheme(), "http" | "https").then(|| parsed.to_string())
}

// ---------------------------------------------------------------- phase 2

fn normalize(nodes: Vec<Node>) -> Vec<Node> {
    let nodes = nodes.into_iter().flat_map(unwrap_markers).collect::<Vec<_>>();
    let nodes = nodes.into_iter().flat_map(hoist_images).collect::<Vec<_>>();
    let mut nodes = relocate_strays(nodes, None);
    trim_edges(&mut nodes);
    nodes
}

/// `build` emits a placeholder for href-less `<a>`; splice its children in.
fn unwrap_markers(n: Node) -> Vec<Node> {
    match n {
        Node::Element(e) if e.tag == "span_unwrap_marker" => e
            .children
            .unwrap_or_default()
            .into_iter()
            .flat_map(unwrap_markers)
            .collect(),
        Node::Element(e) => {
            let children = e
                .children
                .clone()
                .unwrap_or_default()
                .into_iter()
                .flat_map(unwrap_markers)
                .collect();
            vec![e.reshell(children)]
        }
        t => vec![t],
    }
}

/// Rule 4: `<img>` must be top-level or a direct child of `<figure>`.
/// Splits enclosing blocks around each image rather than dropping either.
fn hoist_images(n: Node) -> Vec<Node> {
    let Node::Element(e) = n else { return vec![n] };
    if e.tag == "img" || is_hoist_barrier(&e.tag) {
        return vec![Node::Element(e)];
    }
    let Some(children) = e.children.clone() else { return vec![Node::Element(e)] };

    let mut out = Vec::new();
    let mut buf: Vec<Node> = Vec::new();
    for c in children {
        for piece in hoist_images(c) {
            if piece.is_tag("img") {
                if !buf.is_empty() {
                    out.push(e.reshell(std::mem::take(&mut buf)));
                }
                out.push(piece);
            } else {
                buf.push(piece);
            }
        }
    }
    if !buf.is_empty() {
        out.push(e.reshell(buf));
    }
    if out.is_empty() {
        out.push(e.reshell(vec![]));
    }
    out
}

/// Rule 4: `<li>` only inside `ul`/`ol`; `<figcaption>` only inside `figure`.
/// Strays become `<p>` rather than disappearing.
fn relocate_strays(nodes: Vec<Node>, parent: Option<&str>) -> Vec<Node> {
    nodes
        .into_iter()
        .filter_map(|n| {
            let Node::Element(e) = n else { return Some(n) };
            let ok = match e.tag.as_str() {
                "li" => matches!(parent, Some("ul") | Some("ol")),
                "figcaption" => parent == Some("figure"),
                _ => true,
            };
            let tag = if ok { e.tag.clone() } else { "p".to_owned() };
            let children =
                relocate_strays(e.children.clone().unwrap_or_default(), Some(&e.tag));
            if children.is_empty() && !is_void(&tag) {
                return None;
            }
            Some(Node::element_with(&tag, e.attrs.clone().unwrap_or_default(), children))
        })
        .collect()
}

// ---------------------------------------------------------------- phase 3

/// Rule 6: never emit a partial element. Stop at the last complete top-level node.
fn truncate(nodes: Vec<Node>, max_bytes: usize) -> (Vec<Node>, bool) {
    let mut out = Vec::with_capacity(nodes.len());
    let mut used = 2usize; // the enclosing "[]"

    for n in nodes {
        let len = serde_json::to_vec(&n).map(|v| v.len() + 1).unwrap_or(0);
        if used + len > max_bytes {
            out.push(Node::element("p", vec![Node::text("…")]));
            return (out, true);
        }
        used += len;
        out.push(n);
    }
    (out, false)
}

// ---------------------------------------------------------------- tests

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::map_tag;
    use crate::node::visit_elements;

    fn conv(html: &str) -> Vec<Node> {
        html_to_nodes(html, &ConvertOptions::default()).nodes
    }

    /// Crate-wide invariant. Assert this in every fixture test too.
    fn assert_whitelisted(nodes: &[Node]) {
        visit_elements(nodes, &mut |e| {
            assert!(map_tag(&e.tag) == Some(e.tag.as_str()), "leaked tag <{}>", e.tag);
        });
    }

    #[test]
    fn rule1_unwraps_unknown_containers() {
        // The single most important test in the crate.
        let n = conv("<div><section><p>hello</p></section></div>");
        assert_eq!(n, vec![Node::element("p", vec![Node::text("hello")])]);
        assert_whitelisted(&n);
    }

    #[test]
    fn rule1_discards_script_subtree() {
        let n = conv("<p>a</p><script>var x = '<p>b</p>';</script>");
        assert_eq!(n, vec![Node::element("p", vec![Node::text("a")])]);
    }

    #[test]
    fn rule2_demotes_headings() {
        assert_eq!(conv("<h1>t</h1>")[0].tag(), Some("h3"));
        assert_eq!(conv("<h6>t</h6>")[0].tag(), Some("h4"));
    }

    #[test]
    fn rule3_rejects_javascript_scheme() {
        let n = conv(r#"<a href="javascript:alert(1)">click</a>"#);
        assert_eq!(n, vec![Node::text("click")], "href-less link should unwrap to its text");
    }

    #[test]
    fn rule3_resolves_relative_against_base() {
        let opts = ConvertOptions {
            base_url: Some(Url::parse("https://example.com/blog/post").unwrap()),
            ..Default::default()
        };
        let r = html_to_nodes(r#"<img src="/img/a.png">"#, &opts);
        let Node::Element(e) = &r.nodes[0] else { panic!() };
        assert_eq!(e.attrs.as_ref().unwrap()["src"], "https://example.com/img/a.png");
    }

    #[test]
    fn rule4_hoists_image_out_of_paragraph() {
        let n = conv(r#"<p>before<img src="https://e.com/a.png">after</p>"#);
        assert_eq!(n.len(), 3);
        assert_eq!(n[0].tag(), Some("p"));
        assert_eq!(n[1].tag(), Some("img"));
        assert_eq!(n[2].tag(), Some("p"));
    }

    #[test]
    fn rule5_preserves_inline_spacing() {
        let n = conv("<p><b>a</b> <i>b</i></p>");
        let Node::Element(p) = &n[0] else { panic!() };
        assert_eq!(p.children.as_ref().unwrap()[1], Node::text(" "));
    }

    #[test]
    fn never_panics_on_garbage() {
        for junk in ["", "<<<>>>", "<p", "<p></div></p>", "\u{0}<b>"] {
            let _ = conv(junk);
        }
    }
}
