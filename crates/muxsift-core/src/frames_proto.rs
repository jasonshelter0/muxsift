//! Experimental visual frame selection. This module does not change the stable `frames` path.

use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    process::Stdio,
};

use img_hash::{HashAlg, HasherConfig, ImageHash, image};
use serde::Serialize;
use tokio::process::Command;

use crate::{Error, Result};

#[derive(Debug, Clone)]
pub struct FramePrototypeOptions {
    pub input: PathBuf,
    pub output_dir: PathBuf,
    pub ffmpeg: PathBuf,
    pub sample_interval_seconds: f64,
    /// FFmpeg scdet score (0–100), not the select filter's scene score.
    pub content_threshold: f64,
    pub adaptive_threshold: f64,
    pub adaptive_window: usize,
    pub min_content: f64,
    pub phash_duplicate_distance: u32,
    pub histogram_duplicate_distance: f64,
    pub spatial_duplicate_distance: f64,
    pub min_gap_seconds: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct FrameDecision {
    pub time_seconds: f64,
    pub selected: bool,
    pub reason: &'static str,
    /// Larger means the content or adaptive threshold was exceeded by a larger factor.
    pub score: f64,
    pub content_score: f64,
    pub rolling_mean: f64,
    pub adaptive_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phash_distance: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub histogram_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spatial_change: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub duplicate_of_seconds: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct PrototypeFrame {
    pub time_seconds: f64,
    pub image: String,
    pub reason: &'static str,
    pub score: f64,
    pub content_score: f64,
    pub adaptive_ratio: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub phash_distance: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub histogram_distance: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub spatial_change: Option<f64>,
}

#[derive(Debug, Serialize)]
pub struct FramePrototype {
    pub source: String,
    pub method: &'static str,
    pub sample_interval_seconds: f64,
    pub content_threshold: f64,
    pub adaptive_threshold: f64,
    pub adaptive_window: usize,
    pub min_content: f64,
    pub phash_duplicate_distance: u32,
    pub histogram_duplicate_distance: f64,
    pub spatial_duplicate_distance: f64,
    pub min_gap_seconds: f64,
    pub frames: Vec<PrototypeFrame>,
    pub decisions: Vec<FrameDecision>,
}

#[derive(Debug)]
struct Sample {
    time_seconds: f64,
    content_score: f64,
}

#[derive(Debug)]
struct VisualSignature {
    time_seconds: f64,
    hash: ImageHash,
    histogram: [f64; 24],
    grayscale: Vec<u8>,
}

/// A future OCR backend can return normalized text change for two selected candidates.
/// The first prototype deliberately has no OCR dependency or OCR gate.
#[cfg(feature = "ocr")]
pub trait OcrDiff {
    fn text_change(&self, previous: &Path, candidate: &Path) -> Result<f64>;
}

pub async fn extract_frame_prototype(options: &FramePrototypeOptions) -> Result<FramePrototype> {
    validate(options)?;
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
        .prefix(".muxsift-frames-proto-")
        .tempdir_in(parent)
        .map_err(|source| Error::Io {
            path: parent.to_owned(),
            source,
        })?;
    let candidate_dir = temporary.path().join("candidates");
    let frame_dir = temporary.path().join("frames");
    fs::create_dir(&candidate_dir).map_err(|source| Error::Io {
        path: candidate_dir.clone(),
        source,
    })?;
    fs::create_dir(&frame_dir).map_err(|source| Error::Io {
        path: frame_dir.clone(),
        source,
    })?;
    let score_path = temporary.path().join("scdet.log");
    // t=0 makes scdet attach a score to every sampled frame. It does not select frames.
    let filter = format!(
        "fps={:.8},scale=960:-2:force_original_aspect_ratio=decrease,scdet=t=0,metadata=mode=print:file={}",
        1.0 / options.sample_interval_seconds,
        score_path.display(),
    );
    let output = Command::new(&options.ffmpeg)
        .kill_on_drop(true)
        .args(["-hide_banner", "-nostdin", "-nostats", "-loglevel", "error"])
        .arg("-i")
        .arg(&options.input)
        .args(["-map", "0:v:0", "-an", "-vf"])
        .arg(filter)
        .args(["-fps_mode", "vfr", "-q:v", "4", "-y"])
        .arg(candidate_dir.join("candidate_%06d.jpg"))
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
    let scores = fs::read_to_string(&score_path).map_err(|source| Error::Io {
        path: score_path.clone(),
        source,
    })?;
    let samples = parse_scdet_scores(&scores)?;
    let mut images = fs::read_dir(&candidate_dir)
        .map_err(|source| Error::Io {
            path: candidate_dir.clone(),
            source,
        })?
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<std::io::Result<Vec<_>>>()
        .map_err(|source| Error::Io {
            path: candidate_dir.clone(),
            source,
        })?;
    images.sort();
    if images.is_empty() || samples.len() != images.len() {
        return Err(Error::InvalidInput(format!(
            "FFmpeg produced {} candidate images but {} scores; no result was saved",
            images.len(),
            samples.len()
        )));
    }

    let hasher = HasherConfig::new()
        .hash_size(8, 8)
        .hash_alg(HashAlg::Mean)
        .preproc_dct()
        .to_hasher();
    let mut history = VecDeque::with_capacity(options.adaptive_window);
    let mut recent: VecDeque<VisualSignature> = VecDeque::with_capacity(8);
    let mut frames = Vec::new();
    let mut decisions = Vec::with_capacity(samples.len());
    for (sample, path) in samples.into_iter().zip(images) {
        let rolling_mean = if history.is_empty() {
            0.0
        } else {
            history.iter().sum::<f64>() / history.len() as f64
        };
        let adaptive_ratio = sample.content_score / rolling_mean.max(0.35);
        let score = (sample.content_score / options.content_threshold)
            .max(adaptive_ratio / options.adaptive_threshold);
        let fixed = sample.content_score >= options.content_threshold;
        let adaptive = sample.content_score >= options.min_content
            && adaptive_ratio >= options.adaptive_threshold;
        let mut nearest = None;
        let mut duplicate_match = None;
        let gap = frames
            .last()
            .map(|frame: &PrototypeFrame| sample.time_seconds - frame.time_seconds)
            .unwrap_or(f64::INFINITY);
        let preliminary = if frames.is_empty() {
            "first_frame"
        } else if !fixed && !adaptive {
            "low_change"
        } else if gap + 1e-6 < options.min_gap_seconds {
            "min_gap"
        } else if fixed && adaptive {
            "content_and_adaptive"
        } else if fixed {
            "content"
        } else {
            "adaptive"
        };
        // Most samples are quiet. Decode only a candidate which passed score and gap gates.
        let signature = if matches!(preliminary, "low_change" | "min_gap") {
            None
        } else {
            let visual = image::open(&path).map_err(|error| {
                Error::InvalidInput(format!(
                    "cannot decode candidate {}: {error}",
                    path.display()
                ))
            })?;
            let signature = VisualSignature {
                time_seconds: sample.time_seconds,
                hash: hasher.hash_image(&visual),
                histogram: color_histogram(&visual),
                grayscale: visual
                    .resize_exact(160, 90, image::imageops::FilterType::Triangle)
                    .to_luma8()
                    .into_raw(),
            };
            for previous in &recent {
                let hash_distance = signature.hash.dist(&previous.hash);
                let hist_distance = histogram_distance(&signature.histogram, &previous.histogram);
                let spatial_change = spatial_change(&signature.grayscale, &previous.grayscale);
                if nearest
                    .is_none_or(|(current, _, _, _): (u32, f64, f64, f64)| hash_distance < current)
                {
                    nearest = Some((
                        hash_distance,
                        hist_distance,
                        spatial_change,
                        previous.time_seconds,
                    ));
                }
                if hash_distance <= options.phash_duplicate_distance
                    && hist_distance <= options.histogram_duplicate_distance
                    && spatial_change <= options.spatial_duplicate_distance
                {
                    duplicate_match = Some((
                        hash_distance,
                        hist_distance,
                        spatial_change,
                        previous.time_seconds,
                    ));
                }
            }
            Some(signature)
        };
        let nearest = duplicate_match.or(nearest);
        let reason = if duplicate_match.is_some() {
            "near_duplicate"
        } else {
            preliminary
        };
        let selected = matches!(
            reason,
            "first_frame" | "content_and_adaptive" | "content" | "adaptive"
        );
        let decision = FrameDecision {
            time_seconds: sample.time_seconds,
            selected,
            reason,
            score,
            content_score: sample.content_score,
            rolling_mean,
            adaptive_ratio,
            phash_distance: nearest.map(|pair| pair.0),
            histogram_distance: nearest.map(|pair| pair.1),
            spatial_change: nearest.map(|pair| pair.2),
            duplicate_of_seconds: duplicate_match.map(|pair| pair.3),
        };
        if selected {
            let image_name = format!("frame_{:06}.jpg", frames.len() + 1);
            fs::rename(&path, frame_dir.join(&image_name)).map_err(|source| Error::Io {
                path: path.clone(),
                source,
            })?;
            frames.push(PrototypeFrame {
                time_seconds: sample.time_seconds,
                image: format!("frames/{image_name}"),
                reason,
                score,
                content_score: sample.content_score,
                adaptive_ratio,
                phash_distance: decision.phash_distance,
                histogram_distance: decision.histogram_distance,
                spatial_change: decision.spatial_change,
            });
            recent.push_back(signature.expect("selected frame has a visual signature"));
            if recent.len() > 8 {
                recent.pop_front();
            }
        }
        decisions.push(decision);
        history.push_back(sample.content_score);
        if history.len() > options.adaptive_window {
            history.pop_front();
        }
    }
    let manifest = FramePrototype {
        source: options.input.to_string_lossy().into_owned(),
        method: "ffmpeg_scdet_adaptive_phash_histogram_v1",
        sample_interval_seconds: options.sample_interval_seconds,
        content_threshold: options.content_threshold,
        adaptive_threshold: options.adaptive_threshold,
        adaptive_window: options.adaptive_window,
        min_content: options.min_content,
        phash_duplicate_distance: options.phash_duplicate_distance,
        histogram_duplicate_distance: options.histogram_duplicate_distance,
        spatial_duplicate_distance: options.spatial_duplicate_distance,
        min_gap_seconds: options.min_gap_seconds,
        frames,
        decisions,
    };
    let json = serde_json::to_vec_pretty(&manifest)?;
    fs::write(temporary.path().join("timeline.json"), json).map_err(|source| Error::Io {
        path: temporary.path().join("timeline.json"),
        source,
    })?;
    let markdown = render_markdown(&manifest);
    fs::write(temporary.path().join("frames.md"), markdown).map_err(|source| Error::Io {
        path: temporary.path().join("frames.md"),
        source,
    })?;
    fs::remove_dir_all(&candidate_dir).map_err(|source| Error::Io {
        path: candidate_dir,
        source,
    })?;
    fs::remove_file(&score_path).map_err(|source| Error::Io {
        path: score_path,
        source,
    })?;
    fs::rename(temporary.path(), &options.output_dir).map_err(|source| Error::Io {
        path: options.output_dir.clone(),
        source,
    })?;
    Ok(manifest)
}

fn validate(options: &FramePrototypeOptions) -> Result<()> {
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
            "interval must be 0.2–60 seconds".into(),
        ));
    }
    if !options.content_threshold.is_finite()
        || !(0.01..=100.0).contains(&options.content_threshold)
        || !options.adaptive_threshold.is_finite()
        || !(1.0..=100.0).contains(&options.adaptive_threshold)
        || !(2..=120).contains(&options.adaptive_window)
        || !options.min_content.is_finite()
        || !(0.0..=100.0).contains(&options.min_content)
        || options.phash_duplicate_distance > 64
        || !options.histogram_duplicate_distance.is_finite()
        || !(0.0..=1.0).contains(&options.histogram_duplicate_distance)
        || !options.spatial_duplicate_distance.is_finite()
        || !(0.0..=1.0).contains(&options.spatial_duplicate_distance)
        || !options.min_gap_seconds.is_finite()
        || !(0.0..=600.0).contains(&options.min_gap_seconds)
    {
        return Err(Error::InvalidInput(
            "invalid frames-proto thresholds; see frames-proto --help".into(),
        ));
    }
    Ok(())
}

