use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};

use reqwest::{
    Client,
    header::{ACCEPT, COOKIE, HeaderMap, REFERER, SET_COOKIE},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use tokio::time::sleep;

use crate::{
    Error, Result, VideoInput,
    aria2::Aria2Manager,
    auth::{LoginPoll, LoginPollData, LoginQr},
    bcut::{BcutProgress, TranscriptFormat},
    download::{self, DownloadControl, DownloadProgress},
    models::{DownloadPlan, PageInfo, Quality, Stream, VideoInfo},
    subtitle::{self, SubtitleBody, SubtitleCue, SubtitleFormat, SubtitleTrack},
};

const API: &str = "https://api.bilibili.com";
const PASSPORT: &str = "https://passport.bilibili.com";
const REFERER_VALUE: &str = "https://www.bilibili.com";
const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/112.0.0.0 Safari/537.36";

#[derive(Debug, Clone, Default)]
pub struct ClientOptions {
    pub proxy: Option<String>,
    pub media_proxy: Option<String>,
    pub no_proxy: bool,
    pub credential_path: Option<PathBuf>,
}

#[derive(Debug, Default, Deserialize, Serialize)]
struct Credentials {
    cookies: BTreeMap<String, String>,
}

pub struct BiliClient {
    http: Client,
    media_http: Client,
    proxy: Option<String>,
    no_proxy: bool,
    credentials: Arc<Mutex<Credentials>>,
    credential_path: PathBuf,
}

#[derive(Deserialize)]
struct Envelope<T> {
    code: i64,
    #[serde(default)]
    message: String,
    data: Option<T>,
}

#[derive(Deserialize)]
struct NavData {
    #[serde(rename = "isLogin")]
    is_login: bool,
}

#[derive(Deserialize)]
struct PlayData {
    #[serde(default)]
    accept_quality: Vec<u64>,
    #[serde(default)]
    accept_description: Vec<String>,
    dash: Option<Dash>,
}

#[derive(Deserialize)]
struct Dash {
    #[serde(default)]
    video: Vec<Stream>,
    #[serde(default)]
    audio: Vec<Stream>,
}

#[derive(Deserialize)]
struct PlayerData {
    #[serde(default)]
    subtitle: Option<PlayerSubtitle>,
}

#[derive(Deserialize)]
struct PlayerSubtitle {
    #[serde(default)]
    subtitles: Vec<PlayerSubtitleTrack>,
}

#[derive(Deserialize)]
struct PlayerSubtitleTrack {
    #[serde(default)]
    lan: String,
    #[serde(default, rename = "lan_doc")]
    lan_doc: String,
    #[serde(default, rename = "subtitle_url")]
    subtitle_url: String,
}

impl BiliClient {
    pub fn new(options: ClientOptions) -> Result<Self> {
        let credential_path = options
            .credential_path
            .unwrap_or_else(default_credential_path);
        let credentials = load_credentials(&credential_path)?;
        let base_builder = || {
            Client::builder()
                .user_agent(USER_AGENT)
                .connect_timeout(Duration::from_secs(10))
        };
        let mut builder = base_builder().timeout(Duration::from_secs(30));
        if options.no_proxy {
            builder = builder.no_proxy();
        } else if let Some(proxy) = &options.proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy)
                    .map_err(|error| Error::InvalidInput(format!("invalid proxy URL: {error}")))?,
            );
        }
        let http = builder.build().map_err(|source| Error::Network {
            endpoint: "HTTP client initialization".into(),
            source,
        })?;
        let mut media_builder = base_builder();
        if options.no_proxy {
            media_builder = media_builder.no_proxy();
        }
        if let Some(proxy) = options.media_proxy {
            media_builder = media_builder.proxy(reqwest::Proxy::all(&proxy).map_err(|error| {
                Error::InvalidInput(format!("invalid media proxy URL: {error}"))
            })?);
        } else if !options.no_proxy {
            if let Some(proxy) = &options.proxy {
                media_builder =
                    media_builder.proxy(reqwest::Proxy::all(proxy).map_err(|error| {
                        Error::InvalidInput(format!("invalid proxy URL: {error}"))
                    })?);
            }
        }
        let media_http = media_builder.build().map_err(|source| Error::Network {
            endpoint: "media HTTP client initialization".into(),
            source,
        })?;
        Ok(Self {
            http,
            media_http,
            proxy: options.proxy,
            no_proxy: options.no_proxy,
            credentials: Arc::new(Mutex::new(credentials)),
            credential_path,
        })
    }

    pub fn credential_path(&self) -> &Path {
        &self.credential_path
    }

    pub fn has_credentials(&self) -> bool {
        self.credentials
            .lock()
            .expect("credentials mutex poisoned")
            .cookies
            .contains_key("SESSDATA")
    }

    pub async fn is_logged_in(&self) -> Result<bool> {
        let envelope: Envelope<NavData> = self
            .get_json(&format!("{API}/x/web-interface/nav"), &[])
            .await?;
        if envelope.code == -101 {
            return Ok(false);
        }
        Ok(require_data("nav", envelope)?.is_login)
    }

    pub async fn generate_login_qr(&self) -> Result<LoginQr> {
        let envelope: Envelope<LoginQr> = self
            .get_json(
                &format!("{PASSPORT}/x/passport-login/web/qrcode/generate"),
                &[],
            )
            .await?;
        require_data("qrcode/generate", envelope)
    }

    pub async fn poll_login(&self, key: &str) -> Result<LoginPoll> {
        let envelope: Envelope<LoginPollData> = self
            .get_json(
                &format!("{PASSPORT}/x/passport-login/web/qrcode/poll"),
                &[("qrcode_key", key)],
            )
            .await?;
        let data = require_data("qrcode/poll", envelope)?;
        data.status().ok_or_else(|| Error::Protocol {
            endpoint: "qrcode/poll".into(),
            detail: format!("unknown login status {}: {}", data.code, data.message),
        })
    }

    pub fn save_credentials(&self) -> Result<()> {
        let credentials = self.credentials.lock().expect("credentials mutex poisoned");
        if let Some(parent) = self.credential_path.parent() {
            std::fs::create_dir_all(parent).map_err(|source| Error::Io {
                path: parent.to_owned(),
                source,
            })?;
        }
        let json = serde_json::to_vec_pretty(&*credentials)?;
        let temporary = self.credential_path.with_extension("json.tmp");
        std::fs::write(&temporary, json).map_err(|source| Error::Io {
            path: temporary.clone(),
            source,
        })?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&temporary, std::fs::Permissions::from_mode(0o600)).map_err(
                |source| Error::Io {
                    path: temporary.clone(),
                    source,
                },
            )?;
        }
        match std::fs::rename(&temporary, &self.credential_path) {
            Ok(()) => Ok(()),
            #[cfg(windows)]
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                std::fs::remove_file(&self.credential_path).map_err(|source| Error::Io {
                    path: self.credential_path.clone(),
                    source,
                })?;
                std::fs::rename(&temporary, &self.credential_path).map_err(|source| Error::Io {
                    path: self.credential_path.clone(),
                    source,
                })
            }
            Err(source) => Err(Error::Io {
                path: self.credential_path.clone(),
                source,
            }),
        }
    }

    pub fn clear_credentials(&self) -> Result<()> {
        self.credentials
            .lock()
            .expect("credentials mutex poisoned")
            .cookies
            .clear();
        match std::fs::remove_file(&self.credential_path) {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io {
                path: self.credential_path.clone(),
                source,
            }),
        }
    }

    pub async fn plan(
        &self,
        input: &VideoInput,
        quality: Option<u64>,
        codec: Option<&str>,
    ) -> Result<DownloadPlan> {
        let video = self.fetch_video(&input.bvid).await?;
        let page = video
            .pages
            .get(input.page - 1)
            .cloned()
            .ok_or(Error::PageOutOfRange {
                requested: input.page,
                available: video.pages.len(),
            })?;
        self.plan_page(video, page, quality, codec).await
    }

    pub async fn pages(&self, bvid: &str) -> Result<Vec<PageInfo>> {
        Ok(self.fetch_video(bvid).await?.pages)
    }

    pub async fn video_info(&self, bvid: &str) -> Result<VideoInfo> {
        self.fetch_video(bvid).await
    }

    pub async fn plan_all(
        &self,
        input: &VideoInput,
        quality: Option<u64>,
        codec: Option<&str>,
    ) -> Result<Vec<DownloadPlan>> {
        let video = self.fetch_video(&input.bvid).await?;
        let mut plans = Vec::with_capacity(video.pages.len());
        for page in video.pages.clone() {
            plans.push(self.plan_page(video.clone(), page, quality, codec).await?);
        }
        Ok(plans)
    }

    async fn fetch_video(&self, bvid: &str) -> Result<VideoInfo> {
        let view_envelope: Envelope<VideoInfo> = self
            .get_json(&format!("{API}/x/web-interface/view"), &[("bvid", bvid)])
            .await?;
        let mut video = require_data("view", view_envelope)?;
        video.normalize_collection();
        Ok(video)
    }

    async fn plan_page(
        &self,
        video: VideoInfo,
        page: PageInfo,
        quality: Option<u64>,
        codec: Option<&str>,
    ) -> Result<DownloadPlan> {
        let cid = page.cid.to_string();
        let request_bvid = page.bvid.as_deref().unwrap_or(&video.bvid);
        let qn = quality.map(|value| value.to_string());
        let mut params = vec![
            ("cid", cid.as_str()),
            ("fourk", "1"),
            ("fnver", "0"),
            ("fnval", "4048"),
            ("bvid", request_bvid),
        ];
        if let Some(qn) = &qn {
            params.push(("qn", qn.as_str()));
        }
        let play_envelope: Envelope<PlayData> = self
            .get_json(&format!("{API}/x/player/playurl"), &params)
            .await?;
        let play = require_data("playurl", play_envelope)?;
        let dash = play.dash.ok_or(Error::NoVideoStream)?;
        let selected_video = choose_video(&dash.video, quality, codec)?.clone();
        let selected_audio = dash
            .audio
            .iter()
            .max_by_key(|stream| (stream.bandwidth, stream.id))
            .cloned()
            .ok_or(Error::NoAudioStream)?;
        let mut returned_quality_ids: Vec<_> = dash.video.iter().map(|stream| stream.id).collect();
        returned_quality_ids.sort_unstable_by(|a, b| b.cmp(a));
        returned_quality_ids.dedup();
        let advertised_qualities = play
            .accept_quality
            .into_iter()
            .zip(play.accept_description)
            .map(|(id, description)| Quality { id, description })
            .collect();
        Ok(DownloadPlan {
            video,
            page,
            logged_in: self.has_credentials(),
            advertised_qualities,
            returned_quality_ids,
            selected_video,
            selected_audio,
        })
    }

    pub async fn download<F>(&self, stream: &Stream, path: &Path, progress: F) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        download::download_stream(&self.media_http, stream, path, progress).await
    }

    pub async fn download_with_control<F>(
        &self,
        stream: &Stream,
        path: &Path,
        progress: F,
        control: &DownloadControl,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        download::download_stream_with_control(
            &self.media_http,
            stream,
            path,
            progress,
            Some(control),
        )
        .await
    }

    pub async fn download_with_aria2<F>(
        &self,
        stream: &Stream,
        path: &Path,
        manager: &Aria2Manager,
        progress: F,
        control: Option<&DownloadControl>,
    ) -> Result<()>
    where
        F: FnMut(DownloadProgress) + Send,
    {
        manager
            .download_stream(stream, path, &self.cookie_header(), progress, control)
            .await
    }

    pub async fn transcribe_with_bcut<F>(
        &self,
        path: &Path,
        format: TranscriptFormat,
        progress: F,
    ) -> Result<String>
    where
        F: FnMut(BcutProgress),
    {
        let cookie = self.cookie_header();
        if !cookie.split("; ").any(|item| item.starts_with("SESSDATA=")) {
            return Err(Error::NotLoggedIn);
        }
        crate::bcut::transcribe_audio_with_session(
            path,
            format,
            self.proxy.as_deref(),
            self.no_proxy,
            &cookie,
            progress,
        )
        .await
    }

    pub async fn subtitle_tracks(&self, bvid: &str, cid: u64) -> Result<Vec<SubtitleTrack>> {
        let cid_value = cid.to_string();
        let data: PlayerData = require_data(
            "player/v2",
            self.get_json(
                &format!("{API}/x/player/v2"),
                &[("bvid", bvid), ("cid", &cid_value)],
            )
            .await?,
        )?;
        Ok(data
            .subtitle
            .map(|subtitle| {
                subtitle
                    .subtitles
                    .into_iter()
                    .filter(|track| !track.subtitle_url.is_empty())
                    .map(|track| SubtitleTrack {
                        language: track.lan,
                        language_name: track.lan_doc,
                        url: subtitle::normalize_url(&track.subtitle_url),
                    })
                    .collect()
            })
            .unwrap_or_default())
    }

    pub(crate) async fn subtitle_cues(&self, track: &SubtitleTrack) -> Result<Vec<SubtitleCue>> {
        let mut request = self
            .http
            .get(&track.url)
            .header(REFERER, REFERER_VALUE)
            .header(ACCEPT, "application/json, text/plain, */*");
        let cookie = self.cookie_header();
        if !cookie.is_empty() {
            request = request.header(COOKIE, cookie);
        }
        let response = request.send().await.map_err(|source| Error::Network {
            endpoint: "subtitle".into(),
            source,
        })?;
        if !response.status().is_success() {
            return Err(Error::Subtitle(format!(
                "subtitle URL returned HTTP {}",
                response.status()
            )));
        }
        let body: SubtitleBody = response
            .json()
            .await
            .map_err(|error| Error::Subtitle(format!("invalid subtitle JSON: {error}")))?;
        subtitle::ensure_cues(body.body)
    }

    pub async fn render_subtitle(
        &self,
        track: &SubtitleTrack,
        format: SubtitleFormat,
    ) -> Result<String> {
        let cues = self.subtitle_cues(track).await?;
        Ok(subtitle::render(&cues, format))
    }

    async fn get_json<T: DeserializeOwned>(
        &self,
        endpoint: &str,
        params: &[(&str, &str)],
    ) -> Result<T> {
        for attempt in 1..=3 {
            let mut request = self
                .http
                .get(endpoint)
                .query(params)
                .header(REFERER, REFERER_VALUE)
                .header(ACCEPT, "application/json, text/plain, */*");
            let cookie = self.cookie_header();
            if !cookie.is_empty() {
                request = request.header(COOKIE, cookie);
            }
            let response = match request.send().await {
                Ok(response) => response,
                Err(_source) if attempt < 3 => {
                    sleep(Duration::from_millis(400 * attempt)).await;
                    continue;
                }
                Err(source) => {
                    return Err(Error::Network {
                        endpoint: endpoint.into(),
                        source,
                    });
                }
            };
            self.capture_cookies(response.headers());
            let status = response.status();
            if !status.is_success() {
                if attempt < 3 && (status.as_u16() == 429 || status.is_server_error()) {
                    sleep(Duration::from_millis(400 * attempt)).await;
                    continue;
                }
                return Err(Error::Protocol {
                    endpoint: endpoint.into(),
                    detail: format!("HTTP {status}"),
                });
            }
            let body = match response.text().await {
                Ok(body) => body,
                Err(_source) if attempt < 3 => {
                    sleep(Duration::from_millis(400 * attempt)).await;
                    continue;
                }
                Err(source) => {
                    return Err(Error::Network {
                        endpoint: endpoint.into(),
                        source,
                    });
                }
            };
            return serde_json::from_str(&body).map_err(|error| Error::Protocol {
                endpoint: endpoint.into(),
                detail: format!("invalid JSON shape: {error}"),
            });
        }
        unreachable!("API retry loop always returns or continues")
    }

    fn cookie_header(&self) -> String {
        self.credentials
            .lock()
            .expect("credentials mutex poisoned")
            .cookies
            .iter()
            .map(|(name, value)| format!("{name}={value}"))
            .collect::<Vec<_>>()
            .join("; ")
    }

    fn capture_cookies(&self, headers: &HeaderMap) {
        let mut credentials = self.credentials.lock().expect("credentials mutex poisoned");
        for value in headers.get_all(SET_COOKIE) {
            let Ok(value) = value.to_str() else { continue };
            let first = value.split(';').next().unwrap_or_default();
            let Some((name, value)) = first.split_once('=') else {
                continue;
            };
            if name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            {
                if value.is_empty() || value == "deleted" {
                    credentials.cookies.remove(name);
                } else {
                    credentials
                        .cookies
                        .insert(name.to_owned(), value.to_owned());
                }
            }
        }
    }
}

