use std::{
    io::{self, IsTerminal, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    time::{Duration, Instant},
};

use clap::{Parser, Subcommand, ValueEnum};
use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use muxsift_core::{
    AiClient, AiOptions, Aria2Manager, Aria2Options, AsrBackend, BiliClient, BuzzOptions,
    BuzzProgress, ClientOptions, DownloadProgress, Error, LocalMediaEvent, LocalMediaRequest,
    LoginPoll, Result, ResumeWorkspace, SubtitleFormat, SubtitleTrack, TranscriptFormat,
    final_output_path, final_output_path_with_extension, mux, mux_audio, process_local_media,
    resolve_video_input, sanitize_filename, set_bcut_cooldown_seconds, transcribe_audio_with_buzz,
};
use muxsift_tui::{Language, MediaBackend, TuiOptions};
use qrcode::{QrCode, render::unicode};

mod config;

#[derive(Parser, Debug)]
#[command(
    name = "muxsift",
    version,
    subcommand_precedence_over_arg = true,
    about = "Acquire, process, and distill media"
)]
struct Cli {
    /// BV ID, Bilibili video URL, or b23.tv mobile short link.
    input: Option<String>,

    #[command(subcommand)]
    command: Option<Command>,

    /// Select one page, starting at 1. Overrides URL p=.
    #[arg(long)]
    page: Option<usize>,

    /// Download every page in the video.
    #[arg(long, conflicts_with = "page")]
    pages: Option<String>,

    /// Select UGC collection episodes: `all` or a 1-based list/range such as `1,3,5-7`.
    #[arg(long, conflicts_with_all = ["page", "pages"])]
    collection: Option<String>,

    /// Required quality, such as 1080p, 720p, or a numeric qn.
    #[arg(long, value_parser = parse_quality)]
    quality: Option<u64>,

    /// Preferred codec within the selected quality.
    #[arg(long, value_enum)]
    codec: Option<Codec>,

    /// Output directory.
    #[arg(short = 'o', long, default_value = ".")]
    output: Option<PathBuf>,

    /// Override the JSON configuration file path.
    #[arg(long, global = true)]
    config: Option<PathBuf>,

    /// Resolve metadata and streams without downloading or starting FFmpeg.
    #[arg(long)]
    dry_run: bool,

    /// Continue from the last completed stage saved by a failed earlier run.
    #[arg(long)]
    resume: bool,

    /// Download only the audio stream as an M4A file.
    #[arg(long)]
    audio_only: bool,

    /// Download the audio and transcribe it with the configured ASR backend.
    #[arg(long)]
    transcribe: bool,

    /// ASR backend used by --transcribe.
    #[arg(long, value_enum, requires = "transcribe")]
    asr: Option<AsrBackendArg>,

    /// Buzz source checkout containing its uv environment.
    #[arg(long, requires = "transcribe")]
    buzz_project: Option<PathBuf>,

    /// uv executable used to run the Buzz bridge.
    #[arg(long, requires = "transcribe")]
    buzz_uv: Option<PathBuf>,

    /// Headless Buzz bridge script, or `auto` for the bundled script.
    #[arg(long, requires = "transcribe")]
    buzz_bridge: Option<PathBuf>,

    /// Local faster-whisper model name or directory.
    #[arg(long, requires = "transcribe")]
    buzz_model: Option<String>,

    /// faster-whisper device: auto, cuda, or cpu.
    #[arg(long, requires = "transcribe")]
    buzz_device: Option<String>,

    /// faster-whisper compute type, for example default, float16, or int8.
    #[arg(long, requires = "transcribe")]
    buzz_compute_type: Option<String>,

    /// Optional recognition language code; omitted means auto-detect.
    #[arg(long, requires = "transcribe")]
    buzz_language: Option<String>,

    /// Transcript output format.
    #[arg(long, value_enum, default_value = "srt")]
    transcript_format: TranscriptFormatArg,

    /// Download Bilibili's native subtitle track.
    #[arg(long, conflicts_with_all = ["transcribe", "audio_only"])]
    subtitle: bool,

    /// Native subtitle output format.
    #[arg(long, value_enum, default_value = "srt")]
    subtitle_format: SubtitleFormatArg,

    /// Preferred native subtitle language, such as zh-CN or en-US.
    #[arg(long)]
    subtitle_lang: Option<String>,

    /// Print a sanitized JSON plan.
    #[arg(long)]
    json: bool,

    /// FFmpeg executable or absolute path.
    #[arg(long, default_value = "ffmpeg")]
    ffmpeg: PathBuf,

    /// Media download engine.
    #[arg(long, value_enum)]
    backend: Option<DownloadBackend>,

    /// aria2c executable or absolute path, used with `--backend aria2`.
    #[arg(long)]
    aria2: Option<PathBuf>,

    /// Maximum simultaneous aria2 downloads across all queued media streams.
    #[arg(long)]
    max_concurrent_downloads: Option<usize>,

    /// Connections used by aria2 for one media stream.
    #[arg(long)]
    split: Option<u8>,

    /// Minimum aria2 segment size, for example 15M.
    #[arg(long)]
    min_split_size: Option<String>,

    /// Route HTTP and HTTPS through this proxy.
    #[arg(long, global = true, conflicts_with = "no_proxy")]
    proxy: Option<String>,

    /// Override the proxy only for media CDN requests.
    #[arg(long, global = true)]
    media_proxy: Option<String>,

    /// Ignore proxy environment variables and connect directly.
    #[arg(long, global = true)]
    no_proxy: bool,

