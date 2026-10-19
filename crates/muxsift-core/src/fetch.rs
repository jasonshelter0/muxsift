use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::{Deserialize, Serialize};
use tokio::process::Command;
use url::Url;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct FetchOptions {
    pub url: String,
    pub output_dir: PathBuf,
    pub executable: PathBuf,
    pub ffmpeg: PathBuf,
    pub proxy: Option<String>,
    pub no_proxy: bool,
}

#[derive(Debug, Serialize, Deserialize)]
struct FetchManifest {
    source_url: String,
    media: PathBuf,
}

const FETCH_MANIFEST: &str = "muxsift-fetch.json";

/// Download one public media URL into a new directory using an installed yt-dlp.
pub async fn fetch_with_yt_dlp(options: &FetchOptions) -> Result<PathBuf> {
    validate_url(&options.url)?;
    let parent = options
        .output_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        path: parent.to_owned(),
        source,
    })?;
    if options.output_dir.symlink_metadata().is_ok() {
        return Err(Error::InvalidInput(format!(
            "output already exists: {}",
            options.output_dir.display()
        )));
    }
    let temporary = tempfile::Builder::new()
        .prefix(".muxsift-fetch-")
        .tempdir_in(parent)
        .map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    let path_file = temporary.path().join("downloaded-path.txt");
    let mut command = Command::new(&options.executable);
    command
        .kill_on_drop(true)
        .args(["--ignore-config", "--no-playlist", "--playlist-items", "1"])
        .args(["--no-overwrites", "--no-progress", "-P"])
        .arg(temporary.path())
        .args([
            "-o",
            "%(id)s.%(ext)s",
            "--print-to-file",
            "after_move:filepath",
        ])
        .arg(&path_file);
    if options.ffmpeg != Path::new("ffmpeg") {
        command.arg("--ffmpeg-location").arg(&options.ffmpeg);
    }
    if options.no_proxy {
        command.arg("--proxy").arg("");
    } else if let Some(proxy) = &options.proxy {
        command.arg("--proxy").arg(proxy);
    }
    let output = command
        .arg(&options.url)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .await
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::YtDlpMissing(options.executable.display().to_string())
            } else {
                Error::Io {
                    path: options.executable.clone(),
                    source,
                }
            }
        })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr)
            .lines()
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(Error::YtDlpFailed {
            code: output.status.code(),
            detail,
        });
    }
    let printed_path = fs::read_to_string(&path_file).map_err(|source| Error::Io {
        path: path_file.clone(),
        source,
    })?;
    let paths: Vec<&str> = printed_path
        .lines()
        .filter(|line| !line.trim().is_empty())
        .collect();
    if paths.len() != 1 {
        return Err(Error::InvalidInput(format!(
            "yt-dlp reported {} media files; expected one",
            paths.len()
        )));
    }
    let temporary_root = temporary
        .path()
        .canonicalize()
        .map_err(|source| Error::Io {
            path: temporary.path().to_owned(),
            source,
        })?;
    let downloaded = Path::new(paths[0])
        .canonicalize()
        .map_err(|source| Error::Io {
            path: PathBuf::from(paths[0]),
            source,
        })?;
    let relative = downloaded.strip_prefix(&temporary_root).map_err(|_| {
        Error::InvalidInput("yt-dlp reported a file outside the download directory".into())
    })?;
    if !downloaded.is_file() || downloaded.metadata().is_ok_and(|meta| meta.len() == 0) {
        return Err(Error::InvalidInput(
            "yt-dlp did not produce a non-empty media file".into(),
        ));
    }
    fs::remove_file(&path_file).map_err(|source| Error::Io {
        path: path_file,
        source,
    })?;
    let manifest = FetchManifest {
        source_url: options.url.clone(),
        media: relative.to_owned(),
    };
    let manifest_path = temporary.path().join(FETCH_MANIFEST);
    fs::write(&manifest_path, serde_json::to_vec_pretty(&manifest)?).map_err(|source| {
        Error::Io {
            path: manifest_path,
            source,
        }
    })?;
    let final_path = options.output_dir.join(relative);
    fs::rename(temporary.path(), &options.output_dir).map_err(|source| Error::Io {
        path: options.output_dir.clone(),
        source,
    })?;
    Ok(final_path)
}

