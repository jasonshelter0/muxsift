use std::{
    collections::{HashMap, HashSet, VecDeque},
    io::{self, Read},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use crossterm::{
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use muxsift_core::{
    AiClient, AiOptions, Aria2Manager, Aria2Options, AsrBackend, BcutProgress, BiliClient,
    BuzzOptions, BuzzProgress, ClientOptions, DownloadControl, DownloadProgress, Error,
    LocalMediaEvent, LocalMediaRequest, ResumeWorkspace, SubtitleFormat, TranscriptFormat,
    VideoInput, final_output_path_with_extension, mux, mux_audio, process_local_media,
    resolve_video_input, sanitize_filename, set_bcut_cooldown_seconds, transcribe_audio_with_buzz,
};
use qrcode::{QrCode, render::unicode};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Position},
    style::{Color, Style},
    text::{Line, Span, Text},
    widgets::{Block, Borders, Paragraph, Wrap},
    widgets::{Clear, Padding},
};
use serde_json::Value;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

#[derive(Clone)]
pub struct TuiOptions {
    pub client: ClientOptions,
    pub video_output: PathBuf,
    pub audio_output: PathBuf,
    pub subtitle_output: PathBuf,
    pub transcript_output: PathBuf,
    pub ffmpeg: PathBuf,
    pub backend: MediaBackend,
    pub aria2_options: Aria2Options,
    pub aria2_manager: Option<Aria2Manager>,
    pub bcut_cooldown_seconds: u64,
    pub asr_backend: AsrBackend,
    pub buzz: BuzzOptions,
    pub ai: Option<AiOptions>,
    pub ai_enabled: bool,
    pub ai_base_url: String,
    pub ai_model: String,
    pub ai_api_key_env: String,
    pub ai_system_prompt: String,
    pub language: Language,
    pub config_path: PathBuf,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    English,
    Chinese,
}

impl Language {
    pub fn from_config(value: &str) -> Self {
        match value.trim().to_ascii_lowercase().as_str() {
            "zh" | "zh-cn" | "zh_hans" | "chinese" => Self::Chinese,
            _ => Self::English,
        }
    }

    pub fn config_value(self) -> &'static str {
        match self {
            Self::English => "en",
            Self::Chinese => "zh-CN",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::English => "English",
            Self::Chinese => "中文",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::English => Self::Chinese,
            Self::Chinese => Self::English,
        }
    }
}

fn tr<'a>(language: Language, english: &'a str, chinese: &'a str) -> &'a str {
    match language {
        Language::English => english,
        Language::Chinese => chinese,
    }
}

#[derive(Debug, Clone, Copy)]
pub enum MediaBackend {
    Http,
    Aria2,
}