    /// Override the credential file location.
    #[arg(long, hide = true, global = true)]
    credentials: Option<PathBuf>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Transcribe a local video or audio file and optionally organize it with AI.
    Local {
        /// Existing video or audio file.
        input: PathBuf,
        /// Choose local Buzz or cloud Bcut transcription explicitly.
        #[arg(long, value_enum)]
        asr: AsrBackendArg,
        /// Directory for the resulting SRT and Markdown files.
        #[arg(short, long)]
        output: Option<PathBuf>,
        /// Reuse a completed audio extraction or existing SRT after a failed run.
        #[arg(long)]
        resume: bool,
        /// Stop after creating SRT; AI configuration is not required.
        #[arg(long)]
        transcript_only: bool,
        /// Buzz project directory containing its uv environment.
        #[arg(long)]
        buzz_project: Option<PathBuf>,
        /// Buzz model name or local model path.
        #[arg(long)]
        buzz_model: Option<String>,
    },
    /// Log in by scanning a terminal QR code with the Bilibili mobile app.
    Login,
    /// Remove locally stored Bilibili credentials.
    Logout,
    /// Open the keyboard-first terminal interface.
    Tui,
    /// Test the configured OpenAI-compatible AI provider.
    AiTest,
    /// Manage default output directories.
    Config {
        #[command(subcommand)]
        command: ConfigCommand,
    },
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum DownloadBackend {
    Http,
    Aria2,
}

#[derive(Debug, Clone, Copy, ValueEnum)]
enum AsrBackendArg {
    Bcut,
    Buzz,
}

impl From<AsrBackendArg> for AsrBackend {
    fn from(value: AsrBackendArg) -> Self {
        match value {
            AsrBackendArg::Bcut => Self::Bcut,
            AsrBackendArg::Buzz => Self::Buzz,
        }
    }
}

#[derive(Subcommand, Debug)]
enum ConfigCommand {
    /// Print the effective configuration and file path.
    Show,
    /// Create a commented-free default configuration file.
    Init,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum Codec {
    Avc,
    Hevc,
    Av1,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum TranscriptFormatArg {
    Srt,
    Txt,
    Lrc,
    Json,
}

#[derive(ValueEnum, Clone, Copy, Debug)]
enum SubtitleFormatArg {
    Srt,
    Vtt,
}

impl SubtitleFormatArg {
    fn core(self) -> SubtitleFormat {
        match self {
            Self::Srt => SubtitleFormat::Srt,
            Self::Vtt => SubtitleFormat::Vtt,
        }
    }
}

impl TranscriptFormatArg {
    fn core(self) -> TranscriptFormat {
        match self {
            Self::Srt => TranscriptFormat::Srt,
            Self::Txt => TranscriptFormat::Txt,
            Self::Lrc => TranscriptFormat::Lrc,
            Self::Json => TranscriptFormat::Json,
        }
    }
}

impl Codec {
    fn api_name(self) -> &'static str {
        match self {
            Self::Avc => "avc1",
            Self::Hevc => "hevc",
            Self::Av1 => "av01",
        }
    }
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();
    let work = run(cli);
    tokio::select! {
        result = work => finish(result),
        signal = tokio::signal::ctrl_c() => {
            if let Err(error) = signal {
                eprintln!("error: could not install Ctrl+C handler: {error}");
                ExitCode::from(1)
            } else {
                eprintln!("cancelled");
                ExitCode::from(130)
            }
        }
    }
}

fn finish(result: Result<()>) -> ExitCode {
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::from(match error {
                Error::InvalidInput(_)
                | Error::PageOutOfRange { .. }
                | Error::QualityUnavailable(..) => 2,
                Error::NotLoggedIn => 4,
                Error::FfmpegMissing(_) | Error::FfmpegFailed { .. } => 6,
                Error::Cancelled => 130,
                _ => 1,
            })
        }
    }
}

