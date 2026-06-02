/**
 * Stratos HLS player — manual quality selection, ABR auto mode, custom controls.
 */
export class StratosPlayer {
  constructor(root, src) {
    this.root = root;
    this.src = src;
    this.hls = null;
    this.levels = [];
    this.manualLevel = -1;
    this.ready = false;

    this.root.innerHTML = `
      <div class="video-shell video-shell--paused" id="shell">
        <video id="video" playsinline preload="metadata"></video>
        <div class="player-placeholder" id="placeholder" hidden>
          <div class="spinner"></div>
          <span id="placeholderMsg">Loading stream…</span>
        </div>
        <div class="video-controls" id="controls" hidden>
          <div class="video-controls__row">
            <input type="range" class="seek" id="seek" min="0" max="1000" value="0" aria-label="Seek" />
          </div>
          <div class="video-controls__row">
            <button type="button" class="ctrl-btn" id="playBtn" aria-label="Play">
              <svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>
            </button>
            <span class="time" id="time">0:00 / 0:00</span>
            <input type="range" class="volume" id="volume" min="0" max="1" step="0.05" value="1" aria-label="Volume" />
            <button type="button" class="ctrl-btn" id="muteBtn" aria-label="Mute">
              <svg viewBox="0 0 24 24" fill="currentColor"><path d="M3 9v6h4l5 5V4L7 9H3zm13.5 3c0-1.77-1.02-3.29-2.5-4.03v8.05c1.48-.73 2.5-2.25 2.5-4.02z"/></svg>
            </button>
            <div class="quality-wrap">
              <button type="button" class="quality-btn" id="qualityBtn" aria-expanded="false" aria-haspopup="listbox">
                <span id="qualityLabel">Quality</span>
                <svg width="12" height="12" viewBox="0 0 24 24" fill="currentColor"><path d="M7 10l5 5 5-5z"/></svg>
              </button>
              <div class="quality-menu" id="qualityMenu" role="listbox"></div>
            </div>
            <button type="button" class="ctrl-btn" id="fsBtn" aria-label="Fullscreen">
              <svg viewBox="0 0 24 24" fill="currentColor"><path d="M7 14H5v5h5v-2H7v-3zm-2-4h2V7h3V5H5v5zm12 7h-3v2h5v-5h-2v3zM14 5v2h3v3h2V5h-5z"/></svg>
            </button>
          </div>
        </div>
      </div>
      <div class="stream-stats" id="stats" hidden></div>
    `;

    this.shell = this.root.querySelector("#shell");
    this.video = this.root.querySelector("#video");
    this.placeholder = this.root.querySelector("#placeholder");
    this.controls = this.root.querySelector("#controls");
    this.seek = this.root.querySelector("#seek");
    this.timeEl = this.root.querySelector("#time");
    this.volume = this.root.querySelector("#volume");
    this.qualityBtn = this.root.querySelector("#qualityBtn");
    this.qualityMenu = this.root.querySelector("#qualityMenu");
    this.qualityLabel = this.root.querySelector("#qualityLabel");
    this.stats = this.root.querySelector("#stats");

    this._bindControls();
  }