impl MediaBackend {
    fn label(self) -> &'static str {
        match self {
            Self::Http => "HTTP",
            Self::Aria2 => "aria2",
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Http => Self::Aria2,
            Self::Aria2 => Self::Http,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConfigField {
    Video,
    Audio,
    Subtitle,
    Transcript,
    BcutCooldown,
}

impl ConfigField {
    fn key(self) -> &'static str {
        match self {
            Self::Video => "video",
            Self::Audio => "audio",
            Self::Subtitle => "subtitle",
            Self::Transcript => "transcript",
            Self::BcutCooldown => "cooldown_seconds",
        }
    }

    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Video => tr(language, "video output", "视频输出目录"),
            Self::Audio => tr(language, "audio output", "音频输出目录"),
            Self::Subtitle => tr(language, "subtitle output", "字幕输出目录"),
            Self::Transcript => tr(language, "transcript output", "转写稿输出目录"),
            Self::BcutCooldown => tr(language, "Bcut cooldown (seconds)", "Bcut 间隔（秒）"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsSection {
    General,
    Asr,
    Ai,
    Paths,
}

impl SettingsSection {
    fn label(self, language: Language) -> &'static str {
        match self {
            Self::General => tr(language, "General", "通用"),
            Self::Asr => tr(language, "Transcription", "转写"),
            Self::Ai => "AI",
            Self::Paths => tr(language, "Output paths", "输出目录"),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::General => Self::Asr,
            Self::Asr => Self::Ai,
            Self::Ai => Self::Paths,
            Self::Paths => Self::General,
        }
    }

    fn previous(self) -> Self {
        match self {
            Self::General => Self::Paths,
            Self::Asr => Self::General,
            Self::Ai => Self::Asr,
            Self::Paths => Self::Ai,
        }
    }

    fn fields(self) -> &'static [SettingsField] {
        use SettingsField::*;
        match self {
            Self::General => &[Language, DownloadBackend],
            Self::Asr => &[
                AsrBackend,
                BcutCooldown,
                BuzzProject,
                BuzzUv,
                BuzzBridge,
                BuzzModel,
                BuzzDevice,
                BuzzComputeType,
                BuzzLanguage,
            ],
            Self::Ai => &[
                Ai(AiConfigField::Enabled),
                Ai(AiConfigField::BaseUrl),
                Ai(AiConfigField::Model),
                Ai(AiConfigField::KeyEnvironment),
                Ai(AiConfigField::SystemPrompt),
            ],
            Self::Paths => &[
                Path(ConfigField::Video),
                Path(ConfigField::Audio),
                Path(ConfigField::Subtitle),
                Path(ConfigField::Transcript),
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsField {
    Language,
    DownloadBackend,
    AsrBackend,
    BcutCooldown,
    BuzzProject,
    BuzzUv,
    BuzzBridge,
    BuzzModel,
    BuzzDevice,
    BuzzComputeType,
    BuzzLanguage,
    Ai(AiConfigField),
    Path(ConfigField),
}

impl SettingsField {
    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Language => tr(language, "Interface language", "界面语言"),
            Self::DownloadBackend => tr(language, "Download backend", "下载后端"),
            Self::AsrBackend => tr(language, "ASR backend", "转写后端"),
            Self::BcutCooldown => tr(language, "Bcut cooldown (seconds)", "Bcut 间隔（秒）"),
            Self::BuzzProject => tr(language, "Buzz project directory", "Buzz 项目目录"),
            Self::BuzzUv => tr(language, "Buzz uv executable", "Buzz uv 程序"),
            Self::BuzzBridge => tr(language, "Buzz bridge script", "Buzz 桥接脚本"),
            Self::BuzzModel => tr(language, "Buzz model", "Buzz 模型"),
            Self::BuzzDevice => tr(language, "Buzz device", "Buzz 设备"),
            Self::BuzzComputeType => tr(language, "Buzz compute type", "Buzz 计算类型"),
            Self::BuzzLanguage => tr(
                language,
                "Buzz language (blank = auto)",
                "Buzz 语言（留空自动）",
            ),
            Self::Ai(field) => field.label(language),
            Self::Path(field) => field.label(language),
        }
    }

    fn buzz_key(self) -> Option<&'static str> {
        match self {
            Self::BuzzProject => Some("project_dir"),
            Self::BuzzUv => Some("uv"),
            Self::BuzzBridge => Some("bridge"),
            Self::BuzzModel => Some("model"),
            Self::BuzzDevice => Some("device"),
            Self::BuzzComputeType => Some("compute_type"),
            Self::BuzzLanguage => Some("language"),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    Video,
    Audio,
    Subtitle,
    Transcript,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AiConfigField {
    Enabled,
    BaseUrl,
    Model,
    KeyEnvironment,
    SystemPrompt,
}

impl AiConfigField {
    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Enabled => tr(language, "Enable AI post-processing", "启用 AI 后处理"),
            Self::BaseUrl => tr(language, "API base URL", "API 基础地址"),
            Self::Model => tr(language, "Model", "模型"),
            Self::KeyEnvironment => {
                tr(language, "API key environment variable", "API 密钥环境变量")
            }
            Self::SystemPrompt => tr(language, "System prompt", "系统提示词"),
        }
    }
}

impl Mode {
    fn label(self, language: Language) -> &'static str {
        match self {
            Self::Video => tr(language, "Video + Audio", "视频 + 音频"),
            Self::Audio => tr(language, "Audio", "仅音频"),
            Self::Subtitle => tr(language, "Native Subtitle", "原生字幕"),
            Self::Transcript => tr(language, "Transcript", "语音转写"),
        }
    }

    fn next(self) -> Self {
        match self {
            Self::Video => Self::Audio,
            Self::Audio => Self::Subtitle,
            Self::Subtitle => Self::Transcript,
            Self::Transcript => Self::Video,
        }
    }
}

#[derive(Debug)]
struct Task {
    id: usize,
    input: String,
    local: bool,
    status: String,
    title: String,
    mode: Mode,
    selected: bool,
    running: bool,
    quality: Option<u64>,
    codec: Option<String>,
    page: usize,
    subtitle_format: SubtitleFormat,
    transcript_format: TranscriptFormat,
    control: Option<DownloadControl>,
    grouped: bool,
    group_first: bool,
    group_last: bool,
    group_output_name: Option<String>,
    group_key: Option<String>,
    local_output_subdir: Option<PathBuf>,
    transfers: HashMap<String, TransferProgress>,
    buzz_progress: Option<BuzzTaskProgress>,
    ai_output: Option<PathBuf>,
}

#[derive(Debug)]
struct TransferProgress {
    downloaded: u64,
    total: Option<u64>,
    observed_at: Instant,
    bytes_per_second: f64,
}

#[derive(Debug, Clone, Copy)]
struct BuzzTaskProgress {
    processed_ms: u64,
    total_ms: u64,
}

enum AppEvent {
    AuthStatus(String),
    AuthQr {
        qr: String,
        url: String,
    },
    Planned {
        index: usize,
        title: String,
        pages: usize,
    },
    Phase {
        index: usize,
        message: String,
    },
    Progress {
        index: usize,
        label: String,
        progress: DownloadProgress,
    },
    BuzzProgress {
        index: usize,
        processed_seconds: f64,
        total_seconds: f64,
        elapsed_seconds: f64,
    },
    ExistingSrt {
        index: usize,
        path: PathBuf,
        response: tokio::sync::oneshot::Sender<bool>,
    },
    Finished {
        index: usize,
        path: PathBuf,
        ai_output: Option<PathBuf>,
    },
    Failed {
        index: usize,
        message: String,
    },
    PageList {
        raw: String,
        title: String,
        author: String,
        collection: Option<String>,
        pages: Vec<muxsift_core::PageInfo>,
    },
    LocalList {
        root: PathBuf,
        files: Vec<LocalFile>,
    },
    LocalListFailed(String),
    AiTestResult(String),
}

struct ExistingSrtPrompt {
    index: usize,
    path: PathBuf,
    use_existing: bool,
    response: tokio::sync::oneshot::Sender<bool>,
}

struct PagePicker {
    raw: String,
    title: String,
    author: String,
    collection: Option<String>,
    pages: Vec<muxsift_core::PageInfo>,
    local_files: Option<Vec<LocalFile>>,
    collapsed_local_dirs: HashSet<PathBuf>,
    selected: Vec<bool>,
    groups: Vec<PageGroup>,
    collapsed_groups: HashSet<usize>,
    collapsed_episodes: HashSet<(usize, usize)>,
    cursor: usize,
    search: String,
    search_cursor: usize,
    searching: bool,
}

#[derive(Debug, Clone)]
struct LocalFile {
    path: PathBuf,
    relative: PathBuf,
}

#[derive(Debug)]
struct PageGroup {
    title: Option<String>,
    episodes: Vec<PickerEpisode>,
}

#[derive(Debug)]
struct PickerEpisode {
    title: Option<String>,
    pages: Vec<usize>,
}

#[derive(Debug, Clone)]
enum PickerRow {
    Group(usize),
    Episode(usize, usize),
    Page { index: usize, nested: bool },
    LocalDirectory { relative: PathBuf, depth: usize },
    LocalFile { index: usize, depth: usize },
}

type PageListResult = (
    String,
    String,
    String,
    Option<String>,
    Vec<muxsift_core::PageInfo>,
);

fn is_supported_local_media(path: &Path) -> bool {
    let Some(extension) = path.extension().and_then(|extension| extension.to_str()) else {
        return false;
    };
    match extension.to_ascii_lowercase().as_str() {
        "ts" => {
            let Ok(mut file) = std::fs::File::open(path) else {
                return false;
            };
            let mut header = [0u8; 189];
            file.read_exact(&mut header).is_ok() && header[0] == 0x47 && header[188] == 0x47
        }
        "mp4" | "mkv" | "mov" | "webm" | "m4v" | "avi" | "flv" | "m2ts" | "mts" | "mpeg"
        | "mpg" | "wmv" | "3gp" | "mp3" | "wav" | "m4a" | "aac" | "flac" | "ogg" | "opus"
        | "wma" | "aiff" | "aif" | "mka" | "ape" | "amr" => true,
        _ => false,
    }
}

fn scan_local_media_directory(root: &Path) -> io::Result<(PathBuf, Vec<LocalFile>)> {
    let root = root.canonicalize()?;
    let mut pending = vec![root.clone()];
    let mut files = Vec::new();
    while let Some(directory) = pending.pop() {
        let mut entries = std::fs::read_dir(&directory)?.collect::<io::Result<Vec<_>>>()?;
        entries.sort_by_key(|entry| entry.file_name());
        for entry in entries.into_iter().rev() {
            let path = entry.path();
            let file_type = entry.file_type()?;
            if file_type.is_dir() {
                if entry.file_name() != ".muxsift-work" {
                    pending.push(path);
                }
            } else if file_type.is_file()
                && is_supported_local_media(&path)
                && path.file_stem().and_then(|stem| stem.to_str()).is_some()
                && entry.metadata()?.len() > 0
            {
                files.push(LocalFile {
                    relative: path
                        .strip_prefix(&root)
                        .expect("scanned path is under root")
                        .to_owned(),
                    path,
                });
            }
        }
    }
    files.sort_by(|left, right| left.relative.cmp(&right.relative));
    Ok((root, files))
}

fn local_batch_output_subdir(root_name: &str, relative: &Path, duplicate_stem: bool) -> PathBuf {
    let mut output = PathBuf::from(sanitize_filename(root_name));
    if let Some(parent) = relative.parent() {
        for component in parent.components() {
            if let std::path::Component::Normal(name) = component {
                output.push(sanitize_filename(&name.to_string_lossy()));
            }
        }
    }
    if duplicate_stem {
        let name = relative.file_name().expect("local file has a name");
        output.push(sanitize_filename(&name.to_string_lossy()));
    }
    output
}

fn local_directories(files: &[LocalFile]) -> HashSet<PathBuf> {
    let mut directories = HashSet::new();
    for file in files {
        let mut relative = PathBuf::new();
        if let Some(parent) = file.relative.parent() {
            for component in parent.components() {
                relative.push(component.as_os_str());
                directories.insert(relative.clone());
            }
        }
    }
    directories
}

fn local_directory_counts(
    files: &[LocalFile],
    selected: &[bool],
) -> HashMap<PathBuf, (usize, usize)> {
    let mut counts = HashMap::new();
    for (file, is_selected) in files.iter().zip(selected) {
        let mut relative = PathBuf::new();
        if let Some(parent) = file.relative.parent() {
            for component in parent.components() {
                relative.push(component.as_os_str());
                let count = counts.entry(relative.clone()).or_insert((0, 0));
                count.0 += 1;
                if *is_selected {
                    count.1 += 1;
                }
            }
        }
    }
    counts
}

fn page_groups(pages: &[muxsift_core::PageInfo], show_sections: bool) -> Vec<PageGroup> {
    let mut groups: Vec<PageGroup> = Vec::new();
    for (index, page) in pages.iter().enumerate() {
        let group_title = show_sections.then(|| {
            page.section
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Episodes")
                .to_owned()
        });
        let episode_title = show_sections.then(|| {
            page.episode
                .as_deref()
                .filter(|title| !title.trim().is_empty())
                .unwrap_or("Untitled video")
                .to_owned()
        });
        if groups.last().is_none_or(|group| group.title != group_title) {
            groups.push(PageGroup {
                title: group_title,
                episodes: Vec::new(),
            });
        }
        let group = groups.last_mut().expect("a group was just created");
        if let Some(episode) = group
            .episodes
            .last_mut()
            .filter(|episode| episode.title == episode_title)
        {
            episode.pages.push(index);
        } else {
            group.episodes.push(PickerEpisode {
                title: episode_title,
                pages: vec![index],
            });
        }
    }
    groups
}

fn picker_text_matches(query: &str, value: Option<&str>) -> bool {
    value.is_some_and(|value| value.to_lowercase().contains(query))
}

fn picker_page_matches(picker: &PagePicker, index: usize) -> bool {
    let query = picker.search.trim().to_lowercase();
    if query.is_empty() {
        return true;
    }
    let page = &picker.pages[index];
    page.part.to_lowercase().contains(&query)
        || picker_text_matches(&query, page.section.as_deref())
        || picker_text_matches(&query, page.episode.as_deref())
        || page.page.to_string().contains(&query)
        || page
            .episode_page
            .is_some_and(|number| number.to_string().contains(&query))
}

fn picker_result_pages(picker: &PagePicker) -> Vec<usize> {
    let query = picker.search.trim().to_lowercase();
    if let Some(files) = &picker.local_files {
        return files
            .iter()
            .enumerate()
            .filter(|(_, file)| {
                query.is_empty()
                    || file
                        .relative
                        .to_string_lossy()
                        .to_lowercase()
                        .contains(&query)
            })
            .map(|(index, _)| index)
            .collect();
    }
    if query.is_empty() {
        return (0..picker.pages.len()).collect();
    }
    let mut pages = Vec::new();
    for group in &picker.groups {
        let group_matches = picker_text_matches(&query, group.title.as_deref());
        for episode in &group.episodes {
            let episode_matches =
                group_matches || picker_text_matches(&query, episode.title.as_deref());
            pages.extend(
                episode
                    .pages
                    .iter()
                    .copied()
                    .filter(|index| episode_matches || picker_page_matches(picker, *index)),
            );
        }
    }
    pages
}

fn picker_rows(picker: &PagePicker) -> Vec<PickerRow> {
    if let Some(files) = &picker.local_files {
        let mut rows = Vec::new();
        let mut seen = HashSet::new();
        let searching = !picker.search.trim().is_empty();
        for index in picker_result_pages(picker) {
            let file = &files[index];
            let mut directory = PathBuf::new();
            let mut collapsed = false;
            let mut depth = 0;
            if let Some(parent) = file.relative.parent() {
                for component in parent.components() {
                    directory.push(component.as_os_str());
                    if seen.insert(directory.clone()) {
                        rows.push(PickerRow::LocalDirectory {
                            relative: directory.clone(),
                            depth,
                        });
                    }
                    depth += 1;
                    if !searching && picker.collapsed_local_dirs.contains(&directory) {
                        collapsed = true;
                        break;
                    }
                }
            }
            if !collapsed {
                rows.push(PickerRow::LocalFile { index, depth });
            }
        }
        return rows;
    }
    let mut rows = Vec::new();
    for (group_index, group) in picker.groups.iter().enumerate() {
        let query = picker.search.trim().to_lowercase();
        let group_matches = query.is_empty() || picker_text_matches(&query, group.title.as_deref());
        let matching_episodes: Vec<_> = group
            .episodes
            .iter()
            .enumerate()
            .filter(|(_, episode)| {
                group_matches
                    || picker_text_matches(&query, episode.title.as_deref())
                    || episode
                        .pages
                        .iter()
                        .any(|index| picker_page_matches(picker, *index))
            })
            .collect();
        if matching_episodes.is_empty() {
            continue;
        }
        if group.title.is_some() {
            rows.push(PickerRow::Group(group_index));
        }
        if !picker.collapsed_groups.contains(&group_index) || !query.is_empty() {
            for (episode_index, episode) in matching_episodes {
                let episode_matches =
                    group_matches || picker_text_matches(&query, episode.title.as_deref());
                if episode.title.is_some() {
                    rows.push(PickerRow::Episode(group_index, episode_index));
                }
                if !picker
                    .collapsed_episodes
                    .contains(&(group_index, episode_index))
                    || !query.is_empty()
                {
                    rows.extend(
                        episode
                            .pages
                            .iter()
                            .copied()
                            .filter(|index| episode_matches || picker_page_matches(picker, *index))
                            .map(|index| PickerRow::Page {
                                index,
                                nested: group.title.is_some(),
                            }),
                    );
                }
            }
        }
    }
    rows
}

impl PagePicker {
    fn insert_search(&mut self, text: &str) {
        let mut chars: Vec<char> = self.search.chars().collect();
        let at = self.search_cursor.min(chars.len());
        let inserted: Vec<char> = text.chars().collect();
        chars.splice(at..at, inserted.iter().copied());
        self.search = chars.into_iter().collect();
        self.search_cursor = at + inserted.len();
        self.cursor = 0;
    }

    fn backspace_search(&mut self) {
        let mut chars: Vec<char> = self.search.chars().collect();
        let at = self.search_cursor.min(chars.len());
        if at > 0 {
            chars.remove(at - 1);
            self.search_cursor = at - 1;
            self.search = chars.into_iter().collect();
            self.cursor = 0;
        }
    }

    fn delete_search(&mut self) {
        let mut chars: Vec<char> = self.search.chars().collect();
        let at = self.search_cursor.min(chars.len());
        if at < chars.len() {
            chars.remove(at);
            self.search = chars.into_iter().collect();
            self.cursor = 0;
        }
    }

    fn select_search_results(&mut self) {
        let pages = picker_result_pages(self);
        for index in pages {
            self.selected[index] = true;
        }
    }

    fn invert_search_results(&mut self) {
        let pages = picker_result_pages(self);
        for index in pages {
            self.selected[index] = !self.selected[index];
        }
    }
}

struct App {
    config_path: PathBuf,
    language: Language,
    input: String,
    input_cursor: usize,
    editing: bool,
    mode: Mode,
    tasks: Vec<Task>,
    next_id: usize,
    cursor: usize,
    editing_task: Option<usize>,
    auth_status: String,
    auth_qr: Option<String>,
    auth_busy: bool,
    help: bool,
    help_scroll: u16,
    quality: Option<u64>,
    codec: Option<String>,
    config_field: Option<ConfigField>,
    settings_open: bool,
    settings_section: SettingsSection,
    settings_index: usize,
    settings_editing: Option<SettingsField>,
    ai_editing: Option<AiConfigField>,
    settings_message: String,
    ai_enabled: bool,
    ai_base_url: String,
    ai_model: String,
    ai_api_key_env: String,
    ai_prompt_preview: String,
    collapsed_groups: HashSet<String>,
    page_picker: Option<PagePicker>,
    existing_srt_prompts: VecDeque<ExistingSrtPrompt>,
    show_welcome: bool,
    animation_tick: usize,
    events: tokio::sync::mpsc::UnboundedReceiver<AppEvent>,
    sender: tokio::sync::mpsc::UnboundedSender<AppEvent>,
}

impl App {
    fn new(config_path: PathBuf, language: Language) -> Self {
        let (sender, events) = tokio::sync::mpsc::unbounded_channel();
        Self {
            config_path,
            language,
            input: String::new(),
            input_cursor: 0,
            editing: false,
            mode: Mode::Video,
            tasks: Vec::new(),
            next_id: 0,
            cursor: 0,
            editing_task: None,
            auth_status: "checking".into(),
            auth_qr: None,
            auth_busy: false,
            help: false,
            help_scroll: 0,
            quality: None,
            codec: None,
            config_field: None,
            settings_open: false,
            settings_section: SettingsSection::General,
            settings_index: 0,
            settings_editing: None,
            ai_editing: None,
            settings_message: String::new(),
            ai_enabled: false,
            ai_base_url: String::new(),
            ai_model: String::new(),
            ai_api_key_env: String::new(),
            ai_prompt_preview: String::new(),
            collapsed_groups: HashSet::new(),
            page_picker: None,
            existing_srt_prompts: VecDeque::new(),
            show_welcome: true,
            animation_tick: 0,
            events,
            sender,
        }
    }

    fn submit(&mut self, options: &mut TuiOptions) {
        let raw = self.input.trim().to_owned();
        if raw.is_empty()
            && self.ai_editing != Some(AiConfigField::SystemPrompt)
            && self.settings_editing != Some(SettingsField::BuzzLanguage)
        {
            return;
        }
        if let Some(field) = self.settings_editing {
            self.save_buzz_field(options, field, raw);
            return;
        }
        if let Some(field) = self.ai_editing {
            self.save_ai_field(options, field, raw);
            return;
        }
        if let Some(field) = self.config_field {
            if field == ConfigField::BcutCooldown {
                match raw.parse::<u64>() {
                    Ok(seconds) => match update_config_bcut_cooldown(&self.config_path, seconds) {
                        Ok(()) => {
                            options.bcut_cooldown_seconds = seconds;
                            set_bcut_cooldown_seconds(seconds);
                            self.settings_message =
                                format!("saved {}: {seconds}s", field.label(self.language));
                            self.finish_settings_edit();
                        }
                        Err(error) => self.settings_message = format!("config error · {error}"),
                    },
                    Err(_) => {
                        self.settings_message =
                            "config error · cooldown must be a whole number of seconds".into()
                    }
                }
            } else {
                let path_value = expand_home_path(&raw);
                match update_config_path(&self.config_path, field, &path_value) {
                    Ok(()) => {
                        set_option_path(options, field, PathBuf::from(&path_value));
                        self.settings_message = format!("saved {}", field.label(self.language));
                        self.finish_settings_edit();
                    }
                    Err(error) => self.settings_message = format!("config error · {error}"),
                }
            }
            return;
        } else if let Some(id) = self.editing_task.take() {
            if let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) {
                if !task.running {
                    task.input = raw.clone();
                    task.local = Path::new(&raw).is_file();
                    task.title = raw;
                    task.mode = if task.local {
                        Mode::Transcript
                    } else {
                        self.mode
                    };
                    task.status = "queued · edited".into();
                }
            }
        } else {
            let local_path = PathBuf::from(expand_home_path(&raw));
            if local_path.is_dir() {
                self.request_local_directory(local_path);
            } else if local_path.is_file() {
                let id = self.next_id;
                self.next_id += 1;
                let title = local_path
                    .file_name()
                    .map(|name| name.to_string_lossy().into_owned())
                    .unwrap_or_else(|| raw.clone());
                self.tasks.push(Task {
                    id,
                    input: local_path.display().to_string(),
                    local: true,
                    title,
                    status: "queued · local media → SRT → AI notes".into(),
                    mode: Mode::Transcript,
                    selected: false,
                    running: false,
                    quality: None,
                    codec: None,
                    page: 1,
                    subtitle_format: SubtitleFormat::Srt,
                    transcript_format: TranscriptFormat::Srt,
                    control: None,
                    grouped: false,
                    group_first: false,
                    group_last: false,
                    group_output_name: None,
                    group_key: None,
                    local_output_subdir: None,
                    transfers: HashMap::new(),
                    buzz_progress: None,
                    ai_output: None,
                });
                self.cursor = self.tasks.len() - 1;
                self.show_welcome = false;
            } else if local_path.exists()
                || raw.starts_with('/')
                || raw.starts_with("./")
                || raw.starts_with("../")
                || raw.starts_with("~/")
            {
                self.auth_status = format!("local file not found or not a regular file: {raw}");
            } else {
                self.request_pages(raw, options);
            }
        }
        self.input.clear();
        self.input_cursor = 0;
        self.editing = false;
    }

    fn open_settings(&mut self, section: SettingsSection) {
        self.settings_open = true;
        self.settings_section = section;
        self.settings_index = 0;
        self.settings_editing = None;
        self.config_field = None;
        self.ai_editing = None;
        self.editing = false;
        self.input.clear();
        self.input_cursor = 0;
        self.settings_message.clear();
    }

    fn close_settings(&mut self) {
        self.settings_open = false;
        self.settings_editing = None;
        self.config_field = None;
        self.ai_editing = None;
        self.editing = false;
        self.input.clear();
        self.input_cursor = 0;
    }

    fn finish_settings_edit(&mut self) {
        self.settings_editing = None;
        self.config_field = None;
        self.ai_editing = None;
        self.input.clear();
        self.input_cursor = 0;
        self.editing = false;
    }

    fn selected_settings_field(&self) -> SettingsField {
        self.settings_section.fields()[self.settings_index]
    }

    fn settings_value(&self, options: &TuiOptions, field: SettingsField) -> String {
        match field {
            SettingsField::Language => self.language.label().into(),
            SettingsField::DownloadBackend => options.backend.label().into(),
            SettingsField::AsrBackend => match options.asr_backend {
                AsrBackend::Bcut => "Bcut",
                AsrBackend::Buzz => "Buzz",
            }
            .into(),
            SettingsField::BcutCooldown => options.bcut_cooldown_seconds.to_string(),
            SettingsField::BuzzProject => options.buzz.project_dir.display().to_string(),
            SettingsField::BuzzUv => options.buzz.uv.display().to_string(),
            SettingsField::BuzzBridge => options.buzz.bridge.display().to_string(),
            SettingsField::BuzzModel => options.buzz.model.clone(),
            SettingsField::BuzzDevice => options.buzz.device.clone(),
            SettingsField::BuzzComputeType => options.buzz.compute_type.clone(),
            SettingsField::BuzzLanguage => options.buzz.language.clone().unwrap_or_default(),
            SettingsField::Ai(field) => self.ai_value(field),
            SettingsField::Path(field) => config_field_value(options, field),
        }
    }

    fn activate_settings_field(&mut self, options: &mut TuiOptions, field: SettingsField) {
        match field {
            SettingsField::Language => {
                let language = self.language.next();
                match update_config_language(&self.config_path, language) {
                    Ok(()) => {
                        self.language = language;
                        options.language = language;
                        self.settings_message = format!("saved {}", field.label(language));
                    }
                    Err(error) => self.settings_message = format!("config error · {error}"),
                }
            }
            SettingsField::DownloadBackend => {
                let backend = options.backend.next();
                if matches!(backend, MediaBackend::Aria2) && options.aria2_manager.is_none() {
                    match Aria2Manager::new(options.aria2_options.clone()) {
                        Ok(manager) => options.aria2_manager = Some(manager),
                        Err(error) => {
                            self.settings_message = format!("aria2 error · {error}");
                            return;
                        }
                    }
                }
                match update_config_backend(&self.config_path, backend) {
                    Ok(()) => {
                        options.backend = backend;
                        self.settings_message =
                            format!("saved {}: {}", field.label(self.language), backend.label());
                    }
                    Err(error) => self.settings_message = format!("config error · {error}"),
                }
            }
            SettingsField::AsrBackend => {
                let backend = match options.asr_backend {
                    AsrBackend::Bcut => AsrBackend::Buzz,
                    AsrBackend::Buzz => AsrBackend::Bcut,
                };
                match update_config_asr_backend(&self.config_path, backend) {
                    Ok(()) => {
                        options.asr_backend = backend;
                        self.settings_message = format!("saved {}", field.label(self.language));
                    }
                    Err(error) => self.settings_message = format!("config error · {error}"),
                }
            }
            SettingsField::Ai(ai_field) => {
                self.ai_editing = None;
                self.begin_ai_edit(options, ai_field);
            }
            SettingsField::Path(path_field) => {
                self.config_field = Some(path_field);
                self.input = config_field_value(options, path_field);
                self.input_cursor = self.input.chars().count();
                self.editing = true;
            }
            SettingsField::BcutCooldown => {
                self.config_field = Some(ConfigField::BcutCooldown);
                self.input = options.bcut_cooldown_seconds.to_string();
                self.input_cursor = self.input.chars().count();
                self.editing = true;
            }
            buzz_field => {
                self.settings_editing = Some(buzz_field);
                self.input = self.settings_value(options, buzz_field);
                self.input_cursor = self.input.chars().count();
                self.editing = true;
            }
        }
    }

    fn save_buzz_field(&mut self, options: &mut TuiOptions, field: SettingsField, value: String) {
        let value = if matches!(
            field,
            SettingsField::BuzzProject | SettingsField::BuzzUv | SettingsField::BuzzBridge
        ) {
            expand_home_path(&value)
        } else {
            value
        };
        match update_config_buzz(&self.config_path, field, &value) {
            Ok(()) => {
                match field {
                    SettingsField::BuzzProject => options.buzz.project_dir = PathBuf::from(value),
                    SettingsField::BuzzUv => options.buzz.uv = PathBuf::from(value),
                    SettingsField::BuzzBridge => options.buzz.bridge = PathBuf::from(value),
                    SettingsField::BuzzModel => options.buzz.model = value,
                    SettingsField::BuzzDevice => options.buzz.device = value,
                    SettingsField::BuzzComputeType => options.buzz.compute_type = value,
                    SettingsField::BuzzLanguage => {
                        options.buzz.language = (!value.is_empty()).then_some(value)
                    }
                    _ => unreachable!("only Buzz fields are edited here"),
                }
                self.settings_message = format!("saved {}", field.label(self.language));
                self.finish_settings_edit();
            }
            Err(error) => self.settings_message = format!("config error · {error}"),
        }
    }

    fn ai_value(&self, field: AiConfigField) -> String {
        match field {
            AiConfigField::Enabled => self.ai_enabled.to_string(),
            AiConfigField::BaseUrl => self.ai_base_url.clone(),
            AiConfigField::Model => self.ai_model.clone(),
            AiConfigField::KeyEnvironment => self.ai_api_key_env.clone(),
            AiConfigField::SystemPrompt => self.ai_prompt_preview.clone(),
        }
    }

    fn begin_ai_edit(&mut self, options: &mut TuiOptions, field: AiConfigField) {
        if field == AiConfigField::Enabled {
            let enabled = !options.ai_enabled;
            match update_config_ai(
                &self.config_path,
                AiConfigField::Enabled,
                &enabled.to_string(),
            ) {
                Ok(()) => {
                    options.ai_enabled = enabled;
                    self.ai_enabled = enabled;
                    refresh_ai_options(options);
                    self.settings_message =
                        format!("{}: {}", field.label(self.language), options.ai_enabled);
                }
                Err(error) => self.settings_message = format!("config error · {error}"),
            }
            return;
        }
        self.input = self.ai_value(field);
        self.input_cursor = self.input.chars().count();
        self.ai_editing = Some(field);
        self.editing = true;
    }

    fn save_ai_field(&mut self, options: &mut TuiOptions, field: AiConfigField, value: String) {
        match update_config_ai(&self.config_path, field, &value) {
            Ok(()) => {
                match field {
                    AiConfigField::Enabled => {}
                    AiConfigField::BaseUrl => options.ai_base_url = value,
                    AiConfigField::Model => options.ai_model = value,
                    AiConfigField::KeyEnvironment => options.ai_api_key_env = value,
                    AiConfigField::SystemPrompt => options.ai_system_prompt = value,
                }
                match field {
                    AiConfigField::Enabled => self.ai_enabled = options.ai_enabled,
                    AiConfigField::BaseUrl => self.ai_base_url = options.ai_base_url.clone(),
                    AiConfigField::Model => self.ai_model = options.ai_model.clone(),
                    AiConfigField::KeyEnvironment => {
                        self.ai_api_key_env = options.ai_api_key_env.clone()
                    }
                    AiConfigField::SystemPrompt => {
                        self.ai_prompt_preview = options.ai_system_prompt.clone()
                    }
                }
                refresh_ai_options(options);
                self.settings_message = format!("saved {}", field.label(self.language));
                self.finish_settings_edit();
            }
            Err(error) => self.settings_message = format!("config error · {error}"),
        }
    }

    fn test_ai_connection(&mut self, options: &TuiOptions) {
        let sender = self.sender.clone();
        let base_url = options.ai_base_url.clone();
        let model = options.ai_model.clone();
        let api_key_env = options.ai_api_key_env.clone();
        let system_prompt = options.ai_system_prompt.clone();
        self.settings_message = tr(
            self.language,
            "Testing AI connection...",
            "正在测试 AI 连接...",
        )
        .into();
        tokio::spawn(async move {
            let message = match std::env::var(&api_key_env) {
                Ok(api_key) => match AiClient::new(AiOptions {
                    base_url,
                    model: model.clone(),
                    api_key,
                    system_prompt,
                }) {
                    Ok(client) => match client.test_connection().await {
                        Ok(()) => format!("AI connection succeeded: {model}"),
                        Err(error) => format!("AI connection failed: {error}"),
                    },
                    Err(error) => format!("AI configuration error: {error}"),
                },
                Err(_) => format!("AI API key environment variable is not set: {api_key_env}"),
            };
            let _ = sender.send(AppEvent::AiTestResult(message));
        });
    }

    fn request_pages(&mut self, raw: String, options: &TuiOptions) {
        self.editing = false;
        self.input.clear();
        self.auth_status = "loading page list".into();
        let sender = self.sender.clone();
        let client_options = options.client.clone();
        tokio::spawn(async move {
            let result: Result<PageListResult, Error> = async {
                let client = BiliClient::new(client_options.clone())?;
                let input = resolve_video_input(
                    &raw,
                    None,
                    client_options.proxy.as_deref(),
                    client_options.no_proxy,
                )
                .await?;
                let video = client.video_info(&input.bvid).await?;
                Ok((
                    format!("https://www.bilibili.com/video/{}/", input.bvid),
                    video.title,
                    video.owner.name,
                    video.ugc_season.map(|season| season.name),
                    video.pages,
                ))
            }
            .await;
            match result {
                Ok((raw, title, author, collection, pages)) => {
                    let _ = sender.send(AppEvent::PageList {
                        raw,
                        title,
                        author,
                        collection,
                        pages,
                    });
                }
                Err(error) => {
                    let _ = sender.send(AppEvent::Failed {
                        index: usize::MAX,
                        message: error.to_string(),
                    });
                }
            }
        });
    }

    fn request_local_directory(&mut self, root: PathBuf) {
        self.auth_status = "scanning local folder".into();
        let sender = self.sender.clone();
        tokio::task::spawn_blocking(move || match scan_local_media_directory(&root) {
            Ok((root, files)) if !files.is_empty() => {
                let _ = sender.send(AppEvent::LocalList { root, files });
            }
            Ok(_) => {
                let _ = sender.send(AppEvent::LocalListFailed(
                    "no supported video or audio files found".into(),
                ));
            }
            Err(error) => {
                let _ = sender.send(AppEvent::LocalListFailed(format!(
                    "{}: {error}",
                    root.display()
                )));
            }
        });
    }

    fn confirm_pages(&mut self) {
        let Some(picker) = self.page_picker.take() else {
            return;
        };
        let total = picker.selected.iter().filter(|value| **value).count();
        if let Some(files) = &picker.local_files {
            let first_new_task = self.tasks.len();
            let mut collisions = HashMap::new();
            for (file, selected) in files.iter().zip(&picker.selected) {
                if *selected {
                    let parent = local_batch_output_subdir(&picker.title, &file.relative, false);
                    let stem = sanitize_filename(
                        &file
                            .relative
                            .file_stem()
                            .expect("scanned file has a stem")
                            .to_string_lossy(),
                    );
                    *collisions.entry(parent.join(stem)).or_insert(0usize) += 1;
                }
            }
            let mut position = 0;
            for (index, (file, selected)) in files.iter().zip(&picker.selected).enumerate() {
                if !selected {
                    continue;
                }
                let parent = local_batch_output_subdir(&picker.title, &file.relative, false);
                let stem = sanitize_filename(
                    &file
                        .relative
                        .file_stem()
                        .expect("scanned file has a stem")
                        .to_string_lossy(),
                );
                let duplicate = collisions
                    .get(&parent.join(stem))
                    .is_some_and(|count| *count > 1);
                let id = self.next_id;
                self.next_id += 1;
                self.tasks.push(Task {
                    id,
                    input: file.path.display().to_string(),
                    local: true,
                    title: file.relative.display().to_string(),
                    status: "queued · local media → SRT → AI notes".into(),
                    mode: Mode::Transcript,
                    selected: false,
                    running: false,
                    quality: None,
                    codec: None,
                    page: index + 1,
                    subtitle_format: SubtitleFormat::Srt,
                    transcript_format: TranscriptFormat::Srt,
                    control: None,
                    grouped: total > 1,
                    group_first: position == 0,
                    group_last: position + 1 == total,
                    group_output_name: None,
                    group_key: (total > 1).then(|| picker.raw.clone()),
                    local_output_subdir: Some(local_batch_output_subdir(
                        &picker.title,
                        &file.relative,
                        duplicate,
                    )),
                    transfers: HashMap::new(),
                    buzz_progress: None,
                    ai_output: None,
                });
                position += 1;
            }
            if total > 0 {
                self.show_welcome = false;
                self.cursor = first_new_task;
            }
            return;
        }
        let group_output_name = (total > 1).then(|| {
            picker
                .collection
                .as_deref()
                .filter(|name| !name.trim().is_empty())
                .unwrap_or(&picker.title)
                .to_owned()
        });
        let mut position = 0;
        let first_new_task = self.tasks.len();
        for (page, selected) in picker.pages.iter().zip(picker.selected) {
            if selected {
                let id = self.next_id;
                self.next_id += 1;
                self.tasks.push(Task {
                    id,
                    input: picker.raw.clone(),
                    local: false,
                    title: page.part.clone(),
                    status: "queued".into(),
                    mode: self.mode,
                    selected: false,
                    running: false,
                    quality: self.quality,
                    codec: self.codec.clone(),
                    page: page.page,
                    subtitle_format: SubtitleFormat::Srt,
                    transcript_format: TranscriptFormat::Srt,
                    control: None,
                    grouped: total > 1,
                    group_first: position == 0,
                    group_last: position + 1 == total,
                    group_output_name: group_output_name.clone(),
                    group_key: None,
                    local_output_subdir: None,
                    transfers: HashMap::new(),
                    buzz_progress: None,
                    ai_output: None,
                });
                position += 1;
            }
        }
        if total > 0 {
            self.show_welcome = false;
            self.cursor = first_new_task;
        }
    }

    fn insert_input(&mut self, text: &str) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.input_cursor.min(chars.len());
        let inserted: Vec<char> = text.chars().collect();
        chars.splice(at..at, inserted.iter().copied());
        self.input = chars.into_iter().collect();
        self.input_cursor = at + inserted.len();
    }