async fn run(mut cli: Cli) -> Result<()> {
    cli.ffmpeg = resolve_ffmpeg_executable(&cli.ffmpeg);
    let config_path = cli.config.clone().unwrap_or_else(config::default_path);
    let app_config = config::load(&config_path)?;
    set_bcut_cooldown_seconds(app_config.bcut.cooldown_seconds);
    let asr_backend = cli.asr.map(AsrBackend::from).unwrap_or_else(|| {
        if app_config.asr.backend.eq_ignore_ascii_case("buzz") {
            AsrBackend::Buzz
        } else {
            AsrBackend::Bcut
        }
    });
    let buzz_options = BuzzOptions {
        project_dir: cli
            .buzz_project
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.project_dir.clone()),
        uv: cli
            .buzz_uv
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.uv.clone()),
        bridge: cli
            .buzz_bridge
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.bridge.clone()),
        model: cli
            .buzz_model
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.model.clone()),
        device: cli
            .buzz_device
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.device.clone()),
        compute_type: cli
            .buzz_compute_type
            .clone()
            .unwrap_or_else(|| app_config.asr.buzz.compute_type.clone()),
        language: cli
            .buzz_language
            .clone()
            .or_else(|| app_config.asr.buzz.language.clone()),
    };
    if let Some(Command::Local {
        input,
        asr,
        output,
        resume,
        transcript_only,
        buzz_project,
        buzz_model,
    }) = &cli.command
    {
        let client = BiliClient::new(ClientOptions {
            proxy: cli.proxy.clone(),
            media_proxy: cli.media_proxy.clone(),
            no_proxy: cli.no_proxy,
            credential_path: cli.credentials.clone(),
        })?;
        let ai = if *transcript_only {
            None
        } else {
            Some(ai_client_from_config(&app_config)?)
        };
        let mut local_buzz = buzz_options;
        if let Some(project) = buzz_project {
            local_buzz.project_dir = project.clone();
        }
        if let Some(model) = buzz_model {
            local_buzz.model = model.clone();
        }
        let request = LocalMediaRequest {
            input: input.clone(),
            output_dir: output
                .clone()
                .unwrap_or_else(|| app_config.paths.transcript.clone()),
            ffmpeg: cli.ffmpeg.clone(),
            asr: (*asr).into(),
            buzz: local_buzz,
            resume: *resume,
            make_markdown: !transcript_only,
            control: None,
        };
        let result = process_local_media(&request, &client, ai.as_ref(), |event| match event {
            LocalMediaEvent::Stage(stage) => eprintln!("{stage}"),
            LocalMediaEvent::Bcut(muxsift_core::BcutProgress::Uploading {
                completed,
                total,
                ..
            }) => eprintln!("Bcut upload: {completed}/{total} parts"),
            LocalMediaEvent::Buzz(BuzzProgress::Transcribing {
                processed_seconds,
                total_seconds,
            }) => eprint!(
                "\rBuzz: {} / {}",
                format_media_time(processed_seconds),
                format_media_time(total_seconds)
            ),
            _ => {}
        })
        .await?;
        eprintln!();
        println!("SRT: {}", result.transcript.display());
        if let Some(markdown) = result.markdown {
            println!("Markdown: {}", markdown.display());
        }
        return Ok(());
    }
    let backend = cli.backend.unwrap_or_else(|| {
        if app_config.download.backend.eq_ignore_ascii_case("aria2") {
            DownloadBackend::Aria2
        } else {
            DownloadBackend::Http
        }
    });
    let aria2 = cli
        .aria2
        .clone()
        .unwrap_or_else(|| app_config.download.aria2.clone());
    let aria2_options = Aria2Options {
        executable: aria2.clone(),
        max_concurrent_downloads: cli
            .max_concurrent_downloads
            .unwrap_or(app_config.download.max_concurrent_downloads),
        split: cli.split.unwrap_or(app_config.download.split),
        min_split_size: cli
            .min_split_size
            .clone()
            .unwrap_or_else(|| app_config.download.min_split_size.clone()),
    };
    let aria2_manager = match backend {
        DownloadBackend::Http => None,
        DownloadBackend::Aria2 => Some(Aria2Manager::new(aria2_options.clone())?),
    };
    let client = BiliClient::new(ClientOptions {
        proxy: cli.proxy.clone(),
        media_proxy: cli.media_proxy.clone(),
        no_proxy: cli.no_proxy,
        credential_path: cli.credentials.clone(),
    })?;
    match &cli.command {
        Some(Command::Local { .. }) => unreachable!("local command was handled above"),
        Some(Command::Login) => return login(&client).await,
        Some(Command::Logout) => {
            client.clear_credentials()?;
            println!("Logged out. Local credentials removed.");
            return Ok(());
        }
        Some(Command::AiTest) => {
            let ai = ai_client_from_config(&app_config)?;
            ai.test_connection().await?;
            println!("AI connection test succeeded: {}", app_config.ai.model);
            return Ok(());
        }
        Some(Command::Tui) => {
            muxsift_tui::run(TuiOptions {
                client: ClientOptions {
                    proxy: cli.proxy.clone(),
                    media_proxy: cli.media_proxy.clone(),
                    no_proxy: cli.no_proxy,
                    credential_path: cli.credentials.clone(),
                },
                video_output: app_config.paths.video.clone(),
                audio_output: app_config.paths.audio.clone(),
                subtitle_output: app_config.paths.subtitle.clone(),
                transcript_output: app_config.paths.transcript.clone(),
                ffmpeg: cli.ffmpeg.clone(),
                backend: match backend {
                    DownloadBackend::Http => MediaBackend::Http,
                    DownloadBackend::Aria2 => MediaBackend::Aria2,
                },
                aria2_options: aria2_options.clone(),
                aria2_manager: aria2_manager.clone(),
                bcut_cooldown_seconds: app_config.bcut.cooldown_seconds,
                asr_backend,
                buzz: buzz_options.clone(),
                // The TUI must remain usable to configure AI even before its key is exported.
                ai: if app_config.ai.enabled {
                    ai_options_from_config(&app_config).ok()
                } else {
                    None
                },
                ai_enabled: app_config.ai.enabled,
                ai_base_url: app_config.ai.base_url.clone(),
                ai_model: app_config.ai.model.clone(),
                ai_api_key_env: app_config.ai.api_key_env.clone(),
                ai_system_prompt: app_config.ai.system_prompt.clone(),
                language: Language::from_config(&app_config.ui.language),
                config_path: config_path.clone(),
            })
            .await
            .map_err(|error| Error::InvalidInput(format!("TUI failed: {error}")))?;
            return Ok(());
        }
        Some(Command::Config { command }) => {
            return match command {
                ConfigCommand::Show => {
                    println!("config: {}", config_path.display());
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&app_config).map_err(|error| {
                            Error::InvalidInput(format!("could not serialize config: {error}"))
                        })?
                    );
                    Ok(())
                }
                ConfigCommand::Init => {
                    config::write_default(&config_path)?;
                    println!("created: {}", config_path.display());
                    Ok(())
                }
            };
        }
        None => {}
    }
    let raw_input = cli.input.as_deref().ok_or_else(|| {
        Error::InvalidInput("provide a BV ID/URL, or use the login subcommand".into())
    })?;
    if !cli.transcribe && !matches!(cli.transcript_format, TranscriptFormatArg::Srt) {
        return Err(Error::InvalidInput(
            "--transcript-format requires --transcribe".into(),
        ));
    }
    if cli.subtitle_lang.is_some() && !cli.subtitle {
        return Err(Error::InvalidInput(
            "--subtitle-lang requires --subtitle".into(),
        ));
    }
    let page_selection = cli.pages.as_deref().map(parse_pages).transpose()?;
    let collection_selection = cli.collection.as_deref().map(parse_pages).transpose()?;
    let input =
        resolve_video_input(raw_input, cli.page, cli.proxy.as_deref(), cli.no_proxy).await?;
    let codec = cli.codec.map(Codec::api_name);
    let plans = match collection_selection.or(page_selection) {
        Some(PageSelection::All) => client.plan_all(&input, cli.quality, codec).await?,
        Some(PageSelection::Pages(pages)) => {
            let mut plans = Vec::with_capacity(pages.len());
            for page in pages {
                let page_input = muxsift_core::VideoInput {
                    bvid: input.bvid.clone(),
                    page,
                };
                plans.push(client.plan(&page_input, cli.quality, codec).await?);
            }
            plans
        }
        None => vec![client.plan(&input, cli.quality, codec).await?],
    };
    let multi_page = plans.len() > 1;
    let mode = if cli.subtitle {
        "subtitle"
    } else if cli.transcribe {
        "transcribe"
    } else if cli.audio_only {
        "audio-only"
    } else {
        "video+audio"
    };
    let output_root = cli
        .output
        .clone()
        .unwrap_or_else(|| config::output_dir(&app_config, mode).to_owned());
    let output_dir = if multi_page {
        let group_name = plans
            .first()
            .and_then(|plan| {
                plan.video
                    .ugc_season
                    .as_ref()
                    .map(|season| season.name.as_str())
            })
            .filter(|name| !name.trim().is_empty())
            .unwrap_or_else(|| {
                plans
                    .first()
                    .map(|plan| plan.video.title.as_str())
                    .unwrap_or("untitled")
            });
        output_root.join(sanitize_filename(group_name))
    } else {
        output_root
    };
    let output_for = |plan: &muxsift_core::DownloadPlan| {
        let title = if multi_page {
            format!("{:02}-{}", plan.page.page, plan.page.part)
        } else {
            plan.page.part.clone()
        };
        if cli.subtitle {
            final_output_path_with_extension(
                &output_dir,
                &title,
                cli.subtitle_format.core().extension(),
            )
        } else if cli.transcribe {
            final_output_path_with_extension(
                &output_dir,
                &title,
                cli.transcript_format.core().extension(),
            )
        } else if cli.audio_only {
            final_output_path_with_extension(&output_dir, &title, "m4a")
        } else {
            final_output_path(&output_dir, &title)
        }
    };
    if cli.json {
        let values: Vec<_> = plans
            .iter()
            .map(|plan| {
                let mut json = plan.redacted_json();
                json["output"] = serde_json::Value::String(output_for(plan).display().to_string());
                json["dry_run"] = serde_json::Value::Bool(cli.dry_run);
                json["mode"] = serde_json::Value::String(mode.into());
                json
            })
            .collect();
        if multi_page {
            println!("{}", serde_json::to_string_pretty(&values)?);
        } else if let Some(value) = values.into_iter().next() {
            println!("{}", serde_json::to_string_pretty(&value)?);
        }
    } else {
        for plan in &plans {
            print_plan(
                plan,
                &output_for(plan),
                cli.dry_run,
                cli.audio_only,
                cli.transcribe,
                cli.subtitle,
            );
        }
    }
    if cli.dry_run {
        return Ok(());
    }
    let ai_client = if cli.transcribe && app_config.ai.enabled {
        if !matches!(cli.transcript_format, TranscriptFormatArg::Srt) {
            return Err(Error::InvalidInput(
                "AI post-processing requires --transcript-format srt".into(),
            ));
        }
        Some(ai_client_from_config(&app_config)?)
    } else {
        None
    };
    std::fs::create_dir_all(&output_dir).map_err(|source| Error::Io {
        path: output_dir.clone(),
        source,
    })?;
    let jobs: Vec<_> = plans.iter().map(|plan| (plan, output_for(plan))).collect();
    let has_existing_output = jobs.iter().any(|(_, path)| path.exists());
    let task_concurrency = if has_existing_output {
        1
    } else {
        aria2_manager
            .as_ref()
            .map(|manager| manager.options().max_concurrent_downloads)
            .unwrap_or(1)
    };
    let client_ref = &client;
    let output_dir_ref = &output_dir;
    let cli_ref = &cli;
    let manager_ref = aria2_manager.as_ref();
    let ai_ref = ai_client.as_ref();
    let buzz_ref = &buzz_options;
    let results =
        futures_util::stream::iter(jobs.into_iter().map(move |(plan, final_path)| async move {
            let result = process_plan(
                client_ref,
                plan,
                &final_path,
                output_dir_ref,
                cli_ref,
                backend,
                manager_ref,
                ai_ref,
                asr_backend,
                buzz_ref,
            )
            .await;
            if result.is_err() {
                let checkpoint = ResumeWorkspace::path_for(&final_path);
                if checkpoint.exists() {
                    eprintln!(
                        "P{}: 已保留恢复检查点 {}；修复错误后用相同命令加 --resume 继续",
                        plan.page.page,
                        checkpoint.display()
                    );
                }
            }
            result
        }))
        .buffer_unordered(task_concurrency)
        .collect::<Vec<_>>()
        .await;
    for result in results {
        result?;
    }
    Ok(())
}

