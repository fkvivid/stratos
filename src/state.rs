use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::broadcast;

/// One running or finished job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    pub filename: String,
    pub source_path: String,
    pub output_dir: String,
    pub status: JobStatus,
    pub created_at: u64, // unix seconds — for history ordering
    pub source_info: Option<SourceInfo>,
    pub analysis_points: Vec<DataPoint>, // rate–quality samples (resolution × bitrate)
    pub chosen_ladder: Vec<Rendition>,   // per-title ladder, one rung per resolution
    pub insights: Option<TitleInsights>,
    pub error: Option<String>,
}

/// Per-title vs fixed-ladder comparison for dashboards / blog posts.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TitleInsights {
    pub reference_ladder: Vec<Rendition>,
    pub reference_avg_bitrate_kbps: u32,
    pub optimized_avg_bitrate_kbps: u32,
    pub bandwidth_savings_pct: f64,
    pub mean_vmaf_optimized: f64,
    pub mean_vmaf_reference_est: f64,
    pub delivery_per_hour_reference_mb: f64,
    pub delivery_per_hour_optimized_mb: f64,
    pub delivery_title_reference_mb: f64,
    pub delivery_title_optimized_mb: f64,
    pub encode_analysis_passes: u32,
    pub encode_final_renditions: u32,
    pub encode_reference_renditions: u32,
    pub per_title_overhead_pct: f64,
    pub rung_comparisons: Vec<RungComparison>,
    pub advantages: Vec<String>,
    pub disadvantages: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RungComparison {
    pub resolution: String,
    pub reference_bitrate_kbps: u32,
    pub optimized_bitrate_kbps: u32,
    pub bitrate_saved_pct: f64,
    pub vmaf_optimized: f64,
    pub vmaf_reference_est: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum JobStatus {
    Uploaded,
    Probing,
    Analyzing,  // Phase A — bitrate grid sample encodes
    Optimizing, // convex hull
    Encoding,   // Phase B — final HLS encodes
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SourceInfo {
    pub width: u32,
    pub height: u32,
    pub duration_sec: f64,
    pub fps: f64,
    pub codec: String,
}

/// One cell in the bitrate grid → one data point on the rate-quality plane
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DataPoint {
    pub resolution: String, // "720p"
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
    pub vmaf_mean: f64,
    pub file_size_bytes: u64,
}

/// A rendition chosen by the optimizer for the final ladder
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rendition {
    pub name: String, // "720p"
    pub width: u32,
    pub height: u32,
    pub bitrate_kbps: u32,
    pub vmaf_mean: f64,
}

/// Event broadcast to all SSE listeners for a job
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum JobEvent {
    StatusChanged { status: JobStatus },
    AnalysisPoint { point: DataPoint },
    LadderChosen { ladder: Vec<Rendition> },
    InsightsReady { insights: TitleInsights },
    EncodeProgress { rendition: String, percent: f64 },
    Done,
    Error { message: String },
}

/// Global app state shared across all request handlers
pub struct AppState {
    pub jobs: DashMap<String, Job>,
    /// One broadcast channel per active job — SSE clients subscribe
    pub brokers: DashMap<String, broadcast::Sender<JobEvent>>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            jobs: DashMap::new(),
            brokers: DashMap::new(),
        }
    }

    pub fn publish(&self, job_id: &str, event: JobEvent) {
        if let Some(tx) = self.brokers.get(job_id) {
            let _ = tx.send(event); // ignore if no subscribers
        }
    }
}

pub type SharedState = Arc<AppState>;