    fn backspace_input(&mut self) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.input_cursor.min(chars.len());
        if at > 0 {
            chars.remove(at - 1);
            self.input_cursor = at - 1;
            self.input = chars.into_iter().collect();
        }
    }

    fn delete_input(&mut self) {
        let mut chars: Vec<char> = self.input.chars().collect();
        let at = self.input_cursor.min(chars.len());
        if at < chars.len() {
            chars.remove(at);
            self.input = chars.into_iter().collect();
        }
    }

    fn start_task(&mut self, id: usize, options: &TuiOptions, resume: bool) {
        let Some(task) = self.tasks.iter_mut().find(|task| task.id == id) else {
            return;
        };
        if task.running || task.status.starts_with("completed") {
            return;
        }
        task.running = true;
        task.selected = false;
        task.status = "resolving metadata".into();
        task.transfers.clear();
        task.buzz_progress = None;
        let raw = task.input.clone();
        let local = task.local;
        let mode = task.mode;
        let quality = task.quality;
        let codec = task.codec.clone();
        let page = task.page;
        let subtitle_format = task.subtitle_format;
        let transcript_format = task.transcript_format;
        let group_output_name = task.group_output_name.clone();
        let local_output_subdir = task.local_output_subdir.clone();
        let control = DownloadControl::default();
        task.control = Some(control.clone());
        let sender = self.sender.clone();
        let client_options = options.client.clone();
        let task_options = options.clone();
        tokio::spawn(async move {
            let result = execute_task(
                &raw,
                local,
                mode,
                id,
                page,
                quality,
                codec.as_deref(),
                subtitle_format,
                transcript_format,
                group_output_name.as_deref(),
                local_output_subdir.as_deref(),
                &control,
                &sender,
                &client_options,
                &task_options,
                resume,
            )
            .await;
            match result {
                Ok(output) => {
                    let _ = sender.send(AppEvent::Finished {
                        index: id,
                        path: output.path,
                        ai_output: output.ai_output,
                    });
                }
                Err(error) => {
                    let message = if matches!(&error, Error::Cancelled) {
                        error.to_string()
                    } else {
                        format!("{error} · press r to retry from the last saved stage")
                    };
                    let _ = sender.send(AppEvent::Failed { index: id, message });
                }
            }
        });
    }

    fn start_current(&mut self, options: &TuiOptions) {
        if let Some(task) = self.tasks.get(self.cursor) {
            self.start_task(task.id, options, false);
        }
    }

    fn start_selected(&mut self, options: &TuiOptions) {
        let ids: Vec<usize> = self
            .tasks
            .iter()
            .filter(|task| task.selected)
            .map(|task| task.id)
            .collect();
        for id in ids {
            self.start_task(id, options, false);
        }
    }

    fn begin_edit(&mut self) {
        let Some(task) = self.tasks.get(self.cursor) else {
            return;
        };
        if task.running {
            return;
        }
        self.input = task.input.clone();
        self.input_cursor = self.input.chars().count();
        self.mode = task.mode;
        self.quality = task.quality;
        self.codec = task.codec.clone();
        self.editing_task = Some(task.id);
        self.editing = true;
    }

    fn delete_current(&mut self) {
        if self.tasks.get(self.cursor).is_some_and(|task| task.running) {
            return;
        }
        if self.cursor < self.tasks.len() {
            self.tasks.remove(self.cursor);
            self.cursor = self.cursor.min(self.tasks.len().saturating_sub(1));
        }
    }

    fn move_cursor(&mut self, delta: isize) {
        if self.tasks.is_empty() {
            return;
        }
        let last = self.tasks.len() - 1;
        let mut next = if delta < 0 {
            self.cursor.saturating_sub(delta.unsigned_abs())
        } else {
            (self.cursor + delta as usize).min(last)
        };
        while next < self.tasks.len() && self.is_hidden(next) {
            if delta < 0 {
                next = next.saturating_sub(1);
            } else if next < last {
                next += 1;
            } else {
                break;
            }
        }
        self.cursor = next;
    }

    fn is_hidden(&self, index: usize) -> bool {
        self.tasks.get(index).is_some_and(|task| {
            task.grouped
                && !task.group_first
                && self
                    .collapsed_groups
                    .contains(task.group_key.as_deref().unwrap_or(&task.input))
        })
    }

    fn toggle_current(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            if !task.running {
                task.selected = !task.selected;
            }
        }
    }

    fn toggle_all_tasks(&mut self) {
        let selectable = self.tasks.iter().filter(|task| !task.running).count();
        let marked = self.tasks.iter().filter(|task| task.selected).count();
        let select_all = marked < selectable;
        for task in &mut self.tasks {
            if !task.running {
                task.selected = select_all;
            }
        }
    }

    fn toggle_group(&mut self) {
        let Some(task) = self.tasks.get(self.cursor) else {
            return;
        };
        if !task.grouped {
            return;
        }
        let key = task.group_key.clone().unwrap_or_else(|| task.input.clone());
        let collapsing = !self.collapsed_groups.contains(&key);
        if collapsing {
            self.collapsed_groups.insert(key.clone());
            if let Some(first) = self.tasks.iter().position(|item| {
                item.group_key.as_deref().unwrap_or(&item.input) == key && item.group_first
            }) {
                self.cursor = first;
            }
        } else {
            self.collapsed_groups.remove(&key);
        }
    }

    fn retry_current(&mut self, options: &TuiOptions) {
        let id = self
            .tasks
            .get(self.cursor)
            .filter(|task| {
                task.status.starts_with("failed") || task.status.starts_with("cancelled")
            })
            .map(|task| task.id);
        if let Some(id) = id {
            self.start_task(id, options, true);
        }
    }

    fn cancel_current(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            if let Some(control) = &task.control {
                control.cancel();
                task.status = "cancelling".into();
            }
        }
    }

    fn toggle_pause_current(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            if task.local {
                return;
            }
            if let Some(control) = &task.control {
                let paused = control.toggle_pause();
                task.status = if paused { "paused" } else { "resuming" }.into();
            }
        }
    }

    fn cycle_quality(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            task.quality = match task.quality {
                None => Some(1080),
                Some(1080) => Some(720),
                Some(720) => Some(480),
                Some(480) => Some(360),
                Some(_) => None,
            };
            task.status = "queued · quality changed".into();
        }
    }

    fn cycle_codec(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            task.codec = match task.codec.as_deref() {
                None => Some("avc".into()),
                Some("avc") => Some("hevc".into()),
                Some("hevc") => Some("av1".into()),
                Some(_) => None,
            };
            task.status = "queued · codec changed".into();
        }
    }

    fn change_page(&mut self, delta: isize) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            task.page = if delta < 0 {
                task.page.saturating_sub(delta.unsigned_abs()).max(1)
            } else {
                task.page.saturating_add(delta as usize)
            };
            task.status = format!("queued · page {}", task.page);
        }
    }

    fn cycle_format(&mut self) {
        if let Some(task) = self.tasks.get_mut(self.cursor) {
            let format = match task.mode {
                Mode::Subtitle => {
                    task.subtitle_format = match task.subtitle_format {
                        SubtitleFormat::Srt => SubtitleFormat::Vtt,
                        SubtitleFormat::Vtt => SubtitleFormat::Srt,
                    };
                    Some(match task.subtitle_format {
                        SubtitleFormat::Srt => "SRT",
                        SubtitleFormat::Vtt => "VTT",
                    })
                }
                Mode::Transcript => {
                    task.transcript_format = match task.transcript_format {
                        TranscriptFormat::Srt => TranscriptFormat::Txt,
                        TranscriptFormat::Txt => TranscriptFormat::Lrc,
                        TranscriptFormat::Lrc => TranscriptFormat::Json,
                        TranscriptFormat::Json => TranscriptFormat::Srt,
                    };
                    Some(match task.transcript_format {
                        TranscriptFormat::Srt => "SRT",
                        TranscriptFormat::Txt => "TXT",
                        TranscriptFormat::Lrc => "LRC",
                        TranscriptFormat::Json => "JSON",
                    })
                }
                _ => None,
            };
            if let Some(format) = format {
                task.status = format!("queued · format: {format}");
            }
        }
    }

    fn drain_events(&mut self) {
        while let Ok(event) = self.events.try_recv() {
            match event {
                AppEvent::AuthStatus(status) => {
                    self.auth_status = status;
                    self.auth_qr = None;
                    self.auth_busy = false;
                }
                AppEvent::PageList {
                    raw,
                    title,
                    author,
                    collection,
                    pages,
                } => {
                    let groups = page_groups(&pages, collection.is_some());
                    let collapsed_groups = groups
                        .iter()
                        .enumerate()
                        .filter_map(|(index, group)| group.title.is_some().then_some(index))
                        .collect();
                    let collapsed_episodes = groups
                        .iter()
                        .enumerate()
                        .flat_map(|(group_index, group)| {
                            group.episodes.iter().enumerate().filter_map(
                                move |(episode_index, episode)| {
                                    episode
                                        .title
                                        .is_some()
                                        .then_some((group_index, episode_index))
                                },
                            )
                        })
                        .collect();
                    self.page_picker = Some(PagePicker {
                        selected: vec![pages.len() == 1; pages.len()],
                        pages,
                        local_files: None,
                        collapsed_local_dirs: HashSet::new(),
                        raw,
                        title,
                        author,
                        collection,
                        groups,
                        collapsed_groups,
                        collapsed_episodes,
                        cursor: 0,
                        search: String::new(),
                        search_cursor: 0,
                        searching: false,
                    });
                    self.auth_status = "select pages".into();
                }
                AppEvent::LocalList { root, files } => {
                    let title = root
                        .file_name()
                        .map(|name| name.to_string_lossy().into_owned())
                        .unwrap_or_else(|| root.display().to_string());
                    let pages = files
                        .iter()
                        .enumerate()
                        .map(|(index, file)| muxsift_core::PageInfo {
                            cid: 0,
                            page: index + 1,
                            part: file.relative.display().to_string(),
                            duration: 0,
                            dimension: None,
                            bvid: None,
                            aid: None,
                            section: None,
                            episode: None,
                            episode_page: None,
                        })
                        .collect();
                    self.page_picker = Some(PagePicker {
                        raw: root.display().to_string(),
                        title,
                        author: String::new(),
                        collection: None,
                        selected: vec![files.len() == 1; files.len()],
                        pages,
                        local_files: Some(files),
                        collapsed_local_dirs: HashSet::new(),
                        groups: Vec::new(),
                        collapsed_groups: HashSet::new(),
                        collapsed_episodes: HashSet::new(),
                        cursor: 0,
                        search: String::new(),
                        search_cursor: 0,
                        searching: false,
                    });
                    self.auth_status = "select local files".into();
                }
                AppEvent::LocalListFailed(message) => {
                    self.auth_status = format!("local folder failed · {message}");
                }
                AppEvent::Failed { index, message } if index == usize::MAX => {
                    self.auth_status = format!("page list failed · {message}");
                }
                AppEvent::AuthQr { qr, url } => {
                    self.auth_qr = Some(format!("{qr}\nURL: {url}"));
                    self.auth_status = "scan QR and confirm on phone".into();
                    self.auth_busy = true;
                }
                AppEvent::Planned {
                    index,
                    title,
                    pages,
                } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        task.title = title;
                        task.status = format!("planned · {pages} page(s), preparing download");
                    }
                }
                AppEvent::Failed { index, message } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        task.control = None;
                        task.running = false;
                        task.status = if message == "download cancelled" {
                            "cancelled".into()
                        } else {
                            format!("failed · {message}")
                        };
                    }
                }
                AppEvent::Progress {
                    index,
                    label,
                    progress,
                } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        let now = Instant::now();
                        let transfer = task.transfers.entry(label).or_insert(TransferProgress {
                            downloaded: progress.downloaded,
                            total: progress.total,
                            observed_at: now,
                            bytes_per_second: 0.0,
                        });
                        let elapsed = now.duration_since(transfer.observed_at).as_secs_f64();
                        if elapsed > 0.0 && progress.downloaded >= transfer.downloaded {
                            let instant_speed =
                                (progress.downloaded - transfer.downloaded) as f64 / elapsed;
                            transfer.bytes_per_second = if transfer.bytes_per_second == 0.0 {
                                instant_speed
                            } else {
                                transfer.bytes_per_second * 0.65 + instant_speed * 0.35
                            };
                        }
                        transfer.downloaded = progress.downloaded;
                        transfer.total = progress.total;
                        transfer.observed_at = now;

                        let downloaded = task
                            .transfers
                            .values()
                            .map(|transfer| transfer.downloaded)
                            .sum::<u64>();
                        let total = task
                            .transfers
                            .values()
                            .map(|transfer| transfer.total)
                            .collect::<Option<Vec<_>>>()
                            .map(|values| values.into_iter().sum::<u64>());
                        let bytes_per_second = task
                            .transfers
                            .values()
                            .map(|transfer| transfer.bytes_per_second)
                            .sum::<f64>();
                        let amount = total
                            .map(|total| {
                                format!("{} / {}", format_bytes(downloaded), format_bytes(total))
                            })
                            .unwrap_or_else(|| format_bytes(downloaded));
                        let speed = format!("{}/s", format_bytes_f64(bytes_per_second));
                        let eta = total
                            .filter(|total| *total > downloaded && bytes_per_second >= 1.0)
                            .map(|total| {
                                format_duration((total - downloaded) as f64 / bytes_per_second)
                            })
                            .unwrap_or_else(|| "--".into());
                        task.status = format!("downloading · {amount} · {speed} · ETA {eta}");
                    }
                }
                AppEvent::BuzzProgress {
                    index,
                    processed_seconds,
                    total_seconds,
                    elapsed_seconds,
                } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        task.transfers.clear();
                        let processed_ms = seconds_to_millis(processed_seconds);
                        let total_ms = seconds_to_millis(total_seconds).max(1);
                        task.buzz_progress = Some(BuzzTaskProgress {
                            processed_ms: processed_ms.min(total_ms),
                            total_ms,
                        });
                        let percent = (processed_seconds / total_seconds.max(f64::EPSILON) * 100.0)
                            .clamp(0.0, 100.0);
                        let eta = if processed_seconds > 0.0
                            && processed_seconds < total_seconds
                            && elapsed_seconds > 0.0
                        {
                            format_duration(
                                elapsed_seconds / processed_seconds
                                    * (total_seconds - processed_seconds),
                            )
                        } else {
                            "--".into()
                        };
                        task.status = format!(
                            "Buzz: transcribing · {} / {} · {percent:.1}% · ETA {eta}",
                            format_media_time(processed_seconds),
                            format_media_time(total_seconds)
                        );
                    }
                }
                AppEvent::ExistingSrt {
                    index,
                    path,
                    response,
                } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        task.status = "waiting for existing SRT confirmation".into();
                    }
                    self.existing_srt_prompts.push_back(ExistingSrtPrompt {
                        index,
                        path,
                        use_existing: false,
                        response,
                    });
                }
                AppEvent::Phase { index, message } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        transition_progress_for_phase(
                            &mut task.transfers,
                            &mut task.buzz_progress,
                            &message,
                        );
                        task.status = message;
                    }
                }
                AppEvent::Finished {
                    index,
                    path,
                    ai_output,
                } => {
                    if let Some(task) = self.tasks.iter_mut().find(|task| task.id == index) {
                        task.control = None;
                        task.running = false;
                        task.ai_output = ai_output;
                        task.status = format!("completed · {}", path.display());
                    }
                }
                AppEvent::AiTestResult(message) => self.settings_message = message,
            }
        }
    }

    fn start_auth_check(&mut self, options: &TuiOptions) {
        let sender = self.sender.clone();
        let client_options = options.client.clone();
        tokio::spawn(async move {
            let Ok(client) = BiliClient::new(client_options) else {
                let _ = sender.send(AppEvent::AuthStatus("error: unable to initialize".into()));
                return;
            };
            let status = match client.is_logged_in().await {
                Ok(true) => "logged in".to_owned(),
                Ok(false) => "anonymous".to_owned(),
                Err(error) => format!("unknown · {error}"),
            };
            let _ = sender.send(AppEvent::AuthStatus(status));
        });
    }

    fn start_login(&mut self, options: &TuiOptions) {
        if self.auth_busy {
            return;
        }
        self.auth_busy = true;
        self.auth_qr = None;
        self.auth_status = "generating login QR".into();
        let sender = self.sender.clone();
        let client_options = options.client.clone();
        tokio::spawn(async move {
            let result = async {
                let client = BiliClient::new(client_options)?;
                let qr = client.generate_login_qr().await?;
                let code = QrCode::new(qr.url.as_bytes()).map_err(|error| Error::Protocol {
                    endpoint: "qrcode/generate".into(),
                    detail: error.to_string(),
                })?;
                let rendered = code.render::<unicode::Dense1x2>().quiet_zone(true).build();
                let _ = sender.send(AppEvent::AuthQr {
                    qr: rendered,
                    url: qr.url.clone(),
                });
                let deadline = tokio::time::Instant::now() + Duration::from_secs(180);
                loop {
                    if tokio::time::Instant::now() >= deadline {
                        return Err(Error::LoginTimeout);
                    }
                    match client.poll_login(&qr.qrcode_key).await? {
                        muxsift_core::LoginPoll::WaitingForScan
                        | muxsift_core::LoginPoll::WaitingForConfirmation => {
                            tokio::time::sleep(Duration::from_secs(2)).await;
                        }
                        muxsift_core::LoginPoll::Expired => return Err(Error::LoginExpired),
                        muxsift_core::LoginPoll::Success => {
                            if !client.is_logged_in().await? {
                                return Err(Error::NotLoggedIn);
                            }
                            client.save_credentials()?;
                            return Ok(());
                        }
                    }
                }
            }
            .await;
            match result {
                Ok(()) => {
                    let _ = sender.send(AppEvent::AuthStatus("logged in".into()));
                }
                Err(error) => {
                    let _ = sender.send(AppEvent::AuthStatus(format!("login failed · {error}")));
                }
            }
        });
    }

    fn logout(&mut self, options: &TuiOptions) {
        let sender = self.sender.clone();
        let client_options = options.client.clone();
        self.auth_busy = true;
        self.auth_qr = None;
        self.auth_status = "logging out".into();
        tokio::spawn(async move {
            let status = match BiliClient::new(client_options)
                .and_then(|client| client.clear_credentials().map(|_| "anonymous".to_owned()))
            {
                Ok(status) => status,
                Err(error) => format!("logout failed · {error}"),
            };
            let _ = sender.send(AppEvent::AuthStatus(status));
        });
    }

    fn answer_existing_srt(&mut self, use_existing: bool) {
        let Some(prompt) = self.existing_srt_prompts.pop_front() else {
            return;
        };
        if let Some(task) = self.tasks.iter_mut().find(|task| task.id == prompt.index) {
            task.status = if use_existing {
                "AI: organizing SRT as Markdown".into()
            } else {
                "cancelling".into()
            };
        }
        let _ = prompt.response.send(use_existing);
    }
}