fn resolve_ffmpeg_executable(configured: &Path) -> PathBuf {
    if configured != Path::new("ffmpeg") && configured != Path::new("ffmpeg.exe") {
        return configured.to_owned();
    }
    let executable_name = if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    };
    let Ok(current) = std::env::current_exe() else {
        return configured.to_owned();
    };
    let Some(parent) = current.parent() else {
        return configured.to_owned();
    };
    for candidate in [
        parent.join("bin").join(executable_name),
        parent.join(executable_name),
    ] {
        if candidate.is_file() {
            return candidate;
        }
    }
    configured.to_owned()
}

fn ai_client_from_config(config: &config::Config) -> Result<AiClient> {
    AiClient::new(ai_options_from_config(config)?)
}

fn ai_options_from_config(config: &config::Config) -> Result<AiOptions> {
    if !config.ai.enabled {
        return Err(Error::InvalidInput(
            "AI post-processing is disabled; set ai.enabled to true".into(),
        ));
    }
    let key = std::env::var(&config.ai.api_key_env).map_err(|_| {
        Error::InvalidInput(format!(
            "AI API key environment variable is not set: {}",
            config.ai.api_key_env
        ))
    })?;
    Ok(AiOptions {
        base_url: config.ai.base_url.clone(),
        model: config.ai.model.clone(),
        api_key: key,
        system_prompt: config.ai.system_prompt.clone(),
    })
}

