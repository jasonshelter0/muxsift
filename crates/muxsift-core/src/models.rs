use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Owner {
    pub mid: u64,
    pub name: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Dimension {
    pub width: u64,
    pub height: u64,
    #[serde(default)]
    pub rotate: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PageInfo {
    pub cid: u64,
    pub page: usize,
    pub part: String,
    pub duration: u64,
    pub dimension: Option<Dimension>,
    /// The owning BV for collection episodes. Ordinary pages leave this unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bvid: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub aid: Option<u64>,
    /// The source section when this page originated from a UGC collection.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub section: Option<String>,
    /// The owning collection member title for nested UGC pages.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode: Option<String>,
    /// The page number inside the owning member video. `page` remains the
    /// stable, flattened selection index used by the frontends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub episode_page: Option<usize>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UgcSeason {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    #[serde(alias = "title")]
    pub name: String,
    #[serde(default)]
    pub sections: Vec<UgcSection>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UgcSection {
    #[serde(default)]
    pub id: u64,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub episodes: Vec<UgcEpisode>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct UgcEpisode {
    #[serde(default)]
    pub aid: u64,
    #[serde(default)]
    pub bvid: String,
    #[serde(default)]
    pub cid: u64,
    #[serde(default, alias = "title")]
    pub title: String,
    #[serde(default)]
    pub duration: u64,
    #[serde(default)]
    pub arc: Option<UgcArc>,
    #[serde(default)]
    pub pages: Vec<PageInfo>,
}

#[derive(Debug, Clone, Deserialize, Serialize, Default)]
pub struct UgcArc {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub duration: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct VideoInfo {
    pub aid: u64,
    pub bvid: String,
    pub title: String,
    pub owner: Owner,
    pub pages: Vec<PageInfo>,
    #[serde(default)]
    pub ugc_season: Option<UgcSeason>,
}

impl VideoInfo {
    /// Flatten UGC collection episodes into the same page list used by the
    /// downloader and both frontends. Episode BV values are retained so each
    /// playurl request is sent for the actual member video.
    pub fn normalize_collection(&mut self) {
        let Some(season) = self.ugc_season.as_ref() else {
            return;
        };
        let mut pages = Vec::new();
        for section in &season.sections {
            for episode in &section.episodes {
                let episode_title = if !episode.title.is_empty() {
                    episode.title.clone()
                } else {
                    episode
                        .arc
                        .as_ref()
                        .map_or_else(String::new, |arc| arc.title.clone())
                };
                let nested_pages = if episode.pages.is_empty() {
                    vec![PageInfo {
                        cid: episode.cid,
                        page: 1,
                        part: episode_title.clone(),
                        duration: episode.duration,
                        dimension: None,
                        bvid: None,
                        aid: None,
                        section: None,
                        episode: None,
                        episode_page: None,
                    }]
                } else {
                    episode.pages.clone()
                };
                let single_leaf = nested_pages.len() == 1;
                for nested in nested_pages {
                    let leaf_title = if single_leaf && !episode_title.trim().is_empty() {
                        episode_title.clone()
                    } else if !nested.part.trim().is_empty() {
                        nested.part
                    } else {
                        episode_title.clone()
                    };
                    pages.push(PageInfo {
                        cid: nested.cid,
                        page: pages.len() + 1,
                        part: leaf_title,
                        duration: nested.duration,
                        dimension: nested.dimension,
                        bvid: (!episode.bvid.is_empty()).then(|| episode.bvid.clone()),
                        aid: (episode.aid != 0).then_some(episode.aid),
                        section: (!section.title.is_empty()).then(|| section.title.clone()),
                        episode: (!episode_title.is_empty()).then_some(episode_title.clone()),
                        episode_page: Some(nested.page),
                    });
                }
            }
        }
        if pages.is_empty() {
            return;
        }
        self.pages = pages;
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Stream {
    pub id: u64,
    pub base_url: String,
    pub backup_url: Vec<String>,
    pub bandwidth: u64,
    pub codecs: String,
    pub width: u64,
    pub height: u64,
    pub frame_rate: String,
    pub mime_type: String,
}

impl<'de> Deserialize<'de> for Stream {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct Wire {
            id: u64,
            #[serde(rename = "baseUrl")]
            base_url_camel: Option<String>,
            base_url: Option<String>,
            #[serde(default, rename = "backupUrl")]
            backup_url_camel: Vec<String>,
            #[serde(default)]
            backup_url: Vec<String>,
            #[serde(default)]
            bandwidth: u64,
            #[serde(default)]
            codecs: String,
            #[serde(default)]
            width: u64,
            #[serde(default)]
            height: u64,
            #[serde(default, rename = "frameRate")]
            frame_rate_camel: String,
            #[serde(default)]
            frame_rate: String,
            #[serde(default, rename = "mimeType")]
            mime_type_camel: String,
            #[serde(default)]
            mime_type: String,
        }

        let wire = Wire::deserialize(deserializer)?;
        let base_url = wire
            .base_url_camel
            .or(wire.base_url)
            .ok_or_else(|| serde::de::Error::missing_field("baseUrl/base_url"))?;
        Ok(Self {
            id: wire.id,
            base_url,
            backup_url: if wire.backup_url_camel.is_empty() {
                wire.backup_url
            } else {
                wire.backup_url_camel
            },
            bandwidth: wire.bandwidth,
            codecs: wire.codecs,
            width: wire.width,
            height: wire.height,
            frame_rate: if wire.frame_rate_camel.is_empty() {
                wire.frame_rate
            } else {
                wire.frame_rate_camel
            },
            mime_type: if wire.mime_type_camel.is_empty() {
                wire.mime_type
            } else {
                wire.mime_type_camel
            },
        })
    }
}

impl Stream {
    pub fn codec_family(&self) -> &str {
        self.codecs.split('.').next().unwrap_or(&self.codecs)
    }

    pub fn redacted_url(&self) -> String {
        redact_url(&self.base_url)
    }

    pub fn urls(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.base_url.as_str()).chain(self.backup_url.iter().map(String::as_str))
    }
}

fn redact_url(value: &str) -> String {
    match url::Url::parse(value) {
        Ok(url) => {
            let mut result = format!(
                "{}://{}{}",
                url.scheme(),
                url.host_str().unwrap_or("<invalid-host>"),
                url.path()
            );
            if let Some(port) = url.port() {
                result = format!(
                    "{}://{}:{}{}",
                    url.scheme(),
                    url.host_str().unwrap_or("<invalid-host>"),
                    port,
                    url.path()
                );
            }
            if url.query().is_some() {
                result.push_str("?<redacted>");
            }
            result
        }
        Err(_) => "<invalid-url>".into(),
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Quality {
    pub id: u64,
    pub description: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct DownloadPlan {
    pub video: VideoInfo,
    pub page: PageInfo,
    pub logged_in: bool,
    pub advertised_qualities: Vec<Quality>,
    pub returned_quality_ids: Vec<u64>,
    pub selected_video: Stream,
    pub selected_audio: Stream,
}

impl DownloadPlan {
    pub fn redacted_json(&self) -> serde_json::Value {
        serde_json::json!({
            "bvid": self.video.bvid,
            "title": self.video.title,
            "author": self.video.owner.name,
            "logged_in": self.logged_in,
            "page_count": self.video.pages.len(),
            "collection": self.video.ugc_season.as_ref().map(|season| serde_json::json!({
                "id": season.id,
                "name": season.name,
                "episode_count": self.video.pages.len(),
            })),
            "selected_page": self.page,
            "advertised_qualities": self.advertised_qualities,
            "returned_quality_ids": self.returned_quality_ids,
            "selected_video": {
                "id": self.selected_video.id,
                "codecs": self.selected_video.codecs,
                "bandwidth": self.selected_video.bandwidth,
                "width": self.selected_video.width,
                "height": self.selected_video.height,
                "frame_rate": self.selected_video.frame_rate,
                "mime_type": self.selected_video.mime_type,
                "url": self.selected_video.redacted_url(),
                "backup_count": self.selected_video.backup_url.len(),
            },
            "selected_audio": {
                "id": self.selected_audio.id,
                "codecs": self.selected_audio.codecs,
                "bandwidth": self.selected_audio.bandwidth,
                "mime_type": self.selected_audio.mime_type,
                "url": self.selected_audio.redacted_url(),
                "backup_count": self.selected_audio.backup_url.len(),
            }
        })
    }
}
