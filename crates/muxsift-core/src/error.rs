use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("invalid input: {0}")]
    InvalidInput(String),
    #[error("page {requested} is out of range; video has {available} page(s)")]
    PageOutOfRange { requested: usize, available: usize },
    #[error("network request failed for {endpoint}: {source}")]
    Network {
        endpoint: String,
        #[source]
        source: reqwest::Error,
    },
    #[error("Bilibili API {endpoint} returned code {code}: {message}")]
    Api {
        endpoint: String,
        code: i64,
        message: String,
    },
    #[error("unexpected response from {endpoint}: {detail}")]
    Protocol { endpoint: String, detail: String },
    #[error("requested quality {0} is not available; returned: {1:?}")]
    QualityUnavailable(u64, Vec<u64>),
    #[error("video has no downloadable DASH video stream")]
    NoVideoStream,
    #[error("video has no downloadable DASH audio stream")]
    NoAudioStream,
    #[error("not logged in; run `muxsift login`")]
    NotLoggedIn,
    #[error("QR login expired; run login again")]
    LoginExpired,
    #[error("QR login timed out")]
    LoginTimeout,
    #[error("I/O failed for {path}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("could not serialize credentials: {0}")]
    CredentialFormat(#[from] serde_json::Error),
    #[error("all media URLs failed: {0}")]
    Download(String),
    #[error("aria2c was not found: {0}")]
    Aria2Missing(String),
    #[error("aria2 download failed: {0}")]
    Aria2(String),
    #[error("Bcut transcription failed: {0}")]
    Bcut(String),
    #[error("Buzz local transcription failed: {0}")]
    Buzz(String),
    #[error("subtitle operation failed: {0}")]
    Subtitle(String),
    #[error(
        "FFmpeg was not found: {0}. Use the complete release package, --ffmpeg /path/to/ffmpeg, or install ffmpeg"
    )]
    FfmpegMissing(String),
    #[error("FFmpeg failed with exit code {code:?}: {stderr}")]
    FfmpegFailed { code: Option<i32>, stderr: String },
    #[error("task cancelled")]
    Cancelled,
}

pub type Result<T> = std::result::Result<T, Error>;
