use anyhow::{anyhow, Context, Result};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Read};
use std::path::Path;
use std::process::{Command, Stdio};

fn ffmpeg_verbose() -> bool {
    matches!(
        std::env::var("STRATOS_FFMPEG_VERBOSE").as_deref(),
        Ok("1") | Ok("true") | Ok("TRUE")
    )
}

fn run_ffmpeg(args: &[&str], op: &str) -> Result<()> {
    tracing::debug!(op, cmd = %format!("ffmpeg {}", args.join(" ")));

    let mut cmd = Command::new("ffmpeg");
    cmd.args(args);

    if ffmpeg_verbose() {
        cmd.stderr(Stdio::inherit());
        let status = cmd.status().with_context(|| format!("failed to spawn ffmpeg ({op})"))?;
        if !status.success() {
            return Err(anyhow!("ffmpeg {op} failed (exit {:?})", status.code()));
        }
        return Ok(());
    }

    cmd.stderr(Stdio::piped());
    let output = cmd
        .output()
        .with_context(|| format!("failed to spawn ffmpeg ({op})"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        tracing::error!(op, stderr = %stderr.trim(), "ffmpeg failed");
        return Err(anyhow!("ffmpeg {op} failed: {}", stderr.trim()));
    }
    Ok(())
}

/// Encode a sample clip (for Phase A — analysis)
pub fn encode_sample(
    source: &str,
    output_mp4: &str,
    width: u32,
    height: u32,
    bitrate_kbps: u32,
    clip_start_sec: f64,
    clip_duration_sec: f64,
) -> Result<u64> {
    let scale = format!(
        "scale=w={}:h={}:force_original_aspect_ratio=decrease,\
         pad={}:{}:(ow-iw)/2:(oh-ih)/2",
        width, height, width, height
    );
    let bitrate = format!("{}k", bitrate_kbps);
    let maxrate = format!("{}k", bitrate_kbps + 100);
    let bufsize = format!("{}k", bitrate_kbps * 2);

    run_ffmpeg(
        &[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &clip_start_sec.to_string(),
            "-i",
            source,
            "-t",
            &clip_duration_sec.to_string(),
            "-vf",
            &scale,
            "-c:v",
            "libx264",
            "-preset",
            "fast",
            "-b:v",
            &bitrate,
            "-maxrate",
            &maxrate,
            "-bufsize",
            &bufsize,
            "-g",
            "48",
            "-keyint_min",
            "48",
            "-sc_threshold",
            "0",
            "-an",
            "-movflags",
            "+faststart",
            output_mp4,
        ],
        "encode_sample",
    )?;

    Ok(std::fs::metadata(output_mp4)?.len())
}

/// Extract the analysis window from the source once — shared as the VMAF reference.
pub fn extract_reference_clip(
    source: &str,
    output: &Path,
    clip_start_sec: f64,
    clip_duration_sec: f64,
) -> Result<()> {
    let output = output.to_string_lossy();
    run_ffmpeg(
        &[
            "-y",
            "-hide_banner",
            "-loglevel",
            "error",
            "-ss",
            &clip_start_sec.to_string(),
            "-i",
            source,
            "-t",
            &clip_duration_sec.to_string(),
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-crf",
            "12",
            "-g",
            "48",
            "-keyint_min",
            "48",
            "-sc_threshold",
            "0",
            "-an",
            "-movflags",
            "+faststart",
            output.as_ref(),
        ],
        "extract_reference_clip",
    )
}

/// Encode a full rendition as HLS (for Phase B — final output).
///
/// Streams ffmpeg's `-progress` output and calls `on_progress(percent)` as the encode
/// advances, so the web UI can show a live per-rendition progress bar.
pub fn encode_hls<F: FnMut(f64)>(
    source: &str,
    output_dir: &str,
    width: u32,
    height: u32,
    bitrate_kbps: u32,
    total_duration_sec: f64,
    mut on_progress: F,
) -> Result<()> {
    std::fs::create_dir_all(output_dir)?;

    let segment_pattern = format!("{}/seg%05d.ts", output_dir);
    let playlist = format!("{}/stream.m3u8", output_dir);
    let scale = format!(
        "scale=w={}:h={}:force_original_aspect_ratio=decrease,\
         pad={}:{}:(ow-iw)/2:(oh-ih)/2",
        width, height, width, height
    );
    let bitrate = format!("{}k", bitrate_kbps);
    let maxrate = format!("{}k", bitrate_kbps + 100);
    let bufsize = format!("{}k", bitrate_kbps * 2);

    let args: [&str; 36] = [
        "-y",
        "-hide_banner",
        "-loglevel",
        "error",
        "-progress",
        "pipe:1",
        "-nostats",
        "-i",
        source,
        "-vf",
        &scale,
        "-c:v",
        "libx264",
        "-preset",
        "fast",
        "-profile:v",
        "high",
        "-level",
        "4.1",
        "-b:v",
        &bitrate,
        "-maxrate",
        &maxrate,
        "-bufsize",
        &bufsize,
        "-g",
        "48",
        "-keyint_min",
        "48",
        "-sc_threshold",
        "0",
        "-c:a",
        "aac",
        "-b:a",
        "128k",
        "-ac",
    ];
    let tail = [
        "2",
        "-f",
        "hls",
        "-hls_time",
        "6",
        "-hls_playlist_type",
        "vod",
        "-hls_segment_filename",
        &segment_pattern,
        &playlist,
    ];

    tracing::debug!(op = "encode_hls", cmd = %format!("ffmpeg {} {}", args.join(" "), tail.join(" ")));

    let mut child = Command::new("ffmpeg")
        .args(args)
        .args(tail)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("failed to spawn ffmpeg (encode_hls)")?;

    if let Some(stdout) = child.stdout.take() {
        let reader = BufReader::new(stdout);
        let mut last_emitted = -1i64;
        for line in reader.lines() {
            let Ok(line) = line else { break };
            if let Some(v) = line.strip_prefix("out_time_us=") {
                if total_duration_sec > 0.0 {
                    if let Ok(us) = v.trim().parse::<i64>() {
                        let pct = (us as f64 / 1_000_000.0 / total_duration_sec * 100.0)
                            .clamp(0.0, 100.0);
                        let floored = pct.floor() as i64;
                        if floored > last_emitted {
                            last_emitted = floored;
                            on_progress(pct);
                        }
                    }
                }
            }
        }
    }

    let status = child.wait().context("ffmpeg encode_hls wait failed")?;
    if !status.success() {
        let mut err = String::new();
        if let Some(mut stderr) = child.stderr.take() {
            let _ = stderr.read_to_string(&mut err);
        }
        tracing::error!(op = "encode_hls", stderr = %err.trim(), "ffmpeg failed");
        return Err(anyhow!("ffmpeg encode_hls failed: {}", err.trim()));
    }

    Ok(())
}

#[derive(Deserialize)]
struct VmafReport {
    pooled_metrics: PooledMetrics,
}
#[derive(Deserialize)]
struct PooledMetrics {
    vmaf: VmafMetric,
}
#[derive(Deserialize)]
struct VmafMetric {
    mean: f64,
}

/// Score one encoded sample against a pre-cut reference clip — returns VMAF mean.
pub fn score_vmaf(
    distorted: &str,
    reference_clip: &str,
    width: u32,
    height: u32,
) -> Result<f64> {
    let log_path = format!("{}.vmaf.json", distorted);
    let scale = format!(
        "scale=w={}:h={}:force_original_aspect_ratio=decrease,\
         pad={}:{}:(ow-iw)/2:(oh-ih)/2",
        width, height, width, height
    );

    let filter = format!(
        "[0:v]format=yuv420p10le,setpts=PTS-STARTPTS,settb=AVTB[dist];\
         [1:v]{scale},format=yuv420p10le,setpts=PTS-STARTPTS,settb=AVTB[ref];\
         [dist][ref]libvmaf=shortest=true:ts_sync_mode=nearest:\
         n_threads=4:log_path={log_path}:log_fmt=json",
        log_path = log_path
    );

    run_ffmpeg(
        &[
            "-hide_banner",
            "-loglevel",
            "error",
            "-i",
            distorted,
            "-i",
            reference_clip,
            "-filter_complex",
            &filter,
            "-an",
            "-sn",
            "-dn",
            "-f",
            "null",
            "-",
        ],
        "score_vmaf",
    )?;

    let json = std::fs::read_to_string(&log_path)?;
    let report: VmafReport = serde_json::from_str(&json).context("vmaf json parse failed")?;
    let _ = std::fs::remove_file(&log_path);

    Ok(report.pooled_metrics.vmaf.mean)
}

/// Write the master playlist that references all chosen renditions
pub fn write_master_playlist(
    output_dir: &str,
    renditions: &[(String, u32, u32, u32)],
) -> Result<()> {
    let mut content = String::from("#EXTM3U\n#EXT-X-VERSION:6\n\n");

    for (name, width, height, bitrate_kbps) in renditions {
        let bandwidth = (bitrate_kbps + 128) * 1000;
        content.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},AVERAGE-BANDWIDTH={},RESOLUTION={}x{},\
             CODECS=\"avc1.640028,mp4a.40.2\",NAME=\"{}\"\n{}/stream.m3u8\n\n",
            bandwidth, bandwidth, width, height, name, name
        ));
    }

    let path = Path::new(output_dir).join("master.m3u8");
    std::fs::write(path, content)?;
    Ok(())
}

/// Verify ffmpeg/ffprobe exist and libvmaf is available.
pub fn check_dependencies() -> Result<()> {
    for bin in ["ffmpeg", "ffprobe"] {
        let out = Command::new(bin)
            .arg("-version")
            .output()
            .with_context(|| format!("`{bin}` not found on PATH"))?;
        if !out.status.success() {
            return Err(anyhow!("`{bin} -version` failed"));
        }
        let ver = String::from_utf8_lossy(&out.stdout);
        let first = ver.lines().next().unwrap_or(bin);
        tracing::info!(tool = bin, version = %first, "ok");
    }

    let filters = Command::new("ffmpeg")
        .args(["-hide_banner", "-filters"])
        .output()
        .context("ffmpeg -filters failed")?;
    let text = String::from_utf8_lossy(&filters.stdout);
    if !text.contains("libvmaf") {
        return Err(anyhow!(
            "ffmpeg is missing libvmaf — install ffmpeg with VMAF support (e.g. brew install ffmpeg)"
        ));
    }
    tracing::info!("libvmaf filter available");
    Ok(())
}
