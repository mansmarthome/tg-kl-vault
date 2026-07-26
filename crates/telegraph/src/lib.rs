//! Telegraph (telegra.ph) content model, HTML converter, and API client.
//!
//! # Layering rule
//!
//! [`html`] and [`node`] are **pure**: no I/O, no async, no `reqwest`. They are
//! snapshot-testable and fuzzable on their own, and usable without the client.
//! Only [`api`] touches the network, and it sits behind the `client` feature.
//!
//! ```no_run
//! use telegraph::{html_to_nodes, ConvertOptions};
//! let result = html_to_nodes("<div><p>hello</p></div>", &ConvertOptions::default());
//! assert_eq!(result.nodes.len(), 1);
//! ```

#![forbid(unsafe_code)]

pub mod error;
pub mod html;
pub mod limits;
pub mod node;

#[cfg(feature = "client")]
pub mod api;

pub use error::Error;
pub use html::{html_to_nodes, ConvertOptions, ConvertResult};
pub use node::{Node, NodeElement};