fn require_data<T>(endpoint: &str, envelope: Envelope<T>) -> Result<T> {
    if envelope.code != 0 {
        return Err(Error::Api {
            endpoint: endpoint.into(),
            code: envelope.code,
            message: envelope.message,
        });
    }
    envelope.data.ok_or_else(|| Error::Protocol {
        endpoint: endpoint.into(),
        detail: "missing data object".into(),
    })
}

fn choose_video<'a>(
    streams: &'a [Stream],
    quality: Option<u64>,
    codec: Option<&str>,
) -> Result<&'a Stream> {
    let returned = || {
        let mut ids: Vec<_> = streams.iter().map(|stream| stream.id).collect();
        ids.sort_unstable();
        ids.dedup();
        ids
    };
    let candidates: Vec<_> = streams
        .iter()
        .filter(|stream| quality.is_none_or(|quality| stream.id == quality))
        .collect();
    if candidates.is_empty() {
        return match quality {
            Some(quality) => Err(Error::QualityUnavailable(quality, returned())),
            None => Err(Error::NoVideoStream),
        };
    }
    let codec_matches = |family: &str, requested: &str| {
        family == requested || (requested == "hevc" && matches!(family, "hev1" | "hvc1"))
    };
    let codec_rank = |stream: &&Stream| match codec {
        Some(requested) => u8::from(codec_matches(stream.codec_family(), requested)),
        None => match stream.codec_family() {
            "avc1" => 3,
            "hev1" | "hvc1" => 2,
            "av01" => 1,
            _ => 0,
        },
    };
    if codec.is_some() && !candidates.iter().any(|stream| codec_rank(stream) == 1) {
        return Err(Error::QualityUnavailable(
            quality.unwrap_or(candidates[0].id),
            returned(),
        ));
    }
    candidates
        .into_iter()
        .max_by_key(|stream| {
            (
                stream.width.saturating_mul(stream.height),
                stream.id,
                codec_rank(stream),
                stream.bandwidth,
            )
        })
        .ok_or(Error::NoVideoStream)
}

