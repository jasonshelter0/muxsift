use std::{
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use serde::Serialize;
use tokio::process::Command;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct FrameOptions {
    pub input: PathBuf,
    pub srt: Option<PathBuf>,
    pub output_dir: PathBuf,
    pub ffmpeg: PathBuf,
    pub sample_interval_seconds: f64,
    pub scene_threshold: f64,
    pub min_gap_seconds: f64,
}

#[derive(Debug, Serialize)]
pub struct FrameEntry {
    pub time_seconds: f64,
    pub image: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub transcript: Vec<FrameTranscriptCue>,
}

#[derive(Debug, Serialize)]
pub struct FrameTranscriptCue {
    pub start_seconds: f64,
    pub end_seconds: f64,
    pub text: String,
}

#[derive(Debug, Serialize)]
pub struct FrameManifest {
    pub source: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub srt_source: Option<String>,
    pub method: &'static str,
    pub sample_interval_seconds: f64,
    pub scene_threshold: f64,
    pub min_gap_seconds: f64,
    pub frames: Vec<FrameEntry>,
}

pub async fn extract_scene_frames(options: &FrameOptions) -> Result<FrameManifest> {
    validate(options)?;
    let cues = if let Some(path) = &options.srt {
        let text = fs::read_to_string(path).map_err(|source| Error::Io {
            path: path.clone(),
            source,
        })?;
        parse_srt(&text)?
    } else {
        Vec::new()
    };
    let parent = options
        .output_dir
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent).map_err(|source| Error::Io {
        path: parent.to_owned(),
        source,
    })?;
    // Keep an existing result intact. FFmpeg writes only to a temporary directory.
    if options.output_dir.symlink_metadata().is_ok() {
        return Err(Error::InvalidInput(format!(
            "output already exists: {}",
            options.output_dir.display()
        )));
    }
    let temporary = tempfile::Builder::new()
        .prefix(".muxsift-frames-")
        .tempdir_in(parent)
        .map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    let frame_dir = temporary.path().join("frames");
    fs::create_dir(&frame_dir).map_err(|source| Error::Io {
        path: frame_dir.clone(),
        source,
    })?;

    let filter = format!(
        "fps={:.8},select='eq(n,0)+gt(scene,{:.6})*gte(t-prev_selected_t,{:.3})',showinfo",
        1.0 / options.sample_interval_seconds,
        options.scene_threshold,
        options.min_gap_seconds,
    );
    let pattern = frame_dir.join("frame_%05d.jpg");
    let output = Command::new(&options.ffmpeg)
        .kill_on_drop(true)
        .args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "info"])
        .arg("-i")
        .arg(&options.input)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(filter)
        .args(["-fps_mode", "vfr", "-q:v", "3"])
        .arg(&pattern)
        .stdout(Stdio::null())
        .output()
        .await
        .map_err(|source| {
            if source.kind() == std::io::ErrorKind::NotFound {
                Error::FfmpegMissing(options.ffmpeg.display().to_string())
            } else {
                Error::Io {
                    path: options.ffmpeg.clone(),
                    source,
                }
            }
        })?;
    if !output.status.success() {
        return Err(Error::FfmpegFailed {
            code: output.status.code(),
            stderr: String::from_utf8_lossy(&output.stderr)
                .lines()
                .rev()
                .take(12)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n"),
        });
    }

    let timestamps = parse_showinfo_timestamps(&String::from_utf8_lossy(&output.stderr));
    let mut images = fs::read_dir(&frame_dir)
        .map_err(|source| Error::Io {
            path: frame_dir.clone(),
            source,
        })?
        .map(|entry| entry.map(|entry| entry.file_name()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|source| Error::Io {
            path: frame_dir.clone(),
            source,
        })?;
    images.sort();
    if images.is_empty() || images.len() != timestamps.len() {
        return Err(Error::InvalidInput(format!(
            "FFmpeg produced {} images but {} timestamps; no result was saved",
            images.len(),
            timestamps.len()
        )));
    }
    let mut frames: Vec<FrameEntry> = images
        .into_iter()
        .zip(timestamps)
        .map(|(image, time_seconds)| FrameEntry {
            time_seconds,
            image: format!("frames/{}", image.to_string_lossy()),
            transcript: Vec::new(),
        })
        .collect();
    attach_transcript(&mut frames, cues);
    let manifest = FrameManifest {
        source: options.input.to_string_lossy().into_owned(),
        srt_source: options
            .srt
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned()),
        method: "ffmpeg_scene_change",
        sample_interval_seconds: options.sample_interval_seconds,
        scene_threshold: options.scene_threshold,
        min_gap_seconds: options.min_gap_seconds,
        frames,
    };
    let json = serde_json::to_vec_pretty(&manifest).map_err(|error| {
        Error::InvalidInput(format!("could not serialize frame index: {error}"))
    })?;
    write_file(&temporary.path().join("timeline.json"), &json)?;
    write_file(
        &temporary.path().join("frames.md"),
        render_markdown(&manifest).as_bytes(),
    )?;
    write_file(
        &temporary.path().join("frames.html"),
        render_html(&manifest).as_bytes(),
    )?;
    fs::rename(temporary.path(), &options.output_dir).map_err(|source| Error::Io {
        path: options.output_dir.clone(),
        source,
    })?;
    Ok(manifest)
}