#[allow(clippy::too_many_arguments)]
async fn process_plan(
    client: &BiliClient,
    plan: &muxsift_core::DownloadPlan,
    final_path: &Path,
    output_dir: &Path,
    cli: &Cli,
    backend: DownloadBackend,
    aria2_manager: Option<&Aria2Manager>,
    ai_client: Option<&AiClient>,
    asr_backend: AsrBackend,
    buzz_options: &BuzzOptions,
) -> Result<()> {
    if final_path.exists() {
        if cli.transcribe && matches!(cli.transcript_format, TranscriptFormatArg::Srt) {
            if let Some(ai) = ai_client {
                let markdown_path = final_path.with_extension("md");
                if markdown_path.exists() {
                    return Err(Error::InvalidInput(format!(
                        "AI Markdown output already exists: {}",
                        markdown_path.display()
                    )));
                }
                if !cli.resume && !prompt_reuse_existing_srt(final_path).await? {
                    eprintln!("P{}: 已取消任务，保留现有 SRT", plan.page.page);
                    return Ok(());
                }
                let work = tempfile::Builder::new()
                    .prefix(".muxsift-ai-")
                    .tempdir_in(output_dir)
                    .map_err(|source| Error::Io {
                        path: output_dir.to_owned(),
                        source,
                    })?;
                let markdown_path =
                    organize_srt_with_ai(ai, final_path, work.path(), plan.page.page).await?;
                ResumeWorkspace::remove_for_output(final_path).await?;
                println!("saved: {}", markdown_path.display());
                return Ok(());
            }
        }
        let detail = if cli.transcribe && matches!(cli.transcript_format, TranscriptFormatArg::Srt)
        {
            " AI post-processing is disabled or unavailable; enable AI to reuse this SRT."
        } else {
            ""
        };
        return Err(Error::InvalidInput(format!(
            "output already exists: {}.{detail}",
            final_path.display()
        )));
    }
    let identity = resume_identity(plan, cli);
    let work = ResumeWorkspace::open(final_path, &identity, cli.resume).await?;
    if cli.subtitle {
        let tracks = client
            .subtitle_tracks(&plan.video.bvid, plan.page.cid)
            .await?;
        let track = choose_subtitle_track(&tracks, cli.subtitle_lang.as_deref())?;
        eprintln!("P{}: 下载原生字幕 {}", plan.page.page, track.language_name);
        let rendered = client
            .render_subtitle(track, cli.subtitle_format.core())
            .await?;
        let temporary = work.artifact("subtitle.part");
        tokio::fs::write(&temporary, rendered)
            .await
            .map_err(|source| Error::Io {
                path: temporary.clone(),
                source,
            })?;
        tokio::fs::rename(&temporary, final_path)
            .await
            .map_err(|source| Error::Io {
                path: final_path.to_owned(),
                source,
            })?;
        println!("saved: {}", final_path.display());
        work.finish().await?;
        return Ok(());
    }
    let audio_path = work.artifact("audio.m4s");
    if cli.transcribe {
        if work.stage_complete("audio-download", &audio_path).await {
            eprintln!("P{}: 恢复：复用已下载音频", plan.page.page);
        } else {
            download_with_progress(
                client,
                &plan.selected_audio,
                &audio_path,
                "audio",
                backend,
                aria2_manager,
            )
            .await?;
            work.mark_complete("audio-download", &audio_path).await?;
        }
        let m4a_path = work.artifact("audio.m4a");
        if work.stage_complete("audio-mux", &m4a_path).await {
            eprintln!("P{}: 恢复：复用已封装 M4A", plan.page.page);
        } else {
            work.remove_incomplete_output(&m4a_path).await?;
            eprintln!("P{}: 封装音频为 M4A", plan.page.page);
            mux_audio(&cli.ffmpeg, &audio_path, &m4a_path).await?;
            work.mark_complete("audio-mux", &m4a_path).await?;
        }
        let transcript = match asr_backend {
            AsrBackend::Bcut => {
                eprintln!("P{}: 上传 Bcut 并等待语音识别", plan.page.page);
                client
                    .transcribe_with_bcut(&m4a_path, cli.transcript_format.core(), |_| {})
                    .await?
            }
            AsrBackend::Buzz => {
                eprintln!(
                    "P{}: 使用 Buzz/faster-whisper 本地转写（模型 {}）",
                    plan.page.page, buzz_options.model
                );
                let bar = ProgressBar::new(1);
                bar.set_style(
                    ProgressStyle::with_template(
                        "{spinner:.green} Buzz [{bar:32.cyan/blue}] {percent:>3}% {msg} ETA {eta}",
                    )
                    .expect("valid progress template")
                    .progress_chars("=> "),
                );
                bar.enable_steady_tick(Duration::from_millis(120));
                bar.set_message("loading model");
                let progress_bar = bar.clone();
                let result = transcribe_audio_with_buzz(
                    &m4a_path,
                    cli.transcript_format.core(),
                    buzz_options,
                    move |progress| match progress {
                        BuzzProgress::WaitingForSlot => {
                            progress_bar.set_message("waiting for local ASR slot")
                        }
                        BuzzProgress::LoadingModel => progress_bar.set_message("loading model"),
                        BuzzProgress::Transcribing {
                            processed_seconds,
                            total_seconds,
                        } => {
                            let total_ms = seconds_to_millis(total_seconds).max(1);
                            progress_bar.set_length(total_ms);
                            progress_bar
                                .set_position(seconds_to_millis(processed_seconds).min(total_ms));
                            progress_bar.set_message(format!(
                                "{} / {}",
                                format_media_time(processed_seconds),
                                format_media_time(total_seconds)
                            ));
                        }
                        BuzzProgress::Rendering => {
                            progress_bar.set_position(progress_bar.length().unwrap_or(1));
                            progress_bar.set_message("rendering transcript");
                        }
                    },
                )
                .await;
                match result {
                    Ok(transcript) => {
                        bar.finish_with_message("transcription done");
                        transcript
                    }
                    Err(error) => {
                        bar.abandon_with_message("transcription failed");
                        return Err(error);
                    }
                }
            }
        };
        let temporary = work.artifact("transcript.part");
        tokio::fs::write(&temporary, transcript)
            .await
            .map_err(|source| Error::Io {
                path: temporary.clone(),
                source,
            })?;
        tokio::fs::rename(&temporary, final_path)
            .await
            .map_err(|source| Error::Io {
                path: final_path.to_owned(),
                source,
            })?;
        println!("saved: {}", final_path.display());
        if let Some(ai) = ai_client {
            let markdown_path =
                organize_srt_with_ai(ai, final_path, work.root(), plan.page.page).await?;
            println!("saved: {}", markdown_path.display());
        }
        work.finish().await?;
        return Ok(());
    } else {
        let merged_path = if cli.audio_only {
            if work.stage_complete("audio-download", &audio_path).await {
                eprintln!("P{}: 恢复：复用已下载音频", plan.page.page);
            } else {
                download_with_progress(
                    client,
                    &plan.selected_audio,
                    &audio_path,
                    "audio",
                    backend,
                    aria2_manager,
                )
                .await?;
                work.mark_complete("audio-download", &audio_path).await?;
            }
            work.artifact("merged.m4a")
        } else {
            let video_path = work.artifact("video.m4s");
            eprintln!(
                "P{}: downloading audio and video concurrently",
                plan.page.page
            );
            let audio_download = async {
                if work.stage_complete("audio-download", &audio_path).await {
                    eprintln!("P{}: 恢复：复用已下载音频", plan.page.page);
                    Ok(())
                } else {
                    download_with_progress(
                        client,
                        &plan.selected_audio,
                        &audio_path,
                        "audio",
                        backend,
                        aria2_manager,
                    )
                    .await?;
                    work.mark_complete("audio-download", &audio_path).await
                }
            };
            let video_download = async {
                if work.stage_complete("video-download", &video_path).await {
                    eprintln!("P{}: 恢复：复用已下载视频", plan.page.page);
                    Ok(())
                } else {
                    download_with_progress(
                        client,
                        &plan.selected_video,
                        &video_path,
                        "video",
                        backend,
                        aria2_manager,
                    )
                    .await?;
                    work.mark_complete("video-download", &video_path).await
                }
            };
            let (audio_result, video_result) = tokio::join!(audio_download, video_download);
            audio_result?;
            video_result?;
            eprintln!("P{}: merging with FFmpeg", plan.page.page);
            let merged_path = work.artifact("merged.mp4");
            work.remove_incomplete_output(&merged_path).await?;
            mux(&cli.ffmpeg, &audio_path, &video_path, &merged_path).await?;
            merged_path
        };
        if cli.audio_only {
            work.remove_incomplete_output(&merged_path).await?;
            eprintln!("P{}: 封装音频为 M4A", plan.page.page);
            mux_audio(&cli.ffmpeg, &audio_path, &merged_path).await?;
        }
        tokio::fs::rename(&merged_path, final_path)
            .await
            .map_err(|source| Error::Io {
                path: final_path.to_owned(),
                source,
            })?;
    }
    println!("saved: {}", final_path.display());
    work.finish().await?;
    Ok(())
}