fn default_credential_path() -> PathBuf {
    directories::ProjectDirs::from("dev", "muxsift", "muxsift")
        .map(|dirs| dirs.config_dir().join("credentials.json"))
        .unwrap_or_else(|| PathBuf::from(".muxsift/credentials.json"))
}

fn load_credentials(path: &Path) -> Result<Credentials> {
    match std::fs::read(path) {
        Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(Credentials::default()),
        Err(source) => Err(Error::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stream(id: u64, codec: &str, width: u64, height: u64) -> Stream {
        Stream {
            id,
            codecs: codec.into(),
            width,
            height,
            base_url: "https://example.test/x".into(),
            backup_url: vec![],
            bandwidth: 1,
            frame_rate: String::new(),
            mime_type: String::new(),
        }
    }

    #[test]
    fn quality_precedes_codec() {
        let streams = [
            stream(16, "avc1.x", 640, 360),
            stream(80, "av01.x", 1920, 1080),
        ];
        assert_eq!(choose_video(&streams, None, None).unwrap().id, 80);
    }

    #[test]
    fn explicit_codec_is_honored_within_quality() {
        let streams = [
            stream(32, "avc1.x", 768, 480),
            stream(32, "hvc1.x", 768, 480),
        ];
        assert_eq!(
            choose_video(&streams, Some(32), Some("hevc"))
                .unwrap()
                .codec_family(),
            "hvc1"
        );
    }
}
