use anyhow::{Context, Result};
use rayon::prelude::*;
use std::path::Path;
use std::time::Instant;

use crate::ffmpeg::{
    encode_hls, encode_sample, extract_reference_clip, score_vmaf, write_master_playlist,
};
use crate::insights;
use crate::optimizer::{choose_per_title_ladder, pareto_frontier};
use crate::probe;
use crate::state::*;

const CLIP_DURATION_SEC: f64 = 30.0;

/// Deliver the lowest bitrate that still reaches this VMAF (per-title target).
pub const TARGET_VMAF: f64 = 93.0;

struct AnalysisCell {
    index: usize,
    resolution: String,
    width: u32,
    height: u32,
    bitrate_kbps: u32,
}

/// End-to-end job: probe → parallel VMAF grid → optimize → parallel HLS encodes.
pub fn run_pipeline(state: SharedState, job_id: String) -> Result<()> {
    let started = Instant::now();
    let (source_path, output_dir) = {
        let job = state
            .jobs
            .get(&job_id)
            .context("job not found")?;
        (job.source_path.clone(), job.output_dir.clone())
    };

    tracing::info!(
        job_id = %job_id,
        source = %source_path,
        output = %output_dir,
        "pipeline started"
    );

    std::fs::create_dir_all(&output_dir)?;

    // ── Probe ──────────────────────────────────────────────────────────────
    set_status(&state, &job_id, JobStatus::Probing);
    let t0 = Instant::now();
    let info = probe::probe(&source_path)?;
    tracing::info!(
        job_id = %job_id,
        width = info.width,
        height = info.height,
        duration_sec = info.duration_sec,
        fps = info.fps,
        codec = %info.codec,
        elapsed_ms = t0.elapsed().as_millis(),
        "probe complete"
    );
    state.jobs.alter(&job_id, |_, mut j| {
        j.source_info = Some(info.clone());
        j
    });

    let clip_start = clip_start_sec(info.duration_sec);
    let clip_duration = clip_duration_sec(info.duration_sec);
    let grid = build_analysis_grid(info.height);
    tracing::info!(
        job_id = %job_id,
        tiers = grid.len(),
        clip_start_sec = clip_start,
        clip_duration_sec = clip_duration,
        "analysis grid"
    );
    for cell in &grid {
        tracing::info!(
            job_id = %job_id,
            tier = %cell.resolution,
            bitrate_kbps = cell.bitrate_kbps,
            "planned analysis cell"
        );
    }

    // ── Phase A: parallel sample encodes + VMAF ────────────────────────────
    set_status(&state, &job_id, JobStatus::Analyzing);
    let analysis_dir = Path::new(&output_dir).join("analysis");
    std::fs::create_dir_all(&analysis_dir)?;

    let t_ref = Instant::now();
    let ref_clip = analysis_dir.join("reference_clip.mp4");
    extract_reference_clip(&source_path, &ref_clip, clip_start, clip_duration)?;
    tracing::info!(
        job_id = %job_id,
        path = %ref_clip.display(),
        elapsed_ms = t_ref.elapsed().as_millis(),
        "reference clip ready"
    );
    let ref_clip = ref_clip.to_string_lossy().into_owned();

    let st = state.clone();
    let jid = job_id.clone();
    let src = source_path.clone();
    let adir = analysis_dir.clone();

    let points: Vec<DataPoint> = grid
        .par_iter()
        .map(|cell| {
            let t_cell = Instant::now();
            tracing::info!(
                job_id = %jid,
                tier = %cell.resolution,
                bitrate_kbps = cell.bitrate_kbps,
                "analysis encode start"
            );

            let out = adir.join(format!(
                "{:02}_{}_{}k.mp4",
                cell.index, cell.resolution, cell.bitrate_kbps
            ));
            let out_str = out.to_string_lossy().to_string();

            let size = encode_sample(
                &src,
                &out_str,
                cell.width,
                cell.height,
                cell.bitrate_kbps,
                clip_start,
                clip_duration,
            )?;

            let vmaf = score_vmaf(&out_str, &ref_clip, cell.width, cell.height)?;
            let _ = std::fs::remove_file(&out);

            let point = DataPoint {
                resolution: cell.resolution.clone(),
                width: cell.width,
                height: cell.height,
                bitrate_kbps: cell.bitrate_kbps,
                vmaf_mean: vmaf,
                file_size_bytes: size,
            };

            tracing::info!(
                job_id = %jid,
                tier = %cell.resolution,
                vmaf = point.vmaf_mean,
                bitrate_kbps = point.bitrate_kbps,
                elapsed_ms = t_cell.elapsed().as_millis(),
                "analysis complete"
            );

            st.jobs.alter(&jid, |_, mut j| {
                j.analysis_points.push(point.clone());
                j
            });
            st.publish(&jid, JobEvent::AnalysisPoint { point: point.clone() });

            Ok(point)
        })
        .collect::<Result<Vec<_>>>()?;

    // ── Optimize ───────────────────────────────────────────────────────────
    set_status(&state, &job_id, JobStatus::Optimizing);
    let frontier = pareto_frontier(&points);
    let ladder = choose_per_title_ladder(&points, TARGET_VMAF);

    tracing::info!(
        job_id = %job_id,
        sampled_points = points.len(),
        pareto_points = frontier.len(),
        ladder_rungs = ladder.len(),
        target_vmaf = TARGET_VMAF,
        "per-title ladder selected"
    );
    for r in &ladder {
        tracing::info!(
            job_id = %job_id,
            tier = %r.name,
            width = r.width,
            height = r.height,
            bitrate_kbps = r.bitrate_kbps,
            vmaf = r.vmaf_mean,
            "ladder rung"
        );
    }

    state.jobs.alter(&job_id, |_, mut j| {
        j.chosen_ladder = ladder.clone();
        j
    });
    state.publish(
        &job_id,
        JobEvent::LadderChosen {
            ladder: ladder.clone(),
        },
    );

    let title_insights = insights::compute(&info, &points, &ladder);
    state.jobs.alter(&job_id, |_, mut j| {
        j.insights = Some(title_insights.clone());
        j
    });
    state.publish(
        &job_id,
        JobEvent::InsightsReady {
            insights: title_insights,
        },
    );

    // ── Phase B: parallel full HLS encodes ─────────────────────────────────
    set_status(&state, &job_id, JobStatus::Encoding);

    let st = state.clone();
    let jid = job_id.clone();
    let src = source_path.clone();
    let out_root = output_dir.clone();

    let total_duration = info.duration_sec;
    ladder.par_iter().try_for_each(|r| {
        let t = Instant::now();
        tracing::info!(
            job_id = %jid,
            tier = %r.name,
            bitrate_kbps = r.bitrate_kbps,
            "HLS encode start"
        );
        let dir = Path::new(&out_root).join(&r.name);
        let rendition = r.name.clone();
        encode_hls(
            &src,
            &dir.to_string_lossy(),
            r.width,
            r.height,
            r.bitrate_kbps,
            total_duration,
            |percent| {
                st.publish(
                    &jid,
                    JobEvent::EncodeProgress {
                        rendition: rendition.clone(),
                        percent,
                    },
                );
            },
        )?;
        tracing::info!(
            job_id = %jid,
            tier = %r.name,
            elapsed_ms = t.elapsed().as_millis(),
            "HLS encode complete"
        );
        st.publish(
            &jid,
            JobEvent::EncodeProgress {
                rendition: r.name.clone(),
                percent: 100.0,
            },
        );
        Ok::<_, anyhow::Error>(())
    })?;

    let master_rows: Vec<(String, u32, u32, u32)> = ladder
        .iter()
        .map(|r| (r.name.clone(), r.width, r.height, r.bitrate_kbps))
        .collect();
    write_master_playlist(&output_dir, &master_rows)?;
    tracing::info!(job_id = %job_id, path = %format!("{}/master.m3u8", output_dir), "master playlist written");

    set_status(&state, &job_id, JobStatus::Done);
    state.publish(&job_id, JobEvent::Done);

    tracing::info!(
        job_id = %job_id,
        total_ms = started.elapsed().as_millis(),
        "pipeline finished"
    );

    Ok(())
}