fn validate(options: &FrameOptions) -> Result<()> {
    if !options.input.is_file() {
        return Err(Error::InvalidInput(format!(
            "video file does not exist: {}",
            options.input.display()
        )));
    }
    if !options.sample_interval_seconds.is_finite()
        || !(0.2..=60.0).contains(&options.sample_interval_seconds)
    {
        return Err(Error::InvalidInput(
            "sample interval must be between 0.2 and 60 seconds".into(),
        ));
    }
    if !options.scene_threshold.is_finite() || !(0.0..=1.0).contains(&options.scene_threshold) {
        return Err(Error::InvalidInput(
            "scene threshold must be between 0 and 1".into(),
        ));
    }
    if !options.min_gap_seconds.is_finite() || !(0.0..=600.0).contains(&options.min_gap_seconds) {
        return Err(Error::InvalidInput(
            "minimum frame gap must be between 0 and 600 seconds".into(),
        ));
    }
    Ok(())
}

fn parse_showinfo_timestamps(stderr: &str) -> Vec<f64> {
    stderr
        .lines()
        .filter(|line| line.contains("Parsed_showinfo_") && line.contains(" n:"))
        .filter_map(|line| line.split_once("pts_time:")?.1.split_whitespace().next())
        .filter_map(|value| value.parse::<f64>().ok())
        .filter(|value| value.is_finite() && *value >= 0.0)
        .collect()
}

fn write_file(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents).map_err(|source| Error::Io {
        path: path.to_owned(),
        source,
    })
}

fn parse_srt(text: &str) -> Result<Vec<FrameTranscriptCue>> {
    let normalized = text.trim_start_matches('\u{feff}').replace("\r\n", "\n");
    let mut cues = Vec::new();
    let mut block = Vec::new();
    for line in normalized.lines() {
        let line = line.trim();
        if line.is_empty() {
            parse_srt_block(&block, &mut cues)?;
            block.clear();
        } else {
            block.push(line);
        }
    }
    parse_srt_block(&block, &mut cues)?;
    if cues.is_empty() {
        return Err(Error::Subtitle("SRT file contains no usable cues".into()));
    }
    cues.sort_by(|a, b| a.start_seconds.total_cmp(&b.start_seconds));
    Ok(cues)
}

fn parse_srt_block(lines: &[&str], cues: &mut Vec<FrameTranscriptCue>) -> Result<()> {
    let Some(timing_index) = lines.iter().position(|line| line.contains("-->")) else {
        return Ok(());
    };
    let (start, end) = lines[timing_index].split_once("-->").unwrap();
    let Some(start_seconds) = parse_srt_timestamp(start.trim()) else {
        return Err(Error::Subtitle(format!("invalid SRT start time: {start}")));
    };
    let Some(end_seconds) = end.split_whitespace().next().and_then(parse_srt_timestamp) else {
        return Err(Error::Subtitle(format!("invalid SRT end time: {end}")));
    };
    if end_seconds < start_seconds {
        return Err(Error::Subtitle("SRT cue ends before it starts".into()));
    }
    let text = lines[timing_index + 1..].join(" ");
    if !text.is_empty() {
        cues.push(FrameTranscriptCue {
            start_seconds,
            end_seconds,
            text,
        });
    }
    Ok(())
}

fn parse_srt_timestamp(value: &str) -> Option<f64> {
    let (clock, fraction) = value.rsplit_once([',', '.'])?;
    if fraction.is_empty()
        || fraction.len() > 3
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    let mut parts = clock.split(':');
    let hours = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || minutes >= 60 || seconds >= 60 {
        return None;
    }
    let whole_seconds = hours
        .checked_mul(3600)?
        .checked_add(minutes * 60)?
        .checked_add(seconds)?;
    let subsecond = fraction.parse::<u64>().ok()? as f64 / 10_u64.pow(fraction.len() as u32) as f64;
    Some(whole_seconds as f64 + subsecond)
}

