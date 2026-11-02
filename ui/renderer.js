const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const state = {
  lang: "en",
  strings: null,
  config: {
    photosDir: "",
    lastRating: "1",
    apiBase: "",
    apiKind: "custom",
    appVersion: "",
    buildProfile: "",
  },
  mock: false,
  auth: { connected: false, email: null },
  galleries: [],
  activeTab: "sync",
  currentGalleryId: null,
  rateFiles: [],
  rateRating: "1",
  rateBusy: false,
  syncBusy: false,
  progress: null,
  report: null,
  pendingDeletes: new Set(),
};

const els = {
  tabs: document.getElementById("tabs"),
  tabButtons: Array.from(document.querySelectorAll(".tab")),
  envBadge: document.getElementById("envBadge"),
  accountChip: document.getElementById("accountChip"),
  btnConnect: document.getElementById("btnConnect"),
  btnConnectEmpty: document.getElementById("btnConnectEmpty"),
  btnLogout: document.getElementById("btnLogout"),
  btnInfo: document.getElementById("btnInfo"),
  infoModal: document.getElementById("infoModal"),
  infoTitle: document.getElementById("infoTitle"),
  infoVersionLabel: document.getElementById("infoVersionLabel"),
  infoProfileLabel: document.getElementById("infoProfileLabel"),
  infoModeLabel: document.getElementById("infoModeLabel"),
  infoTargetLabel: document.getElementById("infoTargetLabel"),
  infoVersion: document.getElementById("infoVersion"),
  infoProfile: document.getElementById("infoProfile"),
  infoMode: document.getElementById("infoMode"),
  infoTarget: document.getElementById("infoTarget"),
  btnInfoClose: document.getElementById("btnInfoClose"),
  updateBanner: document.getElementById("updateBanner"),
  updateMsg: document.getElementById("updateMsg"),
  btnUpdate: document.getElementById("btnUpdate"),

  syncTitle: document.getElementById("syncTitle"),
  syncHint: document.getElementById("syncHint"),
  mockBadge: document.getElementById("mockBadge"),
  syncEmpty: document.getElementById("syncEmpty"),
  emptyTitle: document.getElementById("emptyTitle"),
  emptyHint: document.getElementById("emptyHint"),
  btnDemo: document.getElementById("btnDemo"),
  galleryList: document.getElementById("galleryList"),

  syncDetail: document.getElementById("syncDetail"),
  btnBack: document.getElementById("btnBack"),
  detailName: document.getElementById("detailName"),
  detailPath: document.getElementById("detailPath"),
  btnUpdateNow: document.getElementById("btnUpdateNow"),
  btnChangeFolder: document.getElementById("btnChangeFolder"),
  btnOpenFolder: document.getElementById("btnOpenFolder"),

  progressWrap: document.getElementById("progressWrap"),
  progressFill: document.getElementById("progressFill"),
  progressText: document.getElementById("progressText"),

  report: document.getElementById("report"),
  reportSummary: document.getElementById("reportSummary"),
  reportCounts: document.getElementById("reportCounts"),
  reportErrors: document.getElementById("reportErrors"),
  deleteBox: document.getElementById("deleteBox"),
  deleteTitle: document.getElementById("deleteTitle"),
  deleteHint: document.getElementById("deleteHint"),
  deleteList: document.getElementById("deleteList"),
  btnDeleteRemote: document.getElementById("btnDeleteRemote"),
  btnCancelDelete: document.getElementById("btnCancelDelete"),

  rateTitle: document.getElementById("rateTitle"),
  rateHint: document.getElementById("rateHint"),
  folderLabel: document.getElementById("folderLabel"),
  pathBox: document.getElementById("pathBox"),
  btnChooseFolder: document.getElementById("btnChooseFolder"),
  infoRow: document.getElementById("infoRow"),
  fileCount: document.getElementById("fileCount"),
  photosLabel: document.getElementById("photosLabel"),
  ratingLabel: document.getElementById("ratingLabel"),
  ratingVal: document.getElementById("ratingVal"),
  btnRate: document.getElementById("btnRate"),
  spinner: document.getElementById("spinner"),
  results: document.getElementById("results"),
  resultSummary: document.getElementById("resultSummary"),
  detailList: document.getElementById("detailList"),
  starButtons: Array.from(document.querySelectorAll(".star-btn")),
};

