//! Telegraph's structural constraints, in one auditable place.
//!
//! TODO(agent): verify every list here against <https://telegra.ph/api> before
//! shipping. These were written from documentation that may have drifted.

/// Maps an HTML tag to the Telegraph tag it becomes.
///
/// `None` means "not representable" — the element is **unwrapped**: it
/// disappears but its children are hoisted into the parent. See `is_discard`
/// for the much smaller set whose subtree is thrown away instead.
pub fn map_tag(tag: &str) -> Option<&'static str> {
    Some(match tag {
        "a" => "a",
        "aside" => "aside",
        "b" => "b",
        "blockquote" => "blockquote",
        "br" => "br",
        "code" => "code",
        "em" => "em",
        "figcaption" => "figcaption",
        "figure" => "figure",
        "hr" => "hr",
        "i" => "i",
        "iframe" => "iframe",
        "img" => "img",
        "li" => "li",
        "ol" => "ol",
        "p" => "p",
        "pre" => "pre",
        "s" => "s",
        "strong" => "strong",
        "u" => "u",
        "ul" => "ul",
        "video" => "video",

        // Rule 2: Telegraph has no h1/h2.
        "h1" | "h2" | "h3" => "h3",
        "h4" | "h5" | "h6" => "h4",

        // Aliases worth preserving rather than unwrapping.
        "del" | "strike" => "s",
        "ins" => "u",
        "mark" => "b",
        "cite" | "dfn" | "var" => "i",
        "samp" | "kbd" => "code",

        _ => return None,
    })
}

/// Tags whose entire subtree is discarded: content-free or hostile.
///
/// Everything *not* in here and not in `map_tag` is unwrapped, not dropped.
/// Getting this backwards is how converters silently eat whole articles.
pub fn is_discard(tag: &str) -> bool {
    matches!(
        tag,
        "script"
            | "style"
            | "noscript"
            | "head"
            | "meta"
            | "link"
            | "title"
            | "svg"
            | "canvas"
            | "form"
            | "input"
            | "button"
            | "select"
            | "textarea"
            | "object"
            | "embed"
            | "template"
            | "iframe" // only re-admitted via the embed path; see html.rs
    )
}

/// Elements that are meaningful with no children.
pub fn is_void(tag: &str) -> bool {
    matches!(tag, "br" | "hr" | "img")
}

/// Elements an image must not be hoisted out of.
///
/// `figure` is a legal image container. `ul`/`ol` may only contain `li`, so
/// lifting an image to be their direct child would produce invalid structure.
///
/// TODO(agent): this is a judgement call. If Telegraph turns out to tolerate
/// `img` inside `li`, this set can shrink to just `figure`.
pub fn is_hoist_barrier(tag: &str) -> bool {
    matches!(tag, "figure" | "ul" | "ol")
}

/// Which attributes survive on which tag. Everything else is dropped.
pub fn attr_allowed(tag: &str, attr: &str) -> bool {
    matches!(
        (tag, attr),
        ("a", "href") | ("img", "src") | ("iframe", "src") | ("video", "src")
    )
}
