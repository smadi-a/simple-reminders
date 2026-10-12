#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("database error: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("invalid recurrence data: {0}")]
    InvalidRecurrence(String),
    #[error("reminder not found: {0}")]
    NotFound(i64),
}

pub type Result<T> = std::result::Result<T, Error>;