for (const [key, el] of Object.entries(els)) {
  if (Array.isArray(el)) {
    if (el.length === 0) throw new Error(`Missing DOM element: ${key}`);
  } else if (!el) {
    throw new Error(`Missing DOM element: ${key}`);
  }
}

function fmt(template, vars) {
  return String(template || "").replace(/\{(\w+)\}/g, (match, key) =>
    key in vars ? String(vars[key]) : match,
  );
}

function normalizeRating(value) {
  const v = String(value ?? "").trim();
  return ["1", "2", "3", "4", "5"].includes(v) ? v : "1";
}

function show(el) {
  el.classList.remove("hidden");
}

function hide(el) {
  el.classList.add("hidden");
}

function showTab(tab) {
  state.activeTab = tab;
  for (const button of els.tabButtons) {
    button.classList.toggle("active", button.dataset.tab === tab);
  }
  document
    .getElementById("tab-sync")
    .classList.toggle("hidden", tab !== "sync");
  document
    .getElementById("tab-rate")
    .classList.toggle("hidden", tab !== "rate");
}

function formatDate(value) {
  if (!value) return state.strings?.never || "never";
  try {
    return new Date(value).toLocaleString(state.lang === "es" ? "es-ES" : "en-US", {
      dateStyle: "medium",
      timeStyle: "short",
    });
  } catch {
    return String(value);
  }
}

function applyStrings() {
  const s = state.strings;
  if (!s) return;

  els.tabButtons[0].textContent = s.syncTab;
  els.tabButtons[1].textContent = s.rateTab;
  els.btnConnect.textContent = s.connect;
  els.btnConnectEmpty.textContent = s.connect;
  els.btnLogout.textContent = s.logout;
  els.btnInfo.textContent = s.info;
  els.infoTitle.textContent = s.infoTitle;
  els.infoVersionLabel.textContent = s.infoVersion;
  els.infoProfileLabel.textContent = s.infoProfile;
  els.infoModeLabel.textContent = s.infoMode;
  els.infoTargetLabel.textContent = s.infoTarget;
  els.btnInfoClose.textContent = s.infoClose;
  els.btnUpdate.textContent = s.restartUpdate;
  els.mockBadge.textContent = s.mockMode;
  els.syncTitle.textContent = s.syncTab;
  els.syncHint.textContent = state.mock ? s.demoHint : "";
  els.emptyTitle.textContent = s.noGalleries;
  els.emptyHint.textContent = s.noGalleriesHint;
  els.btnDemo.textContent = s.syncDemo;
  els.btnBack.textContent = `← ${s.back}`;
  els.btnUpdateNow.textContent = s.updateNow;
  els.btnChangeFolder.textContent = s.changeFolder;
  els.btnOpenFolder.textContent = s.openFolder;
  els.btnDeleteRemote.textContent = s.confirmDelete;
  els.btnCancelDelete.textContent = s.cancel;
  els.deleteHint.textContent = s.deleteRemoteHint;

  els.rateTitle.textContent = s.rateTab;
  els.folderLabel.textContent = s.folderLabel;
  els.btnChooseFolder.textContent = s.changeBtn;
  els.btnRate.textContent = s.applyBtn;
  els.photosLabel.textContent = s.photos;
  els.ratingLabel.textContent = s.rating;
  els.spinner.textContent = s.applying;

  els.mockBadge.classList.toggle("hidden", !state.mock);
  els.btnDemo.classList.toggle("hidden", !state.mock);
}

