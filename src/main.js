// Webcam capture with a countdown self-timer. Pose your body, count down, snap.
// The captured frames are usable as generation references.

const video = document.getElementById("video");
const startBtn = document.getElementById("start");
const shootBtn = document.getElementById("shoot");
const deviceSel = document.getElementById("device");
const secsSel = document.getElementById("secs");
const burstSel = document.getElementById("burst");
const mirrorChk = document.getElementById("mirror");
const countEl = document.getElementById("count");
const flashEl = document.getElementById("flash");
const idleEl = document.getElementById("idle");
const gallery = document.getElementById("gallery");
const errEl = document.getElementById("err");

const canvas = document.createElement("canvas");
let stream = null;
let shooting = false;
let shotCount = 0;

const sleep = (ms) => new Promise((r) => setTimeout(r, ms));

function showError(e) {
  errEl.textContent = String(e && e.message ? e.message : e);
}

mirrorChk.addEventListener("change", () => {
  video.classList.toggle("mirror", mirrorChk.checked);
});

async function listDevices() {
  try {
    const devices = await navigator.mediaDevices.enumerateDevices();
    const cams = devices.filter((d) => d.kind === "videoinput");
    deviceSel.innerHTML = "";
    cams.forEach((c, i) => {
      const o = document.createElement("option");
      o.value = c.deviceId;
      o.textContent = c.label || `カメラ ${i + 1}`;
      deviceSel.appendChild(o);
    });
  } catch (e) {
    showError(e);
  }
}

async function startCamera(deviceId) {
  try {
    showError("");
    if (stream) stream.getTracks().forEach((t) => t.stop());
    const constraints = {
      audio: false,
      video: deviceId
        ? { deviceId: { exact: deviceId } }
        : { width: { ideal: 1280 }, height: { ideal: 720 } },
    };
    stream = await navigator.mediaDevices.getUserMedia(constraints);
    video.srcObject = stream;
    await video.play();
    idleEl.style.display = "none";
    shootBtn.disabled = false;
    startBtn.textContent = "カメラ再起動";
    await listDevices(); // labels become available after permission
  } catch (e) {
    showError(e);
  }
}

startBtn.addEventListener("click", () => startCamera(deviceSel.value || undefined));
deviceSel.addEventListener("change", () => startCamera(deviceSel.value));

async function countdown(secs) {
  if (secs <= 0) return;
  countEl.style.display = "flex";
  for (let n = secs; n > 0; n--) {
    countEl.textContent = String(n);
    await sleep(1000);
  }
  countEl.style.display = "none";
}

async function flash() {
  flashEl.style.transition = "none";
  flashEl.style.opacity = "0.9";
  await sleep(30);
  flashEl.style.transition = "opacity .35s";
  flashEl.style.opacity = "0";
}

function capture() {
  const w = video.videoWidth;
  const h = video.videoHeight;
  if (!w || !h) return null;
  canvas.width = w;
  canvas.height = h;
  const ctx = canvas.getContext("2d");
  if (mirrorChk.checked) {
    ctx.translate(w, 0);
    ctx.scale(-1, 1);
  }
  ctx.drawImage(video, 0, 0, w, h);
  return canvas.toDataURL("image/png");
}

const IS_TAURI = !!window.__TAURI_INTERNALS__;

function addShot(dataUrl) {
  shotCount++;
  const n = shotCount;
  const div = document.createElement("div");
  div.className = "shot";
  const img = document.createElement("img");
  img.src = dataUrl;
  div.append(img);

  if (IS_TAURI) {
    // Native save: <a download> doesn't work in the webview.
    const btn = document.createElement("button");
    btn.textContent = "保存…";
    btn.style.width = "100%";
    btn.addEventListener("click", async () => {
      try {
        const [{ save }, { invoke }] = await Promise.all([
          import("@tauri-apps/plugin-dialog"),
          import("@tauri-apps/api/core"),
        ]);
        // Ask the user where to save.
        const path = await save({
          defaultPath: `pose-${n}.png`,
          filters: [{ name: "PNG", extensions: ["png"] }],
        });
        if (!path) return; // cancelled
        await invoke("write_capture", { path, dataUrl });
        btn.textContent = "保存済 ✓";
        const p = document.createElement("div");
        p.style.cssText = "font-size:10px;opacity:.6;word-break:break-all;margin-top:2px";
        p.textContent = path;
        div.appendChild(p);
      } catch (e) {
        showError(e);
      }
    });
    div.append(btn);
  } else {
    const a = document.createElement("a");
    a.href = dataUrl;
    a.download = `pose-${n}.png`;
    a.textContent = "保存";
    div.append(a);
  }
  gallery.prepend(div);
}