  load() {
    if (!window.Hls) {
      this._fail("hls.js failed to load");
      return;
    }

    this.placeholder.hidden = false;
    this.controls.hidden = true;
    this.stats.hidden = true;
    this.root.querySelector("#placeholderMsg").textContent = "Loading stream…";

    if (window.Hls.isSupported()) {
      this.hls = new window.Hls({
        enableWorker: true,
        capLevelToPlayerSize: false,
        startLevel: -1,
        maxBufferLength: 30,
        manifestLoadingMaxRetry: 6,
        manifestLoadingRetryDelay: 1000,
      });

      this.hls.on(window.Hls.Events.MANIFEST_PARSED, () => {
        this._refreshLevels();
        this._buildQualityMenu();
      });

      this.hls.on(window.Hls.Events.LEVEL_LOADED, () => {
        this._refreshLevels();
      });

      this.hls.on(window.Hls.Events.LEVEL_SWITCHED, (_e, data) => {
        this._updateQualityLabel(data.level);
        this._updateStats();
      });

      this.hls.on(window.Hls.Events.ERROR, (_e, data) => {
        if (data.fatal) {
          if (data.type === window.Hls.ErrorTypes.NETWORK_ERROR) {
            this.hls.startLoad();
          } else {
            this._fail("Playback error — try refreshing");
          }
        }
      });

      this.hls.loadSource(this.src);
      this.hls.attachMedia(this.video);
    } else if (this.video.canPlayType("application/vnd.apple.mpegurl")) {
      this.shell.classList.add("video-shell--native");
      this.video.src = this.src;
      this.video.addEventListener("loadedmetadata", () => this._onReady());
    } else {
      this._fail("HLS not supported in this browser");
    }
  }

  _onReady() {
    if (this.ready) return;
    this.ready = true;
    this.placeholder.hidden = true;
    this.controls.hidden = false;
    this.stats.hidden = false;
    this.shell.classList.remove("video-shell--paused");

    if (!this.hls) {
      this.qualityBtn.disabled = true;
      this.qualityLabel.textContent = "Safari";
    }

    this._updateStats();
    this._autoplay();
  }

  _autoplay() {
    this.video.play().catch(() => {
      this.video.muted = true;
      this.video.play().finally(() => {
        setTimeout(() => {
          this.video.muted = false;
        }, 300);
      });
    });
  }

  _refreshLevels() {
    if (!this.hls) return;
    this.levels = this.hls.levels.map((level, index) => ({
      index,
      height: level.height,
      width: level.width,
      bitrate: level.bitrate,
      name: level.name || level.attrs?.NAME || `${level.height || "?"}p`,
    }));
    this.levels.sort((a, b) => (a.height || 0) - (b.height || 0));
  }

  _buildQualityMenu() {
    this.qualityMenu.innerHTML = "";

    const auto = document.createElement("button");
    auto.type = "button";
    auto.className = "quality-menu__item active";
    auto.dataset.level = "-1";
    auto.innerHTML = `<span>Auto</span><small>ABR</small>`;
    auto.addEventListener("click", () => this.setQuality(-1));
    this.qualityMenu.appendChild(auto);

    for (const level of this.levels) {
      const btn = document.createElement("button");
      btn.type = "button";
      btn.className = "quality-menu__item";
      btn.dataset.level = String(level.index);
      const mbps = level.bitrate ? `${(level.bitrate / 1e6).toFixed(1)} Mbps` : "";
      btn.innerHTML = `<span>${level.name}</span><small>${level.height}p · ${mbps}</small>`;
      btn.addEventListener("click", () => this.setQuality(level.index));
      this.qualityMenu.appendChild(btn);
    }
  }

  setQuality(levelIndex) {
    if (!this.hls) return;
    this.manualLevel = levelIndex;
    this.hls.currentLevel = levelIndex;
    if (levelIndex >= 0) {
      this.hls.loadLevel = levelIndex;
    }

    this.qualityMenu.querySelectorAll(".quality-menu__item").forEach((el) => {
      el.classList.toggle("active", Number(el.dataset.level) === levelIndex);
    });

    this._updateQualityLabel(levelIndex >= 0 ? levelIndex : this.hls.currentLevel);
    this._closeQualityMenu();
  }

  _updateQualityLabel(levelIndex) {
    if (!this.hls) return;
    if (this.manualLevel < 0) {
      this.qualityLabel.textContent = "Auto";
      return;
    }
    const level = this.hls.levels[levelIndex];
    if (!level) return;
    const name = level.name || level.attrs?.NAME || `${level.height}p`;
    this.qualityLabel.textContent = name;
  }

