use std::{
    fs,
    path::{Path, PathBuf},
};

use serde::{Deserialize, Serialize};

use muxsift_core::{Error, Result};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub paths: PathsConfig,
    pub download: DownloadConfig,
    pub asr: AsrConfig,
    pub bcut: BcutConfig,
    pub ai: AiConfig,
    pub ui: UiConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AsrConfig {
    pub backend: String,
    pub buzz: BuzzConfig,
}

impl Default for AsrConfig {
    fn default() -> Self {
        Self {
            backend: "bcut".into(),
            buzz: BuzzConfig::default(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BuzzConfig {
    pub project_dir: PathBuf,
    pub uv: PathBuf,
    pub bridge: PathBuf,
    pub model: String,
    pub device: String,
    pub compute_type: String,
    pub language: Option<String>,
}

impl Default for BuzzConfig {
    fn default() -> Self {
        Self {
            project_dir: PathBuf::new(),
            uv: PathBuf::from("uv"),
            bridge: PathBuf::from("auto"),
            model: "large-v3-turbo".into(),
            device: "auto".into(),
            compute_type: "default".into(),
            language: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct AiConfig {
    pub enabled: bool,
    pub base_url: String,
    pub model: String,
    pub api_key_env: String,
    pub system_prompt: String,
}

impl Default for AiConfig {
    fn default() -> Self {
        Self { enabled: false, base_url: "https://api.openai.com/v1".into(), model: String::new(), api_key_env: "OPENAI_API_KEY".into(), system_prompt: "你是一位专业的中文及多语种文字编辑与转写校对员。将用户提供的 SRT 字幕整理成干净、准确、按内容分段的 Markdown 笔记。删除序号和时间轴，合并碎片字幕，清理无意义口语、重复和明显错字，补全标点并规范中英文与数字。不得编造事实；不确定处标注 [疑似：xxx]。用 # 标题和必要的 ## 小标题组织内容，只在标题或小标题旁边加时间戳，只输出 Markdown。请一次性通读全部输入后再输出。".into() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UiConfig {
    pub language: String,
}

impl Default for UiConfig {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct BcutConfig {
    pub cooldown_seconds: u64,
}

impl Default for BcutConfig {
    fn default() -> Self {
        Self {
            cooldown_seconds: 20,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadConfig {
    pub backend: String,
    pub aria2: PathBuf,
    pub max_concurrent_downloads: usize,
    pub split: u8,
    pub min_split_size: String,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            backend: "aria2".into(),
            aria2: PathBuf::from("auto"),
            max_concurrent_downloads: 3,
            split: 10,
            min_split_size: "15M".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PathsConfig {
    pub video: PathBuf,
    pub audio: PathBuf,
    pub subtitle: PathBuf,
    pub transcript: PathBuf,
}

impl Default for PathsConfig {
    fn default() -> Self {
        let root = directories::UserDirs::new()
            .and_then(|dirs| dirs.download_dir().map(Path::to_owned))
            .unwrap_or_else(|| PathBuf::from("."));
        Self {
            video: root.join("MuxSift").join("videos"),
            audio: root.join("MuxSift").join("audio"),
            subtitle: root.join("MuxSift").join("subtitles"),
            transcript: root.join("MuxSift").join("transcripts"),
        }
    }
}

pub fn default_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "muxsift", "muxsift")
        .map(|dirs| dirs.config_dir().join("config.json"))
        .unwrap_or_else(|| PathBuf::from(".muxsift/config.json"))
}

pub fn load(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str(&contents).map_err(|error| {
            Error::InvalidInput(format!("invalid config {}: {error}", path.display()))
        }),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Config::default()),
        Err(source) => Err(Error::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

pub fn write_default(path: &Path) -> Result<()> {
    if path.exists() {
        return Err(Error::InvalidInput(format!(
            "config already exists: {}",
            path.display()
        )));
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let contents = serde_json::to_string_pretty(&Config::default())
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    fs::write(path, contents).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

pub fn output_dir<'a>(config: &'a Config, mode: &str) -> &'a Path {
    match mode {
        "audio-only" => &config.paths.audio,
        "subtitle" => &config.paths.subtitle,
        "transcribe" => &config.paths.transcript,
        _ => &config.paths.video,
    }
}
