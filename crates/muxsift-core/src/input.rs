use crate::{Error, Result};
use reqwest::header::USER_AGENT;
use std::time::Duration;
use url::Url;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoInput {
    pub bvid: String,
    pub page: usize,
}

fn valid_bvid(value: &str) -> bool {
    value.len() == 12
        && value.starts_with("BV")
        && value[2..].bytes().all(|byte| byte.is_ascii_alphanumeric())
}

pub fn parse_video_input(value: &str, page_override: Option<usize>) -> Result<VideoInput> {
    let value = value.trim();
    let (bvid, url_page) = if valid_bvid(value) {
        (value.to_owned(), None)
    } else {
        let url = Url::parse(value)
            .map_err(|_| Error::InvalidInput("expected a BV ID or Bilibili video URL".into()))?;
        let host = url.host_str().unwrap_or_default().to_ascii_lowercase();
        if !matches!(
            host.as_str(),
            "bilibili.com" | "www.bilibili.com" | "m.bilibili.com"
        ) {
            return Err(Error::InvalidInput("URL host must be bilibili.com".into()));
        }
        let mut segments = url.path_segments().into_iter().flatten();
        if segments.next() != Some("video") {
            return Err(Error::InvalidInput(
                "URL path must start with /video/".into(),
            ));
        }
        let bvid = segments.next().unwrap_or_default();
        if !valid_bvid(bvid) || segments.any(|part| !part.is_empty()) {
            return Err(Error::InvalidInput(
                "URL does not contain one valid BV ID".into(),
            ));
        }
        let page = url
            .query_pairs()
            .find(|(key, _)| key == "p")
            .map(|(_, value)| {
                value
                    .parse::<usize>()
                    .map_err(|_| Error::InvalidInput("URL page must be an integer".into()))
            })
            .transpose()?;
        (bvid.to_owned(), page)
    };
    let page = page_override.or(url_page).unwrap_or(1);
    if page == 0 {
        return Err(Error::InvalidInput("page must be at least 1".into()));
    }
    Ok(VideoInput { bvid, page })
}

/// Parse a video identifier, resolving Bilibili mobile short links first.
///
/// `b23.tv` links are redirectors rather than video pages, so they cannot be
/// parsed without a network request. The final URL is then passed through the
/// same strict parser used by ordinary Bilibili URLs.
pub async fn resolve_video_input(
    value: &str,
    page_override: Option<usize>,
    proxy: Option<&str>,
    no_proxy: bool,
) -> Result<VideoInput> {
    let trimmed = value.trim();
    if valid_bvid(trimmed) || !is_b23_url(trimmed) {
        return parse_video_input(trimmed, page_override);
    }

    let mut builder = reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (compatible; muxsift)")
        .timeout(Duration::from_secs(15));
    if no_proxy {
        builder = builder.no_proxy();
    } else if let Some(proxy) = proxy {
        builder = builder.proxy(
            reqwest::Proxy::all(proxy)
                .map_err(|error| Error::InvalidInput(format!("invalid proxy URL: {error}")))?,
        );
    }
    let client = builder.build().map_err(|source| Error::Network {
        endpoint: "short-link HTTP client initialization".into(),
        source,
    })?;
    let response = client
        .get(trimmed)
        .header(USER_AGENT, "Mozilla/5.0 (compatible; muxsift)")
        .send()
        .await
        .map_err(|source| Error::Network {
            endpoint: trimmed.to_owned(),
            source,
        })?;
    let final_url = response.url().to_string();
    parse_video_input(&final_url, page_override).map_err(|error| match error {
        Error::InvalidInput(detail) => Error::InvalidInput(format!(
            "short link resolved to {final_url}, but it is not a supported video URL: {detail}"
        )),
        other => other,
    })
}

fn is_b23_url(value: &str) -> bool {
    let Ok(url) = Url::parse(value) else {
        return false;
    };
    matches!(
        url.host_str()
            .unwrap_or_default()
            .to_ascii_lowercase()
            .as_str(),
        "b23.tv" | "www.b23.tv"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drops_tracking_query() {
        let parsed = parse_video_input(
            "https://www.bilibili.com/video/BV1TXtb6iEt4/?spm_id_from=x&vd_source=secret",
            None,
        )
        .unwrap();
        assert_eq!(
            parsed,
            VideoInput {
                bvid: "BV1TXtb6iEt4".into(),
                page: 1
            }
        );
    }

    #[test]
    fn explicit_page_wins() {
        let parsed =
            parse_video_input("https://www.bilibili.com/video/BV1TXtb6iEt4/?p=2", Some(3)).unwrap();
        assert_eq!(parsed.page, 3);
    }

    #[test]
    fn rejects_lookalikes() {
        assert!(parse_video_input("BV1TXtb6iEt4extra", None).is_err());
        assert!(parse_video_input("https://example.com/video/BV1TXtb6iEt4", None).is_err());
    }

    #[test]
    fn recognizes_b23_short_link() {
        assert!(is_b23_url("https://b23.tv/bhQt1jZ"));
        assert!(!is_b23_url("https://example.com/bhQt1jZ"));
    }
}
