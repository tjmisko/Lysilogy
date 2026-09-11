#![forbid(unsafe_code)]

pub mod abstracts;
pub mod analysis;
pub mod api;
pub mod domain;
pub mod error;
pub mod extract;
pub mod jobs;
pub mod layout;
pub mod library;
pub mod markdown;
pub mod reader_tools;
mod remote;
pub mod store;

pub use api::{AppState, build_router};
pub use error::{Error, Result};
