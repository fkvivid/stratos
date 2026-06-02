function fmtMb(n) {
  if (n >= 1024) return `${(n / 1024).toFixed(2)} GB`;
  return `${n.toFixed(1)} MB`;
}

function fmtPct(n) {
  return `${Math.abs(n).toFixed(1)}%`;
}

// Signed saving: + (green, "saves"), − (warning, "costs more").
function deltaCell(savedPct) {
  if (savedPct > 0.05) {
    return `<td class="mono success">−${fmtPct(savedPct)}</td>`;
  }
  if (savedPct < -0.05) {
    return `<td class="mono warn">+${fmtPct(savedPct)}</td>`;
  }
  return `<td class="mono muted">≈0%</td>`;
}

export function renderInsightsPanel(insights) {
  if (!insights) return "";

  const savedMb =
    insights.delivery_title_reference_mb - insights.delivery_title_optimized_mb;
  const savedSign = savedMb >= 0 ? "saves" : "costs";
  const savings = insights.bandwidth_savings_pct;
  const savingsLabel = savings >= 0 ? "Bandwidth saved" : "Bandwidth added";

  const rungRows = (insights.rung_comparisons || [])
    .map(
      (r) => `
    <tr>
      <td><strong>${r.resolution}</strong></td>
      <td class="mono muted">${r.reference_bitrate_kbps} kbps</td>
      <td class="mono accent">${r.optimized_bitrate_kbps} kbps</td>
      ${deltaCell(r.bitrate_saved_pct)}
      <td class="mono success">${r.vmaf_optimized.toFixed(1)}</td>
      <td class="mono muted">~${r.vmaf_reference_est.toFixed(1)}</td>
    </tr>`
    )
    .join("");

  const pros = (insights.advantages || [])
    .map((t) => `<li>${t}</li>`)
    .join("");
  const cons = (insights.disadvantages || [])
    .map((t) => `<li>${t}</li>`)
    .join("");

  return `
    <div class="panel insights-panel">
      <div class="panel__title">Per-title vs fixed ladder</div>
      <p class="insights-lead">
        Compared to a <strong>traditional fixed ABR ladder</strong> (same bitrates for every title).
        Methodology inspired by
        <a href="https://netflixtechblog.com/per-title-encode-optimization-7e99442b62a2" target="_blank" rel="noopener">Netflix per-title encode (2015)</a>.
      </p>

      <div class="metric-grid">
        <div class="metric-card metric-card--highlight">
          <span class="metric-card__label">${savingsLabel}</span>
          <span class="metric-card__value">${savings >= 0 ? "" : "+"}${fmtPct(savings)}</span>
          <span class="metric-card__sub">avg ladder bitrate vs fixed</span>
        </div>
        <div class="metric-card">
          <span class="metric-card__label">Delivery (this title)</span>
          <span class="metric-card__value">${fmtMb(insights.delivery_title_optimized_mb)}</span>
          <span class="metric-card__sub">vs ${fmtMb(insights.delivery_title_reference_mb)} fixed · ${savedSign} ${fmtMb(Math.abs(savedMb))}</span>
        </div>
        <div class="metric-card">
          <span class="metric-card__label">Mean VMAF</span>
          <span class="metric-card__value">${insights.mean_vmaf_optimized.toFixed(1)}</span>
          <span class="metric-card__sub">vs ~${insights.mean_vmaf_reference_est.toFixed(1)} est. at fixed bitrates</span>
        </div>
        <div class="metric-card">
          <span class="metric-card__label">Encode overhead</span>
          <span class="metric-card__value">+${fmtPct(Math.max(0, insights.per_title_overhead_pct))}</span>
          <span class="metric-card__sub">${insights.encode_analysis_passes} analysis + ${insights.encode_final_renditions} final encodes</span>
        </div>
      </div>

      <div class="compare-table-wrap">
        <table class="compare-table">
          <thead>
            <tr>
              <th>Resolution</th>
              <th>Fixed ladder</th>
              <th>Per-title</th>
              <th>Bitrate Δ</th>
              <th>VMAF (measured)</th>
              <th>Fixed (est.)</th>
            </tr>
          </thead>
          <tbody>${rungRows}</tbody>
        </table>
        <p class="panel__hint" style="margin-top:8px">
          Bitrate Δ: green = per-title delivers the same quality with fewer bits at that resolution.
          VMAF is measured on the per-title encode; "Fixed (est.)" interpolates the rate–quality curve at the fixed bitrate.
        </p>
      </div>

      <div class="pros-cons">
        <div>
          <h4>Advantages</h4>
          <ul class="pros">${pros}</ul>
        </div>
        <div>
          <h4>Trade-offs</h4>
          <ul class="cons">${cons}</ul>
        </div>
      </div>

      <p class="panel__hint">
        Per-hour at mean ladder bitrate: ${fmtMb(insights.delivery_per_hour_optimized_mb)} optimized vs
        ${fmtMb(insights.delivery_per_hour_reference_mb)} fixed.
        Reference preset: 360p@600k · 480p@1M · 720p@2.4M · 1080p@4.5M (+128k audio each).
      </p>
    </div>
  `;
}

export function mountInsights(insights) {
  const host = document.getElementById("insightsHost");
  if (!host) return;
  host.innerHTML = renderInsightsPanel(insights);
  host.hidden = !insights;
}