function renderAccount() {
  const s = state.strings;
  if (!s) return;

  if (state.auth.connected) {
    show(els.accountChip);
    els.accountChip.textContent = state.auth.email || "Picstome";
    els.accountChip.title = `${s.connectedAs} ${
      state.auth.email || "Picstome"
    }`;
    show(els.btnLogout);
    hide(els.btnConnect);
    hide(els.btnConnectEmpty);
  } else {
    hide(els.accountChip);
    hide(els.btnLogout);
    show(els.btnConnect);
    show(els.btnConnectEmpty);
  }
}

function modeLabel(kind) {
  const s = state.strings || {};
  if (kind === "production") return s.modeProduction || "Production";
  if (kind === "development") return s.modeDevelopment || "Development";
  if (kind === "local") return s.modeLocal || "Local";
  if (kind === "demo") return s.modeDemo || "Demo";
  return s.modeCustom || "Custom";
}

function renderRuntimeInfo() {
  const config = state.config || {};
  const kind = config.apiKind || "custom";
  const label = modeLabel(kind);
  const badge =
    kind === "production"
      ? ""
      : kind === "development"
        ? "DEV"
        : kind === "local"
          ? "LOCAL"
          : kind === "demo"
            ? "DEMO"
            : "CUSTOM";

  els.envBadge.textContent = badge;
  els.envBadge.classList.toggle("hidden", badge === "");
  els.envBadge.title = `${state.strings?.infoTarget || "API"}: ${
    config.apiBase || ""
  }`;

  els.infoVersion.textContent = `${config.appVersion || "?"} (${
    config.buildProfile || "?"
  })`;
  els.infoProfile.textContent = config.buildProfile || "?";
  els.infoMode.textContent = label;
  els.infoTarget.textContent = config.apiBase || "?";
}

function renderTabs() {
  showTab(state.activeTab);
}

function renderGalleryList() {
  const s = state.strings;
  els.galleryList.replaceChildren();
  els.syncEmpty.classList.toggle("hidden", state.galleries.length > 0);

  if (state.galleries.length === 0) return;

  for (const gallery of state.galleries) {
    const row = document.createElement("div");
    row.className = "gallery-row";
    row.dataset.galleryId = gallery.galleryId;

    const main = document.createElement("div");
    main.className = "gallery-main";

    const avatar = document.createElement("div");
    avatar.className = "gallery-avatar";
    avatar.textContent = (gallery.name || "?").trim().charAt(0).toUpperCase() || "?";
    main.appendChild(avatar);

    const info = document.createElement("div");
    info.className = "gallery-info";

    const name = document.createElement("div");
    name.className = "gallery-name";
    name.textContent = gallery.name;
    info.appendChild(name);

    const meta = document.createElement("div");
    meta.className = "gallery-meta";
    const pathText = gallery.exists ? gallery.localPath : s.folderMissing;
    const last = `${s.lastSync}: ${formatDate(gallery.lastSyncAt)}`;
    meta.textContent = `${pathText} · ${last}`;
    if (!gallery.exists) meta.classList.add("warn");
    info.appendChild(meta);

    const actions = document.createElement("div");
    actions.className = "gallery-actions";

    const btnUpdate = document.createElement("button");
    btnUpdate.type = "button";
    btnUpdate.className = "small";
    btnUpdate.textContent = s.update;
    btnUpdate.dataset.action = "update";
    actions.appendChild(btnUpdate);

    if (gallery.exists) {
      const btnOpen = document.createElement("button");
      btnOpen.type = "button";
      btnOpen.className = "secondary small";
      btnOpen.textContent = s.openFolder;
      btnOpen.dataset.action = "open";
      actions.appendChild(btnOpen);
    } else {
      const btnLocate = document.createElement("button");
      btnLocate.type = "button";
      btnLocate.className = "secondary small";
      btnLocate.textContent = s.locate;
      btnLocate.dataset.action = "locate";
      actions.appendChild(btnLocate);
    }

    const btnUnlink = document.createElement("button");
    btnUnlink.type = "button";
    btnUnlink.className = "ghost small";
    btnUnlink.textContent = s.unlink;
    btnUnlink.dataset.action = "unlink";
    actions.appendChild(btnUnlink);

    main.appendChild(info);
    row.appendChild(main);
    row.appendChild(actions);
    els.galleryList.appendChild(row);
  }
}

