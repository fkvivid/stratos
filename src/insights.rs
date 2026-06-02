use crate::state::{DataPoint, Rendition, SourceInfo, TitleInsights, RungComparison};

/// Industry-style fixed ladder (same for every title) — baseline for per-title comparison.
/// Bitrates aligned with common OTT presets (similar to Netflix pre–per-title catalogs).
const REFERENCE_LADDER: [(&str, u32, u32, u32); 4] = [
    ("360p", 640, 360, 600),
    ("480p", 854, 480, 1_000),
    ("720p", 1280, 720, 2_400),
    ("1080p", 1920, 1080, 4_500),
];

const AUDIO_KBPS: u32 = 128;

pub fn compute(
    source: &SourceInfo,
    analysis_points: &[DataPoint],
    chosen: &[Rendition],
) -> TitleInsights {
    let reference_ladder = reference_ladder_for_source(source.height);
    let rung_comparisons = build_rung_comparisons(&reference_ladder, chosen, analysis_points);

    let reference_avg = avg_video_bitrate(&reference_ladder);
    let optimized_avg = avg_video_bitrate(chosen);
    // Signed: positive = per-title saves bits; negative = harder content needed more.
    let bandwidth_savings_pct = if reference_avg > 0 {
        (reference_avg as f64 - optimized_avg as f64) / reference_avg as f64 * 100.0
    } else {
        0.0
    };

    let mean_vmaf_optimized = mean_vmaf(chosen);
    let mean_vmaf_reference_est = rung_comparisons
        .iter()
        .map(|r| r.vmaf_reference_est)
        .sum::<f64>()
        / rung_comparisons.len().max(1) as f64;

    let title_hours = source.duration_sec / 3600.0;
    let delivery_title_reference_mb = delivery_mb_per_hour(reference_avg) * title_hours;
    let delivery_title_optimized_mb = delivery_mb_per_hour(optimized_avg) * title_hours;

    let analysis_passes = analysis_points.len() as u32;
    let final_renditions = chosen.len() as u32;
    let reference_renditions = reference_ladder.len() as u32;

    let clip_factor = (30.0f64 / source.duration_sec.max(1.0)).min(1.0);
    let per_title_analysis_cost = analysis_passes as f64 * clip_factor;
    let fixed_encode_cost = reference_renditions as f64;
    let per_title_total_cost = per_title_analysis_cost + final_renditions as f64;
    let per_title_overhead_factor =
        (per_title_total_cost / fixed_encode_cost.max(1.0) - 1.0) * 100.0;

    let (advantages, disadvantages) = build_pros_cons(
        bandwidth_savings_pct,
        mean_vmaf_optimized,
        mean_vmaf_reference_est,
        per_title_overhead_factor,
        analysis_passes,
    );

    TitleInsights {
        reference_ladder,
        reference_avg_bitrate_kbps: reference_avg,
        optimized_avg_bitrate_kbps: optimized_avg,
        bandwidth_savings_pct,
        mean_vmaf_optimized,
        mean_vmaf_reference_est,
        delivery_per_hour_reference_mb: delivery_mb_per_hour(reference_avg),
        delivery_per_hour_optimized_mb: delivery_mb_per_hour(optimized_avg),
        delivery_title_reference_mb,
        delivery_title_optimized_mb,
        encode_analysis_passes: analysis_passes,
        encode_final_renditions: final_renditions,
        encode_reference_renditions: reference_renditions,
        per_title_overhead_pct: per_title_overhead_factor,
        rung_comparisons,
        advantages,
        disadvantages,
    }
}

/// Fixed reference ladder for every tier up to the source height (1:1 with per-title rungs).
fn reference_ladder_for_source(source_height: u32) -> Vec<Rendition> {
    let mut tiers: Vec<Rendition> = REFERENCE_LADDER
        .iter()
        .filter(|(_, _, h, _)| *h <= source_height)
        .map(|(name, w, h, br)| Rendition {
            name: (*name).into(),
            width: *w,
            height: *h,
            bitrate_kbps: *br,
            vmaf_mean: 0.0,
        })
        .collect();

    if tiers.is_empty() {
        let h = source_height.max(144);
        let w = (h as f64 * 16.0 / 9.0).round() as u32;
        tiers.push(Rendition {
            name: format!("{h}p"),
            width: w,
            height: h,
            bitrate_kbps: 800,
            vmaf_mean: 0.0,
        });
    }

    tiers
}

