use std::path::{Path, PathBuf};

use tokio::process::Command;

use crate::{
    AiClient, AsrBackend, BcutProgress, BiliClient, BuzzOptions, BuzzProgress, DownloadControl,
    Error, Result, ResumeWorkspace, TranscriptFormat, output::sanitize_filename,
    transcribe_audio_with_buzz,
};

#[derive(Debug, Clone)]
pub struct LocalMediaRequest {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub ffmpeg: PathBuf,
    pub asr: AsrBackend,
    pub buzz: BuzzOptions,
    pub resume: bool,
    pub make_markdown: bool,
    pub control: Option<DownloadControl>,
}

#[derive(Debug, Clone)]
pub struct LocalMediaOutput {
    pub transcript: PathBuf,
    pub markdown: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub enum LocalMediaEvent {
    Stage(&'static str),
    Bcut(BcutProgress),
    Buzz(BuzzProgress),
}

pub async fn process_local_media<F>(
    request: &LocalMediaRequest,
    bili: &BiliClient,
    ai: Option<&AiClient>,
    mut progress: F,
) -> Result<LocalMediaOutput>
where
    F: FnMut(LocalMediaEvent) + Send,
{
    let input = request.input.canonicalize().map_err(|source| Error::Io {
        path: request.input.clone(),
        source,
    })?;
    let metadata = tokio::fs::metadata(&input)
        .await
        .map_err(|source| Error::Io {
            path: input.clone(),
            source,
        })?;
    if !metadata.is_file() || metadata.len() == 0 {
        return Err(Error::InvalidInput(format!(
            "local input must be a non-empty file: {}",
            input.display()
        )));
    }
    if request.make_markdown && ai.is_none() {
        return Err(Error::InvalidInput(
            "AI is not configured; enable it or choose transcript-only".into(),
        ));
    }
    let stem = input
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| Error::InvalidInput("local input filename is not valid UTF-8".into()))?;
    let stem = sanitize_filename(stem);
    tokio::fs::create_dir_all(&request.output_dir)
        .await
        .map_err(|source| Error::Io {
            path: request.output_dir.clone(),
            source,
        })?;
    let transcript = request.output_dir.join(format!("{stem}.srt"));
    let markdown = request
        .make_markdown
        .then(|| request.output_dir.join(format!("{stem}.md")));
    if transcript.exists() && !request.resume {
        return Err(Error::InvalidInput(format!(
            "transcript already exists: {}. Use --resume to reuse it",
            transcript.display()
        )));
    }
    if let Some(path) = &markdown
        && path.exists()
    {
        return Err(Error::InvalidInput(format!(
            "Markdown output already exists: {}",
            path.display()
        )));
    }

    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map_or(0, |time| time.as_nanos());
    let identity = format!(
        "local-v1\n{}\n{}\n{}\n{:?}\n{}\n{:?}",
        input.display(),
        metadata.len(),
        modified,
        request.asr,
        request.buzz.model,
        request.buzz.language
    );
    let work = ResumeWorkspace::open(&transcript, &identity, request.resume).await?;
    ensure_not_cancelled(request)?;
    if !transcript.exists() {
        let audio = work.artifact("audio.m4a");
        if !work.stage_complete("local-audio", &audio).await {
            progress(LocalMediaEvent::Stage("extracting audio with FFmpeg"));
            tokio::select! {
                result = extract_audio(&request.ffmpeg, &input, &audio) => result?,
                () = wait_for_cancel(request.control.as_ref()) => return Err(Error::Cancelled),
            }
            work.mark_complete("local-audio", &audio).await?;
        } else {
            progress(LocalMediaEvent::Stage("reusing extracted audio"));
        }
        let rendered = tokio::select! {
            result = async {
                match request.asr {
                    AsrBackend::Bcut => {
                        progress(LocalMediaEvent::Stage("uploading audio to Bcut cloud ASR"));
                        bili.transcribe_with_bcut(&audio, TranscriptFormat::Srt, |event| {
                            progress(LocalMediaEvent::Bcut(event));
                        }).await
                    }
                    AsrBackend::Buzz => {
                        progress(LocalMediaEvent::Stage("transcribing with local Buzz ASR"));
                        transcribe_audio_with_buzz(&audio, TranscriptFormat::Srt, &request.buzz, |event| {
                            progress(LocalMediaEvent::Buzz(event));
                        }).await
                    }
                }
            } => result?,
            () = wait_for_cancel(request.control.as_ref()) => return Err(Error::Cancelled),
        };
        let temporary = work.artifact("transcript.part");
        tokio::fs::write(&temporary, rendered)
            .await
            .map_err(|source| Error::Io {
                path: temporary.clone(),
                source,
            })?;
        tokio::fs::rename(&temporary, &transcript)
            .await
            .map_err(|source| Error::Io {
                path: transcript.clone(),
                source,
            })?;
        work.mark_complete("local-transcript", &transcript).await?;
    } else {
        progress(LocalMediaEvent::Stage("reusing existing SRT"));
    }

    if let (Some(ai), Some(path)) = (ai, markdown.as_ref()) {
        progress(LocalMediaEvent::Stage("organizing transcript with AI"));
        let srt = tokio::fs::read_to_string(&transcript)
            .await
            .map_err(|source| Error::Io {
                path: transcript.clone(),
                source,
            })?;
        let notes = tokio::select! {
            result = ai.srt_to_markdown(&srt) => result?,
            () = wait_for_cancel(request.control.as_ref()) => return Err(Error::Cancelled),
        };
        let temporary = work.artifact("notes.part");
        tokio::fs::write(&temporary, notes)
            .await
            .map_err(|source| Error::Io {
                path: temporary.clone(),
                source,
            })?;
        tokio::fs::rename(&temporary, path)
            .await
            .map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
    }
    work.finish().await?;
    Ok(LocalMediaOutput {
        transcript,
        markdown,
    })
}

fn ensure_not_cancelled(request: &LocalMediaRequest) -> Result<()> {
    if request
        .control
        .as_ref()
        .is_some_and(DownloadControl::is_cancelled)
    {
        Err(Error::Cancelled)
    } else {
        Ok(())
    }
}

async fn wait_for_cancel(control: Option<&DownloadControl>) {
    let Some(control) = control else {
        std::future::pending::<()>().await;
        return;
    };
    while !control.is_cancelled() {
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
}

async fn extract_audio(ffmpeg: &Path, input: &Path, output: &Path) -> Result<()> {
    let temporary = output.with_extension("part");
    let _ = tokio::fs::remove_file(&temporary).await;
    let result = Command::new(ffmpeg)
        .args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(input)
        .args(["-map", "0:a:0", "-vn", "-ac", "1", "-ar", "16000"])
        .args(["-c:a", "aac", "-b:a", "96k", "-f", "ipod"])
        .arg(&temporary)
        .kill_on_drop(true)
        .output()
        .await;
    let result = match result {
        Ok(value) => value,
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(Error::FfmpegMissing(ffmpeg.display().to_string()));
        }
        Err(source) => {
            return Err(Error::Io {
                path: ffmpeg.to_owned(),
                source,
            });
        }
    };
    if !result.status.success() {
        return Err(Error::FfmpegFailed {
            code: result.status.code(),
            stderr: String::from_utf8_lossy(&result.stderr).trim().to_owned(),
        });
    }
    tokio::fs::rename(&temporary, output)
        .await
        .map_err(|source| Error::Io {
            path: output.to_owned(),
            source,
        })
}