function renderReport() {
  const s = state.strings;
  const report = state.report;
  if (!report || !s) {
    hide(els.report);
    return;
  }

  show(els.report);
  const counts = report.counts || {};
  const total =
    (counts.remoteNew || 0) +
    (counts.remoteModified || 0) +
    (counts.localNew || 0) +
    (counts.localModified || 0) +
    (counts.conflicts || 0);

  if (report.errors && report.errors.length > 0) {
    els.reportSummary.textContent = s.syncFailed;
    els.reportSummary.className = "report-summary error";
  } else if (total === 0 && (report.proposals?.deleteRemote?.length || 0) === 0) {
    els.reportSummary.textContent = s.inSync;
    els.reportSummary.className = "report-summary ok";
  } else {
    els.reportSummary.textContent = s.syncDone;
    els.reportSummary.className = "report-summary ok";
  }

  const chips = [
    [s.downloaded, (report.downloaded || []).length],
    [s.uploaded, (report.uploaded || []).length],
    [s.conflicts, (report.conflicts || []).length],
    [s.errors, (report.errors || []).length],
  ];
  els.reportCounts.replaceChildren();
  for (const [label, value] of chips) {
    const chip = document.createElement("span");
    chip.className = "count-chip";
    chip.textContent = `${label}: ${value}`;
    if (label === s.errors && value > 0) chip.classList.add("warn");
    els.reportCounts.appendChild(chip);
  }

  els.reportErrors.replaceChildren();
  for (const error of report.errors || []) {
    const line = document.createElement("div");
    line.className = "report-error";
    line.textContent = `${error.filename || error.path || "—"} — ${error.message}`;
    els.reportErrors.appendChild(line);
  }
  for (const conflict of report.conflicts || []) {
    const line = document.createElement("div");
    line.className = "report-error";
    line.textContent = `${conflict.local?.filename || conflict.remote?.filename} — ${s.conflicts}`;
    els.reportErrors.appendChild(line);
  }

  const proposals = report.proposals?.deleteRemote || [];
  state.pendingDeletes = new Set(proposals.map((photo) => photo.id));
  els.deleteList.replaceChildren();

  if (proposals.length === 0) {
    hide(els.deleteBox);
  } else {
    show(els.deleteBox);
    els.deleteTitle.textContent = `${s.pendingDeletes} (${proposals.length})`;
    for (const photo of proposals) {
      const label = document.createElement("label");
      label.className = "delete-item";
      const checkbox = document.createElement("input");
      checkbox.type = "checkbox";
      checkbox.checked = true;
      checkbox.dataset.photoId = photo.id;
      const text = document.createElement("span");
      text.textContent = photo.filename;
      label.appendChild(checkbox);
      label.appendChild(text);
      els.deleteList.appendChild(label);
    }
  }
}

function renderSync() {
  const s = state.strings;
  if (!s) return;

  const current = state.galleries.find(
    (gallery) => gallery.galleryId === state.currentGalleryId,
  );

  if (current) {
    hide(els.galleryList);
    hide(els.syncEmpty);
    show(els.syncDetail);
    els.detailName.textContent = current.name;
    els.detailPath.textContent = current.exists
      ? current.localPath
      : s.folderMissing;
    els.detailPath.classList.toggle("warn", !current.exists);
    els.btnUpdateNow.disabled = state.syncBusy || !current.exists;
    els.btnOpenFolder.disabled = !current.exists;
  } else {
    state.currentGalleryId = null;
    show(els.galleryList);
    show(els.syncEmpty);
    hide(els.syncDetail);
  }

  renderGalleryList();
  renderReport();
}

