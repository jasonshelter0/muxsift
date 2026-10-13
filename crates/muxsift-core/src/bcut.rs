use std::{
    path::Path,
    sync::{
        OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use reqwest::{
    Client,
    header::{ACCEPT, COOKIE, HeaderMap, HeaderValue, ORIGIN, REFERER},
};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::{fs, time::sleep};

use rand::Rng;

use crate::{Error, Result};

const CREATE_RESOURCE: &str = "https://member.bilibili.com/x/bcut/rubick-interface/resource/create";
const COMPLETE_RESOURCE: &str =
    "https://member.bilibili.com/x/bcut/rubick-interface/resource/create/complete";
const CREATE_TASK: &str = "https://member.bilibili.com/x/bcut/rubick-interface/task";
const QUERY_RESULT: &str = "https://member.bilibili.com/x/bcut/rubick-interface/task/result";
const MODEL_ID: u64 = 7;
const DEFAULT_BCUT_COOLDOWN_SECONDS: u64 = 20;
static BCUT_GATE: OnceLock<Mutex<BcutGate>> = OnceLock::new();
static BCUT_COOLDOWN_SECONDS: AtomicU64 = AtomicU64::new(DEFAULT_BCUT_COOLDOWN_SECONDS);

struct BcutGate {
    last_finished: Option<Instant>,
}

/// Sets the process-wide Bcut cooldown. Zero keeps serialization but disables
/// the extra delay between completed transcription tasks.
pub fn set_bcut_cooldown_seconds(seconds: u64) {
    BCUT_COOLDOWN_SECONDS.store(seconds.min(3_600), Ordering::Release);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TranscriptFormat {
    Srt,
    Txt,
    Lrc,
    Json,
}

#[derive(Debug, Clone, Copy)]
pub enum BcutProgress {
    WaitingForSlot,
    PreparingUpload,
    Uploading {
        completed: usize,
        total: usize,
        bytes: usize,
    },
    CreatingTask,
    Recognizing {
        elapsed_seconds: u64,
    },
    Rendering,
}

impl TranscriptFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Txt => "txt",
            Self::Lrc => "lrc",
            Self::Json => "json",
        }
    }
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Debug, Deserialize)]
struct ResourceCreate {
    in_boss_key: String,
    resource_id: String,
    upload_urls: Vec<String>,
    upload_id: String,
    per_size: usize,
}

#[derive(Debug, Deserialize)]
struct ResourceComplete {
    download_url: String,
}

#[derive(Debug, Deserialize)]
struct TaskCreate {
    task_id: String,
}

