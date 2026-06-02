const STATUS_COPY = {
  Uploaded: {
    title: "Queued",
    detail: "Your upload is registered. Starting probe…",
  },
  Probing: {
    title: "Probing source",
    detail: "Reading resolution, duration, and codec with ffprobe.",
  },
  Analyzing: {
    title: "Per-title analysis",
    detail: "Sampling several bitrates per resolution, each scored with VMAF.",
  },
  Optimizing: {
    title: "Choosing ladder",
    detail: "Picking the lowest bitrate per resolution that clears the VMAF target.",
  },
  Encoding: {
    title: "Encoding renditions",
    detail: "Rendering the per-title ladder to HLS.",
  },
  Done: {
    title: "Ready",
    detail: "Opening player…",
  },
  Failed: {
    title: "Pipeline failed",
    detail: "Check the server log for details.",
  },
};

export function renderEncodingPanel(root, status, extra = "") {
  const copy = STATUS_COPY[status] || STATUS_COPY.Analyzing;
  root.innerHTML = `
    <div class="encoding-panel" role="status" aria-live="polite">
      <div class="spinner"></div>
      <p class="encoding-panel__title">${copy.title}</p>
      <p class="encoding-panel__detail">${extra || copy.detail}</p>
      <p class="encoding-panel__tiers">Per-title analysis runs in parallel · playback starts when all renditions finish</p>
    </div>
  `;
}

/// Live per-rendition encode progress (bars), driven by ffmpeg `-progress` over SSE.
export function renderEncodeProgress(root, ladder, progress) {
  const rows = (ladder || [])
    .map((r) => {
      const pct = Math.max(0, Math.min(100, Math.round(progress[r.name] ?? 0)));
      const done = pct >= 100;
      return `
        <div class="enc-row">
          <span class="enc-row__label">${r.name}<small>${r.bitrate_kbps}k</small></span>
          <div class="enc-bar"><div class="enc-bar__fill ${done ? "is-done" : ""}" style="width:${pct}%"></div></div>
          <span class="enc-row__pct">${done ? "✓" : pct + "%"}</span>
        </div>`;
    })
    .join("");

  root.innerHTML = `
    <div class="encoding-panel encoding-panel--progress" role="status" aria-live="polite">
      <p class="encoding-panel__title">Encoding ${ladder?.length || 0} renditions</p>
      <div class="enc-list">${rows || '<p class="encoding-panel__detail">Preparing…</p>'}</div>
      <p class="encoding-panel__tiers">Live ffmpeg progress · playback starts when all renditions finish</p>
    </div>
  `;
}