function renderProgress() {
  const s = state.strings;
  const progress = state.progress;
  if (!progress || !state.syncBusy) {
    hide(els.progressWrap);
    return;
  }

  show(els.progressWrap);
  let text = s.syncing;
  if (progress.phase === "scan" || progress.phase === "scanning") {
    text = s.scanning;
  } else if (progress.phase === "download") {
    text = fmt(s.downloadingProgress, {
      done: progress.done || 0,
      total: progress.total || 0,
    });
    if (progress.file) text += ` · ${progress.file}`;
  } else if (progress.phase === "upload") {
    text = fmt(s.uploadingProgress, {
      done: progress.done || 0,
      total: progress.total || 0,
    });
    if (progress.file) text += ` · ${progress.file}`;
  } else if (progress.phase === "manifest") {
    text = s.scanning;
  }
  els.progressText.textContent = text;

  let ratio = 0;
  if (progress.fileTotal && progress.fileDone) {
    ratio = progress.fileDone / progress.fileTotal;
  } else if (progress.total) {
    ratio = (progress.done || 0) / progress.total;
  }
  els.progressFill.style.width = `${Math.max(3, Math.min(100, ratio * 100))}%`;
}

function renderRate() {
  const s = state.strings;
  if (!s) return;

  if (state.config.photosDir) {
    els.pathBox.textContent = state.config.photosDir;
    els.pathBox.classList.remove("empty");
  } else {
    els.pathBox.textContent = s.noFolder;
    els.pathBox.classList.add("empty");
  }

  els.fileCount.textContent = String(state.rateFiles.length);
  els.infoRow.classList.toggle("hidden", state.rateFiles.length === 0);

  const current = Number(state.rateRating);
  for (const button of els.starButtons) {
    const active = Number(button.dataset.rating) <= current;
    button.classList.toggle("active", active);
    button.setAttribute("aria-pressed", active ? "true" : "false");
  }
  els.ratingVal.textContent = state.rateRating;

  els.btnRate.disabled =
    !state.config.photosDir || state.rateFiles.length === 0 || state.rateBusy;

  if (state.rateFiles.length > 0) {
    els.rateHint.textContent = state.config.photosDir
      ? fmt(s.ready, {
          n: state.rateFiles.length,
          s: state.rateFiles.length === 1 ? "" : "s",
          r: state.rateRating,
        })
      : s.needFolder;
  } else {
    els.rateHint.textContent = state.config.photosDir
      ? s.needFiles
      : s.waiting;
  }

  els.spinner.classList.toggle("hidden", !state.rateBusy);
}

function renderAll() {
  applyStrings();
  renderAccount();
  renderRuntimeInfo();
  renderTabs();
  renderSync();
  renderProgress();
  renderRate();
}

function setState(next) {
  if (!next) return;
  state.lang = next.lang || state.lang;
  state.strings = next.strings || state.strings;
  state.config = next.config || state.config;
  state.mock = Boolean(next.mock);
  state.auth = next.auth || state.auth;
  state.galleries = Array.isArray(next.galleries) ? next.galleries : [];
  renderAll();
}

async function refreshState() {
  const next = await invoke("get_state");
  setState(next);
  return next;
}

async function runSync(galleryId) {
  if (state.syncBusy) return;
  state.currentGalleryId = galleryId;
  state.syncBusy = true;
  state.report = null;
  state.progress = { phase: "scanning", done: 0, total: 1 };
  renderAll();

  try {
    state.report = await invoke("sync_gallery", { galleryId });
  } catch (err) {
    state.report = {
      counts: {},
      downloaded: [],
      uploaded: [],
      conflicts: [],
      errors: [{ filename: "—", message: err?.message || String(err) }],
      proposals: { deleteRemote: [] },
    };
    if (err?.code === "unauthorized") {
      show(els.updateBanner);
      els.updateMsg.textContent = state.strings.reconnect;
    }
  } finally {
    state.syncBusy = false;
    state.progress = null;
    await refreshState();
    state.report = state.report;
    renderAll();
  }
}

