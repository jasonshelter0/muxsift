use std::{
    path::{Path, PathBuf},
    process::Stdio,
    sync::OnceLock,
};

use serde::{Deserialize, Serialize};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::Command,
    sync::Mutex,
};

use crate::{Error, Result, TranscriptFormat};

static BUZZ_GATE: OnceLock<Mutex<()>> = OnceLock::new();

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsrBackend {
    Bcut,
    Buzz,
}

#[derive(Debug, Clone)]
pub struct BuzzOptions {
    pub project_dir: PathBuf,
    pub uv: PathBuf,
    pub bridge: PathBuf,
    pub model: String,
    pub device: String,
    pub compute_type: String,
    pub language: Option<String>,
}

#[derive(Debug, Clone, Copy)]
pub enum BuzzProgress {
    WaitingForSlot,
    LoadingModel,
    Transcribing {
        processed_seconds: f64,
        total_seconds: f64,
    },
    Rendering,
}

const PROGRESS_PREFIX: &str = "MUXSIFT_PROGRESS ";

#[derive(Debug, Deserialize)]
struct BridgeProgress {
    processed: f64,
    total: f64,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuzzTranscript {
    language: String,
    language_probability: f64,
    segments: Vec<BuzzSegment>,
}

#[derive(Debug, Deserialize, Serialize)]
struct BuzzSegment {
    start: f64,
    end: f64,
    text: String,
}

pub async fn transcribe_audio_with_buzz<F>(
    path: &Path,
    format: TranscriptFormat,
    options: &BuzzOptions,
    mut progress: F,
) -> Result<String>
where
    F: FnMut(BuzzProgress),
{
    if !path.is_file() {
        return Err(Error::Buzz(format!(
            "audio input does not exist: {}",
            path.display()
        )));
    }
    if options.project_dir.as_os_str().is_empty() || !options.project_dir.is_dir() {
        return Err(Error::Buzz(format!(
            "Buzz project directory does not exist: {}. Set asr.buzz.project_dir or --buzz-project",
            options.project_dir.display()
        )));
    }
    let bridge = resolve_bridge(&options.bridge)?;
    progress(BuzzProgress::WaitingForSlot);
    let _guard = BUZZ_GATE.get_or_init(|| Mutex::new(())).lock().await;
    progress(BuzzProgress::LoadingModel);

    let mut command = Command::new(&options.uv);
    command
        .current_dir(&options.project_dir)
        .arg("run")
        .arg("python")
        .arg(&bridge)
        .arg("--input")
        .arg(path)
        .arg("--model")
        .arg(&options.model)
        .arg("--device")
        .arg(&options.device)
        .arg("--compute-type")
        .arg(&options.compute_type)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(target_os = "linux")]
    if let Some(library_path) = buzz_library_path(&options.project_dir) {
        command.env("LD_LIBRARY_PATH", library_path);
    }
    if let Some(language) = options
        .language
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    {
        command.arg("--language").arg(language);
    }
    let mut child = command.spawn().map_err(|source| {
        Error::Buzz(format!(
            "could not start {}: {source}. Set asr.buzz.uv or --buzz-uv",
            options.uv.display()
        ))
    })?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Buzz("could not capture bridge stdout".into()))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::Buzz("could not capture bridge stderr".into()))?;
    let stdout_task = tokio::spawn(async move {
        let mut output = Vec::new();
        let mut stdout = stdout;
        stdout.read_to_end(&mut output).await.map(|_| output)
    });
    let mut stderr_lines = BufReader::new(stderr).lines();
    let mut stderr_output = Vec::new();
    while let Some(line) = stderr_lines
        .next_line()
        .await
        .map_err(|source| Error::Buzz(format!("could not read bridge progress output: {source}")))?
    {
        if let Some(event) = parse_progress_line(&line) {
            progress(BuzzProgress::Transcribing {
                processed_seconds: event.processed,
                total_seconds: event.total,
            });
        } else {
            stderr_output.push(line);
        }
    }
    let status = child
        .wait()
        .await
        .map_err(|source| Error::Buzz(format!("could not wait for bridge: {source}")))?;
    let stdout = stdout_task
        .await
        .map_err(|error| Error::Buzz(format!("bridge stdout task failed: {error}")))?
        .map_err(|source| Error::Buzz(format!("could not read bridge output: {source}")))?;
    if !status.success() {
        let detail = stderr_output.join("\n").trim().to_owned();
        return Err(Error::Buzz(if detail.is_empty() {
            format!("bridge exited with status {status}")
        } else {
            detail
        }));
    }
    let transcript: BuzzTranscript = serde_json::from_slice(&stdout)
        .map_err(|error| Error::Buzz(format!("bridge returned invalid JSON: {error}")))?;
    if transcript.segments.is_empty() {
        return Err(Error::Buzz("no speech was recognized".into()));
    }
    progress(BuzzProgress::Rendering);
    render(&transcript, format)
}