#[derive(Debug, Deserialize)]
struct TaskResult {
    state: u8,
    #[serde(default)]
    result: Option<String>,
    #[serde(default)]
    remark: String,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct AsrData {
    utterances: Vec<Utterance>,
}

#[derive(Debug, Deserialize, serde::Serialize)]
struct Utterance {
    start_time: u64,
    end_time: u64,
    transcript: String,
}

pub async fn transcribe_audio(
    path: &Path,
    format: TranscriptFormat,
    proxy: Option<&str>,
    no_proxy: bool,
) -> Result<String> {
    transcribe_audio_with_progress(path, format, proxy, no_proxy, |_| {}).await
}

pub async fn transcribe_audio_with_progress<F>(
    path: &Path,
    format: TranscriptFormat,
    proxy: Option<&str>,
    no_proxy: bool,
    progress: F,
) -> Result<String>
where
    F: FnMut(BcutProgress),
{
    transcribe_audio_with_session(path, format, proxy, no_proxy, "", progress).await
}

pub async fn transcribe_audio_with_session<F>(
    path: &Path,
    format: TranscriptFormat,
    proxy: Option<&str>,
    no_proxy: bool,
    cookie: &str,
    mut progress: F,
) -> Result<String>
where
    F: FnMut(BcutProgress),
{
    progress(BcutProgress::WaitingForSlot);
    let gate = BCUT_GATE.get_or_init(|| {
        Mutex::new(BcutGate {
            last_finished: None,
        })
    });
    let mut gate = gate.lock().await;
    if let Some(last_finished) = gate.last_finished {
        let elapsed = last_finished.elapsed();
        let cooldown = Duration::from_secs(BCUT_COOLDOWN_SECONDS.load(Ordering::Acquire));
        let jitter = Duration::from_secs_f64(rand::thread_rng().gen_range(0.0..5.0));
        let total = cooldown + jitter;
        if elapsed < total {
            sleep(total - elapsed).await;
        }
    }
    let result = transcribe_audio_inner(path, format, proxy, no_proxy, cookie, &mut progress).await;
    gate.last_finished = Some(Instant::now());
    result
}

async fn transcribe_audio_inner<F>(
    path: &Path,
    format: TranscriptFormat,
    proxy: Option<&str>,
    no_proxy: bool,
    cookie: &str,
    progress: &mut F,
) -> Result<String>
where
    F: FnMut(BcutProgress),
{
    let bytes = fs::read(path).await.map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })?;
    if bytes.is_empty() {
        return Err(Error::Bcut("audio file is empty".into()));
    }
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("m4a")
        .to_ascii_lowercase();
    if !matches!(extension.as_str(), "m4a" | "aac" | "mp3" | "wav" | "flac") {
        return Err(Error::Bcut(format!(
            "unsupported audio format: {extension}"
        )));
    }

    let mut headers = HeaderMap::new();
    headers.insert(
        REFERER,
        HeaderValue::from_static("https://member.bilibili.com/"),
    );
    headers.insert(
        ORIGIN,
        HeaderValue::from_static("https://member.bilibili.com"),
    );
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/json, text/plain, */*"),
    );
    headers.insert(
        "x-requested-with",
        HeaderValue::from_static("XMLHttpRequest"),
    );
    if !cookie.is_empty() {
        let value = HeaderValue::from_str(cookie)
            .map_err(|error| Error::Bcut(format!("invalid login cookie: {error}")))?;
        headers.insert(COOKIE, value);
    }
    let mut builder = Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36")
        .default_headers(headers)
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(60));
    if no_proxy {
        builder = builder.no_proxy();
    } else if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|error| Error::Bcut(format!("invalid proxy URL: {error}")))?,
        );
    }
    let client = builder
        .build()
        .map_err(|error| Error::Bcut(format!("HTTP client initialization failed: {error}")))?;

    let size = bytes.len().to_string();
    let model_id = MODEL_ID.to_string();
    let resource = request_json::<ResourceCreate>(
        &client,
        CREATE_RESOURCE,
        &[
            ("type", "2"),
            (
                "name",
                path.file_name()
                    .and_then(|v| v.to_str())
                    .unwrap_or("audio.m4a"),
            ),
            ("size", &size),
            ("resource_file_type", &extension),
            ("model_id", &model_id),
        ],
    )
    .await?;
    if resource.upload_urls.is_empty() || resource.per_size == 0 {
        return Err(Error::Bcut("Bcut returned no upload parts".into()));
    }

    progress(BcutProgress::PreparingUpload);
    let mut etags = Vec::with_capacity(resource.upload_urls.len());
    for (index, url) in resource.upload_urls.iter().enumerate() {
        let start = index * resource.per_size;
        let end = ((index + 1) * resource.per_size).min(bytes.len());
        if start >= bytes.len() {
            return Err(Error::Bcut("Bcut upload parts exceed audio size".into()));
        }
        let mut response = None;
        let mut last_failure = "network request failed".to_owned();
        for attempt in 1..=3 {
            match client
                .put(url)
                .body(bytes[start..end].to_vec())
                .send()
                .await
            {
                Ok(candidate) if candidate.status().is_success() => {
                    response = Some(candidate);
                    break;
                }
                Ok(candidate) => {
                    let status = candidate.status();
                    last_failure = format!("HTTP {status}");
                    if attempt < 3 && (status.as_u16() == 429 || status.is_server_error()) {
                        sleep(Duration::from_millis(500 * attempt)).await;
                    } else {
                        break;
                    }
                }
                Err(error) => {
                    last_failure = summarize(&error).into();
                    if attempt < 3 {
                        sleep(Duration::from_millis(500 * attempt)).await;
                    }
                }
            }
        }
        let response = response.ok_or_else(|| {
            Error::Bcut(format!("upload part {} failed: {last_failure}", index + 1))
        })?;
        etags.push(
            response
                .headers()
                .get("etag")
                .and_then(|value| value.to_str().ok())
                .unwrap_or_default()
                .to_owned(),
        );
        progress(BcutProgress::Uploading {
            completed: index + 1,
            total: resource.upload_urls.len(),
            bytes: end,
        });
    }

    let model_id = MODEL_ID.to_string();
    let etags_value = etags.join(",");
    let complete = request_json::<ResourceComplete>(
        &client,
        COMPLETE_RESOURCE,
        &[
            ("in_boss_key", &resource.in_boss_key),
            ("resource_id", &resource.resource_id),
            ("etags", &etags_value),
            ("upload_id", &resource.upload_id),
            ("model_id", &model_id),
        ],
    )
    .await?;
    progress(BcutProgress::CreatingTask);
    let task = request_json_json::<TaskCreate>(
        &client,
        CREATE_TASK,
        serde_json::json!({
            "resource": complete.download_url,
            "model_id": MODEL_ID.to_string(),
        }),
    )
    .await?;

    for attempt in 0..600 {
        progress(BcutProgress::Recognizing {
            elapsed_seconds: attempt,
        });
        let result: TaskResult = request_json_query(
            &client,
            QUERY_RESULT,
            &[
                ("model_id", model_id.as_str()),
                ("task_id", task.task_id.as_str()),
            ],
        )
        .await?;
        match result.state {
            4 => {
                progress(BcutProgress::Rendering);
                let payload = result
                    .result
                    .ok_or_else(|| Error::Bcut("Bcut returned an empty result".into()))?;
                let data: AsrData = serde_json::from_str(&payload)
                    .map_err(|error| Error::Bcut(format!("invalid recognition result: {error}")))?;
                if data.utterances.is_empty() {
                    return Err(Error::Bcut("no speech was recognized".into()));
                }
                return Ok(render(&data, format));
            }
            3 => {
                return Err(Error::Bcut(if result.remark.is_empty() {
                    "recognition task failed".into()
                } else {
                    result.remark
                }));
            }
            _ => sleep(Duration::from_secs(1)).await,
        }
    }
    Err(Error::Bcut(
        "recognition task timed out after 10 minutes".into(),
    ))
}

async fn request_json<V>(client: &Client, endpoint: &str, form: &[(&str, &str)]) -> Result<V>
where
    V: for<'de> Deserialize<'de>,
{
    let response = client
        .post(endpoint)
        .form(form)
        .send()
        .await
        .map_err(|error| {
            Error::Bcut(format!(
                "request failed for {endpoint}: {}",
                summarize(&error)
            ))
        })?;
    parse_response(response, endpoint).await
}

async fn request_json_json<V: for<'de> Deserialize<'de>>(
    client: &Client,
    endpoint: &str,
    body: serde_json::Value,
) -> Result<V> {
    let response = client
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .map_err(|error| {
            Error::Bcut(format!(
                "request failed for {endpoint}: {}",
                summarize(&error)
            ))
        })?;
    parse_response(response, endpoint).await
}

async fn request_json_query<V: for<'de> Deserialize<'de>>(
    client: &Client,
    endpoint: &str,
    params: &[(&str, &str)],
) -> Result<V> {
    let response = client
        .get(endpoint)
        .query(params)
        .send()
        .await
        .map_err(|error| {
            Error::Bcut(format!(
                "request failed for {endpoint}: {}",
                summarize(&error)
            ))
        })?;
    parse_response(response, endpoint).await
}

async fn parse_response<V: for<'de> Deserialize<'de>>(
    response: reqwest::Response,
    endpoint: &str,
) -> Result<V> {
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        let detail = body.trim();
        let detail = if detail.is_empty() {
            String::new()
        } else {
            format!(": {}", detail.chars().take(240).collect::<String>())
        };
        return Err(Error::Bcut(format!("{endpoint}: HTTP {status}{detail}")));
    }
    let body: Envelope<V> = response
        .json()
        .await
        .map_err(|error| Error::Bcut(format!("{endpoint}: invalid JSON ({error})")))?;
    if body.code != 0 {
        return Err(Error::Bcut(format!(
            "{endpoint}: {}: {}",
            body.code, body.message
        )));
    }
    body.data
        .ok_or_else(|| Error::Bcut(format!("{endpoint}: missing response data")))
}

fn render(data: &AsrData, format: TranscriptFormat) -> String {
    match format {
        TranscriptFormat::Txt => data
            .utterances
            .iter()
            .map(|item| item.transcript.as_str())
            .collect::<Vec<_>>()
            .join("\n"),
        TranscriptFormat::Srt => data
            .utterances
            .iter()
            .enumerate()
            .map(|(index, item)| {
                format!(
                    "{}\n{} --> {}\n{}\n",
                    index + 1,
                    timestamp_srt(item.start_time),
                    timestamp_srt(item.end_time),
                    item.transcript
                )
            })
            .collect(),
        TranscriptFormat::Lrc => data
            .utterances
            .iter()
            .map(|item| format!("{}{}\n", timestamp_lrc(item.start_time), item.transcript))
            .collect(),
        TranscriptFormat::Json => serde_json::to_string_pretty(data).unwrap_or_default(),
    }
}

fn timestamp_srt(value: u64) -> String {
    format!(
        "{:02}:{:02}:{:02},{:03}",
        value / 3_600_000,
        value / 60_000 % 60,
        value / 1_000 % 60,
        value % 1_000
    )
}

fn timestamp_lrc(value: u64) -> String {
    format!(
        "[{:02}:{:02}.{:02}]",
        value / 60_000,
        value / 1_000 % 60,
        value % 1_000 / 10
    )
}

fn summarize(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else {
        "network request failed"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_supported_transcript_formats() {
        let data = AsrData {
            utterances: vec![Utterance {
                start_time: 1_230,
                end_time: 4_560,
                transcript: "hello".into(),
            }],
        };
        assert!(render(&data, TranscriptFormat::Srt).contains("00:00:01,230 --> 00:00:04,560"));
        assert_eq!(render(&data, TranscriptFormat::Txt), "hello");
        assert_eq!(render(&data, TranscriptFormat::Lrc), "[00:01.23]hello\n");
        assert!(render(&data, TranscriptFormat::Json).contains("hello"));
    }
}