struct TaskOutput {
    path: PathBuf,
    ai_output: Option<PathBuf>,
}

#[allow(clippy::too_many_arguments)]
async fn execute_task(
    raw: &str,
    local: bool,
    mode: Mode,
    index: usize,
    page: usize,
    quality: Option<u64>,
    codec: Option<&str>,
    subtitle_format: SubtitleFormat,
    transcript_format: TranscriptFormat,
    group_output_name: Option<&str>,
    local_output_subdir: Option<&Path>,
    control: &DownloadControl,
    sender: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
    client_options: &ClientOptions,
    options: &TuiOptions,
    resume: bool,
) -> Result<TaskOutput, Error> {
    let client = BiliClient::new(client_options.clone())?;
    let ai_client = options.ai.clone().map(AiClient::new).transpose()?;
    if local {
        let ai = ai_client.as_ref().ok_or_else(|| {
            Error::InvalidInput(
                "Enable AI in C settings and set its API key environment variable for local media notes"
                    .into(),
            )
        })?;
        let request = LocalMediaRequest {
            input: PathBuf::from(raw),
            output_dir: local_output_subdir
                .map(|subdir| options.transcript_output.join(subdir))
                .unwrap_or_else(|| options.transcript_output.clone()),
            ffmpeg: options.ffmpeg.clone(),
            asr: options.asr_backend,
            buzz: options.buzz.clone(),
            resume,
            make_markdown: true,
            control: Some(control.clone()),
        };
        let started = Instant::now();
        let output = process_local_media(&request, &client, Some(ai), |event| match event {
            LocalMediaEvent::Stage(stage) => {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: stage.into(),
                });
            }
            LocalMediaEvent::Bcut(event) => {
                let message = match event {
                    BcutProgress::WaitingForSlot => "Bcut: waiting for slot".into(),
                    BcutProgress::PreparingUpload => "Bcut: preparing upload".into(),
                    BcutProgress::Uploading {
                        completed, total, ..
                    } => {
                        format!("Bcut: uploading {completed}/{total}")
                    }
                    BcutProgress::CreatingTask => "Bcut: creating task".into(),
                    BcutProgress::Recognizing { elapsed_seconds } => {
                        format!("Bcut: recognizing · {elapsed_seconds}s")
                    }
                    BcutProgress::Rendering => "Bcut: rendering SRT".into(),
                };
                let _ = sender.send(AppEvent::Phase { index, message });
            }
            LocalMediaEvent::Buzz(BuzzProgress::Transcribing {
                processed_seconds,
                total_seconds,
            }) => {
                let _ = sender.send(AppEvent::BuzzProgress {
                    index,
                    processed_seconds,
                    total_seconds,
                    elapsed_seconds: started.elapsed().as_secs_f64(),
                });
            }
            LocalMediaEvent::Buzz(event) => {
                let message = match event {
                    BuzzProgress::WaitingForSlot => "Buzz: waiting for local ASR slot",
                    BuzzProgress::LoadingModel => "Buzz: loading local model",
                    BuzzProgress::Rendering => "Buzz: rendering transcript",
                    BuzzProgress::Transcribing { .. } => unreachable!(),
                };
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: message.into(),
                });
            }
        })
        .await?;
        return Ok(TaskOutput {
            path: output.transcript,
            ai_output: output.markdown,
        });
    }
    let input: VideoInput = resolve_video_input(
        raw,
        Some(page),
        client_options.proxy.as_deref(),
        client_options.no_proxy,
    )
    .await?;
    let plan = client.plan(&input, quality, codec).await?;
    let _ = sender.send(AppEvent::Planned {
        index,
        title: display_task_title(&plan.video.title, &plan.page.part),
        pages: plan.video.pages.len(),
    });
    let (output_root, extension) = match mode {
        Mode::Video => (&options.video_output, "mp4"),
        Mode::Audio => (&options.audio_output, "m4a"),
        Mode::Subtitle => (&options.subtitle_output, subtitle_format.extension()),
        Mode::Transcript => (&options.transcript_output, transcript_format.extension()),
    };
    let output_dir = group_output_name
        .map(sanitize_filename)
        .map(|name| output_root.join(name))
        .unwrap_or_else(|| output_root.to_owned());
    tokio::fs::create_dir_all(&output_dir)
        .await
        .map_err(|source| Error::Io {
            path: output_dir.clone(),
            source,
        })?;
    let final_path = final_output_path_with_extension(&output_dir, &plan.page.part, extension);
    if final_path.exists() {
        if mode == Mode::Transcript && matches!(transcript_format, TranscriptFormat::Srt) {
            let ai = ai_client.as_ref().ok_or_else(|| {
                Error::InvalidInput(format!(
                    "output already exists: {}. AI post-processing is disabled or unavailable; enable AI to reuse this SRT",
                    final_path.display()
                ))
            })?;
            let markdown_path = final_path.with_extension("md");
            if markdown_path.exists() {
                return Err(Error::InvalidInput(format!(
                    "AI Markdown output already exists: {}",
                    markdown_path.display()
                )));
            }
            if !resume {
                let (response, answer) = tokio::sync::oneshot::channel();
                sender
                    .send(AppEvent::ExistingSrt {
                        index,
                        path: final_path.clone(),
                        response,
                    })
                    .map_err(|_| Error::Cancelled)?;
                if !answer.await.unwrap_or(false) {
                    return Err(Error::Cancelled);
                }
            }
            let markdown_path =
                organize_srt_with_ai_tui(ai, &final_path, &output_dir, index, sender).await?;
            ResumeWorkspace::remove_for_output(&final_path).await?;
            return Ok(TaskOutput {
                path: final_path,
                ai_output: Some(markdown_path),
            });
        }
        return Err(Error::InvalidInput(format!(
            "output already exists: {}",
            final_path.display()
        )));
    }
    let identity = tui_resume_identity(&plan, mode, subtitle_format, transcript_format);
    let work = ResumeWorkspace::open(&final_path, &identity, resume).await?;
    let mut ai_output = None;
    match mode {
        Mode::Subtitle => {
            let _ = sender.send(AppEvent::Phase {
                index,
                message: "fetching subtitle".into(),
            });
            let tracks = client
                .subtitle_tracks(&plan.video.bvid, plan.page.cid)
                .await?;
            let track = tracks
                .first()
                .ok_or_else(|| Error::Subtitle("no native subtitle track is available".into()))?;
            let text = client.render_subtitle(track, subtitle_format).await?;
            let temporary = work.artifact("subtitle.part");
            tokio::fs::write(&temporary, text)
                .await
                .map_err(|source| Error::Io {
                    path: temporary.clone(),
                    source,
                })?;
            tokio::fs::rename(&temporary, &final_path)
                .await
                .map_err(|source| Error::Io {
                    path: final_path.clone(),
                    source,
                })?;
        }
        Mode::Transcript => {
            let audio = work.artifact("audio.m4s");
            if work.stage_complete("audio-download", &audio).await {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "resuming: using saved audio download".into(),
                });
            } else {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "downloading audio".into(),
                });
                download_with_progress(
                    &client,
                    &plan.selected_audio,
                    &audio,
                    index,
                    "audio",
                    sender,
                    control,
                    options,
                )
                .await?;
                work.mark_complete("audio-download", &audio).await?;
            }
            let m4a = work.artifact("audio.m4a");
            if work.stage_complete("audio-mux", &m4a).await {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "resuming: using saved M4A".into(),
                });
            } else {
                work.remove_incomplete_output(&m4a).await?;
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "merging audio with FFmpeg".into(),
                });
                mux_audio(&options.ffmpeg, &audio, &m4a).await?;
                work.mark_complete("audio-mux", &m4a).await?;
            }
            let text = match options.asr_backend {
                AsrBackend::Bcut => {
                    let phase_sender = sender.clone();
                    client
                        .transcribe_with_bcut(&m4a, transcript_format, move |progress| {
                            let message = match progress {
                                BcutProgress::WaitingForSlot => {
                                    "Bcut: waiting for the safety queue".into()
                                }
                                BcutProgress::PreparingUpload => "Bcut: preparing upload".into(),
                                BcutProgress::Uploading {
                                    completed,
                                    total,
                                    bytes,
                                } => format!(
                                    "Bcut: uploading {completed}/{total} parts · {}",
                                    format_bytes(bytes as u64)
                                ),
                                BcutProgress::CreatingTask => {
                                    "Bcut: creating recognition task".into()
                                }
                                BcutProgress::Recognizing { elapsed_seconds } => {
                                    format!("Bcut: recognizing · waiting {elapsed_seconds}s")
                                }
                                BcutProgress::Rendering => "Bcut: rendering transcript".into(),
                            };
                            let _ = phase_sender.send(AppEvent::Phase { index, message });
                        })
                        .await?
                }
                AsrBackend::Buzz => {
                    let phase_sender = sender.clone();
                    let started = Instant::now();
                    transcribe_audio_with_buzz(
                        &m4a,
                        transcript_format,
                        &options.buzz,
                        move |progress| {
                            let message = match progress {
                                BuzzProgress::WaitingForSlot => {
                                    "Buzz: waiting for local ASR slot".into()
                                }
                                BuzzProgress::LoadingModel => {
                                    format!("Buzz: loading local model {}", options.buzz.model)
                                }
                                BuzzProgress::Transcribing {
                                    processed_seconds,
                                    total_seconds,
                                } => {
                                    let _ = phase_sender.send(AppEvent::BuzzProgress {
                                        index,
                                        processed_seconds,
                                        total_seconds,
                                        elapsed_seconds: started.elapsed().as_secs_f64(),
                                    });
                                    return;
                                }
                                BuzzProgress::Rendering => "Buzz: rendering transcript".into(),
                            };
                            let _ = phase_sender.send(AppEvent::Phase { index, message });
                        },
                    )
                    .await?
                }
            };
            let temporary = work.artifact("transcript.part");
            tokio::fs::write(&temporary, text)
                .await
                .map_err(|source| Error::Io {
                    path: temporary.clone(),
                    source,
                })?;
            tokio::fs::rename(&temporary, &final_path)
                .await
                .map_err(|source| Error::Io {
                    path: final_path.clone(),
                    source,
                })?;
            if let Some(ai) = ai_client.as_ref() {
                if !matches!(transcript_format, TranscriptFormat::Srt) {
                    return Err(Error::InvalidInput(
                        "AI post-processing requires SRT transcript output".into(),
                    ));
                }
                ai_output = Some(
                    organize_srt_with_ai_tui(ai, &final_path, &output_dir, index, sender).await?,
                );
            }
        }
        Mode::Audio => {
            let audio = work.artifact("audio.m4s");
            if work.stage_complete("audio-download", &audio).await {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "resuming: using saved audio download".into(),
                });
            } else {
                let _ = sender.send(AppEvent::Phase {
                    index,
                    message: "downloading audio".into(),
                });
                download_with_progress(
                    &client,
                    &plan.selected_audio,
                    &audio,
                    index,
                    "audio",
                    sender,
                    control,
                    options,
                )
                .await?;
                work.mark_complete("audio-download", &audio).await?;
            }
            let merged = work.artifact("audio.m4a");
            work.remove_incomplete_output(&merged).await?;
            let _ = sender.send(AppEvent::Phase {
                index,
                message: "merging audio with FFmpeg".into(),
            });
            mux_audio(&options.ffmpeg, &audio, &merged).await?;
            tokio::fs::rename(&merged, &final_path)
                .await
                .map_err(|source| Error::Io {
                    path: final_path.clone(),
                    source,
                })?;
        }
        Mode::Video => {
            let _ = sender.send(AppEvent::Phase {
                index,
                message: "downloading audio + video".into(),
            });
            let audio = work.artifact("audio.m4s");
            let video = work.artifact("video.m4s");
            let audio_download = async {
                if work.stage_complete("audio-download", &audio).await {
                    let _ = sender.send(AppEvent::Phase {
                        index,
                        message: "resuming: using saved audio download".into(),
                    });
                    Ok(())
                } else {
                    download_with_progress(
                        &client,
                        &plan.selected_audio,
                        &audio,
                        index,
                        "audio",
                        sender,
                        control,
                        options,
                    )
                    .await?;
                    work.mark_complete("audio-download", &audio).await
                }
            };
            let video_download = async {
                if work.stage_complete("video-download", &video).await {
                    let _ = sender.send(AppEvent::Phase {
                        index,
                        message: "resuming: using saved video download".into(),
                    });
                    Ok(())
                } else {
                    download_with_progress(
                        &client,
                        &plan.selected_video,
                        &video,
                        index,
                        "video",
                        sender,
                        control,
                        options,
                    )
                    .await?;
                    work.mark_complete("video-download", &video).await
                }
            };
            let (audio_result, video_result) = tokio::join!(audio_download, video_download);
            audio_result?;
            video_result?;
            let merged = work.artifact("video.mp4");
            work.remove_incomplete_output(&merged).await?;
            let _ = sender.send(AppEvent::Phase {
                index,
                message: "merging video and audio with FFmpeg".into(),
            });
            mux(&options.ffmpeg, &audio, &video, &merged).await?;
            tokio::fs::rename(&merged, &final_path)
                .await
                .map_err(|source| Error::Io {
                    path: final_path.clone(),
                    source,
                })?;
        }
    }
    work.finish().await?;
    Ok(TaskOutput {
        path: final_path,
        ai_output,
    })
}

async fn organize_srt_with_ai_tui(
    ai: &AiClient,
    srt_path: &Path,
    output_dir: &Path,
    index: usize,
    sender: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
) -> Result<PathBuf, Error> {
    let markdown_path = srt_path.with_extension("md");
    if markdown_path.exists() {
        return Err(Error::InvalidInput(format!(
            "AI Markdown output already exists: {}",
            markdown_path.display()
        )));
    }
    let _ = sender.send(AppEvent::Phase {
        index,
        message: "AI: organizing SRT as Markdown".into(),
    });
    let srt = tokio::fs::read_to_string(srt_path)
        .await
        .map_err(|source| Error::Io {
            path: srt_path.to_owned(),
            source,
        })?;
    let markdown = ai.srt_to_markdown(&srt).await?;
    let work = tempfile::Builder::new()
        .prefix(".muxsift-ai-")
        .tempdir_in(output_dir)
        .map_err(|source| Error::Io {
            path: output_dir.to_owned(),
            source,
        })?;
    let temporary = work.path().join("transcript.md.part");
    tokio::fs::write(&temporary, markdown)
        .await
        .map_err(|source| Error::Io {
            path: temporary.clone(),
            source,
        })?;
    tokio::fs::rename(&temporary, &markdown_path)
        .await
        .map_err(|source| Error::Io {
            path: markdown_path.clone(),
            source,
        })?;
    Ok(markdown_path)
}

fn option_path(options: &TuiOptions, field: ConfigField) -> PathBuf {
    match field {
        ConfigField::Video => options.video_output.clone(),
        ConfigField::Audio => options.audio_output.clone(),
        ConfigField::Subtitle => options.subtitle_output.clone(),
        ConfigField::Transcript => options.transcript_output.clone(),
        ConfigField::BcutCooldown => PathBuf::new(),
    }
}

fn expand_home_path(value: &str) -> String {
    if value == "~" {
        return std::env::var("HOME").unwrap_or_else(|_| value.to_owned());
    }
    if let Some(suffix) = value.strip_prefix("~/")
        && let Some(home) = std::env::var_os("HOME")
    {
        return PathBuf::from(home).join(suffix).display().to_string();
    }
    value.to_owned()
}

fn config_field_value(options: &TuiOptions, field: ConfigField) -> String {
    match field {
        ConfigField::BcutCooldown => options.bcut_cooldown_seconds.to_string(),
        _ => option_path(options, field).display().to_string(),
    }
}