async function startSync(galleryId, name) {
  if (state.syncBusy) return;
  state.syncBusy = true;
  state.progress = { phase: "scanning", done: 0, total: 1 };
  state.report = null;
  renderAll();

  try {
    const result = await invoke("start_sync", { galleryId, name });
    if (result && result.report) {
      state.currentGalleryId = result.report.galleryId;
      state.report = result.report;
    } else if (result && !result.canceled) {
      state.report = {
        counts: {},
        downloaded: [],
        uploaded: [],
        conflicts: [],
        errors: [{ filename: "—", message: "Error" }],
        proposals: { deleteRemote: [] },
      };
    }
  } catch (err) {
    state.report = {
      counts: {},
      downloaded: [],
      uploaded: [],
      conflicts: [],
      errors: [{ filename: "—", message: err?.message || String(err) }],
      proposals: { deleteRemote: [] },
    };
  } finally {
    state.syncBusy = false;
    state.progress = null;
    await refreshState();
    renderAll();
  }
}

async function init() {
  try {
    setState(await invoke("get_state"));
  } catch (err) {
    console.error("Could not load state:", err);
  }

  await listen("app-state", (event) => setState(event.payload));
  await listen("auth-status", async (event) => {
    const data = event.payload || {};
    if (data.connected) {
      await refreshState();
    } else if (data.error) {
      show(els.updateBanner);
      els.updateMsg.textContent = data.error;
    }
  });
  await listen("incoming-rate", (event) => {
    const { files, rating } = event.payload || {};
    state.rateFiles = Array.isArray(files) ? files : [];
    state.rateRating = rating ? normalizeRating(rating) : state.rateRating;
    hide(els.results);
    showTab("rate");
    renderAll();
  });
  await listen("incoming-sync", (event) => {
    const { galleryId, name } = event.payload || {};
    showTab("sync");
    const existing = state.galleries.find(
      (gallery) => gallery.galleryId === galleryId,
    );
    if (existing) {
      state.currentGalleryId = galleryId;
      renderAll();
      runSync(galleryId);
    } else {
      startSync(galleryId, name);
    }
  });
  await listen("sync-progress", (event) => {
    const progress = event.payload || {};
    if (
      state.currentGalleryId &&
      progress.galleryId !== state.currentGalleryId
    ) {
      return;
    }
    state.progress = progress;
    state.syncBusy = true;
    renderProgress();
  });
  await listen("update-status", (event) => {
    const data = event.payload || {};
    els.updateMsg.textContent = data.msg;
    show(els.updateBanner);
    if (data.type === "ready") show(els.btnUpdate);
  });
}

els.btnInfo.addEventListener("click", () => show(els.infoModal));
els.btnInfoClose.addEventListener("click", () => hide(els.infoModal));
els.infoModal.addEventListener("click", (event) => {
  if (event.target === els.infoModal) hide(els.infoModal);
});
document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") hide(els.infoModal);
});

init();
els.tabs.addEventListener("click", (event) => {
  const button = event.target.closest(".tab");
  if (!button) return;
  showTab(button.dataset.tab);
});

for (const button of [els.btnConnect, els.btnConnectEmpty]) {
  button.addEventListener("click", async () => {
    const result = await invoke("connect");
    if (result.ok) await refreshState();
  });
}

els.btnLogout.addEventListener("click", async () => {
  await invoke("logout");
  await refreshState();
});

els.btnDemo.addEventListener("click", () => startSync("G123", "MiBoda"));

els.galleryList.addEventListener("click", async (event) => {
  const button = event.target.closest("button[data-action]");
  const row = event.target.closest(".gallery-row");
  if (!row) return;
  const galleryId = row.dataset.galleryId;

  if (!button) {
    state.currentGalleryId = galleryId;
    state.report = null;
    renderAll();
    return;
  }

  event.stopPropagation();
  const action = button.dataset.action;

  if (action === "update") {
    state.currentGalleryId = galleryId;
    state.report = null;
    renderAll();
    await runSync(galleryId);
  } else if (action === "open") {
    await invoke("open_folder", { galleryId });
  } else if (action === "locate") {
    const result = await invoke("locate_gallery", { galleryId });
    if (result.ok) await refreshState();
  } else if (action === "unlink") {
    await invoke("unlink_gallery", { galleryId });
    if (state.currentGalleryId === galleryId) state.currentGalleryId = null;
    await refreshState();
  }
});