/// Reuse a completed acquisition only when its URL and media file still match.
pub(crate) fn reuse_fetched_media(options: &FetchOptions) -> Result<PathBuf> {
    validate_url(&options.url)?;
    let manifest_path = options.output_dir.join(FETCH_MANIFEST);
    let bytes = fs::read(&manifest_path).map_err(|source| Error::Io {
        path: manifest_path.clone(),
        source,
    })?;
    let manifest: FetchManifest = serde_json::from_slice(&bytes).map_err(|error| {
        Error::InvalidInput(format!(
            "invalid fetch manifest {}: {error}",
            manifest_path.display()
        ))
    })?;
    if manifest.source_url != options.url {
        return Err(Error::InvalidInput(
            "saved media belongs to a different URL; choose a new output directory".into(),
        ));
    }
    let root = options
        .output_dir
        .canonicalize()
        .map_err(|source| Error::Io {
            path: options.output_dir.clone(),
            source,
        })?;
    let media = root
        .join(&manifest.media)
        .canonicalize()
        .map_err(|source| Error::Io {
            path: root.join(&manifest.media),
            source,
        })?;
    if !media.starts_with(&root)
        || !media.is_file()
        || media.metadata().is_ok_and(|meta| meta.len() == 0)
    {
        return Err(Error::InvalidInput(
            "saved media is missing, empty, or outside its output directory".into(),
        ));
    }
    Ok(media)
}

pub(crate) fn validate_url(value: &str) -> Result<()> {
    let url = Url::parse(value)
        .map_err(|_| Error::InvalidInput("fetch requires an HTTP or HTTPS URL".into()))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(Error::InvalidInput(
            "fetch requires an HTTP or HTTPS URL".into(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{FETCH_MANIFEST, FetchManifest, FetchOptions, reuse_fetched_media, validate_url};
    use std::{fs, path::PathBuf};

    #[test]
    fn fetch_accepts_only_web_urls() {
        assert!(validate_url("https://www.youtube.com/watch?v=abc").is_ok());
        assert!(validate_url("http://127.0.0.1:8080/video.mp4").is_ok());
        assert!(validate_url("file:///tmp/video.mp4").is_err());
        assert!(validate_url("relative/path.mp4").is_err());
    }

    #[test]
    fn resume_reuses_only_the_same_url_and_a_nonempty_file_inside_the_task() {
        let temporary = tempfile::tempdir().unwrap();
        let root = temporary.path().join("media");
        fs::create_dir(&root).unwrap();
        fs::write(root.join("video.mp4"), b"media").unwrap();
        let options = FetchOptions {
            url: "https://example.org/watch?v=1".into(),
            output_dir: root.clone(),
            executable: PathBuf::from("yt-dlp"),
            ffmpeg: PathBuf::from("ffmpeg"),
            proxy: None,
            no_proxy: false,
        };
        let write_manifest = |url: &str, media: &str| {
            fs::write(
                root.join(FETCH_MANIFEST),
                serde_json::to_vec(&FetchManifest {
                    source_url: url.into(),
                    media: PathBuf::from(media),
                })
                .unwrap(),
            )
            .unwrap();
        };
        write_manifest(&options.url, "video.mp4");
        assert_eq!(
            reuse_fetched_media(&options).unwrap(),
            root.join("video.mp4")
        );

        write_manifest("https://example.org/watch?v=2", "video.mp4");
        assert!(reuse_fetched_media(&options).is_err());

        write_manifest(&options.url, "../outside.mp4");
        fs::write(temporary.path().join("outside.mp4"), b"outside").unwrap();
        assert!(reuse_fetched_media(&options).is_err());

        write_manifest(&options.url, "video.mp4");
        fs::write(root.join("video.mp4"), []).unwrap();
        assert!(reuse_fetched_media(&options).is_err());
    }
}
