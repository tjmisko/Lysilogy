#![forbid(unsafe_code)]

pub mod abstracts;
pub mod acquisition;
pub mod analysis;
pub mod api;
pub mod audit;
pub mod citation_graph;
pub mod config;
pub mod domain;
pub mod error;
pub mod expansion;
pub mod extract;
pub mod field_extraction;
mod frontmatter;
pub mod graph;
pub mod history;
pub mod jobs;
pub mod labeling;
pub mod layout;
pub mod library;
pub mod lists;
pub mod literature;
pub mod markdown;
pub mod notes;
pub mod paper_artifacts;
pub mod prominence;
pub mod provenance;
pub mod reader_tools;
pub mod reading_list_planner;
pub mod relevance;
mod remote;
pub mod source_index;
pub mod store;
pub mod walk;

pub use api::{AppState, build_router};
pub use error::{Error, Result};
