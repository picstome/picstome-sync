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
    auth: { connected: false, name: null, email: null },
    galleries: [],
    currentGalleryId: null,
    syncBusy: false,
    progress: null,
    report: null,
    rating: {
        open: false,
        files: [],
        value: "5",
        galleryId: null,
        photosDir: null,
        source: "link",
        busy: false,
        result: null,
    },
    pendingDeletes: new Set(),
};

const els = {
    envBadge: document.getElementById("envBadge"),
    galleriesLabel: document.getElementById("galleriesLabel"),
    btnAddGallery: document.getElementById("btnAddGallery"),
    galleryList: document.getElementById("galleryList"),
    syncEmpty: document.getElementById("syncEmpty"),
    emptyTitle: document.getElementById("emptyTitle"),
    emptyHint: document.getElementById("emptyHint"),
    btnDemo: document.getElementById("btnDemo"),

    accountChip: document.getElementById("accountChip"),
    accountInitial: document.getElementById("accountInitial"),
    accountName: document.getElementById("accountName"),
    accountEmail: document.getElementById("accountEmail"),
    btnConnect: document.getElementById("btnConnect"),
    btnLogout: document.getElementById("btnLogout"),
    btnInfo: document.getElementById("btnInfo"),

    detailName: document.getElementById("detailName"),
    detailPath: document.getElementById("detailPath"),
    detailLastSync: document.getElementById("detailLastSync"),
    btnSync: document.getElementById("btnSync"),
    btnChangeFolder: document.getElementById("btnChangeFolder"),
    btnOpenFolder: document.getElementById("btnOpenFolder"),
    btnUnlink: document.getElementById("btnUnlink"),

    idleState: document.getElementById("idleState"),
    idleTitle: document.getElementById("idleTitle"),
    idleHint: document.getElementById("idleHint"),
    btnIdleSync: document.getElementById("btnIdleSync"),
    welcome: document.getElementById("welcome"),
    welcomeTitle: document.getElementById("welcomeTitle"),
    welcomeHint: document.getElementById("welcomeHint"),
    btnConnectWelcome: document.getElementById("btnConnectWelcome"),
    btnDemoWelcome: document.getElementById("btnDemoWelcome"),

    progressWrap: document.getElementById("progressWrap"),
    progressTitle: document.getElementById("progressTitle"),
    progressPercent: document.getElementById("progressPercent"),
    progressBar: document.getElementById("progressBar"),
    progressFill: document.getElementById("progressFill"),
    progressText: document.getElementById("progressText"),

    report: document.getElementById("report"),
    reportSummary: document.getElementById("reportSummary"),
    reportTime: document.getElementById("reportTime"),
    reportCounts: document.getElementById("reportCounts"),
    reportErrors: document.getElementById("reportErrors"),
    deleteBox: document.getElementById("deleteBox"),
    deleteTitle: document.getElementById("deleteTitle"),
    deleteHint: document.getElementById("deleteHint"),
    deleteList: document.getElementById("deleteList"),
    btnDeleteRemote: document.getElementById("btnDeleteRemote"),
    btnCancelDelete: document.getElementById("btnCancelDelete"),

    infoModal: document.getElementById("infoModal"),
    infoTitle: document.getElementById("infoTitle"),
    infoVersionLabel: document.getElementById("infoVersionLabel"),
    infoProfileLabel: document.getElementById("infoProfileLabel"),
    infoModeLabel: document.getElementById("infoModeLabel"),
    infoTargetLabel: document.getElementById("infoTargetLabel"),
    infoAccountLabel: document.getElementById("infoAccountLabel"),
    infoVersion: document.getElementById("infoVersion"),
    infoProfile: document.getElementById("infoProfile"),
    infoMode: document.getElementById("infoMode"),
    infoTarget: document.getElementById("infoTarget"),
    infoAccount: document.getElementById("infoAccount"),
    btnInfoClose: document.getElementById("btnInfoClose"),
    apiSwitchTitle: document.getElementById("apiSwitchTitle"),
    apiSwitchHint: document.getElementById("apiSwitchHint"),
    btnApiProduction: document.getElementById("btnApiProduction"),
    btnApiDevelopment: document.getElementById("btnApiDevelopment"),

    ratingModal: document.getElementById("ratingModal"),
    ratingTitle: document.getElementById("ratingTitle"),
    ratingSubtitle: document.getElementById("ratingSubtitle"),
    ratingTarget: document.getElementById("ratingTarget"),
    btnRatingFolder: document.getElementById("btnRatingFolder"),
    ratingValue: document.getElementById("ratingValue"),
    ratingResult: document.getElementById("ratingResult"),
    btnRatingApply: document.getElementById("btnRatingApply"),
    btnRatingCancel: document.getElementById("btnRatingCancel"),
    btnRatingClose: document.getElementById("btnRatingClose"),
    starButtons: Array.from(document.querySelectorAll("#ratingStars .star-btn")),
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

function stem(filename) {
    return String(filename || "").replace(/\.[^.]+$/, "");
}

function show(el) {
    el.classList.remove("hidden");
}

function hide(el) {
    el.classList.add("hidden");
}

function formatDateShort(value) {
    if (!value) return state.strings?.never || "never";
    try {
        return new Date(value).toLocaleDateString(
            state.lang === "es" ? "es-ES" : "en-US",
            { month: "short", day: "numeric" },
        );
    } catch {
        return String(value);
    }
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

function modeLabel(kind) {
    const s = state.strings || {};
    if (kind === "production") return s.modeProduction || "Production";
    if (kind === "development") return s.modeDevelopment || "Development";
    if (kind === "local") return s.modeLocal || "Local";
    if (kind === "demo") return s.modeDemo || "Demo";
    return s.modeCustom || "Custom";
}

function currentGallery() {
    return state.galleries.find(
        (gallery) => gallery.galleryId === state.currentGalleryId,
    );
}

function applyStrings() {
    const s = state.strings;
    if (!s) return;

    els.galleriesLabel.textContent = s.galleries;
    els.emptyTitle.textContent = s.noGalleries;
    els.emptyHint.textContent = s.noGalleriesHint;
    els.btnDemo.textContent = s.syncDemo;
    els.btnConnect.textContent = s.connect;
    els.btnConnectWelcome.textContent = s.connect;
    els.btnDemoWelcome.textContent = s.syncDemo;
    els.btnLogout.textContent = s.logout;
    els.btnInfo.textContent = s.info;
    els.btnSync.textContent = s.sync;
    els.btnIdleSync.textContent = s.sync;
    els.idleTitle.textContent = s.readyToSync;
    els.idleHint.textContent = s.syncHint;
    els.btnChangeFolder.textContent = s.changeFolder;
    els.btnOpenFolder.textContent = s.openFolder;
    els.btnUnlink.textContent = s.unlink;
    els.btnDeleteRemote.textContent = s.confirmDelete;
    els.btnCancelDelete.textContent = s.cancel;
    els.deleteHint.textContent = s.deleteRemoteHint;

    els.infoTitle.textContent = s.infoTitle;
    els.infoVersionLabel.textContent = s.infoVersion;
    els.infoProfileLabel.textContent = s.infoProfile;
    els.infoModeLabel.textContent = s.infoMode;
    els.infoTargetLabel.textContent = s.infoTarget;
    els.infoAccountLabel.textContent = s.infoAccount;
    els.btnInfoClose.textContent = s.infoClose;
    els.apiSwitchTitle.textContent = s.apiSwitchTitle;
    els.apiSwitchHint.textContent = s.apiSwitchHint;
    els.btnApiProduction.textContent = s.useProduction;
    els.btnApiDevelopment.textContent = s.useDevelopment;

    els.ratingTitle.textContent =
        state.rating.source === "favorites" ? s.ratingTitle : s.ratePhotosTitle;
    els.ratingSubtitle.textContent =
        state.rating.source === "favorites" ? s.favoritesHint : s.ratePhotosHint;
    els.btnRatingApply.textContent = s.applyRating;
    els.btnRatingCancel.textContent = s.notNow;
    els.btnRatingClose.textContent = s.infoClose;
    els.btnRatingFolder.textContent = s.chooseFolder;

}

function renderAccount() {
    const s = state.strings;
    if (!s) return;

    const connected = Boolean(state.auth.connected);
    const name = state.auth.name || state.auth.email || "";
    els.accountName.textContent = connected ? name || s.connectedAs : s.notConnected;
    els.accountEmail.textContent = connected ? state.auth.email || "" : "";
    els.accountInitial.textContent = (name || "?").trim().charAt(0).toUpperCase() || "?";
    els.accountChip.title = connected ? s.openProfile : s.connect;

    els.btnConnect.classList.toggle("hidden", connected);
    els.btnLogout.classList.toggle("hidden", !connected);
    els.btnConnectWelcome.classList.toggle("hidden", connected);
    els.btnDemo.classList.toggle("hidden", !state.mock);
    els.btnDemoWelcome.classList.toggle("hidden", !state.mock);
}

function renderRuntimeInfo() {
    const s = state.strings;
    if (!s) return;
    const config = state.config || {};
    const kind = config.apiKind || "custom";
    const label = modeLabel(kind);
    const badge =
        kind === "production"
            ? "PROD"
            : kind === "development"
              ? "DEV"
              : kind === "local"
                ? "LOCAL"
                : kind === "demo"
                  ? "DEMO"
                  : "CUSTOM";

    els.envBadge.textContent = badge;
    els.envBadge.classList.remove("hidden");
    els.envBadge.title = `${s.infoTarget}: ${config.apiBase || ""}`;

    const base = (config.apiBase || "").replace(/\/$/, "").toLowerCase();
    const isProduction =
        base === "https://app.picstome.com" || base === "https://picstome.com";
    const isDevelopment = base === "https://devel.picstome.com";
    els.btnApiProduction.classList.toggle("active-target", isProduction);
    els.btnApiDevelopment.classList.toggle("active-target", isDevelopment);

    els.infoVersion.textContent = `${config.appVersion || "?"} (${config.buildProfile || "?"})`;
    els.infoProfile.textContent = config.buildProfile || "?";
    els.infoMode.textContent = label;
    els.infoTarget.textContent = config.apiBase || "?";
    els.infoAccount.textContent = state.auth.connected
        ? [state.auth.name, state.auth.email].filter(Boolean).join(" · ")
        : "—";
}

function renderGalleryList() {
    const s = state.strings;
    if (!s) return;
    els.galleryList.replaceChildren();
    els.syncEmpty.classList.toggle("hidden", state.galleries.length > 0);

    for (const gallery of state.galleries) {
        const row = document.createElement("button");
        row.type = "button";
        row.className = "gallery-row";
        row.dataset.galleryId = gallery.galleryId;
        row.title = gallery.exists ? gallery.localPath : s.folderMissing;
        if (gallery.galleryId === state.currentGalleryId) {
            row.classList.add("selected");
        }

        const avatar = document.createElement("span");
        avatar.className = "gallery-avatar";
        avatar.textContent = (gallery.name || "?").trim().charAt(0).toUpperCase() || "?";
        row.appendChild(avatar);

        const info = document.createElement("span");
        info.className = "gallery-info";
        const name = document.createElement("span");
        name.className = "gallery-name";
        name.textContent = gallery.name;
        const meta = document.createElement("span");
        meta.className = "gallery-meta";
        if (gallery.exists) {
            meta.textContent = `${s.lastSync}: ${formatDateShort(gallery.lastSyncAt)}`;
        } else {
            meta.textContent = s.folderMissing;
            meta.classList.add("warn");
        }
        info.appendChild(name);
        info.appendChild(meta);
        row.appendChild(info);
        els.galleryList.appendChild(row);
    }
}

function renderMain() {
    const s = state.strings;
    if (!s) return;
    const gallery = currentGallery();

    if (!gallery) {
        state.currentGalleryId = null;
        hide(els.progressWrap);
        hide(els.report);
        hide(els.idleState);
        show(els.welcome);
        els.detailName.textContent = s.appTitle;
        els.detailPath.textContent = s.syncHint;
        els.detailLastSync.textContent = "";
        els.welcomeTitle.textContent =
            state.galleries.length === 0 ? s.noGalleryTitle : s.noGalleryTitle;
        els.welcomeHint.textContent = s.noGalleryHint;
        els.btnSync.disabled = true;
        els.btnChangeFolder.disabled = true;
        els.btnOpenFolder.disabled = true;
        els.btnUnlink.disabled = true;
        return;
    }

    hide(els.welcome);
    const showIdle =
        !state.syncBusy &&
        !state.report &&
        !state.progress;
    els.idleState.classList.toggle("hidden", !showIdle);
    els.detailName.textContent = gallery.name;
    els.detailPath.textContent = gallery.exists ? gallery.localPath : s.folderMissing;
    els.detailPath.classList.toggle("warn", !gallery.exists);
    els.detailLastSync.textContent = fmt(s.lastSyncAt, {
        date: formatDate(gallery.lastSyncAt),
    });
    els.btnSync.disabled = state.syncBusy || !gallery.exists;
    els.btnSync.textContent = state.syncBusy ? s.syncing : s.sync;
    els.btnChangeFolder.disabled = false;
    els.btnOpenFolder.disabled = !gallery.exists;
    els.btnUnlink.disabled = false;
}

function renderProgress() {
    const s = state.strings;
    if (!s) return;
    if (!state.syncBusy || !state.progress) {
        hide(els.progressWrap);
        return;
    }
    show(els.progressWrap);
    const progress = state.progress;
    let title = s.checkingChanges;
    let ratio = null;
    let detail = "";

    if (progress.phase === "scan" || progress.phase === "scanning") {
        title = s.scanningLocal;
    } else if (progress.phase === "download") {
        title = fmt(s.downloadingProgress, {
            done: progress.done || 0,
            total: progress.total || 0,
        });
        if (progress.fileTotal) {
            ratio = Math.min(1, (progress.fileDone || 0) / progress.fileTotal);
        } else if (progress.total) {
            ratio = Math.min(1, (progress.done || 0) / progress.total);
        }
        detail = progress.file || "";
    } else if (progress.phase === "upload") {
        title = fmt(s.uploadingProgress, {
            done: progress.done || 0,
            total: progress.total || 0,
        });
        ratio = progress.total
            ? Math.min(1, (progress.done || 0) / progress.total)
            : null;
        detail = progress.file || "";
    } else if (progress.phase === "done") {
        title = s.progressDone;
        ratio = 1;
    }

    const determinate = typeof ratio === "number";
    els.progressTitle.textContent = title;
    els.progressBar.classList.toggle("indeterminate", !determinate);
    if (determinate) {
        els.progressFill.style.width = `${Math.max(2, ratio * 100)}%`;
        els.progressPercent.textContent = `${Math.round(ratio * 100)}%`;
    } else {
        els.progressFill.style.width = "";
        els.progressPercent.textContent = "";
    }
    els.progressText.textContent = detail;
}

function renderReport() {
    const s = state.strings;
    const report = state.report;
    if (!s || !report) {
        hide(els.report);
        return;
    }
    show(els.report);
    const counts = report.counts || {};
    const changes =
        (counts.remoteNew || 0) +
        (counts.remoteModified || 0) +
        (counts.localNew || 0) +
        (counts.localModified || 0) +
        (counts.conflicts || 0);

    if (report.errors && report.errors.length > 0) {
        els.reportSummary.textContent = s.syncFailed;
        els.reportSummary.className = "error";
    } else if (changes === 0 && (report.proposals?.deleteRemote?.length || 0) === 0) {
        els.reportSummary.textContent = s.inSync;
        els.reportSummary.className = "ok";
    } else {
        els.reportSummary.textContent = s.syncDone;
        els.reportSummary.className = "ok";
    }
    els.reportTime.textContent = new Date().toLocaleTimeString(
        state.lang === "es" ? "es-ES" : "en-US",
        { hour: "2-digit", minute: "2-digit" },
    );

    els.reportCounts.replaceChildren();
    for (const [label, value] of [
        [s.downloaded, (report.downloaded || []).length],
        [s.uploaded, (report.uploaded || []).length],
        [s.conflicts, (report.conflicts || []).length],
        [s.errors, (report.errors || []).length],
    ]) {
        const chip = document.createElement("span");
        chip.className = "count-chip";
        if (label === s.errors && value > 0) chip.classList.add("warn");
        chip.textContent = `${label}: ${value}`;
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

function ratingTargetText() {
    if (state.rating.galleryId) {
        const gallery = state.galleries.find(
            (item) => item.galleryId === state.rating.galleryId,
        );
        return gallery?.localPath || "";
    }
    return state.rating.photosDir || state.config.photosDir || "";
}

function renderRating() {
    const s = state.strings;
    if (!s) return;
    if (!state.rating.open) {
        hide(els.ratingModal);
        return;
    }
    show(els.ratingModal);
    els.ratingTitle.textContent =
        state.rating.source === "favorites" ? s.ratingTitle : s.ratePhotosTitle;
    els.ratingSubtitle.textContent =
        state.rating.source === "favorites"
            ? fmt(s.favoritesFound, { n: state.rating.files.length })
            : s.ratePhotosHint;
    const target = ratingTargetText();
    els.ratingTarget.textContent = target || s.noFolder;
    const needsFolder = !state.rating.galleryId && !target;
    els.btnRatingFolder.classList.toggle("hidden", !needsFolder);

    const value = Number(state.rating.value);
    for (const button of els.starButtons) {
        button.classList.toggle("active", Number(button.dataset.rating) <= value);
    }
    els.ratingValue.textContent = state.rating.value;

    els.btnRatingApply.disabled =
        state.rating.busy ||
        state.rating.files.length === 0 ||
        (!state.rating.galleryId && !target);
    els.btnRatingApply.textContent = state.rating.busy ? s.applying : s.applyRating;

    if (state.rating.result) {
        show(els.ratingResult);
        const result = state.rating.result;
        let cls = "ok";
        let text = fmt(s.rated, {
            n: result.rated,
            s: result.rated === 1 ? "" : "s",
        });
        if (result.notFound > 0) {
            cls = "warn";
            text += ` · ${fmt(s.notFoundCount, {
                n: result.notFound,
                s: result.notFound === 1 ? "" : "s",
            })}`;
        }
        if (result.errors > 0) {
            cls = "error";
            text += ` · ${fmt(s.errorCount, {
                n: result.errors,
                s: result.errors === 1 ? "" : "s",
            })}`;
        }
        els.ratingResult.className = `rating-result ${cls}`;
        const details = (result.details || [])
            .map((detail) =>
                detail.status === "ok"
                    ? detail.file
                    : `${detail.name} — ${detail.msg || detail.status}`,
            )
            .join("\n");
        els.ratingResult.textContent = details ? `${text}\n${details}` : text;
    } else {
        hide(els.ratingResult);
    }
}

function renderAll() {
    applyStrings();
    renderAccount();
    renderRuntimeInfo();
    renderGalleryList();
    renderMain();
    renderProgress();
    renderReport();
    renderRating();
}

function setState(next) {
    if (!next) return;
    state.lang = next.lang || state.lang;
    state.strings = next.strings || state.strings;
    state.config = next.config || state.config;
    state.mock = Boolean(next.mock);
    state.auth = next.auth || state.auth;
    state.galleries = Array.isArray(next.galleries) ? next.galleries : [];
    if (
        state.currentGalleryId &&
        !state.galleries.some(
            (gallery) => gallery.galleryId === state.currentGalleryId,
        )
    ) {
        state.currentGalleryId = null;
    }
    if (!state.currentGalleryId && state.galleries.length > 0) {
        state.currentGalleryId = state.galleries[0].galleryId;
    }
    if (
        state.rating.open &&
        state.rating.galleryId &&
        !state.galleries.some(
            (gallery) => gallery.galleryId === state.rating.galleryId,
        )
    ) {
        state.rating.open = false;
    }
    renderAll();
}

async function refreshState() {
    const next = await invoke("get_state");
    setState(next);
    return next;
}

function openFavoritesPrompt(report) {
    const favorites = report?.favorites || [];
    if (favorites.length === 0) return;
    state.rating = {
        open: true,
        files: favorites.map((photo) => stem(photo.filename)),
        value: "5",
        galleryId: report.galleryId,
        photosDir: null,
        source: "favorites",
        busy: false,
        result: null,
    };
    renderRating();
}

function openRateLink({ files, rating, gallery }) {
    state.rating = {
        open: true,
        files: Array.isArray(files) ? files : [],
        value: normalizeRating(rating || state.config.lastRating || "1"),
        galleryId: gallery || null,
        photosDir: state.config.photosDir || null,
        source: "link",
        busy: false,
        result: null,
    };
    renderRating();
}

async function syncCurrent() {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;
    state.syncBusy = true;
    state.progress = { phase: "manifest", done: 0, total: 1 };
    state.report = null;
    renderAll();
    try {
        const report = await invoke("sync_gallery", { galleryId: gallery.galleryId });
        state.report = report;
    } catch (err) {
        state.report = {
            counts: {},
            downloaded: [],
            uploaded: [],
            conflicts: [],
            errors: [{ filename: "—", message: err?.message || String(err) }],
            proposals: { deleteRemote: [] },
            favorites: [],
        };
    } finally {
        state.syncBusy = false;
        state.progress = null;
        await refreshState();
        renderAll();
        openFavoritesPrompt(state.report);
    }
}

async function startSync(galleryId, name) {
    if (state.syncBusy) return;
    state.syncBusy = true;
    state.progress = { phase: "manifest", done: 0, total: 1 };
    state.report = null;
    renderAll();
    try {
        const result = await invoke("start_sync", { galleryId, name });
        if (result?.report) {
            state.currentGalleryId = result.report.galleryId;
            state.report = result.report;
        }
    } catch (err) {
        state.report = {
            counts: {},
            downloaded: [],
            uploaded: [],
            conflicts: [],
            errors: [{ filename: "—", message: err?.message || String(err) }],
            proposals: { deleteRemote: [] },
            favorites: [],
        };
    } finally {
        state.syncBusy = false;
        state.progress = null;
        await refreshState();
        renderAll();
        openFavoritesPrompt(state.report);
    }
}

async function applyRating() {
    if (state.rating.busy || state.rating.files.length === 0) return;
    state.rating.busy = true;
    state.rating.result = null;
    renderRating();
    try {
        const result = await invoke("rate_files", {
            files: state.rating.files,
            rating: state.rating.value,
            photosDir: state.rating.photosDir,
            galleryId: state.rating.galleryId,
        });
        state.rating.result = result;
        state.config.lastRating = state.rating.value;
    } catch (err) {
        state.rating.result = {
            rated: 0,
            notFound: 0,
            errors: 1,
            details: [{ name: "—", status: "error", msg: err?.message || String(err) }],
        };
    } finally {
        state.rating.busy = false;
        renderRating();
    }
}

async function switchApiTarget(baseUrl) {
    try {
        await invoke("set_api_base", { baseUrl });
        hide(els.infoModal);
        await refreshState();
    } catch (err) {
        els.apiSwitchHint.textContent = err?.message || String(err);
    }
}

async function connect() {
    const result = await invoke("connect");
    if (result?.ok) await refreshState();
}

async function openPicstome() {
    if (state.auth.connected) {
        await invoke("open_galleries_page");
    } else {
        await connect();
    }
}

/* Events */

async function init() {
    try {
        setState(await invoke("get_state"));
    } catch (err) {
        console.error("Could not load state:", err);
    }

    await listen("app-state", (event) => setState(event.payload));
    await listen("auth-status", async (event) => {
        const data = event.payload || {};
        if (data.connected) await refreshState();
    });
    await listen("incoming-rate", (event) => {
        openRateLink(event.payload || {});
    });
    await listen("incoming-sync", (event) => {
        const { galleryId, name } = event.payload || {};
        const existing = state.galleries.find(
            (gallery) => gallery.galleryId === galleryId,
        );
        if (existing) {
            state.currentGalleryId = galleryId;
            renderAll();
            syncCurrent();
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
        if (progress.phase === "done") {
            state.syncBusy = false;
            renderProgress();
            renderMain();
            setTimeout(() => {
                if (!state.syncBusy) {
                    state.progress = null;
                    renderProgress();
                }
            }, 900);
        } else {
            state.syncBusy = true;
            renderProgress();
            renderMain();
        }
    });
}

/* UI bindings */

els.galleryList.addEventListener("click", (event) => {
    const row = event.target.closest(".gallery-row");
    if (!row) return;
    state.currentGalleryId = row.dataset.galleryId;
    state.report = null;
    renderAll();
});

els.galleryList.addEventListener("dblclick", (event) => {
    const row = event.target.closest(".gallery-row");
    if (!row) return;
    state.currentGalleryId = row.dataset.galleryId;
    renderAll();
    syncCurrent();
});

els.btnSync.addEventListener("click", syncCurrent);
els.btnIdleSync.addEventListener("click", syncCurrent);
els.btnAddGallery.addEventListener("click", openPicstome);
els.btnConnect.addEventListener("click", openPicstome);
els.btnConnectWelcome.addEventListener("click", openPicstome);
els.btnDemo.addEventListener("click", () => startSync("G123", "MiBoda"));
els.btnDemoWelcome.addEventListener("click", () => startSync("G123", "MiBoda"));

els.btnLogout.addEventListener("click", async () => {
    await invoke("logout");
    await refreshState();
});

els.accountChip.addEventListener("click", async () => {
    if (state.auth.connected) {
        await invoke("open_account_profile");
    } else {
        await connect();
    }
});

els.btnInfo.addEventListener("click", () => show(els.infoModal));
els.btnInfoClose.addEventListener("click", () => hide(els.infoModal));
els.infoModal.addEventListener("click", (event) => {
    if (event.target === els.infoModal) hide(els.infoModal);
});
els.btnApiProduction.addEventListener("click", () =>
    switchApiTarget("https://app.picstome.com"),
);
els.btnApiDevelopment.addEventListener("click", () =>
    switchApiTarget("https://devel.picstome.com"),
);

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

els.btnUnlink.addEventListener("click", async () => {
    if (!state.currentGalleryId) return;
    await invoke("unlink_gallery", { galleryId: state.currentGalleryId });
    state.currentGalleryId = null;
    state.report = null;
    await refreshState();
});

els.btnDeleteRemote.addEventListener("click", async () => {
    if (!state.currentGalleryId) return;
    const ids = Array.from(
        els.deleteList.querySelectorAll("input[type=checkbox]:checked"),
    ).map((checkbox) => checkbox.dataset.photoId);
    if (ids.length === 0) return;
    try {
        await invoke("apply_deletions", {
            galleryId: state.currentGalleryId,
            photoIds: ids,
        });
        if (state.report) {
            state.report.proposals.deleteRemote =
                state.report.proposals.deleteRemote.filter(
                    (photo) => !ids.includes(photo.id),
                );
        }
        hide(els.deleteBox);
        await refreshState();
        renderReport();
    } catch (err) {
        state.report = {
            ...(state.report || {}),
            errors: [{ filename: "—", message: err?.message || String(err) }],
            proposals: { deleteRemote: [] },
        };
        renderReport();
    }
});

els.btnCancelDelete.addEventListener("click", () => hide(els.deleteBox));

for (const button of els.starButtons) {
    button.addEventListener("click", () => {
        state.rating.value = normalizeRating(button.dataset.rating);
        renderRating();
    });
}

els.btnRatingApply.addEventListener("click", applyRating);
els.btnRatingCancel.addEventListener("click", () => {
    state.rating.open = false;
    renderRating();
});
els.btnRatingClose.addEventListener("click", () => {
    state.rating.open = false;
    renderRating();
});
els.ratingModal.addEventListener("click", (event) => {
    if (event.target === els.ratingModal) {
        state.rating.open = false;
        renderRating();
    }
});
els.btnRatingFolder.addEventListener("click", async () => {
    const folder = await invoke("choose_folder");
    if (folder) {
        state.rating.photosDir = folder;
        state.config.photosDir = folder;
        renderRating();
    }
});

document.addEventListener("keydown", (event) => {
    if (event.key !== "Escape") return;
    hide(els.infoModal);
    state.rating.open = false;
    renderRating();
});

init();
