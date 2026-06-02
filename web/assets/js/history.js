const STATUS_META = {
  Uploaded: { label: "Queued", cls: "" },
  Probing: { label: "Probing", cls: "" },
  Analyzing: { label: "Analyzing", cls: "" },
  Optimizing: { label: "Optimizing", cls: "" },
  Encoding: { label: "Encoding", cls: "" },
  Done: { label: "Ready", cls: "badge--success" },
  Failed: { label: "Failed", cls: "badge--danger" },
};

const ACTIVE = ["Uploaded", "Probing", "Analyzing", "Optimizing", "Encoding"];

function timeAgo(epochSec) {
  if (!epochSec) return "";
  const s = Math.max(0, Math.floor(Date.now() / 1000 - epochSec));
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  if (s < 86400) return `${Math.floor(s / 3600)}h ago`;
  return `${Math.floor(s / 86400)}d ago`;
}

function fmtDuration(sec) {
  if (!sec) return "";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}

function card(job) {
  const meta = STATUS_META[job.status] || { label: job.status, cls: "" };
  const active = ACTIVE.includes(job.status);
  const badge = `<span class="badge ${meta.cls}">${
    active ? '<span class="badge__dot"></span>' : ""
  }${meta.label}</span>`;

  const res = job.height ? `${job.width}×${job.height}` : "";
  const dur = fmtDuration(job.duration_sec);
  const dims = [res, dur].filter(Boolean).join(" · ");

  let savingsChip = "";
  if (job.status === "Done" && typeof job.bandwidth_savings_pct === "number") {
    const pct = job.bandwidth_savings_pct;
    const cls = pct >= 0 ? "chip--good" : "chip--warn";
    const sign = pct >= 0 ? "−" : "+";
    const saved =
      job.saved_mb && job.saved_mb > 0 ? ` · saves ${job.saved_mb.toFixed(0)} MB` : "";
    savingsChip = `<span class="chip ${cls}">${sign}${Math.abs(pct).toFixed(0)}% bitrate${saved}</span>`;
  } else if (job.status === "Failed") {
    savingsChip = `<span class="chip chip--warn">${(job.error || "failed").slice(0, 40)}</span>`;
  }

  const ladderChip =
    job.ladder_rungs > 0
      ? `<span class="chip">${job.ladder_rungs} renditions</span>`
      : "";

  return `
    <a class="history-card" href="/player.html?job=${encodeURIComponent(job.id)}">
      <div class="history-card__main">
        <span class="history-card__name" title="${job.filename}">${job.filename}</span>
        <span class="history-card__sub">${dims}${dims ? " · " : ""}${timeAgo(job.created_at)}</span>
      </div>
      <div class="history-card__chips">
        ${savingsChip}
        ${ladderChip}
      </div>
      ${badge}
    </a>`;
}

async function refresh() {
  const host = document.getElementById("historyList");
  const section = document.getElementById("history");
  if (!host || !section) return false;

  let jobs = [];
  try {
    const res = await fetch("/api/jobs", { cache: "no-store" });
    if (!res.ok) throw new Error("bad status");
    jobs = await res.json();
  } catch (_) {
    return false;
  }

  if (!jobs.length) {
    section.hidden = true;
    return false;
  }

  section.hidden = false;
  host.innerHTML = jobs.map(card).join("");
  return jobs.some((j) => ACTIVE.includes(j.status));
}

export function initHistory() {
  refresh();
  // Poll while anything is in-flight; otherwise back off to occasional refresh.
  let tick = 0;
  setInterval(async () => {
    tick += 1;
    const active = await refresh();
    // Always refresh when active; every ~15s when idle.
    if (!active && tick % 5 !== 0) return;
  }, 3000);
}