els.btnBack.addEventListener("click", () => {
  state.currentGalleryId = null;
  state.report = null;
  renderAll();
});

els.btnUpdateNow.addEventListener("click", () => {
  if (state.currentGalleryId) runSync(state.currentGalleryId);
});

els.btnChangeFolder.addEventListener("click", async () => {
  if (!state.currentGalleryId) return;
  const result = await invoke("change_folder", {
    galleryId: state.currentGalleryId,
  });
  if (result.ok) {
    state.report = null;
    await refreshState();
  }
});

els.btnOpenFolder.addEventListener("click", async () => {
  if (!state.currentGalleryId) return;
  await invoke("open_folder", { galleryId: state.currentGalleryId });
});

els.btnDeleteRemote.addEventListener("click", async () => {
  if (!state.currentGalleryId) return;
  const ids = Array.from(
    els.deleteList.querySelectorAll("input[type=checkbox]:checked"),
  ).map((checkbox) => checkbox.dataset.photoId);
  if (ids.length === 0) return;
  let result;
  try {
    result = { ok: true, result: await invoke("apply_deletions", {
      galleryId: state.currentGalleryId,
      photoIds: ids,
    }) };
  } catch (err) {
    result = { ok: false, error: err?.message || String(err) };
  }
  if (result.ok) {
    if (state.report) {
      state.report.proposals.deleteRemote =
        state.report.proposals.deleteRemote.filter(
          (photo) => !ids.includes(photo.id),
        );
    }
    hide(els.deleteBox);
    await refreshState();
    renderReport();
  }
});

els.btnCancelDelete.addEventListener("click", () => {
  hide(els.deleteBox);
});

els.btnChooseFolder.addEventListener("click", async () => {
  const folder = await invoke("choose_folder");
  if (folder) {
    state.config.photosDir = folder;
    renderRate();
  }
});

for (const button of els.starButtons) {
  button.addEventListener("click", () => {
    state.rateRating = normalizeRating(button.dataset.rating);
    renderRate();
  });
}

els.btnRate.addEventListener("click", async () => {
  if (!state.config.photosDir || state.rateFiles.length === 0 || state.rateBusy) {
    return;
  }
  state.rateBusy = true;
  hide(els.results);
  renderRate();

  try {
    const result = await invoke("rate_files", {
      files: state.rateFiles,
      rating: state.rateRating,
      photosDir: state.config.photosDir,
    });
    const s = state.strings;
    let cls = "ok";
    const parts = [fmt(s.rated, { n: result.rated, s: result.rated === 1 ? "" : "s" })];
    if (result.notFound > 0) {
      parts.push(
        fmt(s.notFoundCount, {
          n: result.notFound,
          s: result.notFound === 1 ? "" : "s",
        }),
      );
      cls = "warn";
    }
    if (result.errors > 0) {
      parts.push(
        fmt(s.errorCount, { n: result.errors, s: result.errors === 1 ? "" : "s" }),
      );
      cls = "error";
    }
    els.resultSummary.textContent = parts.join(" · ");
    els.resultSummary.className = `result-summary ${cls}`;

    const fragment = document.createDocumentFragment();
    for (const detail of result.details || []) {
      const row = document.createElement("div");
      row.className =
        detail.status === "ok"
          ? "ok"
          : detail.status === "notfound"
            ? "miss"
            : "err";
      row.textContent =
        detail.status === "ok" ? detail.file : `${detail.name} — ${detail.msg}`;
      fragment.appendChild(row);
    }
    els.detailList.replaceChildren(fragment);
    show(els.results);
  } catch (err) {
    els.resultSummary.textContent = String(err);
    els.resultSummary.className = "result-summary error";
    els.detailList.replaceChildren();
    show(els.results);
  } finally {
    state.rateBusy = false;
    renderRate();
  }
});

els.btnUpdate.addEventListener("click", () => invoke("install_update"));

