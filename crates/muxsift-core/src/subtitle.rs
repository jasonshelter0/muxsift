use serde::{Deserialize, Serialize};

use crate::{Error, Result};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubtitleFormat {
    Srt,
    Vtt,
}

impl SubtitleFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Srt => "srt",
            Self::Vtt => "vtt",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SubtitleTrack {
    pub language: String,
    pub language_name: String,
    pub url: String,
}

#[derive(Debug, Deserialize)]
pub(crate) struct SubtitleBody {
    pub body: Vec<SubtitleCue>,
}

#[derive(Debug, Deserialize, Serialize)]
pub(crate) struct SubtitleCue {
    pub from: f64,
    pub to: f64,
    pub content: String,
}

pub(crate) fn normalize_url(value: &str) -> String {
    if value.starts_with("//") {
        format!("https:{value}")
    } else {
        value.to_owned()
    }
}

pub(crate) fn render(cues: &[SubtitleCue], format: SubtitleFormat) -> String {
    match format {
        SubtitleFormat::Srt => cues
            .iter()
            .enumerate()
            .map(|(index, cue)| {
                format!(
                    "{}\n{} --> {}\n{}\n",
                    index + 1,
                    timestamp(cue.from, ','),
                    timestamp(cue.to, ','),
                    cue.content
                )
            })
            .collect(),
        SubtitleFormat::Vtt => {
            let mut output = String::from("WEBVTT\n\n");
            for cue in cues {
                output.push_str(&format!(
                    "{} --> {}\n{}\n\n",
                    timestamp(cue.from, '.'),
                    timestamp(cue.to, '.'),
                    cue.content
                ));
            }
            output
        }
    }
}

fn timestamp(seconds: f64, millisecond_separator: char) -> String {
    let milliseconds = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        milliseconds / 3_600_000,
        milliseconds / 60_000 % 60,
        milliseconds / 1_000 % 60,
        millisecond_separator,
        milliseconds % 1_000
    )
}

pub(crate) fn ensure_cues(cues: Vec<SubtitleCue>) -> Result<Vec<SubtitleCue>> {
    if cues.is_empty() {
        Err(Error::Subtitle("subtitle track is empty".into()))
    } else {
        Ok(cues)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn renders_srt_and_vtt_timestamps() {
        let cues = vec![SubtitleCue {
            from: 1.25,
            to: 3.5,
            content: "hello".into(),
        }];
        assert!(render(&cues, SubtitleFormat::Srt).contains("00:00:01,250 --> 00:00:03,500"));
        assert!(render(&cues, SubtitleFormat::Vtt).contains("00:00:01.250 --> 00:00:03.500"));
    }
}
