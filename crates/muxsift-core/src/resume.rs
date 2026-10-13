use std::path::{Path, PathBuf};

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct ResumeWorkspace {
    root: PathBuf,
}

impl ResumeWorkspace {
    pub async fn open(output: &Path, identity: &str, resume: bool) -> Result<Self> {
        let root = Self::path_for(output);
        let identity_path = root.join("identity");
        if root.exists() {
            let saved = tokio::fs::read_to_string(&identity_path)
                .await
                .map_err(|source| Error::Io {
                    path: identity_path.clone(),
                    source,
                })?;
            if saved != identity {
                return Err(Error::InvalidInput(format!(
                    "resume checkpoint does not match this task: {}",
                    root.display()
                )));
            }
            if !resume {
                return Err(Error::InvalidInput(format!(
                    "resume checkpoint exists: {}. Use --resume in CLI or press r in TUI",
                    root.display()
                )));
            }
        } else {
            tokio::fs::create_dir_all(&root)
                .await
                .map_err(|source| Error::Io {
                    path: root.clone(),
                    source,
                })?;
            tokio::fs::write(&identity_path, identity)
                .await
                .map_err(|source| Error::Io {
                    path: identity_path,
                    source,
                })?;
        }
        Ok(Self { root })
    }

    pub fn path_for(output: &Path) -> PathBuf {
        let parent = output.parent().unwrap_or_else(|| Path::new("."));
        let name = output
            .file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_else(|| "output".into());
        parent.join(".muxsift-work").join(name.as_ref())
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn artifact(&self, name: &str) -> PathBuf {
        self.root.join(name)
    }

    pub async fn stage_complete(&self, stage: &str, artifact: &Path) -> bool {
        let marker = self.marker(stage);
        marker.is_file()
            && tokio::fs::metadata(artifact)
                .await
                .is_ok_and(|metadata| metadata.is_file() && metadata.len() > 0)
    }

    pub async fn mark_complete(&self, stage: &str, artifact: &Path) -> Result<()> {
        let metadata = tokio::fs::metadata(artifact)
            .await
            .map_err(|source| Error::Io {
                path: artifact.to_owned(),
                source,
            })?;
        if !metadata.is_file() || metadata.len() == 0 {
            return Err(Error::InvalidInput(format!(
                "cannot checkpoint an empty stage output: {}",
                artifact.display()
            )));
        }
        let marker = self.marker(stage);
        tokio::fs::write(&marker, b"complete")
            .await
            .map_err(|source| Error::Io {
                path: marker,
                source,
            })
    }

    pub async fn remove_incomplete_output(&self, artifact: &Path) -> Result<()> {
        match tokio::fs::remove_file(artifact).await {
            Ok(()) => Ok(()),
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io {
                path: artifact.to_owned(),
                source,
            }),
        }
    }

    pub async fn finish(self) -> Result<()> {
        Self::remove_for_output_root(&self.root).await
    }

    pub async fn remove_for_output(output: &Path) -> Result<()> {
        Self::remove_for_output_root(&Self::path_for(output)).await
    }

    async fn remove_for_output_root(root: &Path) -> Result<()> {
        match tokio::fs::remove_dir_all(root).await {
            Ok(()) => {
                if let Some(parent) = root.parent() {
                    let _ = tokio::fs::remove_dir(parent).await;
                }
                Ok(())
            }
            Err(source) if source.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(source) => Err(Error::Io {
                path: root.to_owned(),
                source,
            }),
        }
    }

    fn marker(&self, stage: &str) -> PathBuf {
        self.root.join(format!("{stage}.complete"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn resumes_only_complete_matching_stages() {
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("episode.srt");
        let workspace = ResumeWorkspace::open(&output, "task-a", false)
            .await
            .unwrap();
        let audio = workspace.artifact("audio.m4a");
        tokio::fs::write(&audio, b"audio").await.unwrap();
        assert!(!workspace.stage_complete("audio-mux", &audio).await);
        workspace.mark_complete("audio-mux", &audio).await.unwrap();
        assert!(workspace.stage_complete("audio-mux", &audio).await);

        assert!(
            ResumeWorkspace::open(&output, "task-a", false)
                .await
                .is_err()
        );
        assert!(
            ResumeWorkspace::open(&output, "task-b", true)
                .await
                .is_err()
        );
        let resumed = ResumeWorkspace::open(&output, "task-a", true)
            .await
            .unwrap();
        assert!(resumed.stage_complete("audio-mux", &audio).await);
        resumed.finish().await.unwrap();
        assert!(!ResumeWorkspace::path_for(&output).exists());
    }
}