fn parse_scdet_scores(log: &str) -> Result<Vec<Sample>> {
    let mut samples = Vec::new();
    let mut pending_time = None;
    for line in log.lines() {
        if line.starts_with("frame:") {
            if pending_time.is_some() {
                return Err(Error::InvalidInput(
                    "scdet score missing for a frame".into(),
                ));
            }
            pending_time = line
                .split_once("pts_time:")
                .and_then(|(_, value)| value.trim().parse::<f64>().ok());
        } else if let Some(value) = line.strip_prefix("lavfi.scd.score=") {
            let Some(time_seconds) = pending_time.take() else {
                return Err(Error::InvalidInput("scdet score has no timestamp".into()));
            };
            let content_score = value
                .trim()
                .parse::<f64>()
                .map_err(|_| Error::InvalidInput(format!("invalid scdet score: {value}")))?;
            if !time_seconds.is_finite()
                || time_seconds < 0.0
                || !content_score.is_finite()
                || content_score < 0.0
            {
                return Err(Error::InvalidInput(
                    "invalid scdet timestamp or score".into(),
                ));
            }
            samples.push(Sample {
                time_seconds,
                content_score,
            });
        }
    }
    if pending_time.is_some() {
        return Err(Error::InvalidInput(
            "scdet score missing for final frame".into(),
        ));
    }
    Ok(samples)
}