fn set_option_path(options: &mut TuiOptions, field: ConfigField, path: PathBuf) {
    match field {
        ConfigField::Video => options.video_output = path,
        ConfigField::Audio => options.audio_output = path,
        ConfigField::Subtitle => options.subtitle_output = path,
        ConfigField::Transcript => options.transcript_output = path,
        ConfigField::BcutCooldown => {}
    }
}

fn update_config_bcut_cooldown(path: &Path, seconds: u64) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let bcut = root
        .as_object_mut()
        .expect("config object")
        .entry("bcut")
        .or_insert_with(|| Value::Object(Default::default()));
    if !bcut.is_object() {
        *bcut = Value::Object(Default::default());
    }
    bcut.as_object_mut()
        .expect("bcut object")
        .insert("cooldown_seconds".into(), Value::Number(seconds.into()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_language(path: &Path, language: Language) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let ui = root
        .as_object_mut()
        .expect("config object")
        .entry("ui")
        .or_insert_with(|| Value::Object(Default::default()));
    if !ui.is_object() {
        *ui = Value::Object(Default::default());
    }
    ui.as_object_mut().expect("ui object").insert(
        "language".into(),
        Value::String(language.config_value().into()),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_path(path: &Path, field: ConfigField, value: &str) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let paths = root
        .as_object_mut()
        .expect("config object")
        .entry("paths")
        .or_insert_with(|| Value::Object(Default::default()));
    if !paths.is_object() {
        *paths = Value::Object(Default::default());
    }
    paths
        .as_object_mut()
        .expect("paths object")
        .insert(field.key().into(), Value::String(value.into()));
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_backend(path: &Path, backend: MediaBackend) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let download = root
        .as_object_mut()
        .expect("config object")
        .entry("download")
        .or_insert_with(|| Value::Object(Default::default()));
    if !download.is_object() {
        *download = Value::Object(Default::default());
    }
    download
        .as_object_mut()
        .expect("download config object")
        .insert(
            "backend".into(),
            Value::String(backend.label().to_ascii_lowercase()),
        );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_asr_backend(path: &Path, backend: AsrBackend) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let asr = root
        .as_object_mut()
        .expect("config object")
        .entry("asr")
        .or_insert_with(|| Value::Object(Default::default()));
    if !asr.is_object() {
        *asr = Value::Object(Default::default());
    }
    asr.as_object_mut().expect("asr config object").insert(
        "backend".into(),
        Value::String(
            match backend {
                AsrBackend::Bcut => "bcut",
                AsrBackend::Buzz => "buzz",
            }
            .into(),
        ),
    );
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_ai(path: &Path, field: AiConfigField, value: &str) -> Result<(), Error> {
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let ai = root
        .as_object_mut()
        .expect("config object")
        .entry("ai")
        .or_insert_with(|| Value::Object(Default::default()));
    if !ai.is_object() {
        *ai = Value::Object(Default::default());
    }
    let (key, json_value) = match field {
        AiConfigField::Enabled => (
            "enabled",
            Value::Bool(value.parse::<bool>().unwrap_or(false)),
        ),
        AiConfigField::BaseUrl => ("base_url", Value::String(value.to_owned())),
        AiConfigField::Model => ("model", Value::String(value.to_owned())),
        AiConfigField::KeyEnvironment => ("api_key_env", Value::String(value.to_owned())),
        AiConfigField::SystemPrompt => ("system_prompt", Value::String(value.to_owned())),
    };
    ai.as_object_mut()
        .expect("AI config object")
        .insert(key.into(), json_value);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn update_config_buzz(path: &Path, field: SettingsField, value: &str) -> Result<(), Error> {
    let key = field.buzz_key().expect("Buzz field");
    let mut root = match std::fs::read_to_string(path) {
        Ok(contents) => serde_json::from_str::<Value>(&contents)
            .map_err(|error| Error::InvalidInput(format!("invalid config: {error}")))?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            Value::Object(Default::default())
        }
        Err(source) => {
            return Err(Error::Io {
                path: path.to_owned(),
                source,
            });
        }
    };
    if !root.is_object() {
        root = Value::Object(Default::default());
    }
    let asr = root
        .as_object_mut()
        .expect("config object")
        .entry("asr")
        .or_insert_with(|| Value::Object(Default::default()));
    if !asr.is_object() {
        *asr = Value::Object(Default::default());
    }
    let buzz = asr
        .as_object_mut()
        .expect("asr object")
        .entry("buzz")
        .or_insert_with(|| Value::Object(Default::default()));
    if !buzz.is_object() {
        *buzz = Value::Object(Default::default());
    }
    let json_value = if field == SettingsField::BuzzLanguage && value.is_empty() {
        Value::Null
    } else {
        Value::String(value.to_owned())
    };
    buzz.as_object_mut()
        .expect("buzz object")
        .insert(key.into(), json_value);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    }
    let text = serde_json::to_string_pretty(&root)
        .map_err(|error| Error::InvalidInput(format!("could not serialize config: {error}")))?;
    std::fs::write(path, text).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn refresh_ai_options(options: &mut TuiOptions) {
    if !options.ai_enabled {
        options.ai = None;
        return;
    }
    let Ok(api_key) = std::env::var(&options.ai_api_key_env) else {
        options.ai = None;
        return;
    };
    let ai = AiOptions {
        base_url: options.ai_base_url.clone(),
        model: options.ai_model.clone(),
        api_key,
        system_prompt: options.ai_system_prompt.clone(),
    };
    options.ai = AiClient::new(ai.clone()).ok().map(|_| ai);
}

#[allow(clippy::too_many_arguments)]
async fn download_with_progress(
    client: &BiliClient,
    stream: &muxsift_core::Stream,
    path: &Path,
    index: usize,
    label: &str,
    sender: &tokio::sync::mpsc::UnboundedSender<AppEvent>,
    control: &DownloadControl,
    options: &TuiOptions,
) -> Result<(), Error> {
    let on_progress = |progress| {
        let _ = sender.send(AppEvent::Progress {
            index,
            label: label.to_owned(),
            progress,
        });
    };
    match options.backend {
        MediaBackend::Http => {
            client
                .download_with_control(stream, path, on_progress, control)
                .await
        }
        MediaBackend::Aria2 => {
            client
                .download_with_aria2(
                    stream,
                    path,
                    options
                        .aria2_manager
                        .as_ref()
                        .expect("aria2 manager exists for aria2 backend"),
                    on_progress,
                    Some(control),
                )
                .await
        }
    }
}

pub async fn run(options: TuiOptions) -> Result<(), Box<dyn std::error::Error>> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;
    terminal.show_cursor()?;
    let result = run_loop(&mut terminal, options).await;
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

async fn run_loop<B: ratatui::backend::Backend>(
    terminal: &mut Terminal<B>,
    options: TuiOptions,
) -> Result<(), Box<dyn std::error::Error>>
where
    B::Error: 'static,
{
    let mut options = options;
    let mut app = App::new(options.config_path.clone(), options.language);
    app.ai_enabled = options.ai_enabled;
    app.ai_base_url = options.ai_base_url.clone();
    app.ai_model = options.ai_model.clone();
    app.ai_api_key_env = options.ai_api_key_env.clone();
    app.ai_prompt_preview = options.ai_system_prompt.clone();
    app.start_auth_check(&options);
    loop {
        app.animation_tick = app.animation_tick.wrapping_add(1);
        app.drain_events();
        terminal.draw(|frame| draw(frame, &app, &options))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                Event::Paste(text)
                    if app
                        .page_picker
                        .as_ref()
                        .is_some_and(|picker| picker.searching) =>
                {
                    if let Some(picker) = app.page_picker.as_mut() {
                        picker.insert_search(&text);
                    }
                }
                Event::Paste(text) if app.editing => app.insert_input(&text),
                Event::Key(KeyEvent {
                    code,
                    modifiers,
                    kind: KeyEventKind::Press,
                    ..
                }) => {
                    if let Some(prompt) = app.existing_srt_prompts.front_mut() {
                        let answer = match code {
                            KeyCode::Left | KeyCode::Right | KeyCode::Tab => {
                                prompt.use_existing = !prompt.use_existing;
                                None
                            }
                            KeyCode::Char('y') | KeyCode::Char('Y') => Some(true),
                            KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => Some(false),
                            KeyCode::Enter => Some(prompt.use_existing),
                            _ => None,
                        };
                        if let Some(use_existing) = answer {
                            app.answer_existing_srt(use_existing);
                        }
                        continue;
                    }
                    if app.page_picker.is_some() {
                        let mut confirm = false;
                        let mut cancel_picker = false;
                        if let Some(picker) = app.page_picker.as_mut() {
                            if picker.searching {
                                match code {
                                    KeyCode::Esc | KeyCode::Enter => picker.searching = false,
                                    KeyCode::Backspace => picker.backspace_search(),
                                    KeyCode::Delete => picker.delete_search(),
                                    KeyCode::Left => {
                                        picker.search_cursor =
                                            picker.search_cursor.saturating_sub(1)
                                    }
                                    KeyCode::Right => {
                                        picker.search_cursor = (picker.search_cursor + 1)
                                            .min(picker.search.chars().count())
                                    }
                                    KeyCode::Home => picker.search_cursor = 0,
                                    KeyCode::End => {
                                        picker.search_cursor = picker.search.chars().count()
                                    }
                                    KeyCode::Char(ch)
                                        if !modifiers.contains(KeyModifiers::CONTROL) =>
                                    {
                                        picker.insert_search(&ch.to_string())
                                    }
                                    _ => {}
                                }
                                continue;
                            }
                            let rows = picker_rows(picker);
                            picker.cursor = picker.cursor.min(rows.len().saturating_sub(1));
                            match code {
                                KeyCode::Esc => cancel_picker = true,
                                KeyCode::Char('/') => {
                                    picker.searching = true;
                                    picker.search_cursor = picker.search.chars().count();
                                }
                                KeyCode::Up | KeyCode::Char('k') => {
                                    picker.cursor = picker.cursor.saturating_sub(1)
                                }
                                KeyCode::Down | KeyCode::Char('j') => {
                                    picker.cursor =
                                        (picker.cursor + 1).min(rows.len().saturating_sub(1))
                                }
                                KeyCode::Char(' ') => match rows.get(picker.cursor) {
                                    Some(PickerRow::LocalFile { index, .. }) => {
                                        picker.selected[*index] = !picker.selected[*index];
                                    }
                                    Some(PickerRow::LocalDirectory { relative, .. }) => {
                                        let indexes: Vec<_> = picker_result_pages(picker)
                                            .into_iter()
                                            .filter(|index| {
                                                picker.local_files.as_ref().is_some_and(|files| {
                                                    files[*index].relative.starts_with(relative)
                                                })
                                            })
                                            .collect();
                                        let select_all =
                                            indexes.iter().any(|index| !picker.selected[*index]);
                                        for index in indexes {
                                            picker.selected[index] = select_all;
                                        }
                                    }
                                    Some(PickerRow::Page { index, .. }) => {
                                        if let Some(selected) = picker.selected.get_mut(*index) {
                                            *selected = !*selected;
                                        }
                                    }
                                    Some(PickerRow::Group(group_index)) => {
                                        let matching = picker_result_pages(picker);
                                        let pages = picker.groups[*group_index]
                                            .episodes
                                            .iter()
                                            .flat_map(|episode| episode.pages.iter().copied())
                                            .filter(|index| matching.contains(index))
                                            .collect::<Vec<_>>();
                                        let select_all =
                                            pages.iter().any(|index| !picker.selected[*index]);
                                        for index in pages {
                                            picker.selected[index] = select_all;
                                        }
                                    }
                                    Some(PickerRow::Episode(group_index, episode_index)) => {
                                        let matching = picker_result_pages(picker);
                                        let pages = &picker.groups[*group_index].episodes
                                            [*episode_index]
                                            .pages;
                                        let pages = pages
                                            .iter()
                                            .copied()
                                            .filter(|index| matching.contains(index))
                                            .collect::<Vec<_>>();
                                        let select_all =
                                            pages.iter().any(|index| !picker.selected[*index]);
                                        for index in pages {
                                            picker.selected[index] = select_all;
                                        }
                                    }
                                    None => {}
                                },
                                KeyCode::Left => match rows.get(picker.cursor) {
                                    Some(PickerRow::LocalDirectory { relative, .. }) => {
                                        picker.collapsed_local_dirs.insert(relative.clone());
                                    }
                                    Some(PickerRow::Group(group_index)) => {
                                        picker.collapsed_groups.insert(*group_index);
                                    }
                                    Some(PickerRow::Episode(group_index, episode_index)) => {
                                        picker
                                            .collapsed_episodes
                                            .insert((*group_index, *episode_index));
                                    }
                                    _ => {}
                                },
                                KeyCode::Right => match rows.get(picker.cursor) {
                                    Some(PickerRow::LocalDirectory { relative, .. }) => {
                                        picker.collapsed_local_dirs.remove(relative);
                                    }
                                    Some(PickerRow::Group(group_index)) => {
                                        picker.collapsed_groups.remove(group_index);
                                    }
                                    Some(PickerRow::Episode(group_index, episode_index)) => {
                                        picker
                                            .collapsed_episodes
                                            .remove(&(*group_index, *episode_index));
                                    }
                                    _ => {}
                                },
                                KeyCode::Char('A') => {
                                    if picker.search.trim().is_empty() {
                                        let select_all =
                                            picker.selected.iter().any(|selected| !selected);
                                        picker.selected.fill(select_all);
                                    } else {
                                        picker.select_search_results();
                                    }
                                }
                                KeyCode::Char('I') => {
                                    picker.invert_search_results();
                                }
                                KeyCode::Char('E') => {
                                    if let Some(files) = &picker.local_files {
                                        let directories = local_directories(files);
                                        if directories.iter().all(|directory| {
                                            picker.collapsed_local_dirs.contains(directory)
                                        }) {
                                            picker.collapsed_local_dirs.clear();
                                        } else {
                                            picker.collapsed_local_dirs = directories;
                                        }
                                    } else {
                                        let all_collapsed = picker.groups.iter().enumerate().all(
                                            |(group_index, group)| {
                                                (group.title.is_none()
                                                    || picker
                                                        .collapsed_groups
                                                        .contains(&group_index))
                                                    && group.episodes.iter().enumerate().all(
                                                        |(episode_index, episode)| {
                                                            episode.title.is_none()
                                                                || picker
                                                                    .collapsed_episodes
                                                                    .contains(&(
                                                                        group_index,
                                                                        episode_index,
                                                                    ))
                                                        },
                                                    )
                                            },
                                        );
                                        if all_collapsed {
                                            picker.collapsed_groups.clear();
                                            picker.collapsed_episodes.clear();
                                        } else {
                                            picker.collapsed_groups = picker
                                                .groups
                                                .iter()
                                                .enumerate()
                                                .filter_map(|(index, group)| {
                                                    group.title.is_some().then_some(index)
                                                })
                                                .collect();
                                            picker.collapsed_episodes = picker
                                                .groups
                                                .iter()
                                                .enumerate()
                                                .flat_map(|(group_index, group)| {
                                                    group.episodes.iter().enumerate().filter_map(
                                                        move |(episode_index, episode)| {
                                                            episode.title.is_some().then_some((
                                                                group_index,
                                                                episode_index,
                                                            ))
                                                        },
                                                    )
                                                })
                                                .collect();
                                        }
                                    }
                                }
                                KeyCode::Enter => confirm = true,
                                _ => {}
                            }
                        }
                        if cancel_picker {
                            app.page_picker = None;
                        }
                        if confirm {
                            app.confirm_pages();
                        }
                        continue;
                    }
                    if app.help {
                        match code {
                            KeyCode::Esc | KeyCode::Char('h') | KeyCode::Char('?') => {
                                app.help = false;
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                app.help_scroll = app.help_scroll.saturating_sub(1);
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.help_scroll = app.help_scroll.saturating_add(1);
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                    if app.settings_open && !app.editing {
                        match code {
                            KeyCode::Esc | KeyCode::Char('c') | KeyCode::Char('C') => {
                                app.close_settings()
                            }
                            KeyCode::Tab | KeyCode::Right => {
                                app.settings_section = app.settings_section.next();
                                app.settings_index = 0;
                                app.settings_message.clear();
                            }
                            KeyCode::BackTab | KeyCode::Left => {
                                app.settings_section = app.settings_section.previous();
                                app.settings_index = 0;
                                app.settings_message.clear();
                            }
                            KeyCode::Up | KeyCode::Char('k') => {
                                let count = app.settings_section.fields().len();
                                app.settings_index = (app.settings_index + count - 1) % count;
                            }
                            KeyCode::Down | KeyCode::Char('j') => {
                                app.settings_index =
                                    (app.settings_index + 1) % app.settings_section.fields().len();
                            }
                            KeyCode::Enter | KeyCode::Char(' ') => {
                                let field = app.selected_settings_field();
                                app.activate_settings_field(&mut options, field);
                            }
                            KeyCode::Char('t') if app.settings_section == SettingsSection::Ai => {
                                app.test_ai_connection(&options)
                            }
                            _ => {}
                        }
                        continue;
                    }
                    if app.editing && app.ai_editing == Some(AiConfigField::SystemPrompt) {
                        match code {
                            KeyCode::Esc => {
                                app.close_settings();
                                continue;
                            }
                            KeyCode::F(2) => {
                                let value = app.input.clone();
                                app.save_ai_field(&mut options, AiConfigField::SystemPrompt, value);
                                continue;
                            }
                            KeyCode::Enter if modifiers.contains(KeyModifiers::CONTROL) => {
                                let value = app.input.clone();
                                app.save_ai_field(&mut options, AiConfigField::SystemPrompt, value);
                                continue;
                            }
                            KeyCode::Char('s') if modifiers.contains(KeyModifiers::CONTROL) => {
                                let value = app.input.clone();
                                app.save_ai_field(&mut options, AiConfigField::SystemPrompt, value);
                                continue;
                            }
                            KeyCode::Enter => {
                                app.insert_input("\n");
                                continue;
                            }
                            _ => {}
                        }
                    }
                    match (app.editing, code) {
                        (false, KeyCode::Char('q')) => break,
                        (false, KeyCode::Char('h')) | (false, KeyCode::Char('?')) => {
                            app.help = true;
                            app.help_scroll = 0;
                        }
                        (false, KeyCode::Char('c')) => app.open_settings(SettingsSection::General),
                        (false, KeyCode::Char('C')) => app.open_settings(SettingsSection::Ai),
                        (false, KeyCode::Char('a')) => {
                            app.input.clear();
                            app.input_cursor = 0;
                            app.editing_task = None;
                            app.editing = true;
                        }
                        (false, KeyCode::Char('g')) => {
                            app.activate_settings_field(&mut options, SettingsField::Language)
                        }
                        (false, KeyCode::Char('m')) => {
                            app.mode = app.mode.next();
                        }
                        (false, KeyCode::Char('b')) => app
                            .activate_settings_field(&mut options, SettingsField::DownloadBackend),
                        (false, KeyCode::Char('B')) => {
                            app.activate_settings_field(&mut options, SettingsField::AsrBackend)
                        }
                        (false, KeyCode::Char('Q')) => app.cycle_quality(),
                        (false, KeyCode::Char('v')) => app.cycle_codec(),
                        (false, KeyCode::Char('[')) => app.change_page(-1),
                        (false, KeyCode::Char(']')) => app.change_page(1),
                        (false, KeyCode::Char('f')) => app.cycle_format(),
                        (false, KeyCode::Char('l')) => app.start_login(&options),
                        (false, KeyCode::Char('L')) => app.logout(&options),
                        (false, KeyCode::Up) | (false, KeyCode::Char('k')) => app.move_cursor(-1),
                        (false, KeyCode::Down) | (false, KeyCode::Char('j')) => app.move_cursor(1),
                        (false, KeyCode::Char(' ')) => app.toggle_current(),
                        (false, KeyCode::Char('A')) => app.toggle_all_tasks(),
                        (false, KeyCode::Enter) => app.start_current(&options),
                        (false, KeyCode::Char('S')) => app.start_selected(&options),
                        (false, KeyCode::Char('e')) => app.begin_edit(),
                        (false, KeyCode::Char('E')) => app.toggle_group(),
                        (false, KeyCode::Char('d')) => app.delete_current(),
                        (false, KeyCode::Char('r')) => app.retry_current(&options),
                        (false, KeyCode::Char('x')) => app.cancel_current(),
                        (false, KeyCode::Char('P')) => app.toggle_pause_current(),
                        (true, KeyCode::Esc) => {
                            if app.settings_open {
                                app.close_settings();
                            } else {
                                app.editing = false;
                                app.input.clear();
                                app.input_cursor = 0;
                                app.editing_task = None;
                                app.config_field = None;
                                app.ai_editing = None;
                            }
                        }
                        (true, KeyCode::Enter) => app.submit(&mut options),
                        (true, KeyCode::Backspace) => {
                            app.backspace_input();
                        }
                        (true, KeyCode::Delete) => app.delete_input(),
                        (true, KeyCode::Left) => {
                            app.input_cursor = app.input_cursor.saturating_sub(1)
                        }
                        (true, KeyCode::Right) => {
                            app.input_cursor = (app.input_cursor + 1).min(app.input.chars().count())
                        }
                        (true, KeyCode::Home) => app.input_cursor = 0,
                        (true, KeyCode::End) => app.input_cursor = app.input.chars().count(),
                        (true, KeyCode::Char(ch)) => {
                            if modifiers.contains(KeyModifiers::CONTROL) {
                                // Ctrl+V arrives as Event::Paste on terminals that support it.
                            } else {
                                app.insert_input(&ch.to_string());
                            }
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

fn draw(frame: &mut ratatui::Frame<'_>, app: &App, options: &TuiOptions) {
    let status_height = if app.editing {
        3
    } else if app.auth_status.contains("failed") || app.auth_status.starts_with("error") {
        6
    } else {
        4
    };
    let areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(5),
            Constraint::Length(status_height),
            Constraint::Length(2),
        ])
        .split(frame.area());
    let mut task_lines = Vec::new();
    let mut selected_start = 0usize;
    let mut selected_end = 0usize;
    let row_width = areas[0].width.saturating_sub(4) as usize;
    if let Some(qr) = &app.auth_qr {
        task_lines.push(Line::from(Span::styled(
            tr(
                app.language,
                "  Bilibili login · scan this QR code, then confirm on your phone",
                "  哔哩哔哩登录 · 请扫码后在手机上确认",
            ),
            Style::default().fg(Color::Cyan),
        )));
        for line in qr.lines() {
            task_lines.push(Line::from(Span::raw(line.to_owned())));
        }
        task_lines.push(Line::from(""));
    }
    for (index, task) in app.tasks.iter().enumerate() {
        if task.grouped
            && !task.group_first
            && app
                .collapsed_groups
                .contains(task.group_key.as_deref().unwrap_or(&task.input))
        {
            continue;
        }
        if index == app.cursor {
            selected_start = task_lines.len();
        }
        let status_color = if task.status.starts_with("failed") {
            Color::Red
        } else if task.status.starts_with("planned") || task.status.starts_with("completed") {
            Color::Green
        } else if task.status.starts_with("queued") {
            Color::DarkGray
        } else {
            Color::Yellow
        };
        let marker = if index == app.cursor { ">" } else { " " };
        let checked = if task.selected { "x" } else { " " };
        let tree = if task.grouped {
            if task.group_first {
                "┌"
            } else if task.group_last {
                "└"
            } else {
                "│"
            }
        } else {
            " "
        };
        task_lines.push(Line::from(vec![
            Span::styled(
                format!("{}{}[{}] {:02} ", tree, marker, checked, index + 1),
                Style::default().fg(if index == app.cursor {
                    Color::Yellow
                } else {
                    Color::DarkGray
                }),
            ),
            Span::styled(
                format!("[{}] ", task.mode.label(app.language)),
                Style::default().fg(Color::Cyan),
            ),
            Span::styled(
                if task.local {
                    tr(app.language, "[local] ", "[本地] ").to_owned()
                } else {
                    format!(
                        "[p{} {} {}] ",
                        task.page,
                        task.quality.map_or("best".into(), |q| format!("{}p", q)),
                        task.codec.as_deref().unwrap_or("auto")
                    )
                },
                Style::default().fg(Color::Magenta),
            ),
            Span::styled(
                if task.grouped
                    && task.group_first
                    && app
                        .collapsed_groups
                        .contains(task.group_key.as_deref().unwrap_or(&task.input))
                {
                    "[+] "
                } else {
                    ""
                },
                Style::default().fg(Color::Yellow),
            ),
            Span::styled(
                format!(
                    "[{}{}]",
                    if task.running && !task.status.starts_with("paused") {
                        format!("{} ", spinner(app.animation_tick))
                    } else {
                        String::new()
                    },
                    task_status_label(&task.status, app.language)
                ),
                Style::default().fg(status_color),
            ),
        ]));
        let continues_group = task.grouped && !task.group_last;
        let title_prefix = if continues_group {
            "│     "
        } else {
            "      "
        };
        let title_label = format!("{}: ", tr(app.language, "Title", "标题"));
        for (line_index, title_line) in wrap_text(&task.title, row_width.saturating_sub(13))
            .into_iter()
            .enumerate()
        {
            let continuation_prefix = format!(
                "{}{}",
                title_prefix,
                " ".repeat(title_label.chars().count())
            );
            let (prefix, text) = if line_index == 0 {
                (
                    title_prefix.to_owned(),
                    format!("{title_label}{title_line}"),
                )
            } else {
                (continuation_prefix, title_line)
            };
            task_lines.push(Line::from(vec![
                Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                Span::styled(text, Style::default().fg(Color::White)),
            ]));
        }
        let gutter = if continues_group { "│" } else { " " };
        task_lines.push(Line::from(vec![
            Span::styled(
                format!("{}     {}: ", gutter, tr(app.language, "Input", "输入")),
                Style::default().fg(Color::DarkGray),
            ),
            Span::raw(clip_text(&task.input, row_width.saturating_sub(12))),
        ]));
        if let Some(output) = task.status.strip_prefix("completed · ") {
            let output_prefix = format!("{}     {}: ", gutter, tr(app.language, "Out", "输出"));
            let output_width = row_width
                .saturating_sub(output_prefix.chars().count())
                .max(1);
            for (line_index, output_line) in wrap_text(output, output_width).into_iter().enumerate()
            {
                let prefix = if line_index == 0 {
                    output_prefix.clone()
                } else {
                    format!(
                        "{}     {}",
                        gutter,
                        " ".repeat(
                            output_prefix
                                .chars()
                                .count()
                                .saturating_sub(gutter.chars().count() + 5)
                        )
                    )
                };
                task_lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                    Span::styled(output_line, Style::default().fg(Color::Green)),
                ]));
            }
        }
        if let Some(markdown) = &task.ai_output {
            let markdown_prefix = format!("{}     AI: ", gutter);
            let markdown_width = row_width
                .saturating_sub(markdown_prefix.chars().count())
                .max(1);
            for (line_index, line) in wrap_text(&markdown.display().to_string(), markdown_width)
                .into_iter()
                .enumerate()
            {
                let prefix = if line_index == 0 {
                    markdown_prefix.clone()
                } else {
                    format!(
                        "{}     {}",
                        gutter,
                        " ".repeat(
                            markdown_prefix
                                .chars()
                                .count()
                                .saturating_sub(gutter.chars().count() + 5)
                        )
                    )
                };
                task_lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                    Span::styled(line, Style::default().fg(Color::Cyan)),
                ]));
            }
        }
        if let Some(error) = task.status.strip_prefix("failed · ") {
            let error_prefix = format!("{}   {}: ", gutter, tr(app.language, "Error", "错误"));
            let error_width = row_width
                .saturating_sub(error_prefix.chars().count())
                .max(1);
            for (line_index, error_line) in wrap_text(error, error_width).into_iter().enumerate() {
                let prefix = if line_index == 0 {
                    error_prefix.clone()
                } else {
                    format!(
                        "{}   {}",
                        gutter,
                        " ".repeat(
                            error_prefix
                                .chars()
                                .count()
                                .saturating_sub(gutter.chars().count() + 3)
                        )
                    )
                };
                task_lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                    Span::styled(error_line, Style::default().fg(Color::Red)),
                ]));
            }
        }
        if task.running {
            if let Some((downloaded, total)) = task_progress_totals(task) {
                let progress_prefix = format!(
                    "{}     {}: ",
                    gutter,
                    if task.buzz_progress.is_some() {
                        tr(app.language, "Buzz Progress", "Buzz 进度")
                    } else {
                        tr(app.language, "Progress", "进度")
                    }
                );
                let percent = downloaded
                    .saturating_mul(100)
                    .checked_div(total)
                    .unwrap_or(0);
                let bar_width = row_width
                    .saturating_sub(progress_prefix.chars().count())
                    .saturating_sub(5);
                if bar_width >= 3 {
                    task_lines.push(Line::from(vec![
                        Span::styled(progress_prefix, Style::default().fg(Color::DarkGray)),
                        Span::styled(
                            progress_bar(downloaded, total, bar_width.min(32)),
                            Style::default().fg(Color::Cyan),
                        ),
                        Span::styled(format!(" {percent}%"), Style::default().fg(status_color)),
                    ]));
                }
            }
            let status_prefix =
                format!("{}     {}: ", gutter, tr(app.language, "Progress", "进度"));
            let status_width = row_width.saturating_sub(status_prefix.chars().count());
            let localized_status = localized_task_status(&task.status, app.language);
            for (line_index, status_line) in wrap_text(&localized_status, status_width)
                .into_iter()
                .enumerate()
            {
                let prefix = if line_index == 0 {
                    status_prefix.clone()
                } else {
                    format!(
                        "{}     {}",
                        gutter,
                        " ".repeat(
                            status_prefix
                                .chars()
                                .count()
                                .saturating_sub(gutter.chars().count() + 5)
                        )
                    )
                };
                task_lines.push(Line::from(vec![
                    Span::styled(prefix, Style::default().fg(Color::DarkGray)),
                    Span::styled(status_line, Style::default().fg(status_color)),
                ]));
            }
        }
        if index == app.cursor {
            selected_end = task_lines.len();
        }
        if index + 1 < app.tasks.len() {
            task_lines.push(Line::from(Span::styled(
                format!("{}     {}", gutter, "─".repeat(row_width.saturating_sub(5))),
                Style::default().fg(Color::DarkGray),
            )));
        }
    }
    if task_lines.is_empty() && app.show_welcome {
        task_lines.extend(welcome_lines(row_width, app.language));
    } else if task_lines.is_empty() {
        task_lines.push(Line::from(Span::styled(
            tr(
                app.language,
                "  No tasks yet. Press a to add a URL, local file, or folder.",
                "  暂无任务。按 a 添加链接、本地文件或文件夹。",
            ),
            Style::default().fg(Color::DarkGray),
        )));
    }
    let viewport_height = areas[0].height.saturating_sub(2) as usize;
    let viewport_height = viewport_height.max(1);
    let mut scroll = selected_start.saturating_sub(viewport_height.saturating_sub(1));
    if selected_end > scroll + viewport_height {
        scroll = selected_end.saturating_sub(viewport_height);
    }
    if selected_start < scroll {
        scroll = selected_start;
    }
    let list = Paragraph::new(Text::from(task_lines))
        .block(
            Block::default()
                .title(tr(app.language, " Tasks ", " 任务 "))
                .borders(Borders::ALL),
        )
        .scroll((scroll.min(u16::MAX as usize) as u16, 0));
    frame.render_widget(list, areas[0]);
    let prompt = if app.editing {
        let action = if let Some(field) = app.ai_editing {
            field.label(app.language)
        } else if let Some(field) = app.settings_editing {
            field.label(app.language)
        } else if app.config_field.is_some() {
            tr(app.language, "Config", "配置")
        } else if app.editing_task.is_some() {
            tr(app.language, "Edit input", "编辑输入")
        } else {
            tr(
                app.language,
                "Add URL / file / folder",
                "添加链接／文件／文件夹",
            )
        };
        let suffix = (app.ai_editing.is_some() || app.settings_editing.is_some())
            .then(String::new)
            .unwrap_or_else(|| {
                app.config_field
                    .map(|field| format!(" ({})", field.label(app.language)))
                    .unwrap_or_else(|| format!(" [{}]", app.mode.label(app.language)))
            });
        format!("{}{} > {}", action, suffix, app.input)
    } else {
        let selected = app.tasks.iter().filter(|task| task.selected).count();
        let auth_status = localized_auth_status(&app.auth_status, app.language);
        format!(
            "{}: {}\n{}: {}   {}: {}   ASR: {}   {}: {}   {}: {}   {}: {}",
            tr(app.language, "Status", "状态"),
            auth_status,
            tr(app.language, "Mode", "模式"),
            app.mode.label(app.language),
            tr(app.language, "Backend", "后端"),
            options.backend.label(),
            match options.asr_backend {
                AsrBackend::Bcut => "Bcut",
                AsrBackend::Buzz => "Buzz",
            },
            tr(app.language, "Tasks", "任务"),
            app.tasks.len(),
            tr(app.language, "Selected", "已选"),
            selected,
            tr(app.language, "Current", "当前"),
            app.cursor.saturating_add(1)
        )
    };
    let mut input_cursor_cells = 0usize;
    let prompt_display = if app.ai_editing == Some(AiConfigField::SystemPrompt) {
        tr(
            app.language,
            "Editing system prompt · Enter newline · F2 save · Esc close",
            "编辑系统提示词 · Enter 换行 · F2 保存 · Esc 关闭",
        )
        .to_owned()
    } else if app.editing {
        let mut label = if let Some(field) = app.ai_editing {
            format!("{} > ", field.label(app.language))
        } else if let Some(field) = app.settings_editing {
            format!("{} > ", field.label(app.language))
        } else if let Some(field) = app.config_field {
            format!(
                "{} ({}) > ",
                tr(app.language, "Config", "配置"),
                field.label(app.language)
            )
        } else if app.editing_task.is_some() {
            format!(
                "{} [{}] > ",
                tr(app.language, "Edit input", "编辑输入"),
                app.mode.label(app.language)
            )
        } else {
            format!(
                "{} [{}] > ",
                tr(
                    app.language,
                    "Add URL / file / folder",
                    "添加链接／文件／文件夹"
                ),
                app.mode.label(app.language)
            )
        };
        if app.editing_task.is_none()
            && app.config_field.is_none()
            && app.ai_editing.is_none()
            && app.settings_editing.is_none()
        {
            label = format!("{}[{} chars] ", label, app.input.chars().count());
        }
        let available = areas[1].width.saturating_sub(2) as usize;
        let max_input = available.saturating_sub(label.width());
        let (visible, cursor_cells) = visible_input_line(&app.input, app.input_cursor, max_input);
        input_cursor_cells = cursor_cells;
        format!("{label}{visible}")
    } else {
        prompt
    };
    frame.render_widget(
        Paragraph::new(prompt_display)
            .block(Block::default().borders(Borders::ALL))
            .wrap(Wrap { trim: false }),
        areas[1],
    );
    if app.editing && app.ai_editing != Some(AiConfigField::SystemPrompt) {
        let mut label = if let Some(field) = app.ai_editing {
            format!("{} > ", field.label(app.language))
        } else if let Some(field) = app.settings_editing {
            format!("{} > ", field.label(app.language))
        } else if let Some(field) = app.config_field {
            format!(
                "{} ({}) > ",
                tr(app.language, "Config", "配置"),
                field.label(app.language)
            )
        } else if app.editing_task.is_some() {
            format!(
                "{} [{}] > ",
                tr(app.language, "Edit input", "编辑输入"),
                app.mode.label(app.language)
            )
        } else {
            format!(
                "{} [{}] > ",
                tr(
                    app.language,
                    "Add URL / file / folder",
                    "添加链接／文件／文件夹"
                ),
                app.mode.label(app.language)
            )
        };
        if app.editing_task.is_none()
            && app.config_field.is_none()
            && app.ai_editing.is_none()
            && app.settings_editing.is_none()
        {
            label = format!("{}[{} chars] ", label, app.input.chars().count());
        }
        let cursor_x = areas[1]
            .x
            .saturating_add(1)
            .saturating_add(label.width() as u16)
            .saturating_add(input_cursor_cells as u16);
        frame.set_cursor_position(Position::new(
            cursor_x.min(areas[1].x + areas[1].width.saturating_sub(2)),
            areas[1].y + 1,
        ));
    }
    frame.render_widget(
        Paragraph::new(
            tr(app.language, "h/? help   a add   e edit   b download   B ASR   c config   C AI settings   g language   ↑/↓ or j/k select   Space mark   Enter start   S start marked   d delete   r resume   l login   L logout   m mode   q quit", "h/? 帮助   a 添加   e 编辑   b 下载后端   B 转写后端   c 配置   C AI 设置   g 语言   ↑/↓ 或 j/k 选择   空格 勾选   Enter 启动   S 启动已选   d 删除   r 断点续作   l 登录   L 登出   m 模式   q 退出"),
        ),
        areas[2],
    );
    if app.help {
        let popup = centered_rect(72, 78, frame.area());
        frame.render_widget(Clear, popup);
        frame.render_widget(
            Paragraph::new(Text::from(help_lines(app.language)))
                .block(
                    Block::default()
                        .title(tr(app.language, " Help ", " 帮助 "))
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(1)),
                )
                .wrap(Wrap { trim: false })
                .scroll((app.help_scroll, 0)),
            popup,
        );
    }
    if let Some(picker) = &app.page_picker {
        let popup = centered_rect(78, 78, frame.area());
        frame.render_widget(Clear, popup);
        let mut lines = vec![Line::from(Span::styled(
            tr(
                app.language,
                "/ search · Space select file/folder · A all · E fold · Enter confirm · Esc cancel",
                "/ 搜索 · 空格勾选文件/目录 · A 全选 · E 折叠 · Enter 确认 · Esc 取消",
            ),
            Style::default().fg(Color::Cyan),
        ))];
        if picker.local_files.is_some() {
            lines.push(Line::from(format!(
                "{}: {}",
                tr(app.language, "Folder", "文件夹"),
                clip_text(&picker.raw, popup.width.saturating_sub(14) as usize)
            )));
            lines.push(Line::from(format!(
                "{}: {}",
                tr(app.language, "Media files", "媒体文件"),
                picker.selected.len()
            )));
        } else {
            lines.push(Line::from(format!(
                "{}: {}",
                tr(app.language, "Title", "标题"),
                picker.title
            )));
            lines.push(Line::from(format!(
                "{}: {}   {}: {}",
                tr(app.language, "Author", "作者"),
                picker.author,
                tr(app.language, "Pages", "分 P"),
                picker.pages.len()
            )));
        }
        if let Some(collection) = &picker.collection {
            lines.push(Line::from(format!(
                "{}: {}   {}: {}",
                tr(app.language, "Collection", "合集"),
                collection,
                tr(app.language, "Episodes", "视频数"),
                picker.pages.len()
            )));
        }
        let result_pages = picker_result_pages(picker);
        let selected_results = result_pages
            .iter()
            .filter(|index| picker.selected[**index])
            .count();
        let mut search_text = picker.search.clone();
        if picker.searching {
            let mut chars: Vec<char> = search_text.chars().collect();
            let cursor = picker.search_cursor.min(chars.len());
            chars.insert(cursor, '|');
            search_text = chars.into_iter().collect();
        } else if search_text.is_empty() {
            search_text = tr(app.language, "all pages", "全部页面").to_owned();
        }
        let max_search = (popup.width as usize).saturating_sub(28).max(1);
        lines.push(Line::from(format!(
            "{}: {}   {}: {selected_results}/{} {}",
            tr(app.language, "Search", "搜索"),
            clip_text(&search_text, max_search),
            tr(app.language, "Results", "结果"),
            result_pages.len(),
            tr(app.language, "selected", "已选")
        )));
        lines.push(Line::from(""));
        let available_rows = (popup.height as usize)
            .saturating_sub(2)
            .saturating_sub(lines.len())
            .max(1);
        let item_width = (popup.width as usize).saturating_sub(4).max(1);
        let rows = picker_rows(picker);
        let local_counts = picker
            .local_files
            .as_ref()
            .map(|files| local_directory_counts(files, &picker.selected))
            .unwrap_or_default();
        let item_lines: Vec<Vec<Line<'static>>> = rows
            .iter()
            .enumerate()
            .map(|(index, row)| match row {
                PickerRow::LocalDirectory { relative, depth } => picker_local_directory_lines(
                    index == picker.cursor,
                    relative,
                    *depth,
                    *local_counts.get(relative).expect("local directory count"),
                    picker.collapsed_local_dirs.contains(relative),
                    item_width,
                ),
                PickerRow::LocalFile {
                    index: file_index,
                    depth,
                } => picker_local_file_lines(
                    index == picker.cursor,
                    picker.selected[*file_index],
                    &picker.local_files.as_ref().expect("local picker files")[*file_index],
                    *depth,
                    item_width,
                ),
                PickerRow::Group(group_index) => picker_group_lines(
                    index == picker.cursor,
                    &picker.groups[*group_index],
                    &picker.selected,
                    picker.collapsed_groups.contains(group_index),
                ),
                PickerRow::Episode(group_index, episode_index) => picker_episode_lines(
                    index == picker.cursor,
                    &picker.groups[*group_index].episodes[*episode_index],
                    &picker.selected,
                    picker
                        .collapsed_episodes
                        .contains(&(*group_index, *episode_index)),
                ),
                PickerRow::Page {
                    index: page_index,
                    nested,
                } => picker_page_lines(
                    index == picker.cursor,
                    picker.selected[*page_index],
                    &picker.pages[*page_index],
                    item_width,
                    *nested,
                    app.language,
                ),
            })
            .collect();

        if item_lines.is_empty() {
            lines.push(Line::from(tr(
                app.language,
                "No matching items found.",
                "没有匹配的项目。",
            )));
        } else {
            // Keep the selected page in view even when a title occupies several rows.
            let mut first = picker.cursor;
            let mut used_rows = item_lines[first].len().min(available_rows);
            while first > 0 {
                let previous_rows = item_lines[first - 1].len();
                if used_rows + previous_rows > available_rows {
                    break;
                }
                first -= 1;
                used_rows += previous_rows;
            }
            let mut rendered_rows = 0;
            for item in item_lines.iter().skip(first) {
                if rendered_rows >= available_rows {
                    break;
                }
                let count = item.len().min(available_rows - rendered_rows);
                lines.extend(item.iter().take(count).cloned());
                rendered_rows += count;
            }
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines)).block(
                Block::default()
                    .title(if picker.local_files.is_some() {
                        tr(app.language, " Local media ", " 选择本地媒体 ")
                    } else {
                        tr(app.language, " Pages ", " 选择分 P ")
                    })
                    .borders(Borders::ALL),
            ),
            popup,
        );
    }
    if app.settings_open {
        let popup = centered_rect(88, 86, frame.area());
        frame.render_widget(Clear, popup);
        let mut tabs = Vec::new();
        for section in [
            SettingsSection::General,
            SettingsSection::Asr,
            SettingsSection::Ai,
            SettingsSection::Paths,
        ] {
            tabs.push(Span::styled(
                format!("  {}  ", section.label(app.language)),
                Style::default()
                    .fg(if section == app.settings_section {
                        Color::Black
                    } else {
                        Color::Cyan
                    })
                    .bg(if section == app.settings_section {
                        Color::Cyan
                    } else {
                        Color::Reset
                    }),
            ));
            tabs.push(Span::raw(" "));
        }
        let mut lines = vec![Line::from(tabs)];
        lines.push(Line::from(Span::styled(
            tr(
                app.language,
                "Tab/←→ section · ↑↓ item · Enter edit/toggle · Esc close",
                "Tab/←→ 切换分类 · ↑↓ 选项 · Enter 编辑/切换 · Esc 关闭",
            ),
            Style::default().fg(Color::Cyan),
        )));
        lines.push(Line::from(""));
        let value_width = (popup.width as usize).saturating_sub(28).max(8);
        for (index, field) in app.settings_section.fields().iter().copied().enumerate() {
            let selected = index == app.settings_index;
            let value = if app.editing
                && app.selected_settings_field() == field
                && field != SettingsField::Ai(AiConfigField::SystemPrompt)
            {
                app.input.clone()
            } else {
                app.settings_value(options, field)
            };
            let value = if field == SettingsField::Ai(AiConfigField::SystemPrompt) {
                format!(
                    "{} {}",
                    value.chars().count(),
                    tr(app.language, "characters", "字")
                )
            } else if value.is_empty() {
                tr(app.language, "(empty)", "（空）").to_owned()
            } else {
                clip_text(&value.replace('\n', " "), value_width)
            };
            lines.push(Line::from(vec![
                Span::styled(
                    format!(
                        "{} {}: ",
                        if selected { ">" } else { " " },
                        field.label(app.language)
                    ),
                    Style::default().fg(if selected {
                        Color::Yellow
                    } else {
                        Color::DarkGray
                    }),
                ),
                Span::styled(value, Style::default().fg(Color::White)),
            ]));
        }
        if app.settings_section == SettingsSection::Ai {
            lines.push(Line::from(""));
            lines.push(Line::from(Span::styled(
                tr(
                    app.language,
                    "Prompt preview · Enter newline, F2 save while editing · t test AI",
                    "提示词预览 · 编辑时 Enter 换行、F2 保存 · t 测试 AI",
                ),
                Style::default().fg(Color::Cyan),
            )));
            let editing_prompt = app.ai_editing == Some(AiConfigField::SystemPrompt);
            let prompt = if editing_prompt {
                let mut preview = app.input.clone();
                let byte_index = preview
                    .char_indices()
                    .nth(app.input_cursor)
                    .map_or(preview.len(), |(index, _)| index);
                preview.insert(byte_index, '▏');
                preview
            } else {
                app.ai_prompt_preview.clone()
            };
            let wrapped = wrap_text(&prompt, popup.width.saturating_sub(6).max(1) as usize);
            let preview_rows = (popup.height as usize)
                .saturating_sub(lines.len() + 5)
                .clamp(1, 8);
            let first_line = if editing_prompt {
                wrapped
                    .iter()
                    .position(|line| line.contains('▏'))
                    .unwrap_or(wrapped.len().saturating_sub(1))
                    .saturating_sub(preview_rows - 1)
            } else {
                0
            };
            if first_line > 0 {
                lines.push(Line::from("    ..."));
            }
            for line in wrapped.iter().skip(first_line).take(preview_rows) {
                lines.push(Line::from(format!("    {line}")));
            }
            if wrapped.len() > first_line + preview_rows {
                lines.push(Line::from("    ..."));
            }
        }
        lines.push(Line::from(""));
        if !app.settings_message.is_empty() {
            lines.push(Line::from(Span::styled(
                app.settings_message.clone(),
                Style::default().fg(Color::Cyan),
            )));
        }
        frame.render_widget(
            Paragraph::new(Text::from(lines))
                .block(
                    Block::default()
                        .title(tr(app.language, " Settings ", " 设置 "))
                        .borders(Borders::ALL),
                )
                .wrap(Wrap { trim: false }),
            popup,
        );
    }
    if let Some(prompt) = app.existing_srt_prompts.front() {
        let popup = centered_rect(72, 38, frame.area());
        frame.render_widget(Clear, popup);
        let mut lines = vec![
            Line::from(Span::styled(
                tr(
                    app.language,
                    "The SRT output already exists.",
                    "转写输出 SRT 已存在。",
                ),
                Style::default().fg(Color::Yellow),
            )),
            Line::from(""),
        ];
        let path_width = popup.width.saturating_sub(6) as usize;
        for line in wrap_text(&prompt.path.display().to_string(), path_width.max(1)) {
            lines.push(Line::from(line));
        }
        lines.extend([
            Line::from(""),
            Line::from(tr(
                app.language,
                "Skip download and transcription, then use this SRT for AI organization?",
                "是否跳过下载和转写，直接使用此 SRT 进行 AI 整理？",
            )),
            Line::from(""),
            Line::from(vec![
                Span::styled(
                    tr(app.language, "[ Use existing SRT ]", "[ 使用现有 SRT ]"),
                    Style::default()
                        .fg(if prompt.use_existing {
                            Color::Black
                        } else {
                            Color::DarkGray
                        })
                        .bg(if prompt.use_existing {
                            Color::Cyan
                        } else {
                            Color::Reset
                        }),
                ),
                Span::raw("    "),
                Span::styled(
                    tr(app.language, "[ Cancel task ]", "[ 取消任务 ]"),
                    Style::default()
                        .fg(if prompt.use_existing {
                            Color::DarkGray
                        } else {
                            Color::Black
                        })
                        .bg(if prompt.use_existing {
                            Color::Reset
                        } else {
                            Color::Yellow
                        }),
                ),
            ]),
            Line::from(""),
            Line::from(Span::styled(
                tr(
                    app.language,
                    "Left/Right or Tab select · Enter confirm · Esc cancel",
                    "左/右 或 Tab 选择 · Enter 确认 · Esc 取消",
                ),
                Style::default().fg(Color::DarkGray),
            )),
        ]);
        frame.render_widget(
            Paragraph::new(lines)
                .block(
                    Block::default()
                        .title(tr(app.language, " Existing SRT ", " 已有 SRT "))
                        .borders(Borders::ALL)
                        .padding(Padding::horizontal(1)),
                )
                .wrap(Wrap { trim: false }),
            popup,
        );
    }
}

