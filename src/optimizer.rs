use crate::state::{DataPoint, Rendition};

/// Convex-hull / Pareto-frontier filter:
/// keep only points where no other point has both lower bitrate AND higher VMAF.
pub fn pareto_frontier(points: &[DataPoint]) -> Vec<DataPoint> {
    let mut hull = Vec::new();

    for (i, candidate) in points.iter().enumerate() {
        let dominated = points.iter().enumerate().any(|(j, other)| {
            if i == j {
                return false;
            }
            // other dominates candidate if it's better or equal on both axes, strictly better on at least one
            other.vmaf_mean >= candidate.vmaf_mean
                && other.bitrate_kbps <= candidate.bitrate_kbps
                && (other.vmaf_mean > candidate.vmaf_mean
                    || other.bitrate_kbps < candidate.bitrate_kbps)
        });

        if !dominated {
            hull.push(candidate.clone());
        }
    }

    // Sort by bitrate ascending so the ladder is bottom-up
    hull.sort_by_key(|p| p.bitrate_kbps);
    hull
}

/// Optional subset picker (chart / experiments). Production ladder uses [`ladder_all_resolution_tiers`].
#[allow(dead_code)]
pub fn select_ladder(frontier: &[DataPoint], target_rungs: usize) -> Vec<Rendition> {
    if frontier.is_empty() {
        return Vec::new();
    }

    // If we have fewer points than rungs, just return all of them
    if frontier.len() <= target_rungs {
        return frontier.iter().map(to_rendition).collect();
    }

    // Walk the frontier picking points whose VMAF jumps are evenly distributed
    let vmaf_min = frontier[0].vmaf_mean;
    let vmaf_max = frontier[frontier.len() - 1].vmaf_mean;
    let step = (vmaf_max - vmaf_min) / (target_rungs as f64 - 1.0);

    let mut chosen: Vec<DataPoint> = Vec::with_capacity(target_rungs);
    chosen.push(frontier[0].clone()); // always include lowest

    for i in 1..target_rungs - 1 {
        let target_vmaf = vmaf_min + step * (i as f64);
        let nearest = frontier
            .iter()
            .min_by(|a, b| {
                let da = (a.vmaf_mean - target_vmaf).abs();
                let db = (b.vmaf_mean - target_vmaf).abs();
                da.partial_cmp(&db).unwrap()
            })
            .unwrap();
        if !chosen.iter().any(|p| p.resolution == nearest.resolution) {
            chosen.push(nearest.clone());
        }
    }

    chosen.push(frontier[frontier.len() - 1].clone()); // always include highest
    chosen.sort_by_key(|p| p.bitrate_kbps);
    chosen.iter().map(to_rendition).collect()
}

fn to_rendition(p: &DataPoint) -> Rendition {
    Rendition {
        name: p.resolution.clone(),
        width: p.width,
        height: p.height,
        bitrate_kbps: p.bitrate_kbps,
        vmaf_mean: p.vmaf_mean,
    }
}

/// Per-title ladder selection (the heart of the savings claim).
///
/// For each resolution tier we sampled several bitrates. Netflix's per-title idea:
/// deliver the *lowest* bitrate that still reaches a perceptual-quality target
/// (here VMAF ≥ `target_vmaf`). Easy content (animation, flat scenes) hits the
/// target far below a one-size-fits-all fixed ladder → real bandwidth savings.
///
/// Returns one rung per resolution tier, sorted low → high.
pub fn choose_per_title_ladder(points: &[DataPoint], target_vmaf: f64) -> Vec<Rendition> {
    use std::collections::BTreeMap;

    let mut by_height: BTreeMap<u32, Vec<&DataPoint>> = BTreeMap::new();
    for p in points {
        by_height.entry(p.height).or_default().push(p);
    }

    let mut ladder: Vec<Rendition> = by_height
        .into_values()
        .filter_map(|mut group| {
            group.sort_by_key(|p| p.bitrate_kbps);
            // Lowest sampled bitrate that meets the quality target; else the best we have.
            let chosen = group
                .iter()
                .find(|p| p.vmaf_mean >= target_vmaf)
                .copied()
                .or_else(|| group.last().copied())?;
            Some(to_rendition(chosen))
        })
        .collect();

    ladder.sort_by_key(|r| r.height);
    ladder
}
