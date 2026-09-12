#![forbid(unsafe_code)]

pub mod abstracts;
pub mod analysis;
pub mod api;
pub mod citation_graph;
pub mod domain;
pub mod error;
pub mod extract;
mod frontmatter;
pub mod jobs;
pub mod layout;
pub mod library;
pub mod markdown;
pub mod notes;
pub mod reader_tools;
mod remote;
pub mod source_index;
pub mod store;

pub use api::{AppState, build_router};
pub use error::{Error, Result};
