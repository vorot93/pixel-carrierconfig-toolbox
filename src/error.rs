//! Crate error type.
use thiserror::Error;

#[derive(Error, Debug)]
pub enum Error {
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("malformed confseq: {0}")]
    Confseq(String),
    #[error("malformed CLZ4: {0}")]
    Clz4(String),
    #[error("project: {0}")]
    Project(String),
}

pub type Result<T> = std::result::Result<T, Error>;