fn color_histogram(image: &image::DynamicImage) -> [f64; 24] {
    let small = image.thumbnail_exact(64, 64).to_rgb8();
    let mut bins = [0.0; 24];
    for pixel in small.pixels() {
        for channel in 0..3 {
            bins[channel * 8 + pixel[channel] as usize / 32] += 1.0;
        }
    }
    let total = (small.width() * small.height()) as f64;
    for bin in &mut bins {
        *bin /= total;
    }
    bins
}

fn histogram_distance(left: &[f64; 24], right: &[f64; 24]) -> f64 {
    left.iter()
        .zip(right)
        .map(|(a, b)| (a - b).abs())
        .sum::<f64>()
        / 6.0
}

fn spatial_change(left: &[u8], right: &[u8]) -> f64 {
    left.iter()
        .zip(right)
        .filter(|(a, b)| a.abs_diff(**b) > 20)
        .count() as f64
        / left.len() as f64
}

fn render_markdown(manifest: &FramePrototype) -> String {
    let mut output = String::from(
        "# 场景候选帧实验报告\n\n本结果只做视觉检测；请人工核对知识画面。详细筛选过程见 `timeline.json` 的 `decisions`。\n\n",
    );
    for frame in &manifest.frames {
        let seconds = frame.time_seconds.round() as u64;
        output.push_str(&format!(
            "## {:02}:{:02}:{:02} · {}\n\n![候选帧]({})\n\nscore {:.2} · scdet {:.2} · adaptive {:.2} · pHash {:?} · histogram {:?} · spatial {:?}\n\n",
            seconds / 3600,
            seconds / 60 % 60,
            seconds % 60,
            frame.reason,
            frame.image,
            frame.score,
            frame.content_score,
            frame.adaptive_ratio,
            frame.phash_distance,
            frame.histogram_distance,
            frame.spatial_change,
        ));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::parse_scdet_scores;

    #[test]
    fn parses_scdet_metadata_for_every_frame() {
        let log = "frame:0 pts:0 pts_time:0\nlavfi.scd.mafd=0.000\nlavfi.scd.score=0.000\nframe:1 pts:1 pts_time:1.5\nlavfi.scd.score=2.027\n";
        let samples = parse_scdet_scores(log).unwrap();
        assert_eq!(samples.len(), 2);
        assert_eq!(samples[1].time_seconds, 1.5);
        assert_eq!(samples[1].content_score, 2.027);
    }
}
