const PHASES = [
  { key: "Probing", label: "Probe" },
  { key: "Analyzing", label: "Analyze" },
  { key: "Optimizing", label: "Optimize" },
  { key: "Encoding", label: "Encode" },
  { key: "Done", label: "Ready" },
];

const PHASE_ORDER = ["Uploaded", "Probing", "Analyzing", "Optimizing", "Encoding", "Done"];

export function createJobMonitor({
  jobId,
  onDone,
  onStatus,
  onEncodeProgress,
  onLadder,
  onInsights,
}) {
  const points = [];
  let chosen = [];

  const badge = document.getElementById("badge");
  const stepsEl = document.getElementById("pipelineSteps");

  function setBadge(status, failed = false) {
    badge.className = "badge" + (failed ? " badge--danger" : status === "Done" ? " badge--success" : "");
    const showDot = !["Done", "Failed"].includes(status);
    badge.innerHTML = showDot
      ? '<span class="badge__dot"></span>' + formatStatus(status)
      : formatStatus(status);
  }

  function formatStatus(s) {
    if (s === "Done") return "Ready";
    if (s === "Failed") return "Failed";
    return s.replace(/([A-Z])/g, " $1").trim();
  }

  function updateSteps(status) {
    if (!stepsEl) return;
    const idx = PHASE_ORDER.indexOf(status);
    stepsEl.querySelectorAll(".step").forEach((el) => {
      const phaseIdx = PHASE_ORDER.indexOf(el.dataset.phase);
      el.classList.remove("active", "done", "failed");
      if (status === "Failed" && phaseIdx === idx) {
        el.classList.add("failed");
      } else if (phaseIdx < idx) {
        el.classList.add("done");
      } else if (phaseIdx === idx) {
        el.classList.add("active");
      } else if (status === "Uploaded" && el.dataset.phase === "Probing") {
        el.classList.add("active");
      }
    });
  }

  function drawChart() {
    const svg = document.getElementById("chart");
    if (!svg) return;
    const w = svg.clientWidth;
    const h = svg.clientHeight;
    const pad = { l: 52, r: 16, t: 16, b: 44 };
    const inner = { w: w - pad.l - pad.r, h: h - pad.t - pad.b };

    svg.innerHTML = "";

    if (points.length === 0) {
      svg.innerHTML =
        '<text x="50%" y="50%" text-anchor="middle" fill="#5c6478" font-size="13">Collecting analysis samples…</text>';
      return;
    }

    const maxBitrate = Math.max(...points.map((p) => p.bitrate_kbps)) * 1.1;
    const minVmaf = Math.min(40, ...points.map((p) => p.vmaf_mean)) - 5;
    const maxVmaf = 100;

    const x = (b) => pad.l + (b / maxBitrate) * inner.w;
    const y = (v) => pad.t + ((maxVmaf - v) / (maxVmaf - minVmaf)) * inner.h;

    svg.innerHTML += `
      <line x1="${pad.l}" y1="${pad.t}" x2="${pad.l}" y2="${h - pad.b}" stroke="#1e2430"/>
      <line x1="${pad.l}" y1="${h - pad.b}" x2="${w - pad.r}" y2="${h - pad.b}" stroke="#1e2430"/>
      <text x="${w / 2}" y="${h - 10}" text-anchor="middle" fill="#5c6478" font-size="11">Bitrate (kbps)</text>
      <text x="16" y="${h / 2}" text-anchor="middle" fill="#5c6478" font-size="11" transform="rotate(-90 16 ${h / 2})">VMAF</text>
    `;

    for (let i = 0; i <= 5; i++) {
      const v = (maxBitrate / 5) * i;
      svg.innerHTML += `<text x="${x(v)}" y="${h - pad.b + 16}" text-anchor="middle" fill="#5c6478" font-size="10">${v.toFixed(0)}</text>`;
    }

    for (const p of points) {
      svg.innerHTML += `<circle cx="${x(p.bitrate_kbps)}" cy="${y(p.vmaf_mean)}" r="4" fill="#3d4455"/>`;
    }

    const frontier = points
      .filter(
        (p) =>
          !points.some(
            (q) =>
              q !== p &&
              q.vmaf_mean >= p.vmaf_mean &&
              q.bitrate_kbps <= p.bitrate_kbps &&
              (q.vmaf_mean > p.vmaf_mean || q.bitrate_kbps < p.bitrate_kbps)
          )
      )
      .sort((a, b) => a.bitrate_kbps - b.bitrate_kbps);

    if (frontier.length > 1) {
      const path = frontier.map((p, i) => `${i === 0 ? "M" : "L"}${x(p.bitrate_kbps)},${y(p.vmaf_mean)}`).join(" ");
      svg.innerHTML += `<path d="${path}" fill="none" stroke="#34d399" stroke-width="2" opacity="0.9"/>`;
    }

    for (const r of chosen) {
      svg.innerHTML += `<circle cx="${x(r.bitrate_kbps)}" cy="${y(r.vmaf_mean)}" r="7" fill="#5b9cff" stroke="#e8eaef" stroke-width="1.5"/>`;
    }
  }

  function renderLadder() {
    const panel = document.getElementById("ladderPanel");
    const div = document.getElementById("ladder");
    if (!chosen.length || !panel || !div) return;
    panel.hidden = false;
      div.innerHTML = `
        <p class="panel__hint" style="margin-bottom:10px">${chosen.length} rungs · click to switch quality in player</p>
      ` + chosen
      .map(
        (r) => `
      <div class="ladder-item" data-rendition="${r.name}" title="Switch player to ${r.name}">
        <span class="ladder-item__label">${r.name} · ${r.width}×${r.height}</span>
        <span class="ladder-item__meta">
          <strong>${r.bitrate_kbps} kbps</strong>
          <span class="ladder-item__vmaf">VMAF ${r.vmaf_mean.toFixed(1)}</span>
        </span>
      </div>`
      )
      .join("");
    onLadder?.(chosen);
  }

  const es = new EventSource(`/api/events/${jobId}`);
  es.onmessage = (e) => {
    const data = JSON.parse(e.data);

    if (data.event === "status_changed") {
      setBadge(data.status);
      updateSteps(data.status);
      onStatus?.(data.status);
    }
    if (data.event === "encode_progress") {
      onEncodeProgress?.(data.rendition, data.percent ?? 0);
    }
    if (data.event === "analysis_point") {
      points.push(data.point);
      drawChart();
    }
    if (data.event === "ladder_chosen") {
      chosen = data.ladder;
      renderLadder();
      drawChart();
    }
    if (data.event === "insights_ready") {
      onInsights?.(data.insights);
    }
    if (data.event === "done") {
      setBadge("Done");
      updateSteps("Done");
      es.close();
      onDone?.();
    }
    if (data.event === "error") {
      setBadge("Failed", true);
      updateSteps("Failed");
      onStatus?.("Failed");
    }
  };

  fetch(`/api/jobs/${jobId}`)
    .then((r) => {
      if (!r.ok) throw new Error("Job not found");
      return r.json();
    })
    .then((job) => {
      document.getElementById("jobFilename").textContent = job.filename || jobId;
      points.push(...(job.analysis_points || []));
      chosen = job.chosen_ladder || [];
      drawChart();
      renderLadder();
      if (job.insights) onInsights?.(job.insights);
      setBadge(job.status);
      updateSteps(job.status);
      onStatus?.(job.status);
      if (job.status === "Done") {
        es.close();
        onDone?.();
      }
      if (job.status === "Failed") setBadge("Failed", true);
    })
    .catch(() => {
      setBadge("Failed", true);
    });

  window.addEventListener("resize", drawChart);

  return { getChosen: () => chosen };
}

export function initPipelineSteps() {
  const el = document.getElementById("pipelineSteps");
  if (!el) return;
  el.innerHTML = PHASES.map(
    (p) => `<div class="step" data-phase="${p.key}">${p.label}</div>`
  ).join("");
}