fn welcome_lines(width: usize, language: Language) -> Vec<Line<'static>> {
    const ART: [&str; 6] = [
        "███╗   ███╗██╗   ██╗██╗  ██╗███████╗██╗███████╗████████╗",
        "████╗ ████║██║   ██║╚██╗██╔╝██╔════╝██║██╔════╝╚══██╔══╝",
        "██╔████╔██║██║   ██║ ╚███╔╝ ███████╗██║█████╗     ██║",
        "██║╚██╔╝██║██║   ██║ ██╔██╗ ╚════██║██║██╔══╝     ██║",
        "██║ ╚═╝ ██║╚██████╔╝██╔╝ ██╗███████║██║██║        ██║",
        "╚═╝     ╚═╝ ╚═════╝ ╚═╝  ╚═╝╚══════╝╚═╝╚═╝        ╚═╝",
    ];
    let art_width = ART
        .iter()
        .map(|line| line.chars().count())
        .max()
        .unwrap_or(0);
    if width < art_width.saturating_add(4) {
        return vec![
            Line::from(""),
            Line::from(Span::styled("  MuxSift", Style::default().fg(Color::Green))),
            Line::from(""),
            Line::from(Span::styled(
                tr(
                    language,
                    "  Press a to add a Bilibili URL, local file, or folder.",
                    "  按 a 添加哔哩哔哩链接、本地文件或文件夹。请使用英文键盘输入快捷键。",
                ),
                Style::default().fg(Color::DarkGray),
            )),
        ];
    }
    let padding = " ".repeat((width.saturating_sub(art_width)) / 2);
    let mut lines = Vec::with_capacity(ART.len() + 3);
    lines.push(Line::from(""));
    lines.extend(ART.into_iter().enumerate().map(|(index, line)| {
        Line::from(Span::styled(
            format!("{padding}{line}"),
            Style::default().fg(if index == 1 {
                Color::LightGreen
            } else {
                Color::Green
            }),
        ))
    }));
    lines.push(Line::from(""));
    lines.push(Line::from(Span::styled(
        tr(
            language,
            "  Press a to add a Bilibili URL, local file, or folder.",
            "  按 a 添加哔哩哔哩链接、本地文件或文件夹。请使用英文键盘输入快捷键。",
        ),
        Style::default().fg(Color::DarkGray),
    )));
    lines
}