fn set_status(state: &SharedState, job_id: &str, status: JobStatus) {
    tracing::info!(job_id = %job_id, ?status, "status");
    state.jobs.alter(job_id, |_, mut j| {
        j.status = status.clone();
        j
    });
    state.publish(
        job_id,
        JobEvent::StatusChanged {
            status: status.clone(),
        },
    );
}

fn clip_start_sec(duration_sec: f64) -> f64 {
    let clip = clip_duration_sec(duration_sec);
    if duration_sec <= clip {
        0.0
    } else {
        (duration_sec - clip) / 2.0
    }
}

fn clip_duration_sec(duration_sec: f64) -> f64 {
    duration_sec.min(CLIP_DURATION_SEC)
}

/// Rate–quality grid: each standard tier (≤ source height) × several candidate bitrates.
/// We sweep below and around a typical fixed-ladder bitrate so the search can find the
/// lowest bitrate that still hits the VMAF target.
fn build_analysis_grid(source_height: u32) -> Vec<AnalysisCell> {
    let tiers: [(&str, u32, u32); 4] = [
        ("360p", 640, 360),
        ("480p", 854, 480),
        ("720p", 1280, 720),
        ("1080p", 1920, 1080),
    ];

    let mut resolutions: Vec<_> = tiers
        .iter()
        .filter(|(_, _, h)| *h <= source_height)
        .map(|(name, w, h)| (name.to_string(), *w, *h))
        .collect();

    if resolutions.is_empty() {
        let h = source_height.max(144);
        let w = (h as f64 * 16.0 / 9.0).round() as u32;
        resolutions.push((format!("{h}p"), w, h));
    }

    let mut grid = Vec::new();
    let mut index = 0;
    for (name, w, h) in resolutions {
        for &bitrate_kbps in candidate_bitrates(h) {
            grid.push(AnalysisCell {
                index,
                resolution: name.clone(),
                width: w,
                height: h,
                bitrate_kbps,
            });
            index += 1;
        }
    }
    grid
}

/// Candidate bitrates per resolution tier (kbps) — sampled to build the rate–quality curve.
fn candidate_bitrates(height: u32) -> &'static [u32] {
    match height {
        0..=360 => &[200, 400, 600, 900],
        361..=480 => &[400, 700, 1_100, 1_600],
        481..=720 => &[900, 1_500, 2_200, 3_200],
        _ => &[1_600, 2_800, 4_000, 5_500],
    }
}
