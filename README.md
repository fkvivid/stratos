# 🎬 Stratos

**Per-title video encoding in Rust.** Upload a video and Stratos probes it, samples a
rate–quality curve with **VMAF**, picks the most efficient HLS ladder *for that specific
title*, and reports how much bandwidth it saves versus a traditional fixed ladder — with a
live, adaptive-bitrate player.

It's a compact, end-to-end implementation of the idea behind
[Netflix's per-title encode optimization](https://netflixtechblog.com/per-title-encode-optimization-7e99442b62a2):
*easy content (animation, flat scenes) doesn't need as many bits as a one-size-fits-all
ladder assumes.*

```bash
# Requires ffmpeg built with libvmaf (macOS: brew install ffmpeg)
cargo run
# → open http://localhost:8080
```

---

## What it does

1. **Upload** a video in the browser (drag & drop).
2. **Probe** — `ffprobe` reads resolution, duration, fps, codec.
3. **Analyze** — for each resolution tier (360p…1080p, capped to source) Stratos encodes a
   30-second clip at **several candidate bitrates** and scores each against the source with
   **VMAF**, building a rate–quality curve.
4. **Optimize** — for each resolution it keeps the **lowest bitrate that still clears the
   VMAF target** (default 93). That's the per-title ladder.
5. **Encode** — renders one HLS rendition per chosen rung, with aligned keyframes and a
   master playlist.
6. **Report** — the player page shows a live ABR player **and** a per-title-vs-fixed-ladder
   savings breakdown.

Everything streams to the browser live over **Server-Sent Events**: analysis points appear
on a rate–quality chart as they're measured, then per-rendition encode progress bars, then
the player.

---

## Highlights

- **Real per-title optimization** — not a fixed ladder. Bitrate is matched to content
  complexity via a VMAF rate–quality search.
- **Savings report** — per-rendition bitrate delta, estimated VMAF at the fixed bitrate,
  total delivery savings (MB), and the encoder-cost trade-off. Built for writing up results.
- **Live pipeline** — rate–quality chart, pipeline step tracker, and per-rendition encode
  progress bars stream in real time.
- **Adaptive player** — `hls.js` with manual quality selection (Auto / 360p / 720p / …) and
  custom controls.
- **History homepage** — every job with status, resolution, and savings; polls live.
- **Observable** — structured `tracing` logs for every phase, optional live ffmpeg progress.

---

## Quick start

**Prerequisites**

- Rust (stable) — <https://rustup.rs>
- `ffmpeg` **with `libvmaf`** and `ffprobe` on your `PATH`
  - macOS: `brew install ffmpeg`
  - Verify: `ffmpeg -hide_banner -filters | grep libvmaf`

**Run**

```bash
cargo run
# Stratos listening on http://0.0.0.0:8080
```

On startup the server verifies `ffmpeg`, `ffprobe`, and `libvmaf` and refuses to start with a
clear message if they're missing. Uploads and output are written to `./uploads` and `./output`.

---

## How it works

```
upload ─▶ probe ─▶ analyze (rate–quality grid) ─▶ optimize (per-title ladder) ─▶ encode (HLS) ─▶ done
              │            │ parallel (rayon)                                      │ parallel (rayon)
              └── ffprobe  └── ffmpeg + libvmaf per (resolution × bitrate)         └── ffmpeg → HLS
```

- **CPU-bound work** (ffmpeg, VMAF) runs on a `rayon` pool inside `tokio::task::spawn_blocking`,
  so the async runtime stays responsive.
- **State** lives in a `dashmap` keyed by `job_id`; each job has a `tokio::sync::broadcast`
  channel that fans out events to every connected SSE client.

### Ladder policy (the algorithm)

- **Analysis** samples each resolution at several candidate bitrates (see
  `candidate_bitrates` in `pipeline.rs`) and measures VMAF for each.
- **Selection** (`optimizer::choose_per_title_ladder`) keeps, per resolution, the lowest
  sampled bitrate with `VMAF ≥ TARGET_VMAF` (default `93`); if none clear it, the best
  available is used. Easy content clears the bar far below a fixed ladder → real savings.
- **Chart**: every sample is a dot, the green line is the Pareto frontier, and blue dots are
  the chosen per-title rungs.

Both `TARGET_VMAF` and the candidate bitrates live in `src/pipeline.rs` and are easy to tune.

---

## Reading the code

If you read four files, read these in order — they're the intellectual core:

1. `src/state.rs` — the data: jobs, events, data points, insights
2. `src/optimizer.rs` — Pareto frontier + per-title ladder selection
3. `src/pipeline.rs` — how the phases compose and stream events
4. `src/api.rs` — how axum bridges HTTP/SSE to the blocking pipeline

| File | Role |
|------|------|
| `src/main.rs` | Router, static file serving, dependency check, logging setup |
| `src/api.rs` | Upload, job state, job list, SSE handlers |
| `src/pipeline.rs` | Probe → analyze → optimize → encode orchestration |
| `src/ffmpeg.rs` | ffmpeg/ffprobe wrappers: sample encode, VMAF, HLS (with progress), playlist |
| `src/optimizer.rs` | Pareto frontier and per-title ladder selection |
| `src/insights.rs` | Per-title vs fixed-ladder savings model |
| `src/probe.rs` | `ffprobe` JSON → `SourceInfo` |
| `src/state.rs` | Shared types and `AppState` |
| `web/` | Frontend: upload, player, charts, insights, history (vanilla JS modules) |

---

## API

| Method | Path                | Purpose                                       |
|--------|---------------------|-----------------------------------------------|
| POST   | `/api/upload`       | Accept a video (multipart `video`), return `job_id` |
| GET    | `/api/jobs`         | List all jobs (newest first) for the history view |
| GET    | `/api/jobs/{id}`    | Full job state as JSON                        |
| GET    | `/api/events/{id}`  | SSE stream of live pipeline events            |
| GET    | `/stream/{id}/...`  | HLS master/variant playlists and TS segments  |

### SSE events

```json
{ "event": "status_changed", "status": "Analyzing" }
{ "event": "analysis_point", "point": { "resolution": "720p", "bitrate_kbps": 1500, "vmaf_mean": 93.4 } }
{ "event": "ladder_chosen", "ladder": [ { "name": "720p", "bitrate_kbps": 1500, "vmaf_mean": 93.4 } ] }
{ "event": "insights_ready", "insights": { "bandwidth_savings_pct": 28.6, "rung_comparisons": [ ] } }
{ "event": "encode_progress", "rendition": "720p", "percent": 42.0 }
{ "event": "done" }
{ "event": "error", "message": "ffmpeg vmaf failed: ..." }
```

---

## Debugging & observability

Stratos logs every pipeline phase to **stderr** (structured with `tracing`).

| Command | What you see |
|---------|--------------|
| `cargo run` | `info` — probe, each tier, ladder rungs, HLS encodes, timings |
| `RUST_LOG=stratos=debug cargo run` | Full ffmpeg command lines |
| `STRATOS_FFMPEG_VERBOSE=1 cargo run` | Live ffmpeg progress streamed to the terminal |

Failed jobs log `pipeline failed` with the `job_id` and ffmpeg's stderr. See `.env.example`
for the available variables (export them inline or load them with your shell).

---

## Tech stack

| Layer            | Choice                                        |
|------------------|-----------------------------------------------|
| Web framework    | [`axum`](https://docs.rs/axum/) + [`tower-http`](https://docs.rs/tower-http/) |
| Async runtime    | [`tokio`](https://docs.rs/tokio/)             |
| CPU parallelism  | [`rayon`](https://docs.rs/rayon/)             |
| Shared state     | [`dashmap`](https://docs.rs/dashmap/)         |
| Live events      | `tokio::sync::broadcast` → SSE                |
| Encoding         | `ffmpeg` (subprocess) with `libx264`          |
| Quality scoring  | `ffmpeg` `libvmaf` filter                     |
| Streaming format | HLS (m3u8 + MPEG-TS segments)                 |
| Player           | [`hls.js`](https://github.com/video-dev/hls.js/) + vanilla JS |

---

## Benchmarks

Indicative, on an M2 MacBook Pro with a 1080p source (clip-based analysis):

| Phase     | Wall clock | What happens                                            |
|-----------|------------|---------------------------------------------------------|
| Probe     | ~1 s       | ffprobe metadata extraction                             |
| Analysis  | ~2–4 min   | resolutions × candidate bitrates, 30 s clip + VMAF (parallel) |
| Optimize  | < 1 ms     | pure Rust, in-memory                                    |
| Encode    | scales with duration | one full HLS encode per chosen rung (parallel) |

Analysis cost scales with the number of candidate bitrates per resolution; lower
`candidate_bitrates` / raise `TARGET_VMAF` to trade accuracy for speed. Savings depend on
content: easy/animated titles save the most, grainy/complex titles save least.

---

## Lessons learned

- **Filter graph alignment is everything for VMAF.** Newer `libvmaf` rejects mismatched pixel
  formats, sizes, and timebases. Normalize explicitly:
  `format=yuv420p10le,setpts=PTS-STARTPTS,settb=AVTB`, and scale the reference to the
  distorted geometry before comparing.
- **One reference clip, many comparisons.** Cut the source clip once and reuse it for every
  VMAF run instead of re-trimming the full source in parallel (which corrupts reads).
- **Unique output paths under parallelism.** Parallel ffmpeg writes to the same filename
  produce corrupt MP4s (`Invalid NAL unit size`). Name every analysis cell uniquely.
- **CPU-bound work needs `rayon`, not `tokio`.** Spawning ffmpeg from `tokio::spawn` starves
  the runtime; use `spawn_blocking` + `rayon`.
- **Keyframe alignment matters for HLS.** Without `-g 48 -keyint_min 48 -sc_threshold 0`,
  segments don't align across renditions and quality switching breaks.
- **axum's default body limit is 2 MiB.** Video uploads need an explicit `DefaultBodyLimit`.

---

## Current project scope

This repository currently focuses on a complete single-node per-title VOD workflow:

- Browser upload (`multipart/form-data`) to local storage (`./uploads`)
- Probe + multi-bitrate VMAF analysis per resolution tier
- Per-title ladder selection (lowest bitrate per tier that meets target VMAF)
- Parallel HLS encoding with live per-rendition progress over SSE
- Interactive player + rate-quality chart + per-title vs fixed-ladder report
- Homepage history of previous jobs

---

## Roadmap

- [ ] Multi-clip sampling for more representative VMAF
- [ ] Persist jobs across restarts (currently in-memory)
- [ ] Kubernetes Jobs as the parallelism primitive (one Job per cell)
- [ ] CLI mode (`stratos encode video.mp4 --out ./out/`)

---

## References

- Netflix Tech Blog — [Per-Title Encode Optimization](https://netflixtechblog.com/per-title-encode-optimization-7e99442b62a2) (2015)
- Netflix Tech Blog — [Optimized shot-based encodes](https://netflixtechblog.com/optimized-shot-based-encodes-now-streaming-4b9464204830) (2018)
- VMAF — [GitHub](https://github.com/Netflix/vmaf) · [paper](https://arxiv.org/abs/1907.07999)
- HLS spec — [RFC 8216](https://datatracker.ietf.org/doc/html/rfc8216)
- Video systems learning resource — [howvideo.works](https://howvideo.works/)

---

## License

MIT
