//! Telegraph's content node model.
//!
//! A Telegraph page body is a JSON array of nodes. A node is either a bare
//! string or an object with `tag` / `attrs` / `children`.
//!
//! `BTreeMap` (not `HashMap`) is deliberate: attribute order must be
//! deterministic or the `insta` snapshot tests become flaky.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// A single Telegraph content node.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    /// A bare text run.
    Text(String),
    /// An element node.
    Element(NodeElement),
}

/// An element node: a tag, optional attributes, optional children.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NodeElement {
    pub tag: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub attrs: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub children: Option<Vec<Node>>,
}

impl Node {
    pub fn text(s: impl Into<String>) -> Self {
        Node::Text(s.into())
    }

    pub fn element(tag: &str, children: Vec<Node>) -> Self {
        Node::Element(NodeElement {
            tag: tag.to_owned(),
            attrs: None,
            children: if children.is_empty() { None } else { Some(children) },
        })
    }

    pub fn element_with(tag: &str, attrs: BTreeMap<String, String>, children: Vec<Node>) -> Self {
        Node::Element(NodeElement {
            tag: tag.to_owned(),
            attrs: if attrs.is_empty() { None } else { Some(attrs) },
            children: if children.is_empty() { None } else { Some(children) },
        })
    }

    pub fn tag(&self) -> Option<&str> {
        match self {
            Node::Element(e) => Some(&e.tag),
            Node::Text(_) => None,
        }
    }

    pub fn is_tag(&self, t: &str) -> bool {
        self.tag() == Some(t)
    }

    /// True for a text node containing nothing but whitespace.
    pub fn is_blank_text(&self) -> bool {
        matches!(self, Node::Text(s) if s.trim().is_empty())
    }
}

impl NodeElement {
    /// Same tag and attributes, different children. Used when splitting a
    /// block element around a hoisted image.
    pub fn reshell(&self, children: Vec<Node>) -> Node {
        Node::Element(NodeElement {
            tag: self.tag.clone(),
            attrs: self.attrs.clone(),
            children: if children.is_empty() { None } else { Some(children) },
        })
    }
}

/// Depth-first visit over every element in a node list.
///
/// Primarily a test helper: the crate-wide invariant is that no emitted node
/// carries a tag outside the Telegraph whitelist, and this is how you assert it.
pub fn visit_elements<'a>(nodes: &'a [Node], f: &mut impl FnMut(&'a NodeElement)) {
    for n in nodes {
        if let Node::Element(e) = n {
            f(e);
            if let Some(children) = &e.children {
                visit_elements(children, f);
            }
        }
    }
}