fn build_rung_comparisons(
    reference: &[Rendition],
    chosen: &[Rendition],
    analysis: &[DataPoint],
) -> Vec<RungComparison> {
    chosen
        .iter()
        .map(|opt| {
            let ref_rung = reference
                .iter()
                .find(|r| r.height == opt.height)
                .or_else(|| reference.iter().find(|r| r.name == opt.name))
                .cloned()
                .unwrap_or_else(|| Rendition {
                    name: opt.name.clone(),
                    width: opt.width,
                    height: opt.height,
                    bitrate_kbps: opt.bitrate_kbps + 400,
                    vmaf_mean: 0.0,
                });

            let vmaf_opt = opt.vmaf_mean;
            let vmaf_ref_est = estimate_vmaf_at_bitrate(
                analysis,
                opt.height,
                ref_rung.bitrate_kbps,
                vmaf_opt,
            );

            // Signed saving: + means per-title uses fewer kbps than the fixed ladder.
            let saved = if ref_rung.bitrate_kbps > 0 {
                (ref_rung.bitrate_kbps as f64 - opt.bitrate_kbps as f64)
                    / ref_rung.bitrate_kbps as f64
                    * 100.0
            } else {
                0.0
            };

            RungComparison {
                resolution: opt.name.clone(),
                reference_bitrate_kbps: ref_rung.bitrate_kbps,
                optimized_bitrate_kbps: opt.bitrate_kbps,
                bitrate_saved_pct: saved,
                vmaf_optimized: vmaf_opt,
                vmaf_reference_est: vmaf_ref_est,
            }
        })
        .collect()
}

/// Estimate VMAF at `target_br` for a resolution by interpolating the sampled
/// rate–quality points (VMAF ~ linear in log(bitrate)).
fn estimate_vmaf_at_bitrate(
    analysis: &[DataPoint],
    height: u32,
    target_br: u32,
    fallback: f64,
) -> f64 {
    let mut pts: Vec<&DataPoint> = analysis
        .iter()
        .filter(|p| p.height == height && p.bitrate_kbps > 0)
        .collect();
    if pts.is_empty() || target_br == 0 {
        return fallback;
    }
    pts.sort_by_key(|p| p.bitrate_kbps);

    let target = (target_br as f64).ln();

    // Clamp to the sampled range.
    if target_br <= pts[0].bitrate_kbps {
        return pts[0].vmaf_mean;
    }
    if target_br >= pts[pts.len() - 1].bitrate_kbps {
        return pts[pts.len() - 1].vmaf_mean;
    }

    // Linear interpolation between the bracketing samples on a log-bitrate axis.
    for w in pts.windows(2) {
        let (a, b) = (w[0], w[1]);
        if target_br >= a.bitrate_kbps && target_br <= b.bitrate_kbps {
            let la = (a.bitrate_kbps as f64).ln();
            let lb = (b.bitrate_kbps as f64).ln();
            let t = if (lb - la).abs() < f64::EPSILON {
                0.0
            } else {
                (target - la) / (lb - la)
            };
            return (a.vmaf_mean + t * (b.vmaf_mean - a.vmaf_mean)).clamp(0.0, 100.0);
        }
    }
    fallback
}

fn avg_video_bitrate(rungs: &[Rendition]) -> u32 {
    if rungs.is_empty() {
        return 0;
    }
    rungs.iter().map(|r| r.bitrate_kbps).sum::<u32>() / rungs.len() as u32
}

fn mean_vmaf(rungs: &[Rendition]) -> f64 {
    if rungs.is_empty() {
        return 0.0;
    }
    rungs.iter().map(|r| r.vmaf_mean).sum::<f64>() / rungs.len() as f64
}

/// MB per hour of playback if a viewer watched at the average ladder bitrate (video + audio).
fn delivery_mb_per_hour(avg_video_kbps: u32) -> f64 {
    let total_kbps = avg_video_kbps + AUDIO_KBPS;
    total_kbps as f64 * 3600.0 / 8.0 / 1024.0
}

fn build_pros_cons(
    savings_pct: f64,
    vmaf_opt: f64,
    vmaf_ref: f64,
    overhead_pct: f64,
    analysis_passes: u32,
) -> (Vec<String>, Vec<String>) {
    let mut advantages = vec![];
    if savings_pct >= 0.0 {
        advantages.push(format!(
            "~{:.0}% lower average ladder bitrate vs a fixed industry preset",
            savings_pct
        ));
    } else {
        advantages.push(
            "Bitrate raised where this title needs it — quality protected over raw savings".into(),
        );
    }
    advantages.push("Bitrate matched to this title's complexity via VMAF rate–quality search".into());
    advantages.push("Lowest bitrate that still clears the VMAF target per resolution".into());
    advantages.push(
        "HLS ladder tuned per title — aligns with Netflix per-title optimization (2015)".into(),
    );

    if vmaf_opt >= vmaf_ref - 1.0 {
        advantages.push(format!(
            "Mean VMAF {:.1} on chosen rungs vs ~{:.1} estimated at fixed preset bitrates",
            vmaf_opt, vmaf_ref
        ));
    }

    let disadvantages = vec![
        format!(
            "~{:.0}% more encoder work up front ({} analysis encodes + final renditions)",
            overhead_pct.max(0.0),
            analysis_passes
        ),
        "Requires VMAF-capable ffmpeg and offline analysis before publish".into(),
        "Not real-time — suited to VOD catalogs, not live origin".into(),
        "Single 30s clip sample; full Netflix stack uses multi-clip + shot-based encodes".into(),
    ];

    (advantages, disadvantages)
}