shootBtn.addEventListener("click", async () => {
  if (shooting || !stream) return;
  shooting = true;
  shootBtn.disabled = true;
  startBtn.disabled = true;
  try {
    await countdown(Number(secsSel.value));
    const shots = Number(burstSel.value);
    for (let i = 0; i < shots; i++) {
      const url = capture();
      if (url) addShot(url);
      await flash();
      if (i < shots - 1) await sleep(500);
    }
  } catch (e) {
    showError(e);
  } finally {
    shooting = false;
    shootBtn.disabled = false;
    startBtn.disabled = false;
  }
});

// ---------------------------------------------------------------------------
// MCP bridge: when running inside Tauri with --mcp, the Rust side emits
// "mcp-snap"; we count down, capture, and deliver the frame back.
// ---------------------------------------------------------------------------
async function setupTauriMcpBridge() {
  if (!window.__TAURI_INTERNALS__) return; // plain browser: nothing to do
  const [{ listen }, { invoke }] = await Promise.all([
    import("@tauri-apps/api/event"),
    import("@tauri-apps/api/core"),
  ]);
  await listen("mcp-snap", async (ev) => {
    const { id, countdown: secs } = ev.payload || {};
    try {
      if (!stream) await startCamera();
      let tries = 0;
      while ((!video.videoWidth || !video.videoHeight) && tries++ < 40) {
        await sleep(100);
      }
      await countdown(Number(secs) || 0);
      const url = capture();
      if (url) addShot(url);
      await flash();
      await invoke("deliver_capture", { id, dataUrl: url || "" });
    } catch (e) {
      showError(e);
      try {
        await invoke("deliver_capture", { id, dataUrl: "" });
      } catch {}
    }
  });

  // Start the camera up front so the permission prompt is handled before a
  // snap is requested, then tell Rust we're ready to receive snap events.
  try {
    await startCamera();
  } catch {}
  await invoke("frontend_ready");
}
setupTauriMcpBridge();

// ---------------------------------------------------------------------------
// Settings page: network (HTTP) MCP server config.
// ---------------------------------------------------------------------------
(function setupSettings() {
  const $ = (id) => document.getElementById(id);
  const overlay = $("settingsOverlay");
  const enabledEl = $("setEnabled");
  const portEl = $("setPort");
  const lanEl = $("setLan");
  const tokenEl = $("setToken");
  const statusEl = $("serverStatus");
  const connInfo = $("connInfo");
  const connUrl = $("connUrl");
  const connSnippet = $("connSnippet");
  const setErr = $("setErr");

  async function inv(cmd, args) {
    const { invoke } = await import("@tauri-apps/api/core");
    return invoke(cmd, args);
  }

  const current = () => ({
    http_enabled: enabledEl.checked,
    port: Number(portEl.value) || 8790,
    token: tokenEl.value.trim(),
    lan: lanEl.value === "true",
  });

  const snippet = (url, token) =>
    JSON.stringify(
      { mcpServers: { "jidori-kun": { url, headers: { Authorization: `Bearer ${token}` } } } },
      null,
      2,
    );

  async function refreshStatus() {
    try {
      const running = await inv("http_status");
      statusEl.textContent = running ? "● 起動中" : "停止中";
      statusEl.style.color = running ? "#2ecc71" : "";
      if (running) {
        const ip = (await inv("local_ip")) || "127.0.0.1";
        const host = lanEl.value === "true" ? ip : "127.0.0.1";
        const url = `http://${host}:${Number(portEl.value) || 8790}/mcp`;
        connUrl.textContent = url;
        connSnippet.value = snippet(url, tokenEl.value.trim());
        connInfo.hidden = false;
      }
    } catch {}
  }

  async function load() {
    setErr.textContent = "";
    if (!IS_TAURI) {
      setErr.textContent = "ネットワーク MCP はデスクトップ版でのみ使えます。";
      return;
    }
    try {
      const s = await inv("get_settings");
      enabledEl.checked = s.http_enabled;
      portEl.value = s.port;
      lanEl.value = String(s.lan);
      tokenEl.value = s.token || "";
      await refreshStatus();
    } catch (e) {
      setErr.textContent = String(e);
    }
  }

  $("openSettings").addEventListener("click", async () => {
    overlay.hidden = false;
    await load();
  });
  $("closeSettings").addEventListener("click", () => (overlay.hidden = true));
  overlay.addEventListener("click", (e) => {
    if (e.target === overlay) overlay.hidden = true;
  });
  $("genToken").addEventListener("click", () => {
    tokenEl.value = (crypto.randomUUID?.() || Date.now().toString(36)).replace(/-/g, "");
  });
  $("saveSettings").addEventListener("click", async () => {
    setErr.textContent = "";
    try {
      await inv("save_settings", { new: current() });
      const b = $("saveSettings");
      b.textContent = "保存済 ✓";
      setTimeout(() => (b.textContent = "保存"), 1200);
    } catch (e) {
      setErr.textContent = String(e);
    }
  });
  $("startServer").addEventListener("click", async () => {
    setErr.textContent = "";
    if (!tokenEl.value.trim()) {
      setErr.textContent = "トークンを設定してください。";
      return;
    }
    try {
      await inv("save_settings", { new: current() });
      const url = await inv("start_http");
      connUrl.textContent = url;
      connSnippet.value = snippet(url, tokenEl.value.trim());
      connInfo.hidden = false;
      await refreshStatus();
    } catch (e) {
      setErr.textContent = String(e);
    }
  });
  $("stopServer").addEventListener("click", async () => {
    try {
      await inv("stop_http");
      await refreshStatus();
    } catch (e) {
      setErr.textContent = String(e);
    }
  });
})();