fn resume_identity(plan: &muxsift_core::DownloadPlan, cli: &Cli) -> String {
    let mode = if cli.subtitle {
        format!("subtitle:{}", cli.subtitle_format.core().extension())
    } else if cli.transcribe {
        format!("transcribe:{}", cli.transcript_format.core().extension())
    } else if cli.audio_only {
        "audio".into()
    } else {
        "video".into()
    };
    let streams = if cli.subtitle {
        String::new()
    } else if cli.audio_only || cli.transcribe {
        format!("{}:{}", plan.selected_audio.id, plan.selected_audio.codecs)
    } else {
        format!(
            "{}:{}\n{}:{}",
            plan.selected_video.id,
            plan.selected_video.codecs,
            plan.selected_audio.id,
            plan.selected_audio.codecs
        )
    };
    format!(
        "{}\n{}\n{}\n{}",
        plan.video.bvid, plan.page.cid, mode, streams
    )
}

async fn prompt_reuse_existing_srt(path: &Path) -> Result<bool> {
    let path = path.to_owned();
    tokio::task::spawn_blocking(move || {
        if !io::stdin().is_terminal() {
            return Err(Error::InvalidInput(format!(
                "output already exists: {}. Run interactively to choose whether to use this SRT for AI post-processing",
                path.display()
            )));
        }
        eprintln!("输出已存在: {}", path.display());
        eprint!("是否跳过下载和转写，直接使用此 SRT 进行 AI 整理？[y/N] ");
        io::stderr().flush().map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        let mut answer = String::new();
        io::stdin()
            .read_line(&mut answer)
            .map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
        Ok(accepts_reuse_answer(&answer))
    })
    .await
    .map_err(|error| Error::InvalidInput(format!("confirmation task failed: {error}")))?
}