fn parse_progress_line(line: &str) -> Option<BridgeProgress> {
    serde_json::from_str(line.strip_prefix(PROGRESS_PREFIX)?).ok()
}

#[cfg(target_os = "linux")]
fn buzz_library_path(project_dir: &Path) -> Option<std::ffi::OsString> {
    let mut paths = Vec::new();
    let venv_lib = project_dir.join(".venv").join("lib");
    let python_dirs = std::fs::read_dir(&venv_lib).ok()?;
    for python_dir in python_dirs.flatten().map(|entry| entry.path()) {
        if !python_dir
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| name.starts_with("python"))
        {
            continue;
        }
        let nvidia = python_dir.join("site-packages").join("nvidia");
        let Ok(packages) = std::fs::read_dir(nvidia) else {
            continue;
        };
        for library_dir in packages
            .flatten()
            .map(|entry| entry.path().join("lib"))
            .filter(|path| path.is_dir())
        {
            paths.push(library_dir);
        }
    }
    if let Some(existing) = std::env::var_os("LD_LIBRARY_PATH") {
        paths.extend(std::env::split_paths(&existing));
    }
    (!paths.is_empty())
        .then(|| std::env::join_paths(paths).ok())
        .flatten()
}

fn resolve_bridge(configured: &Path) -> Result<PathBuf> {
    if configured != Path::new("auto") {
        return configured
            .is_file()
            .then(|| configured.to_owned())
            .ok_or_else(|| {
                Error::Buzz(format!("bridge script not found: {}", configured.display()))
            });
    }
    if let Ok(executable) = std::env::current_exe()
        && let Some(parent) = executable.parent()
    {
        let bundled = parent.join("libexec").join("buzz-asr.py");
        if bundled.is_file() {
            return Ok(bundled);
        }
    }
    let development = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("scripts")
        .join("buzz-asr.py");
    if development.is_file() {
        return Ok(development);
    }
    Err(Error::Buzz(
        "bundled Buzz bridge was not found; expected libexec/buzz-asr.py beside muxsift. Set asr.buzz.bridge or --buzz-bridge".into(),
    ))
}

fn render(transcript: &BuzzTranscript, format: TranscriptFormat) -> Result<String> {
    match format {
        TranscriptFormat::Txt => Ok(transcript
            .segments
            .iter()
            .map(|segment| segment.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")),
        TranscriptFormat::Srt => Ok(transcript
            .segments
            .iter()
            .enumerate()
            .map(|(index, segment)| {
                format!(
                    "{}\n{} --> {}\n{}\n",
                    index + 1,
                    timestamp_srt(segment.start),
                    timestamp_srt(segment.end),
                    segment.text
                )
            })
            .collect()),
        TranscriptFormat::Lrc => Ok(transcript
            .segments
            .iter()
            .map(|segment| format!("{}{}\n", timestamp_lrc(segment.start), segment.text))
            .collect()),
        TranscriptFormat::Json => serde_json::to_string_pretty(transcript)
            .map_err(|error| Error::Buzz(format!("could not render transcript JSON: {error}"))),
    }
}

fn milliseconds(seconds: f64) -> u64 {
    (seconds.max(0.0) * 1000.0).round() as u64
}

fn timestamp_srt(seconds: f64) -> String {
    let value = milliseconds(seconds);
    format!(
        "{:02}:{:02}:{:02},{:03}",
        value / 3_600_000,
        value / 60_000 % 60,
        value / 1_000 % 60,
        value % 1_000
    )
}

fn timestamp_lrc(seconds: f64) -> String {
    let value = milliseconds(seconds);
    format!(
        "[{:02}:{:02}.{:02}]",
        value / 60_000,
        value / 1_000 % 60,
        value % 1_000 / 10
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> BuzzTranscript {
        BuzzTranscript {
            language: "zh".into(),
            language_probability: 0.99,
            segments: vec![BuzzSegment {
                start: 1.23,
                end: 4.56,
                text: "hello".into(),
            }],
        }
    }

    #[test]
    fn renders_supported_formats() {
        assert!(
            render(&sample(), TranscriptFormat::Srt)
                .unwrap()
                .contains("00:00:01,230 --> 00:00:04,560")
        );
        assert_eq!(render(&sample(), TranscriptFormat::Txt).unwrap(), "hello");
        assert_eq!(
            render(&sample(), TranscriptFormat::Lrc).unwrap(),
            "[00:01.23]hello\n"
        );
        assert!(
            render(&sample(), TranscriptFormat::Json)
                .unwrap()
                .contains("\"language\": \"zh\"")
        );
    }

    #[test]
    fn parses_bridge_progress_without_treating_logs_as_events() {
        let event = parse_progress_line(r#"MUXSIFT_PROGRESS {"processed":12.5,"total":100.0}"#)
            .expect("progress event");
        assert_eq!(event.processed, 12.5);
        assert_eq!(event.total, 100.0);
        assert!(parse_progress_line("Buzz: loading model").is_none());
        assert!(parse_progress_line("MUXSIFT_PROGRESS invalid").is_none());
    }
}