fn attach_transcript(frames: &mut [FrameEntry], cues: Vec<FrameTranscriptCue>) {
    for cue in cues {
        let index = frames
            .partition_point(|frame| frame.time_seconds <= cue.start_seconds)
            .saturating_sub(1);
        frames[index].transcript.push(cue);
    }
}

fn render_markdown(manifest: &FrameManifest) -> String {
    let mut output = String::from("# 视频场景候选帧\n\n");
    output.push_str("按场景变化检测生成；请人工核对课件动画和逐步推导。\n\n");
    for frame in &manifest.frames {
        let seconds = frame.time_seconds.round() as u64;
        output.push_str(&format!(
            "## {:02}:{:02}:{:02}\n\n![场景候选帧]({})\n\n",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            frame.image
        ));
        for cue in &frame.transcript {
            output.push_str(&format!("> {}\n\n", cue.text));
        }
    }
    output
}

fn render_html(manifest: &FrameManifest) -> String {
    let mut output = String::from(
        "<!doctype html>\n<html lang=\"zh-CN\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>视频场景候选帧</title>\n<style>\n:root{color-scheme:light dark;font-family:system-ui,sans-serif}body{max-width:960px;margin:2rem auto;padding:0 1rem;line-height:1.6}header{position:sticky;top:0;background:Canvas;padding:.5rem 0;border-bottom:1px solid GrayText}input{font:inherit;width:100%;padding:.5rem;box-sizing:border-box}article{margin:2rem 0;padding-bottom:2rem;border-bottom:1px solid GrayText}img{max-width:100%;height:auto;border-radius:.4rem}time{font-weight:bold}p{margin:.5rem 0}small{color:GrayText}\n</style>\n</head>\n<body>\n<header><h1>视频场景候选帧</h1><input id=\"search\" type=\"search\" placeholder=\"搜索字幕\" aria-label=\"搜索字幕\"><small>按场景变化筛选；课件动画、代码和公式增量需要人工核对。</small></header>\n<main>\n",
    );
    for (index, frame) in manifest.frames.iter().enumerate() {
        let seconds = frame.time_seconds.round() as u64;
        output.push_str(&format!(
            "<article><h2><time>{:02}:{:02}:{:02}</time></h2><img loading=\"lazy\" src=\"{}\" alt=\"场景候选帧 {}\">\n",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            escape_html(&frame.image),
            index + 1,
        ));
        for cue in &frame.transcript {
            output.push_str(&format!("<p>{}</p>\n", escape_html(&cue.text)));
        }
        output.push_str("</article>\n");
    }
    output.push_str("</main>\n<script>const search=document.querySelector('#search');search.addEventListener('input',()=>{const query=search.value.trim().toLocaleLowerCase();for(const card of document.querySelectorAll('article'))card.hidden=!!query&&!card.textContent.toLocaleLowerCase().includes(query)});</script>\n</body>\n</html>\n");
    output
}

fn escape_html(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

#[cfg(test)]
mod tests {
    use super::{FrameEntry, attach_transcript, escape_html, parse_showinfo_timestamps, parse_srt};

    #[test]
    fn parses_only_selected_frame_times() {
        let log = "[Parsed_showinfo_2 @ 0x1] n:   0 pts:      0 pts_time:0\n\
                   [Parsed_showinfo_2 @ 0x1] n:   1 pts:  20480 pts_time:2.5\n\
                   frame=    2 fps=0.0\n";
        assert_eq!(parse_showinfo_timestamps(log), vec![0.0, 2.5]);
    }

    #[test]
    fn aligns_srt_cues_to_preceding_frame() {
        let cues = parse_srt("1\r\n00:00:01,000 --> 00:00:02,000\r\nFirst line\r\nsecond line\r\n \r\n2\r\n00:00:03,250 --> 00:00:04,000\r\nNext slide\r\n").unwrap();
        let mut frames = vec![
            FrameEntry {
                time_seconds: 0.0,
                image: "a.jpg".into(),
                transcript: Vec::new(),
            },
            FrameEntry {
                time_seconds: 3.0,
                image: "b.jpg".into(),
                transcript: Vec::new(),
            },
        ];
        attach_transcript(&mut frames, cues);
        assert_eq!(frames[0].transcript[0].text, "First line second line");
        assert_eq!(frames[1].transcript[0].start_seconds, 3.25);
    }

    #[test]
    fn escapes_subtitle_text_in_html() {
        assert_eq!(
            escape_html("<img src='x'>&"),
            "&lt;img src=&#39;x&#39;&gt;&amp;"
        );
    }
}
