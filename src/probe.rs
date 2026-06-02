use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::process::Command;

use crate::state::SourceInfo;

#[derive(Deserialize)]
struct FfprobeOutput {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: String,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    r_frame_rate: Option<String>,
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
}

pub fn probe(file_path: &str) -> Result<SourceInfo> {
    let output = Command::new("ffprobe")
        .args(&[
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_streams",
            "-show_format",
            file_path,
        ])
        .output()
        .context("failed to run ffprobe")?;

    if !output.status.success() {
        return Err(anyhow!(
            "ffprobe failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }

    let parsed: FfprobeOutput =
        serde_json::from_slice(&output.stdout).context("ffprobe JSON parse failed")?;

    let video = parsed
        .streams
        .iter()
        .find(|s| s.codec_type == "video")
        .ok_or_else(|| anyhow!("no video stream found"))?;

    let width = video.width.ok_or_else(|| anyhow!("missing width"))?;
    let height = video.height.ok_or_else(|| anyhow!("missing height"))?;
    let codec = video.codec_name.clone().unwrap_or_else(|| "unknown".into());
    let fps = parse_fps(video.r_frame_rate.as_deref().unwrap_or("0/1"));

    let duration_sec = parsed
        .format
        .duration
        .ok_or_else(|| anyhow!("missing duration"))?
        .parse::<f64>()
        .context("invalid duration")?;

    Ok(SourceInfo {
        width,
        height,
        duration_sec,
        fps,
        codec,
    })
}

fn parse_fps(s: &str) -> f64 {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 2 {
        return 0.0;
    }
    let num: f64 = parts[0].parse().unwrap_or(0.0);
    let den: f64 = parts[1].parse().unwrap_or(1.0);
    if den == 0.0 {
        0.0
    } else {
        num / den
    }
}