fn help_lines(language: Language) -> Vec<Line<'static>> {
    let lines = match language {
        Language::English => vec![
            "MuxSift TUI HELP",
            "",
            "Tasks",
            "  a       add a Bilibili URL, local file, or folder",
            "  e       edit the selected task URL and mode",
            "  Up/Down j/k   move selection",
            "  Space   mark or unmark the selected task",
            "  Enter   start the selected task",
            "  S       start all marked tasks",
            "  d       delete the selected task",
            "  E       expand or collapse the selected multi-page group",
            "  b       switch media backend: HTTP or aria2",
            "  B       switch transcript backend: Bcut or Buzz",
            "  r       resume the failed task from its last saved stage",
            "  x       cancel the selected running task",
            "  P       pause or resume the selected download",
            "  Q       cycle quality",
            "  v       cycle codec",
            "  [ / ]   change selected page number",
            "  f       cycle subtitle/transcript format",
            "  /       search pages in the page picker",
            "  A       select search results; empty search toggles all",
            "  I       invert search results",
            "",
            "Modes and account",
            "  m       cycle download mode",
            "  c       open all settings: General / ASR / AI / Paths",
            "  C       open the AI section of settings",
            "  Tab     switch settings sections; Up/Down select a setting",
            "  Enter   edit or toggle; F2 saves a multiline prompt",
            "  g       switch interface language",
            "  l       QR login",
            "  L       clear local credentials (logout)",
            "",
            "  h / ?   close this help",
            "  Up/Down scroll this help",
            "  Esc     close dialogs or cancel editing",
            "  q       quit TUI",
        ],
        Language::Chinese => vec![
            "MuxSift TUI 帮助",
            "",
            "任务操作",
            "  a       添加哔哩哔哩链接、本地文件或文件夹",
            "  e       编辑当前任务的链接和模式",
            "  上/下 或 j/k   移动选择",
            "  空格    勾选或取消勾选当前任务",
            "  Enter   启动当前任务",
            "  S       启动全部已勾选任务",
            "  d       删除当前任务",
            "  E       展开或收起当前多 P 任务",
            "  b       切换下载后端：HTTP / aria2",
            "  B       切换转写后端：Bcut / Buzz",
            "  r       从最近成功阶段继续当前失败任务",
            "  x       取消当前运行任务",
            "  P       暂停或继续当前下载",
            "  Q       切换画质",
            "  v       切换编码",
            "  [ / ]   切换分 P 编号",
            "  f       切换字幕/转写格式",
            "  /       在 Pages 选择页搜索",
            "  A       全选搜索结果；空搜索切换全部",
            "  I       反选搜索结果",
            "",
            "模式与账号",
            "  m       切换下载模式",
            "  c       打开统一设置：通用／转写／AI／输出目录",
            "  C       直接打开设置中的 AI 分类",
            "  Tab     切换设置分类；上/下选择设置项",
            "  Enter   编辑或切换；F2 保存多行提示词",
            "  g       切换界面语言",
            "  l       二维码登录",
            "  L       清除本地凭据（登出）",
            "",
            "  h / ?   关闭帮助",
            "  上/下    滚动帮助",
            "  Esc     关闭窗口或取消输入",
            "  q       退出 TUI",
        ],
    };
    lines
        .into_iter()
        .enumerate()
        .map(|(index, line)| {
            if index == 0 {
                Line::from(Span::styled(line, Style::default().fg(Color::Cyan)))
            } else {
                Line::from(line)
            }
        })
        .collect()
}

fn task_status_label(status: &str, language: Language) -> &str {
    if status.starts_with("downloading") {
        tr(language, "downloading", "下载中")
    } else if status.starts_with("resolving") {
        tr(language, "resolving", "解析中")
    } else if status.starts_with("resuming:") {
        tr(language, "resuming", "恢复中")
    } else if status.starts_with("merging") {
        tr(language, "merging", "合并中")
    } else if status.starts_with("transcribing") {
        tr(language, "transcribing", "转写中")
    } else if status.starts_with("completed") {
        tr(language, "completed", "已完成")
    } else if status.starts_with("failed") {
        tr(language, "failed", "失败")
    } else if status.starts_with("Buzz:") {
        tr(language, "transcribing", "转写中")
    } else {
        status
    }
}

