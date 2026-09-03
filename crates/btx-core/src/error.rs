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
}

pub type AppResult<T> = Result<T, AppError>;
