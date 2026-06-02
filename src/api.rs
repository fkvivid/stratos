use anyhow::Result;
use axum::{
    extract::{multipart::MultipartRejection, Multipart, Path, State},
    http::StatusCode,
    response::{
        sse::{Event, KeepAlive, Sse},
        IntoResponse, Json,
    },
};
use serde::Serialize;
use std::convert::Infallible;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::io::AsyncWriteExt;
use tokio::sync::broadcast;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use uuid::Uuid;

use crate::pipeline;
use crate::state::*;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// POST /api/upload — accept a video file, kick off the pipeline, return job_id
pub async fn upload_handler(
    State(state): State<SharedState>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<Json<serde_json::Value>, (StatusCode, String)> {
    let mut multipart = multipart.map_err(multipart_error)?;

    while let Some(mut field) = multipart.next_field().await.map_err(bad_request)? {
        let name = field.name().unwrap_or("").to_string();
        if name != "video" {
            continue;
        }

        let filename = sanitize_filename(field.file_name().unwrap_or("upload.mp4"));

        let job_id = format!("job_{}", &Uuid::new_v4().to_string()[..8]);
        let source_path = format!("uploads/{}_{}", job_id, filename);
        let output_dir = format!("output/{}", job_id);

        let mut out = tokio::fs::File::create(&source_path)
            .await
            .map_err(internal)?;
        loop {
            match field.chunk().await {
                Ok(Some(chunk)) => out.write_all(&chunk).await.map_err(internal)?,
                Ok(None) => break,
                Err(e) => return Err(bad_request(e)),
            }
        }
        out.flush().await.map_err(internal)?;

        // Register job
        let job = Job {
            id: job_id.clone(),
            filename: filename.clone(),
            source_path: source_path.clone(),
            output_dir: output_dir.clone(),
            status: JobStatus::Uploaded,
            created_at: now_secs(),
            source_info: None,
            analysis_points: Vec::new(),
            chosen_ladder: Vec::new(),
            insights: None,
            error: None,
        };
        state.jobs.insert(job_id.clone(), job);

        // Create broadcast channel for SSE
        let (tx, _) = broadcast::channel::<JobEvent>(128);
        state.brokers.insert(job_id.clone(), tx);

        // Spawn pipeline in a blocking task (it does CPU-heavy work via rayon)
        let st = state.clone();
        let jid = job_id.clone();
        tracing::info!(
            job_id = %job_id,
            filename = %filename,
            path = %source_path,
            "upload complete, pipeline spawning"
        );

        tokio::task::spawn_blocking(move || {
            if let Err(e) = pipeline::run_pipeline(st.clone(), jid.clone()) {
                tracing::error!(job_id = %jid, error = %e, "pipeline failed");
                st.jobs.alter(&jid, |_, mut j| {
                    j.status = JobStatus::Failed;
                    j.error = Some(e.to_string());
                    j
                });
                st.publish(
                    &jid,
                    JobEvent::Error {
                        message: e.to_string(),
                    },
                );
            }
        });

        return Ok(Json(
            serde_json::json!({ "job_id": job_id, "filename": filename }),
        ));
    }

    Err((StatusCode::BAD_REQUEST, "no 'video' field".into()))
}

/// GET /api/jobs/:id — return job state as JSON
pub async fn get_job_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> Result<Json<Job>, StatusCode> {
    state
        .jobs
        .get(&id)
        .map(|j| Json(j.clone()))
        .ok_or(StatusCode::NOT_FOUND)
}

/// Compact job record for the homepage history list.
#[derive(Serialize)]
pub struct JobSummary {
    id: String,
    filename: String,
    status: JobStatus,
    created_at: u64,
    duration_sec: Option<f64>,
    width: Option<u32>,
    height: Option<u32>,
    ladder_rungs: usize,
    bandwidth_savings_pct: Option<f64>,
    saved_mb: Option<f64>,
    error: Option<String>,
}

/// GET /api/jobs — list all jobs (most recent first) for the history view.
pub async fn list_jobs_handler(State(state): State<SharedState>) -> Json<Vec<JobSummary>> {
    let mut summaries: Vec<JobSummary> = state
        .jobs
        .iter()
        .map(|entry| {
            let j = entry.value();
            let saved_mb = j.insights.as_ref().map(|i| {
                (i.delivery_title_reference_mb - i.delivery_title_optimized_mb).max(0.0)
            });
            JobSummary {
                id: j.id.clone(),
                filename: j.filename.clone(),
                status: j.status.clone(),
                created_at: j.created_at,
                duration_sec: j.source_info.as_ref().map(|s| s.duration_sec),
                width: j.source_info.as_ref().map(|s| s.width),
                height: j.source_info.as_ref().map(|s| s.height),
                ladder_rungs: j.chosen_ladder.len(),
                bandwidth_savings_pct: j.insights.as_ref().map(|i| i.bandwidth_savings_pct),
                saved_mb,
                error: j.error.clone(),
            }
        })
        .collect();

    summaries.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Json(summaries)
}

/// GET /api/events/:id — SSE stream of job events
pub async fn sse_handler(
    State(state): State<SharedState>,
    Path(id): Path<String>,
) -> impl IntoResponse {
    let rx = match state.brokers.get(&id) {
        Some(tx) => tx.subscribe(),
        None => {
            // Job already done — emit a synthetic Done so client unblocks
            let (tx, rx) = broadcast::channel(1);
            let _ = tx.send(JobEvent::Done);
            rx
        }
    };

    let stream = BroadcastStream::new(rx).filter_map(|res| {
        res.ok().and_then(|event| {
            serde_json::to_string(&event)
                .ok()
                .map(|json| Ok::<_, Infallible>(Event::default().data(json)))
        })
    });

    Sse::new(stream).keep_alive(KeepAlive::default())
}

fn multipart_error(err: MultipartRejection) -> (StatusCode, String) {
    tracing::warn!("multipart upload rejected: {}", err);
    let msg = err.to_string();
    if msg.contains("length limit") || msg.contains("too large") {
        (
            StatusCode::PAYLOAD_TOO_LARGE,
            "upload exceeds size limit (max 4 GiB)".into(),
        )
    } else if msg.contains("multipart") {
        (
            StatusCode::BAD_REQUEST,
            "invalid multipart upload — use field name \"video\" and a single file".into(),
        )
    } else {
        (StatusCode::BAD_REQUEST, msg)
    }
}

fn sanitize_filename(name: &str) -> String {
    let base = std::path::Path::new(name)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("upload.mp4");
    let safe: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if safe.is_empty() {
        "upload.mp4".into()
    } else {
        safe
    }
}

fn bad_request(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::BAD_REQUEST, e.to_string())
}

fn internal(e: impl std::fmt::Display) -> (StatusCode, String) {
    (StatusCode::INTERNAL_SERVER_ERROR, e.to_string())
}