fn accepts_reuse_answer(answer: &str) -> bool {
    matches!(
        answer.trim().to_ascii_lowercase().as_str(),
        "y" | "yes" | "是"
    )
}

async fn organize_srt_with_ai(
    ai: &AiClient,
    srt_path: &Path,
    temporary_dir: &Path,
    page: usize,
) -> Result<PathBuf> {
    let markdown_path = srt_path.with_extension("md");
    if markdown_path.exists() {
        return Err(Error::InvalidInput(format!(
            "AI Markdown output already exists: {}",
            markdown_path.display()
        )));
    }
    eprintln!("P{page}: 正在用 AI 整理 SRT 为 Markdown");
    let srt = tokio::fs::read_to_string(srt_path)
        .await
        .map_err(|source| Error::Io {
            path: srt_path.to_owned(),
            source,
        })?;
    let markdown = ai.srt_to_markdown(&srt).await?;
    let temporary = temporary_dir.join("transcript.md.part");
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

fn choose_subtitle_track<'a>(
    tracks: &'a [SubtitleTrack],
    requested: Option<&str>,
) -> Result<&'a SubtitleTrack> {
    if tracks.is_empty() {
        return Err(Error::Subtitle(
            "no native subtitle track is available".into(),
        ));
    }
    if let Some(requested) = requested {
        tracks
            .iter()
            .find(|track| track.language.eq_ignore_ascii_case(requested))
            .ok_or_else(|| {
                Error::Subtitle(format!("subtitle language is unavailable: {requested}"))
            })
    } else {
        Ok(&tracks[0])
    }
}

