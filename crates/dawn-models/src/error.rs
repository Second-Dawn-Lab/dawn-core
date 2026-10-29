use std::fmt;

#[derive(Debug)]
pub enum ModelError {
    /// The user cancelled. Partial downloads are kept so the next attempt can resume.
    Cancelled,
    /// The downloaded file does not match the catalog checksum; it has been deleted.
    Checksum { expected: String, actual: String },
    /// The catalog entry has no real url/checksum yet.
    NotPublished(String),
    /// The archive is not a valid model (missing files, unsafe paths, bad zip).
    InvalidArchive(String),
    Network(String),
    Io(std::io::Error),
    UnknownLanguage(String),
    /// A language cannot be activated because some of its models are not on disk.
    NotAvailable(String),
}

impl ModelError {
    /// Stable code the UI can switch on.
    pub fn code(&self) -> &'static str {
        match self {
            Self::Cancelled => "cancelled",
            Self::Checksum { .. } => "checksum",
            Self::NotPublished(_) => "not_published",
            Self::InvalidArchive(_) => "invalid_archive",
            Self::Network(_) => "network",
            Self::Io(_) => "io",
            Self::UnknownLanguage(_) => "unknown_language",
            Self::NotAvailable(_) => "not_available",
        }
    }
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cancelled => write!(f, "download cancelled"),
            Self::Checksum { expected, actual } => {
                write!(f, "checksum mismatch (expected {expected}, got {actual}); the file was deleted")
            }
            Self::NotPublished(id) => write!(f, "model {id} has no download location yet"),
            Self::InvalidArchive(why) => write!(f, "invalid model archive: {why}"),
            Self::Network(why) => write!(f, "download failed: {why}"),
            Self::Io(err) => write!(f, "file error: {err}"),
            Self::UnknownLanguage(code) => write!(f, "unknown language {code}"),
            Self::NotAvailable(code) => write!(f, "{code} is not downloaded yet"),
        }
    }
}

impl std::error::Error for ModelError {}

impl From<std::io::Error> for ModelError {
    fn from(err: std::io::Error) -> Self {
        Self::Io(err)
    }
}

impl From<zip::result::ZipError> for ModelError {
    fn from(err: zip::result::ZipError) -> Self {
        Self::InvalidArchive(err.to_string())
    }
}

pub type Result<T> = std::result::Result<T, ModelError>;