  _updateStats() {
    if (!this.hls || !this.hls.levels.length) return;
    const idx = this.hls.currentLevel;
    const level = idx >= 0 ? this.hls.levels[idx] : null;
    const autoLabel = this.manualLevel < 0 ? "ABR" : "Manual";
    const parts = [autoLabel];
    if (level) {
      if (level.height) parts.push(`${level.width}×${level.height}`);
      if (level.bitrate) parts.push(`${Math.round(level.bitrate / 1000)} kbps`);
    }
    this.stats.innerHTML = parts.map((p) => `<span>${p}</span>`).join("");
  }

  _bindControls() {
    const playBtn = this.root.querySelector("#playBtn");
    const muteBtn = this.root.querySelector("#muteBtn");
    const fsBtn = this.root.querySelector("#fsBtn");

    playBtn.addEventListener("click", () => this._togglePlay());
    this.video.addEventListener("click", () => this._togglePlay());
    this.video.addEventListener("loadeddata", () => this._onReady());
    this.video.addEventListener("canplay", () => this._onReady());
    this.video.addEventListener("play", () => {
      this.shell.classList.remove("video-shell--paused");
      playBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M6 4h4v16H6V4zm8 0h4v16h-4V4z"/></svg>';
    });
    this.video.addEventListener("pause", () => {
      this.shell.classList.add("video-shell--paused");
      playBtn.innerHTML = '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M8 5v14l11-7z"/></svg>';
    });

    this.seek.addEventListener("input", () => {
      const t = (this.seek.value / 1000) * (this.video.duration || 0);
      this.timeEl.textContent = `${fmt(t)} / ${fmt(this.video.duration || 0)}`;
    });
    this.seek.addEventListener("change", () => {
      this.video.currentTime = (this.seek.value / 1000) * (this.video.duration || 0);
    });
    this.video.addEventListener("timeupdate", () => {
      if (!this.video.duration) return;
      this.seek.value = (this.video.currentTime / this.video.duration) * 1000;
      this.timeEl.textContent = `${fmt(this.video.currentTime)} / ${fmt(this.video.duration)}`;
    });

    this.volume.addEventListener("input", () => {
      this.video.volume = this.volume.value;
      this.video.muted = this.volume.value === "0";
    });
    muteBtn.addEventListener("click", () => {
      this.video.muted = !this.video.muted;
      if (!this.video.muted && this.video.volume === 0) this.video.volume = 1;
      this.volume.value = this.video.muted ? 0 : this.video.volume;
    });

    this.qualityBtn.addEventListener("click", (e) => {
      e.stopPropagation();
      const open = this.qualityMenu.classList.toggle("open");
      this.qualityBtn.setAttribute("aria-expanded", open);
    });
    document.addEventListener("click", () => this._closeQualityMenu());

    fsBtn.addEventListener("click", () => {
      if (document.fullscreenElement) document.exitFullscreen();
      else this.shell.requestFullscreen?.();
    });

    let hideTimer;
    this.shell.addEventListener("mousemove", () => {
      this.shell.classList.add("controls-visible");
      clearTimeout(hideTimer);
      hideTimer = setTimeout(() => this.shell.classList.remove("controls-visible"), 2500);
    });
  }

  _togglePlay() {
    if (this.video.paused) this.video.play();
    else this.video.pause();
  }

  _closeQualityMenu() {
    this.qualityMenu.classList.remove("open");
    this.qualityBtn.setAttribute("aria-expanded", "false");
  }

  _fail(msg) {
    this.placeholder.innerHTML = `<span style="color:var(--danger)">${msg}</span>`;
    this.placeholder.hidden = false;
  }
}

function fmt(sec) {
  if (!Number.isFinite(sec)) return "0:00";
  const m = Math.floor(sec / 60);
  const s = Math.floor(sec % 60);
  return `${m}:${s.toString().padStart(2, "0")}`;
}
