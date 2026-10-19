use std::{fs, path::PathBuf};

use crate::{
    AiClient, AsrBackend, BiliClient, BuzzOptions, DownloadControl, Error, FetchOptions,
    LocalMediaEvent, LocalMediaRequest, Result, fetch::reuse_fetched_media, fetch::validate_url,
    fetch_with_yt_dlp, process_local_media,
};

#[derive(Debug, Clone)]
pub struct WebMediaRequest {
    pub url: String,
    pub output_dir: PathBuf,
    pub yt_dlp: PathBuf,
    pub ffmpeg: PathBuf,
    pub proxy: Option<String>,
    pub no_proxy: bool,
    pub asr: Option<AsrBackend>,
    pub buzz: BuzzOptions,
    pub resume: bool,
    pub make_markdown: bool,
    pub control: Option<DownloadControl>,
}

#[derive(Debug, Clone)]
pub struct WebMediaOutput {
    pub media: PathBuf,
    pub transcript: Option<PathBuf>,
    pub markdown: Option<PathBuf>,
}

#[derive(Debug, Clone, Copy)]
pub enum WebMediaEvent {
    Stage(&'static str),
    Local(LocalMediaEvent),
}

/// Acquire one webpage video and optionally run the existing local ASR/AI stages.
pub async fn process_web_media<F>(
    request: &WebMediaRequest,
    bili: &BiliClient,
    ai: Option<&AiClient>,
    mut progress: F,
) -> Result<WebMediaOutput>
where
    F: FnMut(WebMediaEvent) + Send,
{
    validate_url(&request.url)?;
    if request.make_markdown && (request.asr.is_none() || ai.is_none()) {
        return Err(Error::InvalidInput(
            "web AI notes require an ASR backend and configured AI".into(),
        ));
    }
    if request.output_dir.symlink_metadata().is_ok() {
        if !request.resume || !request.output_dir.is_dir() {
            return Err(Error::InvalidInput(format!(
                "output already exists: {}. Use --resume to continue it",
                request.output_dir.display()
            )));
        }
    } else {
        fs::create_dir_all(&request.output_dir).map_err(|source| Error::Io {
            path: request.output_dir.clone(),
            source,
        })?;
    }
    ensure_not_cancelled(request)?;
    let fetch_options = FetchOptions {
        url: request.url.clone(),
        output_dir: request.output_dir.join("media"),
        executable: request.yt_dlp.clone(),
        ffmpeg: request.ffmpeg.clone(),
        proxy: request.proxy.clone(),
        no_proxy: request.no_proxy,
    };
    let media = if fetch_options.output_dir.is_dir() {
        progress(WebMediaEvent::Stage("reusing downloaded media"));
        reuse_fetched_media(&fetch_options)?
    } else {
        progress(WebMediaEvent::Stage("downloading with yt-dlp"));
        tokio::select! {
            result = fetch_with_yt_dlp(&fetch_options) => result?,
            () = wait_for_cancel(request.control.as_ref()) => return Err(Error::Cancelled),
        }
    };
    ensure_not_cancelled(request)?;
    let Some(asr) = request.asr else {
        return Ok(WebMediaOutput {
            media,
            transcript: None,
            markdown: None,
        });
    };
    let local = LocalMediaRequest {
        input: media.clone(),
        output_dir: request.output_dir.join("notes"),
        ffmpeg: request.ffmpeg.clone(),
        asr,
        buzz: request.buzz.clone(),
        resume: request.resume,
        make_markdown: request.make_markdown,
        control: request.control.clone(),
    };
    let result = process_local_media(&local, bili, ai, |event| {
        progress(WebMediaEvent::Local(event));
    })
    .await?;
    Ok(WebMediaOutput {
        media,
        transcript: Some(result.transcript),
        markdown: result.markdown,
    })
}

fn ensure_not_cancelled(request: &WebMediaRequest) -> Result<()> {
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
