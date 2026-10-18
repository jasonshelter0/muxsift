mod ai;
mod aria2;
mod auth;
mod bcut;
mod buzz;
mod client;
mod download;
mod error;
mod fetch;
mod frames;
mod input;
mod local;
mod models;
mod output;
mod resume;
mod subtitle;

pub use ai::{AiClient, AiOptions};
pub use aria2::{Aria2Manager, Aria2Options};
pub use auth::{LoginPoll, LoginQr};
pub use bcut::{
    BcutProgress, TranscriptFormat, set_bcut_cooldown_seconds, transcribe_audio,
    transcribe_audio_with_progress,
};
pub use buzz::{AsrBackend, BuzzOptions, BuzzProgress, transcribe_audio_with_buzz};
pub use client::{BiliClient, ClientOptions};
pub use download::{DownloadControl, DownloadProgress, mux, mux_audio};
pub use error::{Error, Result};
pub use fetch::{FetchOptions, fetch_with_yt_dlp};
pub use frames::{FrameEntry, FrameManifest, FrameOptions, extract_scene_frames};
pub use input::{VideoInput, parse_video_input, resolve_video_input};
pub use local::{LocalMediaEvent, LocalMediaOutput, LocalMediaRequest, process_local_media};
pub use models::{
    DownloadPlan, PageInfo, Stream, UgcArc, UgcEpisode, UgcSeason, UgcSection, VideoInfo,
};
pub use output::{final_output_path, final_output_path_with_extension, sanitize_filename};
pub use resume::ResumeWorkspace;
pub use subtitle::{SubtitleFormat, SubtitleTrack};