fn localized_task_status(status: &str, language: Language) -> String {
    if language == Language::English {
        return status.to_owned();
    }
    let prefixes = [
        (
            "queued · local media → SRT → AI notes",
            "排队 · 本地媒体 → SRT → AI 笔记",
        ),
        ("queued · ", "排队 · "),
        ("planned · ", "已计划 · "),
        ("downloading · ", "下载中 · "),
        ("completed · ", "已完成 · "),
        ("failed · ", "失败 · "),
        ("resolving metadata", "正在解析视频信息"),
        ("cancelling", "正在取消"),
        ("cancelled", "已取消"),
        (
            "waiting for existing SRT confirmation",
            "等待确认是否使用已有 SRT",
        ),
        ("paused", "已暂停"),
        (
            "resuming: using saved audio download",
            "恢复：复用已下载音频",
        ),
        (
            "resuming: using saved video download",
            "恢复：复用已下载视频",
        ),
        ("resuming: using saved M4A", "恢复：复用已封装 M4A"),
        ("resuming", "正在继续"),
        ("fetching subtitle", "正在获取字幕"),
        ("downloading audio + video", "正在下载音频和视频"),
        ("downloading audio", "正在下载音频"),
        (
            "merging video and audio with FFmpeg",
            "正在用 FFmpeg 合并音视频",
        ),
        ("merging audio with FFmpeg", "正在用 FFmpeg 合并音频"),
        ("extracting audio with FFmpeg", "正在用 FFmpeg 提取音频"),
        ("reusing extracted audio", "恢复：复用已提取音频"),
        ("reusing existing SRT", "恢复：复用已有 SRT"),
        (
            "uploading audio to Bcut cloud ASR",
            "正在上传音频至 Bcut 云端转写",
        ),
        ("transcribing with local Buzz ASR", "正在用 Buzz 本地转写"),
        ("organizing transcript with AI", "正在用 AI 整理转写稿"),
        ("Bcut: waiting for slot", "Bcut：正在等待队列"),
        ("Bcut: creating task", "Bcut：正在创建任务"),
        ("Bcut: recognizing · ", "Bcut：正在识别 · "),
        ("Bcut: rendering SRT", "Bcut：正在生成 SRT"),
        (
            "Bcut: waiting for the safety queue",
            "Bcut：正在等待转写队列",
        ),
        ("Bcut: preparing upload", "Bcut：正在准备上传"),
        ("Bcut: creating recognition task", "Bcut：正在创建识别任务"),
        ("Bcut: rendering transcript", "Bcut：正在生成转写稿"),
        (
            "Buzz: waiting for local ASR slot",
            "Buzz：正在等待本地转写队列",
        ),
        ("Buzz: loading local model ", "Buzz：正在加载本地模型 "),
        ("Buzz: loading local model", "Buzz：正在加载本地模型"),
        ("Buzz: transcribing · ", "Buzz：本地转写中 · "),
        ("Buzz: rendering transcript", "Buzz：正在生成转写稿"),
        (
            "AI: organizing SRT as Markdown",
            "AI：正在将 SRT 整理为 Markdown",
        ),
    ];
    let mut localized = status.to_owned();
    for (english, chinese) in prefixes {
        if let Some(rest) = status.strip_prefix(english) {
            localized = format!("{chinese}{rest}");
            break;
        }
    }
    localized.replace(
        "press r to retry from the last saved stage",
        "按 r 从最近成功阶段重试",
    )
}

fn localized_auth_status(status: &str, language: Language) -> String {
    if language == Language::English {
        return status.to_owned();
    }
    let exact = [
        ("checking", "正在检查登录状态"),
        ("logged in", "已登录"),
        ("anonymous", "未登录"),
        ("loading page list", "正在加载分 P 列表"),
        ("select pages", "请选择分 P"),
        ("scanning local folder", "正在扫描本地文件夹"),
        ("select local files", "请选择本地媒体文件"),
        ("scan QR and confirm on phone", "请扫码后在手机上确认"),
        ("generating login QR", "正在生成登录二维码"),
        ("logging out", "正在登出"),
    ];
    if let Some((_, chinese)) = exact.iter().find(|(english, _)| *english == status) {
        return (*chinese).to_owned();
    }
    if let Some(rest) = status.strip_prefix("page list failed · ") {
        return format!("加载分 P 失败 · {rest}");
    }
    if let Some(rest) = status.strip_prefix("local folder failed · ") {
        return format!("扫描本地文件夹失败 · {rest}");
    }
    if let Some(rest) = status.strip_prefix("login failed · ") {
        return format!("登录失败 · {rest}");
    }
    if let Some(rest) = status.strip_prefix("local file not found or not a regular file: ") {
        return format!("本地文件不存在或不是普通文件：{rest}");
    }
    status.to_owned()
}

fn task_progress_totals(task: &Task) -> Option<(u64, u64)> {
    if let Some(progress) = task.buzz_progress {
        return Some((progress.processed_ms, progress.total_ms));
    }
    let downloaded = task
        .transfers
        .values()
        .map(|transfer| transfer.downloaded)
        .sum();
    let total = task
        .transfers
        .values()
        .map(|transfer| transfer.total)
        .collect::<Option<Vec<_>>>()?
        .into_iter()
        .sum();
    (total > 0).then_some((downloaded, total))
}

fn transition_progress_for_phase(
    transfers: &mut HashMap<String, TransferProgress>,
    buzz_progress: &mut Option<BuzzTaskProgress>,
    message: &str,
) {
    if message.starts_with("Buzz:") {
        transfers.clear();
        buzz_progress.get_or_insert(BuzzTaskProgress {
            processed_ms: 0,
            total_ms: 1,
        });
    } else if !message.starts_with("downloading") {
        transfers.clear();
        *buzz_progress = None;
    }
}

fn seconds_to_millis(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn format_media_time(seconds: f64) -> String {
    let seconds = seconds.max(0.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}",
        seconds / 3_600,
        seconds / 60 % 60,
        seconds % 60
    )
}

fn progress_bar(downloaded: u64, total: u64, width: usize) -> String {
    let width = width.max(1);
    let filled = if total == 0 {
        0
    } else {
        ((downloaded as u128 * width as u128) / total as u128).min(width as u128) as usize
    };
    format!("[{}{}]", "#".repeat(filled), "-".repeat(width - filled))
}

fn clip_text(value: &str, max_chars: usize) -> String {
    let count = value.chars().count();
    if count <= max_chars {
        return value.to_owned();
    }
    if max_chars <= 3 {
        return value.chars().take(max_chars).collect();
    }
    let mut clipped: String = value.chars().take(max_chars - 3).collect();
    clipped.push_str("...");
    clipped
}

/// Returns the current logical input line, clipped by terminal display cells.
/// `input_cursor` is a character offset, while CJK characters usually occupy two cells.
fn visible_input_line(value: &str, input_cursor: usize, max_width: usize) -> (String, usize) {
    let chars: Vec<char> = value.chars().collect();
    let cursor = input_cursor.min(chars.len());
    let line_start = chars[..cursor]
        .iter()
        .rposition(|character| *character == '\n')
        .map_or(0, |index| index + 1);
    let line_end = chars[cursor..]
        .iter()
        .position(|character| *character == '\n')
        .map_or(chars.len(), |offset| cursor + offset);
    let content_width = max_width.saturating_sub(6).max(1);
    let mut start = line_start;
    while start < cursor
        && chars[start..cursor]
            .iter()
            .map(|character| character.width().unwrap_or(0))
            .sum::<usize>()
            > content_width
    {
        start += 1;
    }
    let mut end = cursor;
    let mut used = chars[start..cursor]
        .iter()
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    while end < line_end {
        let width = chars[end].width().unwrap_or(0);
        if used + width > content_width {
            break;
        }
        used += width;
        end += 1;
    }
    let left = start > line_start;
    let right = end < line_end;
    let before_cursor_width = chars[start..cursor]
        .iter()
        .map(|character| character.width().unwrap_or(0))
        .sum::<usize>();
    let mut display = String::new();
    if left {
        display.push_str("...");
    }
    display.extend(chars[start..end].iter());
    if right {
        display.push_str("...");
    }
    (display, before_cursor_width + if left { 3 } else { 0 })
}

fn format_bytes(value: u64) -> String {
    format_bytes_f64(value as f64)
}

fn format_bytes_f64(value: f64) -> String {
    const UNITS: [&str; 4] = ["B", "KB", "MB", "GB"];
    let mut amount = value.max(0.0);
    let mut unit = 0;
    while amount >= 1024.0 && unit + 1 < UNITS.len() {
        amount /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{amount:.0} {}", UNITS[unit])
    } else {
        format!("{amount:.1} {}", UNITS[unit])
    }
}

fn format_duration(seconds: f64) -> String {
    let seconds = seconds.ceil().max(0.0) as u64;
    if seconds >= 3_600 {
        format!("{}h{:02}m", seconds / 3_600, (seconds % 3_600) / 60)
    } else if seconds >= 60 {
        format!("{}m{:02}s", seconds / 60, seconds % 60)
    } else {
        format!("{seconds}s")
    }
}

fn wrap_text(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    if value.is_empty() {
        return vec![String::new()];
    }
    let mut lines = Vec::new();
    let mut line = String::new();
    let mut line_width = 0;
    for character in value.chars() {
        if character == '\n' {
            lines.push(line);
            line = String::new();
            line_width = 0;
            continue;
        }
        let character_width = character.width().unwrap_or(0);
        if line_width > 0 && line_width + character_width > width {
            lines.push(line);
            line = String::new();
            line_width = 0;
        }
        line.push(character);
        line_width += character_width;
    }
    if !line.is_empty() || value.ends_with('\n') {
        lines.push(line);
    }
    lines
}

fn picker_local_directory_lines(
    focused: bool,
    relative: &Path,
    depth: usize,
    counts: (usize, usize),
    collapsed: bool,
    width: usize,
) -> Vec<Line<'static>> {
    let (total, selected_count) = counts;
    let marker = if selected_count == total {
        "x"
    } else if selected_count == 0 {
        " "
    } else {
        "~"
    };
    let name = relative
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    vec![Line::from(clip_text(
        &format!(
            "{}{} [{}] [{}] {}/ ({selected_count}/{})",
            "  ".repeat(depth),
            if focused { ">" } else { " " },
            marker,
            if collapsed { "+" } else { "-" },
            name,
            total
        ),
        width,
    ))]
}

fn picker_local_file_lines(
    focused: bool,
    selected: bool,
    file: &LocalFile,
    depth: usize,
    width: usize,
) -> Vec<Line<'static>> {
    let name = file
        .relative
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_default();
    vec![Line::from(clip_text(
        &format!(
            "{}{} [{}] {}",
            "  ".repeat(depth),
            if focused { ">" } else { " " },
            if selected { "x" } else { " " },
            name
        ),
        width,
    ))]
}

fn picker_page_lines(
    focused: bool,
    selected: bool,
    page: &muxsift_core::PageInfo,
    width: usize,
    nested: bool,
    language: Language,
) -> Vec<Line<'static>> {
    let cursor = if focused { ">" } else { " " };
    let marker = if selected { "x" } else { " " };
    let indent = if nested { "    " } else { "" };
    let mut lines = vec![Line::from(format!(
        "{indent}{cursor} [{marker}] P{}",
        page.episode_page.unwrap_or(page.page)
    ))];
    let title_label = format!("{}: ", tr(language, "Title", "标题"));
    let prefix = format!("{indent}      {title_label}");
    let continuation = format!("{indent}      {}", " ".repeat(title_label.chars().count()));
    let title_width = width.saturating_sub(prefix.len()).max(1);
    for (index, title_line) in wrap_text(&page.part, title_width).into_iter().enumerate() {
        let line_indent = if index == 0 { &prefix } else { &continuation };
        lines.push(Line::from(format!("{line_indent}{title_line}")));
    }
    lines
}

fn picker_group_lines(
    focused: bool,
    group: &PageGroup,
    selected: &[bool],
    collapsed: bool,
) -> Vec<Line<'static>> {
    let cursor = if focused { ">" } else { " " };
    let pages: Vec<_> = group
        .episodes
        .iter()
        .flat_map(|episode| &episode.pages)
        .collect();
    let selected_count = pages.iter().filter(|index| selected[***index]).count();
    let marker = if selected_count == pages.len() {
        "x"
    } else if selected_count == 0 {
        " "
    } else {
        "~"
    };
    let fold = if collapsed { "+" } else { "-" };
    let title = group.title.as_deref().unwrap_or("Pages");
    vec![Line::from(format!(
        "{cursor} [{marker}] [{fold}] {title} ({selected_count}/{})",
        pages.len()
    ))]
}

fn picker_episode_lines(
    focused: bool,
    episode: &PickerEpisode,
    selected: &[bool],
    collapsed: bool,
) -> Vec<Line<'static>> {
    let selected_count = episode
        .pages
        .iter()
        .filter(|index| selected[**index])
        .count();
    let marker = if selected_count == episode.pages.len() {
        "x"
    } else if selected_count == 0 {
        " "
    } else {
        "~"
    };
    let cursor = if focused { ">" } else { " " };
    let fold = if collapsed { "+" } else { "-" };
    vec![Line::from(format!(
        "  {cursor} [{marker}] [{fold}] {} ({selected_count}/{})",
        episode.title.as_deref().unwrap_or("Video"),
        episode.pages.len()
    ))]
}

fn centered_rect(
    percent_x: u16,
    percent_y: u16,
    area: ratatui::layout::Rect,
) -> ratatui::layout::Rect {
    let vertical = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(area);
    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(vertical[1])[1]
}

fn spinner(tick: usize) -> &'static str {
    // Six positions across a compact 2x3-style dot sequence.
    ["●··", "·●·", "··●", "··●", "·●·", "●··"][tick % 6]
}

fn display_task_title(video_title: &str, page_title: &str) -> String {
    let page_title = page_title.trim();
    if page_title.is_empty() {
        video_title.to_owned()
    } else {
        page_title.to_owned()
    }
}

fn tui_resume_identity(
    plan: &muxsift_core::DownloadPlan,
    mode: Mode,
    subtitle_format: SubtitleFormat,
    transcript_format: TranscriptFormat,
) -> String {
    let mode = match mode {
        Mode::Video => "video".to_owned(),
        Mode::Audio => "audio".to_owned(),
        Mode::Subtitle => format!("subtitle:{}", subtitle_format.extension()),
        Mode::Transcript => format!("transcribe:{}", transcript_format.extension()),
    };
    let streams = match mode.as_str() {
        "video" => format!(
            "{}:{}\n{}:{}",
            plan.selected_video.id,
            plan.selected_video.codecs,
            plan.selected_audio.id,
            plan.selected_audio.codecs
        ),
        value if value.starts_with("subtitle:") => String::new(),
        _ => format!("{}:{}", plan.selected_audio.id, plan.selected_audio.codecs),
    };
    format!(
        "{}\n{}\n{}\n{}",
        plan.video.bvid, plan.page.cid, mode, streams
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn transfer() -> TransferProgress {
        TransferProgress {
            downloaded: 100,
            total: Some(100),
            observed_at: Instant::now(),
            bytes_per_second: 10.0,
        }
    }

    #[test]
    fn switches_from_completed_download_to_buzz_progress() {
        let mut transfers = HashMap::from([("audio".into(), transfer())]);
        let mut buzz_progress = None;

        transition_progress_for_phase(
            &mut transfers,
            &mut buzz_progress,
            "merging audio with FFmpeg",
        );
        assert!(transfers.is_empty());
        assert!(buzz_progress.is_none());

        transfers.insert("audio".into(), transfer());
        transition_progress_for_phase(
            &mut transfers,
            &mut buzz_progress,
            "Buzz: loading local model large-v3-turbo",
        );
        assert!(transfers.is_empty());
        let progress = buzz_progress.expect("Buzz progress should start immediately");
        assert_eq!(progress.processed_ms, 0);
        assert_eq!(progress.total_ms, 1);

        buzz_progress = Some(BuzzTaskProgress {
            processed_ms: 40,
            total_ms: 100,
        });
        transition_progress_for_phase(
            &mut transfers,
            &mut buzz_progress,
            "Buzz: rendering transcript",
        );
        assert_eq!(
            buzz_progress
                .expect("Buzz progress should remain")
                .processed_ms,
            40
        );

        transition_progress_for_phase(
            &mut transfers,
            &mut buzz_progress,
            "AI: organizing SRT as Markdown",
        );
        assert!(buzz_progress.is_none());
    }

    #[test]
    fn existing_srt_prompt_returns_selected_action() {
        let mut app = App::new(
            PathBuf::from("/tmp/muxsift-test-config.json"),
            Language::Chinese,
        );
        let (response, mut answer) = tokio::sync::oneshot::channel();
        app.existing_srt_prompts.push_back(ExistingSrtPrompt {
            index: 42,
            path: PathBuf::from("existing.srt"),
            use_existing: false,
            response,
        });

        app.answer_existing_srt(true);

        assert!(app.existing_srt_prompts.is_empty());
        assert_eq!(answer.try_recv(), Ok(true));
    }

    #[test]
    fn planned_multi_page_task_keeps_its_page_title() {
        assert_eq!(
            display_task_title("多 P 合集总标题", "第三 P 的标题"),
            "第三 P 的标题"
        );
        assert_eq!(display_task_title("单视频标题", "  "), "单视频标题");
    }

    #[test]
    fn wrap_text_preserves_prompt_line_breaks() {
        assert_eq!(wrap_text("第一行\n第二行\n", 20), ["第一行", "第二行", ""]);
    }

    #[test]
    fn expands_home_paths_for_buzz_settings() {
        let home = std::env::var("HOME").expect("HOME should be set during tests");
        assert_eq!(
            expand_home_path("~/Projects/buzz"),
            format!("{home}/Projects/buzz")
        );
        assert_eq!(expand_home_path("uv"), "uv");
    }

    #[test]
    fn local_folder_picker_keeps_tree_and_distinct_outputs() {
        let directory = tempfile::tempdir().expect("temporary directory");
        let root = directory.path().join("讲座");
        std::fs::create_dir_all(root.join("第一章/片段")).expect("nested directory");
        std::fs::create_dir_all(root.join("第二章")).expect("second directory");
        for relative in [
            "第一章/片段/同名.mp4",
            "第一章/片段/同名.mkv",
            "第二章/声音.MP3",
        ] {
            std::fs::write(root.join(relative), b"media").expect("media file");
        }
        std::fs::write(root.join("第二章/忽略.txt"), b"text").expect("ignored file");
        std::fs::write(root.join("第二章/空.mp4"), b"").expect("empty media");
        std::fs::write(root.join("第二章/脚本.ts"), b"export const example = 1;")
            .expect("TypeScript file");
        let mut transport = [0u8; 189];
        transport[0] = 0x47;
        transport[188] = 0x47;
        std::fs::write(root.join("第二章/视频.ts"), transport).expect("MPEG transport stream");
        let (root, files) = scan_local_media_directory(&root).expect("scan local media");
        assert_eq!(files.len(), 4);
        assert!(
            files
                .iter()
                .any(|file| file.relative == Path::new("第一章/片段/同名.mkv"))
        );
        assert!(
            files
                .iter()
                .any(|file| file.relative == Path::new("第二章/视频.ts"))
        );

        let mut app = App::new(directory.path().join("config.json"), Language::Chinese);
        app.sender
            .send(AppEvent::LocalList {
                root: root.clone(),
                files,
            })
            .expect("queue folder");
        app.drain_events();
        let picker = app.page_picker.as_mut().expect("local picker");
        let rows = picker_rows(picker);
        assert!(rows.iter().any(|row| matches!(row, PickerRow::LocalDirectory { relative, depth: 1 } if relative == Path::new("第一章/片段"))));
        assert_eq!(
            rows.iter()
                .filter(|row| matches!(row, PickerRow::LocalFile { .. }))
                .count(),
            4
        );
        picker.collapsed_local_dirs.insert(PathBuf::from("第一章"));
        assert_eq!(
            picker_rows(picker)
                .iter()
                .filter(|row| matches!(row, PickerRow::LocalFile { .. }))
                .count(),
            2
        );
        picker.selected.fill(true);
        app.confirm_pages();
        assert_eq!(app.tasks.len(), 4);
        assert!(app.tasks.iter().all(|task| task.local && task.grouped));
        let outputs: HashSet<_> = app
            .tasks
            .iter()
            .map(|task| {
                task.local_output_subdir
                    .clone()
                    .expect("output subdir")
                    .join(Path::new(&task.input).file_stem().expect("stem"))
            })
            .collect();
        assert_eq!(outputs.len(), 4);
    }
}
