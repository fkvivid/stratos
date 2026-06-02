const dz = document.getElementById("dropzone");
const fi = document.getElementById("fileInput");
const st = document.getElementById("status");
const stTxt = document.getElementById("statusText");
const bar = document.getElementById("bar");
const meta = document.getElementById("fileMeta");
const pickBtn = document.getElementById("pickBtn");

function formatBytes(n) {
  if (n < 1024) return `${n} B`;
  if (n < 1024 ** 2) return `${(n / 1024).toFixed(1)} KB`;
  return `${(n / 1024 ** 2).toFixed(1)} MB`;
}

function showFile(file) {
  meta.textContent = `${file.name} · ${formatBytes(file.size)}`;
  meta.classList.add("visible");
}

dz.addEventListener("click", (e) => {
  if (e.target === pickBtn) return;
  fi.click();
});
pickBtn?.addEventListener("click", (e) => {
  e.stopPropagation();
  fi.click();
});

fi.addEventListener("change", () => {
  const file = fi.files?.[0];
  if (file) {
    showFile(file);
    upload(file);
  }
});

dz.addEventListener("dragover", (e) => {
  e.preventDefault();
  dz.classList.add("drag");
});
dz.addEventListener("dragleave", () => dz.classList.remove("drag"));
dz.addEventListener("drop", (e) => {
  e.preventDefault();
  dz.classList.remove("drag");
  const file = e.dataTransfer.files?.[0];
  if (file?.type.startsWith("video/")) {
    showFile(file);
    upload(file);
  } else {
    st.classList.add("visible");
    stTxt.textContent = "Please drop a video file.";
    document.getElementById("progressWrap").hidden = true;
  }
});

function upload(file) {
  const form = new FormData();
  form.append("video", file);
  st.classList.add("visible");
  document.getElementById("progressWrap").hidden = false;
  stTxt.textContent = `Uploading ${file.name}…`;
  bar.style.width = "0%";
  dz.style.pointerEvents = "none";

  const xhr = new XMLHttpRequest();
  xhr.open("POST", "/api/upload");
  xhr.upload.onprogress = (e) => {
    if (e.lengthComputable) bar.style.width = `${(e.loaded / e.total) * 100}%`;
  };
  xhr.onload = () => {
    dz.style.pointerEvents = "";
    if (xhr.status === 200) {
      const { job_id } = JSON.parse(xhr.responseText);
      stTxt.textContent = "Starting pipeline…";
      window.location.href = `/player.html?job=${encodeURIComponent(job_id)}`;
    } else if (xhr.status === 413) {
      stTxt.textContent = "File too large (max 4 GiB)";
      bar.style.background = "var(--danger)";
    } else {
      stTxt.textContent = xhr.responseText || `Upload failed (${xhr.status})`;
      bar.style.background = "var(--danger)";
    }
  };
  xhr.onerror = () => {
    dz.style.pointerEvents = "";
    stTxt.textContent = "Network error — is the server running?";
  };
  xhr.send(form);
}