// ---------------------------------------------------------------------------
// Manual update: fetch a small version manifest and NOTIFY only. Downloading
// and installing is left to the user (no auto-download, no self-replace).
// ---------------------------------------------------------------------------
// Point this at your hosted manifest, e.g. a GitHub release asset:
//   { "version": "0.2.0", "url": "https://.../jidori-kun_0.2.0_x64-setup.exe",
//     "notes_url": "https://.../releases/tag/v0.2.0" }
const UPDATE_MANIFEST_URL =
  "https://raw.githubusercontent.com/yatmita/jidori-kun/main/latest.json";

function cmpVer(a, b) {
  const pa = String(a).split("."), pb = String(b).split(".");
  for (let i = 0; i < 3; i++) {
    const x = Number(pa[i] || 0), y = Number(pb[i] || 0);
    if (x > y) return 1;
    if (x < y) return -1;
  }
  return 0;
}

async function checkUpdate(manual) {
  if (!IS_TAURI) return;
  const msg = document.getElementById("updMsg");
  if (manual && msg) msg.textContent = "確認中…";
  try {
    const [{ getVersion }, { invoke }] = await Promise.all([
      import("@tauri-apps/api/app"),
      import("@tauri-apps/api/core"),
    ]);
    const current = await getVersion();
    const res = await fetch(UPDATE_MANIFEST_URL, { cache: "no-store" });
    if (!res.ok) throw new Error("manifest " + res.status);
    const m = await res.json();
    if (cmpVer(m.version, current) > 0) {
      const banner = document.getElementById("updateBanner");
      document.getElementById("updVer").textContent = m.version;
      document.getElementById("updDownload").onclick = () =>
        invoke("open_url", { url: m.url });
      const notes = document.getElementById("updNotes");
      if (m.notes_url) {
        notes.hidden = false;
        notes.onclick = () => invoke("open_url", { url: m.notes_url });
      }
      document.getElementById("updDismiss").onclick = () => (banner.hidden = true);
      banner.hidden = false;
      if (manual && msg) msg.textContent = "新バージョン " + m.version + " あり";
    } else if (manual && msg) {
      msg.textContent = "最新版です（v" + current + "）";
    }
  } catch (e) {
    if (manual && msg) msg.textContent = "確認に失敗: " + e;
  }
}

const chkBtn = document.getElementById("checkUpdate");
if (chkBtn) chkBtn.addEventListener("click", () => checkUpdate(true));
// Check on launch (version info only — never downloads a binary).
checkUpdate(false);

// Signals for the headless smoke test.
window.__capture = () => shootBtn.click();
window.__shots = () => gallery.querySelectorAll(".shot").length;
