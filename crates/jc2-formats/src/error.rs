use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error on {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("{0}")]
    Format(String),
    #[error("file not found: {0}")]
    NotFound(String),
    #[error("truncated data: needed {needed} bytes at offset {offset}, have {len}")]
    Truncated {
        offset: usize,
        needed: usize,
        len: usize,
    },
}

pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    pub fn io(path: impl Into<PathBuf>, source: std::io::Error) -> Self {
        Error::Io {
            path: path.into(),
            source,
        }
    }
    pub fn format(msg: impl Into<String>) -> Self {
        Error::Format(msg.into())
    }
}
