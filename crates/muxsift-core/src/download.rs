use std::{
    path::Path,
    process::Stdio,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

use futures_util::StreamExt;
use reqwest::{Client, header::REFERER};
use tokio::{
    fs::File,
    io::AsyncWriteExt,
    process::Command,
    time::{Duration, sleep},
};

use crate::{Error, Result, Stream};

#[derive(Debug, Clone, Copy)]
pub struct DownloadProgress {
    pub downloaded: u64,
    pub total: Option<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct DownloadControl {
    cancelled: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
}

impl DownloadControl {
    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::Release);
    }
    pub fn toggle_pause(&self) -> bool {
        let next = !self.paused.load(Ordering::Acquire);
        self.paused.store(next, Ordering::Release);
        next
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancelled.load(Ordering::Acquire)
    }
    pub fn is_paused(&self) -> bool {
        self.paused.load(Ordering::Acquire)
    }
}

pub(crate) async fn download_stream<F>(
    client: &Client,
    stream: &Stream,
    path: &Path,
    progress: F,
) -> Result<()>
where
    F: FnMut(DownloadProgress) + Send,
{
    download_stream_with_control(client, stream, path, progress, None).await
}

pub(crate) async fn download_stream_with_control<F>(
    client: &Client,
    stream: &Stream,
    path: &Path,
    mut progress: F,
    control: Option<&DownloadControl>,
) -> Result<()>
where
    F: FnMut(DownloadProgress) + Send,
{
    let mut errors = Vec::new();
    for url in stream.urls() {
        let mut attempt = 0;
        let response = loop {
            if control.is_some_and(DownloadControl::is_cancelled) {
                return Err(Error::Cancelled);
            }
            attempt += 1;
            match client
                .get(url)
                .header(REFERER, "https://www.bilibili.com")
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => break Some(response),
                Ok(response) => {
                    let status = response.status();
                    errors.push(format!("{}: HTTP {status}", redacted_host(url)));
                    if attempt < 3 && (status.as_u16() == 429 || status.is_server_error()) {
                        sleep(Duration::from_millis(500 * attempt)).await;
                        continue;
                    }
                    break None;
                }
                Err(error) => {
                    errors.push(format!(
                        "{}: {}",
                        redacted_host(url),
                        summarize_network_error(&error)
                    ));
                    if attempt < 3 {
                        sleep(Duration::from_millis(500 * attempt)).await;
                        continue;
                    }
                    break None;
                }
            }
        };
        let Some(response) = response else { continue };
        let total = response.content_length();
        let temporary = path.with_extension("part");
        let mut file = File::create(&temporary).await.map_err(|source| Error::Io {
            path: temporary.clone(),
            source,
        })?;
        let mut downloaded = 0;
        let mut body = response.bytes_stream();
        let result: Result<()> = async {
            while let Some(chunk) = body.next().await {
                if control.is_some_and(DownloadControl::is_cancelled) {
                    return Err(Error::Cancelled);
                }
                while control.is_some_and(DownloadControl::is_paused) {
                    if control.is_some_and(DownloadControl::is_cancelled) {
                        return Err(Error::Cancelled);
                    }
                    sleep(Duration::from_millis(100)).await;
                }
                let chunk = chunk
                    .map_err(|error| Error::Download(summarize_network_error(&error).into()))?;
                file.write_all(&chunk).await.map_err(|source| Error::Io {
                    path: temporary.clone(),
                    source,
                })?;
                downloaded += chunk.len() as u64;
                progress(DownloadProgress { downloaded, total });
            }
            file.flush().await.map_err(|source| Error::Io {
                path: temporary.clone(),
                source,
            })?;
            drop(file);
            tokio::fs::rename(&temporary, path)
                .await
                .map_err(|source| Error::Io {
                    path: path.to_owned(),
                    source,
                })?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(Error::Cancelled) => {
                let _ = tokio::fs::remove_file(&temporary).await;
                return Err(Error::Cancelled);
            }
            Err(error) => errors.push(format!("{}: {}", redacted_host(url), error)),
        }
    }
    if control.is_some_and(DownloadControl::is_cancelled) {
        Err(Error::Cancelled)
    } else {
        Err(Error::Download(errors.join("; ")))
    }
}

pub async fn mux(ffmpeg: &Path, audio: &Path, video: &Path, output: &Path) -> Result<()> {
    let result = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
        .arg(audio)
        .arg("-i")
        .arg(video)
        .args(["-map", "1:v:0", "-map", "0:a:0", "-c", "copy", "-f", "mp4"])
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await;
    let output_result = match result {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::FfmpegMissing(ffmpeg.display().to_string()));
        }
        Err(source) => {
            return Err(Error::Io {
                path: ffmpeg.to_owned(),
                source,
            });
        }
    };
    if !output_result.status.success() {
        return Err(Error::FfmpegFailed {
            code: output_result.status.code(),
            stderr: String::from_utf8_lossy(&output_result.stderr)
                .trim()
                .to_owned(),
        });
    }
    Ok(())
}

pub async fn mux_audio(ffmpeg: &Path, audio: &Path, output: &Path) -> Result<()> {
    let result = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-n", "-i"])
        .arg(audio)
        .args(["-vn", "-map", "0:a:0", "-c:a", "copy", "-f", "ipod"])
        .arg(output)
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .await;
    finish_ffmpeg(result, ffmpeg).await
}

async fn finish_ffmpeg(result: std::io::Result<std::process::Output>, ffmpeg: &Path) -> Result<()> {
    let output_result = match result {
        Ok(value) => value,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::FfmpegMissing(ffmpeg.display().to_string()));
        }
        Err(source) => {
            return Err(Error::Io {
                path: ffmpeg.to_owned(),
                source,
            });
        }
    };
    if !output_result.status.success() {
        return Err(Error::FfmpegFailed {
            code: output_result.status.code(),
            stderr: String::from_utf8_lossy(&output_result.stderr)
                .trim()
                .to_owned(),
        });
    }
    Ok(())
}

fn redacted_host(value: &str) -> String {
    url::Url::parse(value)
        .ok()
        .and_then(|url| url.host_str().map(str::to_owned))
        .unwrap_or_else(|| "<invalid-url>".into())
}

fn summarize_network_error(error: &reqwest::Error) -> &'static str {
    if error.is_timeout() {
        "request timed out"
    } else if error.is_connect() {
        "connection failed"
    } else if error.is_body() {
        "response body failed"
    } else if error.is_decode() {
        "response decoding failed"
    } else {
        "network request failed"
    }
}
