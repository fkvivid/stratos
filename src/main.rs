mod api;
mod ffmpeg;
mod insights;
mod optimizer;
mod pipeline;
mod probe;
mod state;

use axum::{
    extract::DefaultBodyLimit,
    http::{header, Request, Response},
    middleware::Next,
    routing::{get, post},
    Router,
};

/// Video uploads exceed axum's default 2 MiB body limit without this.
const MAX_UPLOAD_BYTES: usize = 4 * 1024 * 1024 * 1024; // 4 GiB

use std::sync::Arc;
use tower_http::services::ServeDir;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

use crate::state::AppState;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                EnvFilter::new("stratos=info,tower_http=info,axum=warn")
            }),
        )
        .with_target(false)
        .init();

    if let Err(e) = ffmpeg::check_dependencies() {
        tracing::error!("{:#}", e);
        eprintln!("\nStratos cannot start: {e:#}\n");
        std::process::exit(1);
    }

    std::fs::create_dir_all("uploads").unwrap();
    std::fs::create_dir_all("output").unwrap();

    let state = Arc::new(AppState::new());

    let app = Router::new()
        .route(
            "/api/upload",
            post(api::upload_handler).layer(DefaultBodyLimit::max(MAX_UPLOAD_BYTES)),
        )
        .route("/api/jobs", get(api::list_jobs_handler))
        .route("/api/jobs/:id", get(api::get_job_handler))
        .route("/api/events/:id", get(api::sse_handler))
        .nest_service("/stream", ServeDir::new("output"))
        .nest_service("/", ServeDir::new("web"))
        .layer(axum::middleware::from_fn(html_charset_middleware))
        .layer(TraceLayer::new_for_http())
        .with_state(state);

    let addr = "0.0.0.0:8080";
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();

    tracing::info!("Stratos listening on http://{}", addr);
    tracing::info!("Debug: RUST_LOG=stratos=debug cargo run");
    tracing::info!("ffmpeg progress: STRATOS_FFMPEG_VERBOSE=1 cargo run");

    axum::serve(listener, app).await.unwrap();
}

async fn html_charset_middleware(
    request: Request<axum::body::Body>,
    next: Next,
) -> Response<axum::body::Body> {
    let mut response = next.run(request).await;
    if let Some(ct) = response.headers().get(header::CONTENT_TYPE) {
        if let Ok(value) = ct.to_str() {
            if value.starts_with("text/html") && !value.contains("charset=") {
                if let Ok(header_value) =
                    header::HeaderValue::from_str(&format!("{value}; charset=utf-8"))
                {
                    response
                        .headers_mut()
                        .insert(header::CONTENT_TYPE, header_value);
                }
            }
        }
    }
    response
}