async fn login(client: &BiliClient) -> Result<()> {
    if client.is_logged_in().await? {
        println!(
            "Already logged in. Credentials: {}",
            client.credential_path().display()
        );
        return Ok(());
    }
    let qr = client.generate_login_qr().await?;
    let code = QrCode::new(qr.url.as_bytes()).map_err(|error| Error::Protocol {
        endpoint: "qrcode/generate".into(),
        detail: error.to_string(),
    })?;
    println!("Scan with the Bilibili mobile app, then confirm on your phone:\n");
    println!(
        "{}",
        code.render::<unicode::Dense1x2>().quiet_zone(true).build()
    );
    let deadline = Instant::now() + Duration::from_secs(180);
    let mut previous = None;
    while Instant::now() < deadline {
        let status = client.poll_login(&qr.qrcode_key).await?;
        if previous != Some(status) {
            match status {
                LoginPoll::WaitingForScan => eprintln!("waiting for scan"),
                LoginPoll::WaitingForConfirmation => eprintln!("waiting for phone confirmation"),
                LoginPoll::Success => {}
                LoginPoll::Expired => return Err(Error::LoginExpired),
            }
            previous = Some(status);
        }
        if status == LoginPoll::Success {
            if !client.is_logged_in().await? {
                return Err(Error::NotLoggedIn);
            }
            client.save_credentials()?;
            println!(
                "Login successful. Credentials: {}",
                client.credential_path().display()
            );
            return Ok(());
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
    Err(Error::LoginTimeout)
}

async fn download_with_progress(
    client: &BiliClient,
    stream: &muxsift_core::Stream,
    path: &Path,
    label: &'static str,
    backend: DownloadBackend,
    aria2_manager: Option<&Aria2Manager>,
) -> Result<()> {
    let bar = ProgressBar::new_spinner();
    bar.set_style(ProgressStyle::with_template(
        "{spinner:.green} {msg:5} [{bar:32.cyan/blue}] {bytes}/{total_bytes} {bytes_per_sec} {eta}",
    ).expect("valid progress template").progress_chars("=> "));
    bar.set_message(label);
    let progress_bar = bar.clone();
    let on_progress = move |progress: DownloadProgress| {
        if let Some(total) = progress.total {
            progress_bar.set_length(total);
        }
        progress_bar.set_position(progress.downloaded);
    };
    match backend {
        DownloadBackend::Http => client.download(stream, path, on_progress).await?,
        DownloadBackend::Aria2 => {
            client
                .download_with_aria2(
                    stream,
                    path,
                    aria2_manager.expect("aria2 manager exists for aria2 backend"),
                    on_progress,
                    None,
                )
                .await?
        }
    }
    bar.finish_with_message(format!("{label} done"));
    Ok(())
}

fn print_plan(
    plan: &muxsift_core::DownloadPlan,
    output: &Path,
    dry_run: bool,
    audio_only: bool,
    transcribe: bool,
    subtitle: bool,
) {
    println!("{}", plan.video.title);
    println!("author: {}", plan.video.owner.name);
    println!(
        "BV: {}  page: {}/{}  CID: {}",
        plan.video.bvid,
        plan.page.page,
        plan.video.pages.len(),
        plan.page.cid
    );
    println!("login: {}", if plan.logged_in { "yes" } else { "no" });
    println!("available streams: {:?}", plan.returned_quality_ids);
    println!(
        "video: qn={} {}x{} {}",
        plan.selected_video.id,
        plan.selected_video.width,
        plan.selected_video.height,
        plan.selected_video.codecs
    );
    println!(
        "audio: id={} {} bps {}",
        plan.selected_audio.id, plan.selected_audio.bandwidth, plan.selected_audio.codecs
    );
    println!("output: {}", output.display());
    println!(
        "mode: {}",
        if subtitle {
            "native subtitle"
        } else if transcribe {
            "transcribe"
        } else if audio_only {
            "audio only"
        } else {
            "video + audio"
        }
    );
    if dry_run {
        println!("dry-run: no media downloaded; FFmpeg not started");
    }
}

fn parse_quality(value: &str) -> std::result::Result<u64, String> {
    let normalized = value.trim().to_ascii_lowercase();
    let result = match normalized.as_str() {
        "240p" => 6,
        "360p" => 16,
        "480p" => 32,
        "720p" => 64,
        "720p60" => 74,
        "1080p" => 80,
        "1080p+" => 112,
        "1080p60" => 116,
        "4k" | "2160p" => 120,
        "hdr" => 125,
        "dolby" => 126,
        "8k" => 127,
        _ => normalized
            .parse()
            .map_err(|_| "expected 1080p, 720p, 4k, or a numeric Bilibili qn".to_owned())?,
    };
    Ok(result)
}

enum PageSelection {
    All,
    Pages(Vec<usize>),
}

fn parse_pages(value: &str) -> Result<PageSelection> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("all") {
        return Ok(PageSelection::All);
    }
    if value.is_empty() {
        return Err(Error::InvalidInput("--pages cannot be empty".into()));
    }
    let mut pages = std::collections::BTreeSet::new();
    for item in value.split(',') {
        let item = item.trim();
        if item.is_empty() {
            return Err(Error::InvalidInput("--pages contains an empty item".into()));
        }
        if let Some((start, end)) = item.split_once('-') {
            let start = parse_page_number(start)?;
            let end = parse_page_number(end)?;
            if start > end {
                return Err(Error::InvalidInput(format!(
                    "--pages range is reversed: {item}"
                )));
            }
            if end - start > 10_000 {
                return Err(Error::InvalidInput("--pages range is too large".into()));
            }
            pages.extend(start..=end);
        } else {
            pages.insert(parse_page_number(item)?);
        }
    }
    if pages.is_empty() {
        return Err(Error::InvalidInput(
            "--pages contains no page numbers".into(),
        ));
    }
    Ok(PageSelection::Pages(pages.into_iter().collect()))
}

fn parse_page_number(value: &str) -> Result<usize> {
    let page = value
        .trim()
        .parse::<usize>()
        .map_err(|_| Error::InvalidInput(format!("invalid page number: {value}")))?;
    if page == 0 {
        return Err(Error::InvalidInput("page must be at least 1".into()));
    }
    Ok(page)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_quality_names_and_ids() {
        assert_eq!(parse_quality("1080p").unwrap(), 80);
        assert_eq!(parse_quality("4K").unwrap(), 120);
        assert_eq!(parse_quality("32").unwrap(), 32);
        assert!(parse_quality("bestish").is_err());
    }

    #[test]
    fn parses_page_expressions() {
        assert!(matches!(parse_pages("all").unwrap(), PageSelection::All));
        assert!(matches!(
            parse_pages("1, 3-5, 5").unwrap(),
            PageSelection::Pages(pages) if pages == vec![1, 3, 4, 5]
        ));
        assert!(parse_pages("0").is_err());
        assert!(parse_pages("5-2").is_err());
    }

    #[test]
    fn parses_existing_srt_confirmation() {
        assert!(accepts_reuse_answer("y\n"));
        assert!(accepts_reuse_answer("YES"));
        assert!(accepts_reuse_answer("是"));
        assert!(!accepts_reuse_answer(""));
        assert!(!accepts_reuse_answer("n"));
    }

    #[test]
    fn parses_resume_option() {
        let cli = Cli::try_parse_from(["muxsift", "BV1TXtb6iEt4", "--resume"]).unwrap();
        assert!(cli.resume);
    }

    #[test]
    fn parses_local_media_command() {
        let cli = Cli::try_parse_from([
            "muxsift",
            "local",
            "meeting.mp4",
            "--asr",
            "buzz",
            "--transcript-only",
        ])
        .unwrap();
        assert!(matches!(
            cli.command,
            Some(Command::Local {
                asr: AsrBackendArg::Buzz,
                transcript_only: true,
                ..
            })
        ));
    }
}
