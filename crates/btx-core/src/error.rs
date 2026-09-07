use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("http error: {0}")]
    Http(String),
    #[error("rpc error {code}: {message}")]
    Rpc { code: i64, message: String },
    #[error("decode error: {0}")]
    Decode(String),
    #[error("process error: {0}")]
    Process(String),
    #[error("config error: {0}")]
    Config(String),
    /// Not enough free space to let btxd run. Its own variant because the
    /// answer is "free some space", never the repair path's "remove node data"
    /// — a datadir on a full volume is intact, not corrupt, and wiping it is
    /// the one action that turns a recoverable state into a re-sync.
    #[error("disk error: {0}")]
    Disk(String),
}

pub type AppResult<T> = Result<T, AppError>;
