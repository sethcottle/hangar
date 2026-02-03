// SPDX-License-Identifier: MPL-2.0

mod db;
mod feeds;
mod images;
mod posts;
mod profiles;
mod schema;

pub use db::CacheDb;
pub use feeds::{FeedCache, FeedState};
pub use images::{CacheStats, ImageCache};
pub use posts::PostCache;
pub use profiles::ProfileCache;

use thiserror::Error;

#[derive(Error, Debug)]
pub enum CacheError {
    #[error("database error: {0}")]
    Database(#[from] rusqlite::Error),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("not found")]
    NotFound,
    #[error("database path error: {0}")]
    Path(String),
}
