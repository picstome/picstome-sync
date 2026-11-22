/**
 * Picstome Sync renderer.
 *
 * A single `state` object is the source of truth; `render*` functions project
 * it onto the static DOM in index.html and never read back from it (except
 * for checkbox selections at the moment an action runs). Every backend call
 * goes through Tauri `invoke`; the backend pushes `app-state` and
 * `sync-progress` events.
 */
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
    galleryQuery: "",
    remoteGalleries: {
        page: 0,
        perPage: 25,
        items: [],
        meta: null,
        loading: false,
        error: null,
        requestId: 0,
    },
    recentGalleries: {
        loadedFor: null,
        loading: false,
        items: [],
        error: null,
    },
    currentGalleryId: null,
    syncBusy: false,
    syncTarget: null,
    progress: null,
    report: null,
    rating: {
        open: false,
        files: [],
        value: "5",
        galleryId: null,
        photosDir: null,
        source: "link",
        favorites: [],
        plan: [],
        busy: false,
        result: null,
    },
    pendingDeletes: new Set(),
    syncLog: [],
    remoteDeleted: {
        open: false,
        galleryId: null,
        items: [],
        busy: false,
        result: null,
        pendingFavorites: null,
    },
    favorites: {
        open: false,
        galleryId: null,
        items: [],
        tab: "lightroom",
        query: "",
        busy: false,
        error: null,
    },
    confirm: {
        open: false,
        title: "",
        message: "",
        confirmLabel: "",
        danger: true,
        resolve: null,
    },
    firstSync: {
        open: false,
        preview: null,
        localAction: "upload",
        download: true,
        resolve: null,
    },
};

const els = {
    envBadge: document.getElementById("envBadge"),
    galleriesLabel: document.getElementById("galleriesLabel"),
    btnAddGallery: document.getElementById("btnAddGallery"),
    galleryList: document.getElementById("galleryList"),
    gallerySearch: document.getElementById("gallerySearch"),
    searchEmpty: document.getElementById("searchEmpty"),
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
    btnOpenWebGallery: document.getElementById("btnOpenWebGallery"),
    detailPath: document.getElementById("detailPath"),
    detailPathText: document.getElementById("detailPathText"),
    pathRow: document.getElementById("pathRow"),
    mainFooter: document.getElementById("mainFooter"),
    detailStats: document.getElementById("detailStats"),
    detailLastSync: document.getElementById("detailLastSync"),
    btnSync: document.getElementById("btnSync"),
    btnFavoritesList: document.getElementById("btnFavoritesList"),
    btnChangeFolder: document.getElementById("btnChangeFolder"),
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
    syncModal: document.getElementById("syncModal"),
    syncModalTitle: document.getElementById("syncModalTitle"),
    syncModalPhase: document.getElementById("syncModalPhase"),
    syncModalFill: document.getElementById("syncModalFill"),
    syncModalFile: document.getElementById("syncModalFile"),
    syncModalPercent: document.getElementById("syncModalPercent"),
    syncModalHint: document.getElementById("syncModalHint"),

    report: document.getElementById("report"),
    reportIcon: document.getElementById("reportIcon"),
    reportIconUse: document.getElementById("reportIconUse"),
    reportSummary: document.getElementById("reportSummary"),
    reportTime: document.getElementById("reportTime"),
    reportCounts: document.getElementById("reportCounts"),
    reportErrors: document.getElementById("reportErrors"),
    reportWarnings: document.getElementById("reportWarnings"),
    reportFiles: document.getElementById("reportFiles"),
    reportIgnored: document.getElementById("reportIgnored"),
    reportIgnoredText: document.getElementById("reportIgnoredText"),
    btnIncludeIgnored: document.getElementById("btnIncludeIgnored"),
    syncLog: document.getElementById("syncLog"),
    syncLogTitle: document.getElementById("syncLogTitle"),
    syncLogCount: document.getElementById("syncLogCount"),
    syncLogEntries: document.getElementById("syncLogEntries"),
    btnClearLog: document.getElementById("btnClearLog"),
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
    infoAccountLabel: document.getElementById("infoAccountLabel"),
    infoVersion: document.getElementById("infoVersion"),
    infoProfile: document.getElementById("infoProfile"),
    infoAccount: document.getElementById("infoAccount"),
    btnInfoClose: document.getElementById("btnInfoClose"),
    btnTestMode: document.getElementById("btnTestMode"),
    testModeHint: document.getElementById("testModeHint"),

    ratingModal: document.getElementById("ratingModal"),
    ratingTitle: document.getElementById("ratingTitle"),
    ratingSubtitle: document.getElementById("ratingSubtitle"),
    ratingTarget: document.getElementById("ratingTarget"),
    btnRatingFolder: document.getElementById("btnRatingFolder"),
    ratingPlan: document.getElementById("ratingPlan"),
    ratingStars: document.getElementById("ratingStars"),
    ratingValue: document.getElementById("ratingValue"),
    ratingResult: document.getElementById("ratingResult"),
    btnRatingApply: document.getElementById("btnRatingApply"),
    btnRatingCancel: document.getElementById("btnRatingCancel"),
    btnRatingClose: document.getElementById("btnRatingClose"),
    starButtons: Array.from(document.querySelectorAll("#ratingStars .star-btn")),

    favoritesModal: document.getElementById("favoritesModal"),
    favoritesTitle: document.getElementById("favoritesTitle"),
    favoritesSubtitle: document.getElementById("favoritesSubtitle"),
    favoritesFilter: document.getElementById("favoritesFilter"),
    favoritesTabs: document.getElementById("favoritesTabs"),
    favoritesText: document.getElementById("favoritesText"),
    favoritesHint: document.getElementById("favoritesHint"),
    favoritesResult: document.getElementById("favoritesResult"),
    btnFavoritesClose: document.getElementById("btnFavoritesClose"),
    btnFavoritesCopy: document.getElementById("btnFavoritesCopy"),
    favoritesTabButtons: Array.from(
        document.querySelectorAll("#favoritesTabs .favorites-tab"),
    ),

    firstSyncModal: document.getElementById("firstSyncModal"),
    firstSyncTitle: document.getElementById("firstSyncTitle"),
    firstSyncHint: document.getElementById("firstSyncHint"),
    firstSyncUploadSection: document.getElementById("firstSyncUploadSection"),
    firstSyncUploadLabel: document.getElementById("firstSyncUploadLabel"),
    firstSyncUploadHint: document.getElementById("firstSyncUploadHint"),
    firstSyncUploadList: document.getElementById("firstSyncUploadList"),
    firstSyncLocalUpload: document.getElementById("firstSyncLocalUpload"),
    firstSyncLocalKeep: document.getElementById("firstSyncLocalKeep"),
    firstSyncLocalTrash: document.getElementById("firstSyncLocalTrash"),
    firstSyncLocalUploadLabel: document.getElementById("firstSyncLocalUploadLabel"),
    firstSyncLocalKeepLabel: document.getElementById("firstSyncLocalKeepLabel"),
    firstSyncLocalTrashLabel: document.getElementById("firstSyncLocalTrashLabel"),
    firstSyncDownloadSection: document.getElementById("firstSyncDownloadSection"),
    firstSyncDownload: document.getElementById("firstSyncDownload"),
    firstSyncDownloadLabel: document.getElementById("firstSyncDownloadLabel"),
    firstSyncDownloadHint: document.getElementById("firstSyncDownloadHint"),
    firstSyncDownloadList: document.getElementById("firstSyncDownloadList"),
    btnFirstSyncClose: document.getElementById("btnFirstSyncClose"),
    btnFirstSyncConfirm: document.getElementById("btnFirstSyncConfirm"),
    btnFirstSyncCancel: document.getElementById("btnFirstSyncCancel"),

    remoteDeletedModal: document.getElementById("remoteDeletedModal"),
    remoteDeletedTitle: document.getElementById("remoteDeletedTitle"),
    remoteDeletedHint: document.getElementById("remoteDeletedHint"),
    remoteDeletedList: document.getElementById("remoteDeletedList"),
    remoteDeletedResult: document.getElementById("remoteDeletedResult"),
    btnRemoteClose: document.getElementById("btnRemoteClose"),
    btnRemoteDeleteLocal: document.getElementById("btnRemoteDeleteLocal"),
    btnRemoteKeepLocal: document.getElementById("btnRemoteKeepLocal"),
    btnRemoteUploadAgain: document.getElementById("btnRemoteUploadAgain"),

    confirmModal: document.getElementById("confirmModal"),
    confirmTitle: document.getElementById("confirmTitle"),
    confirmMessage: document.getElementById("confirmMessage"),
    btnConfirmOk: document.getElementById("btnConfirmOk"),
    btnConfirmCancel: document.getElementById("btnConfirmCancel"),
};

for (const [key, el] of Object.entries(els)) {
    if (Array.isArray(el)) {
        if (el.length === 0) throw new Error(`Missing DOM element: ${key}`);
    } else if (!el) {
        throw new Error(`Missing DOM element: ${key}`);
    }
}

/* DOM helpers */

const SVG_NS = "http://www.w3.org/2000/svg";

/** Build an icon from the sprite in index.html (`#i-<name>`). */
function icon(name, className = "") {
    const svg = document.createElementNS(SVG_NS, "svg");
    svg.setAttribute("class", `icon ${className}`.trim());
    svg.setAttribute("aria-hidden", "true");
    const use = document.createElementNS(SVG_NS, "use");
    use.setAttribute("href", `#i-${name}`);
    svg.appendChild(use);
    return svg;
}

/**
 * Create an element with an optional class and text. Text always goes through
 * `textContent`, so filenames and server messages can never inject markup.
 */
function el(tag, className = "", text = null) {
    const node = document.createElement(tag);
    if (className) node.className = className;
    if (text != null) node.textContent = String(text);
    return node;
}

/**
 * Set the visible text of a button that also carries an icon. Buttons built
 * as `<svg/><span class="label">` keep their icon; plain buttons get text.
 */
function setLabel(button, text) {
    const label = button.querySelector(":scope > .label");
    (label || button).textContent = text;
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

async function copyText(text) {
    if (!text) return false;
    try {
        if (navigator.clipboard?.writeText) {
            await navigator.clipboard.writeText(text);
            return true;
        }
    } catch {
        // Fall through to the legacy textarea copy.
    }
    try {
        const helper = document.createElement("textarea");
        helper.value = text;
        helper.setAttribute("readonly", "");
        helper.style.position = "fixed";
        helper.style.opacity = "0";
        document.body.appendChild(helper);
        helper.select();
        const copied = document.execCommand("copy");
        document.body.removeChild(helper);
        return copied;
    } catch {
        return false;
    }
}

function warningText(warning) {
    const s = state.strings;
    const vars = {
        filename: warning.filename || "—",
        preferred: warning.preferred || "—",
    };
    if (warning.kind === "duplicate_local_ignored" && s?.duplicateLocalIgnored) {
        return fmt(s.duplicateLocalIgnored, vars);
    }
    if (warning.kind === "file_too_large" && s?.fileTooLarge) {
        return fmt(s.fileTooLarge, vars);
    }
    return warning.message || "";
}

function syncErrorReport(err) {
    return {
        counts: {},
        downloaded: [],
        uploaded: [],
        conflicts: [],
        errors: [{ filename: "—", message: err?.message || String(err) }],
        proposals: { deleteRemote: [] },
        favorites: [],
    };
}

/** The header sync button: label plus a spinning icon while busy. */
function renderSyncButton(s) {
    setLabel(els.btnSync, state.syncBusy ? s.syncing : s.sync);
    els.btnSync.classList.toggle("busy", state.syncBusy);
}

function formatBytes(bytes) {
    const value = Number(bytes) || 0;
    if (value < 1024) return `${value} B`;
    const units = ["KB", "MB", "GB", "TB", "PB"];
    let amount = value;
    let index = -1;
    do {
        amount /= 1024;
        index += 1;
    } while (amount >= 1024 && index < units.length - 1);
    return `${amount.toFixed(amount >= 10 ? 0 : 1)} ${units[index]}`;
}

function galleryStatsText(gallery) {
    return fmt(state.strings?.galleryStats || "{n} · {size}", {
        n: Number(gallery.photoCount) || 0,
        size: formatBytes(gallery.localBytes),
    });
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

/** First letter of a name for avatars, `?` when there is none. */
function initial(name) {
    return String(name || "").trim().charAt(0).toUpperCase() || "?";
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
    els.gallerySearch.placeholder = s.searchGalleries;
    els.btnAddGallery.title = s.createGallery;
    els.btnAddGallery.setAttribute("aria-label", s.createGallery);
    els.searchEmpty.textContent = s.noSearchResults;
    els.emptyTitle.textContent = s.noGalleries;
    els.emptyHint.textContent = s.noGalleriesHint;
    els.btnDemo.textContent = s.syncDemo;
    els.btnConnect.textContent = s.connect;
    els.btnConnectWelcome.textContent = s.connect;
    els.btnDemoWelcome.textContent = s.syncDemo;
    els.btnLogout.textContent = s.logout;
    els.btnInfo.textContent = s.info;
    renderSyncButton(s);
    setLabel(els.btnChangeFolder, s.changeFolder);
    els.btnChangeFolder.title = s.changeFolder;
    setLabel(els.btnUnlink, s.unlink);
    els.detailPath.title = s.openFolder;
    els.btnOpenWebGallery.title = s.openGalleryInPicstome;
    els.btnOpenWebGallery.setAttribute("aria-label", s.openGalleryInPicstome);
    els.btnDeleteRemote.textContent = s.confirmDelete;
    els.btnCancelDelete.textContent = s.cancel;
    els.deleteHint.textContent = s.deleteRemoteHint;

    els.infoTitle.textContent = s.infoTitle;
    els.infoVersionLabel.textContent = s.infoVersion;
    els.infoProfileLabel.textContent = s.infoProfile;
    els.infoAccountLabel.textContent = s.infoAccount;
    els.btnInfoClose.textContent = s.infoClose;
    els.btnTestMode.textContent = s.testMode;

    setLabel(els.btnFavoritesList, s.favoritesList);
    els.favoritesTitle.textContent = s.favoritesListTitle;
    els.favoritesFilter.placeholder = s.favoritesListSearch;
    els.btnFavoritesClose.textContent = s.infoClose;
    els.btnFavoritesCopy.textContent = s.favoritesListCopy;
    const favoritesTabs = {
        lightroom: s.favoritesTabLightroom,
        captureone: s.favoritesTabCaptureOne,
        finder: s.favoritesTabFinder,
        list: s.favoritesTabList,
    };
    for (const button of els.favoritesTabButtons) {
        button.textContent = favoritesTabs[button.dataset.tab] || "";
    }
    els.btnRatingApply.textContent = s.applyRating;
    els.btnRatingCancel.textContent = s.notNow;
    els.btnRatingClose.textContent = s.infoClose;
    els.btnRatingFolder.textContent = s.chooseFolder;
    els.remoteDeletedTitle.textContent = s.remoteDeletedTitle;
    els.syncLogTitle.textContent = s.activityLog;
    els.btnClearLog.textContent = s.clearLog;
    els.btnRemoteClose.textContent = s.infoClose;
    els.btnRemoteDeleteLocal.textContent = s.deleteLocal;
    els.btnRemoteKeepLocal.textContent = s.keepLocal;
    els.btnRemoteUploadAgain.textContent = s.uploadAgain;
    els.btnConfirmCancel.textContent = s.cancel;
    for (const button of els.starButtons) {
        const n = Number(button.dataset.rating);
        button.setAttribute("aria-label", fmt(s.starsLabel || "{n}", { n }));
    }
}

function renderAccount() {
    const s = state.strings;
    if (!s) return;

    const connected = Boolean(state.auth.connected);
    const name = state.auth.name || state.auth.email || "";
    els.accountName.textContent = connected ? name || s.connectedAs : s.notConnected;
    els.accountEmail.textContent = connected ? state.auth.email || "" : "";
    els.accountInitial.textContent = initial(name);
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
    const badge =
        kind === "development"
            ? "DEV"
            : kind === "local"
              ? "LOCAL"
              : kind === "demo"
                ? "DEMO"
                : kind === "custom"
                  ? "CUSTOM"
                  : "";
    const showBadge = kind !== "production" && Boolean(badge);

    els.envBadge.textContent = badge;
    els.envBadge.classList.toggle("hidden", !showBadge);
    els.envBadge.title = showBadge ? `${badge} API` : "";

    els.infoVersion.textContent = `${config.appVersion || "?"} (${config.buildProfile || "?"})`;
    els.infoProfile.textContent = config.buildProfile || "?";
    els.infoAccount.textContent = state.auth.connected
        ? [state.auth.name, state.auth.email].filter(Boolean).join(" · ")
        : "—";
    const testMode = kind === "development";
    els.btnTestMode.textContent = testMode ? s.testModeActive : s.testMode;
    els.testModeHint.textContent = testMode ? s.testModeActiveHint : s.testModeHint;
}

function filteredGalleries() {
    const query = state.galleryQuery.trim().toLowerCase();
    if (!query) return state.galleries;
    return state.galleries.filter(
        (gallery) =>
            gallery.name.toLowerCase().includes(query) ||
            gallery.localPath.toLowerCase().includes(query),
    );
}

function recentGalleryItems() {
    const linked = new Set(state.galleries.map((gallery) => gallery.galleryId));
    const seen = new Set();
    return state.recentGalleries.items
        .filter((gallery) => {
            if (linked.has(gallery.id) || seen.has(gallery.id)) return false;
            seen.add(gallery.id);
            return true;
        })
        .slice(0, 5);
}

function resetRecentGalleries() {
    state.recentGalleries.loadedFor = null;
    state.recentGalleries.items = [];
    state.recentGalleries.loading = false;
    state.recentGalleries.error = null;
}

async function loadRecentGalleries() {
    const key = state.config.apiBase;
    if (!state.auth.connected || state.recentGalleries.loading) return;
    if (state.recentGalleries.loadedFor === key) return;
    const recent = state.recentGalleries;
    recent.loading = true;
    recent.error = null;
    renderGalleryList();
    try {
        const result = await invoke("search_remote_galleries", {
            search: "",
            page: 1,
            perPage: 10,
        });
        if (!state.auth.connected || key !== state.config.apiBase) return;
        recent.items = Array.isArray(result.data) ? result.data : [];
        recent.loadedFor = key;
    } catch (err) {
        recent.error = err?.message || String(err);
        recent.loadedFor = key;
    } finally {
        recent.loading = false;
        renderGalleryList();
    }
}

function maybeLoadRecentGalleries() {
    if (!state.auth.connected) {
        resetRecentGalleries();
        return;
    }
    loadRecentGalleries();
}

function remoteGalleryRow(gallery, s) {
    const row = el("button", "gallery-row remote-gallery-row");
    row.type = "button";
    row.dataset.remote = "1";
    row.dataset.galleryId = gallery.id;
    row.dataset.galleryName = gallery.name || "Gallery";
    row.title = s.remoteGalleryHint;

    const avatar = el("span", "gallery-avatar remote-gallery-avatar");
    avatar.appendChild(icon("cloud"));
    const info = el("span", "gallery-info");
    info.append(
        el("span", "gallery-name", gallery.name || "Gallery"),
        el("span", "gallery-meta", s.remoteGalleryHint),
    );
    row.append(avatar, info);
    return row;
}

function renderRecentGallerySection(s) {
    if (remoteSearchQuery()) return;
    const recent = state.recentGalleries;
    const items = recentGalleryItems();
    if (
        !state.auth.connected ||
        (items.length === 0 && !recent.loading && !recent.error)
    ) {
        return;
    }

    const section = remoteSection("recent-gallery-section", s.recentGalleries, null);
    for (const gallery of items) {
        section.appendChild(remoteGalleryRow(gallery, s));
    }
    const status = remoteStatus(recent, s);
    if (status) section.appendChild(status);
    els.galleryList.appendChild(section);
}

/** A sidebar section listing Picstome galleries that are not linked yet. */
function remoteSection(extraClass, title, count) {
    const section = el("div", `remote-gallery-section ${extraClass}`.trim());
    const head = el("div", "remote-gallery-head");
    head.appendChild(el("span", "", title));
    if (count != null) head.appendChild(el("span", "remote-gallery-count", count));
    section.appendChild(head);
    return section;
}

/** Loading / error line shared by the recent and search sections. */
function remoteStatus(source, s) {
    if (source.loading) return el("div", "remote-gallery-status", s.remoteSearching);
    if (source.error) {
        return el(
            "div",
            "remote-gallery-status error",
            `${s.remoteSearchError}: ${source.error}`,
        );
    }
    return null;
}

function remoteSearchQuery() {
    const query = state.galleryQuery.trim();
    return state.auth.connected && query.length >= 2 ? query : "";
}

function remoteGalleries() {
    const linked = new Set(state.galleries.map((gallery) => gallery.galleryId));
    const seen = new Set();
    return state.remoteGalleries.items.filter((gallery) => {
        if (linked.has(gallery.id) || seen.has(gallery.id)) return false;
        seen.add(gallery.id);
        return true;
    });
}

let remoteSearchTimer = null;

function clearRemoteSearch() {
    if (remoteSearchTimer) {
        clearTimeout(remoteSearchTimer);
        remoteSearchTimer = null;
    }
    state.remoteGalleries.requestId += 1;
    state.remoteGalleries.page = 0;
    state.remoteGalleries.items = [];
    state.remoteGalleries.meta = null;
    state.remoteGalleries.loading = false;
    state.remoteGalleries.error = null;
}

function scheduleRemoteSearch() {
    if (remoteSearchTimer) clearTimeout(remoteSearchTimer);
    if (!remoteSearchQuery()) {
        clearRemoteSearch();
        renderGalleryList();
        return;
    }
    state.remoteGalleries.loading = true;
    state.remoteGalleries.error = null;
    renderGalleryList();
    remoteSearchTimer = setTimeout(() => {
        remoteSearchTimer = null;
        searchRemoteGalleries(1, true);
    }, 320);
}

async function searchRemoteGalleries(page, replace) {
    const query = remoteSearchQuery();
    if (!query) return;
    const remote = state.remoteGalleries;
    const requestId = ++remote.requestId;
    if (replace) {
        remote.items = [];
        remote.meta = null;
        remote.page = 0;
    }
    remote.loading = true;
    remote.error = null;
    renderGalleryList();

    try {
        const result = await invoke("search_remote_galleries", {
            search: query,
            page,
            perPage: remote.perPage,
        });
        if (requestId !== remote.requestId || query !== remoteSearchQuery()) {
            return;
        }
        const existing = new Set(remote.items.map((gallery) => gallery.id));
        const incoming = Array.isArray(result.data) ? result.data : [];
        remote.items = replace
            ? incoming
            : [
                  ...remote.items,
                  ...incoming.filter((gallery) => !existing.has(gallery.id)),
              ];
        remote.meta = result.meta || null;
        remote.page = result.meta?.currentPage || page;
    } catch (err) {
        if (requestId !== remote.requestId) return;
        remote.error = err?.message || String(err);
    } finally {
        if (requestId === remote.requestId) {
            remote.loading = false;
            renderGalleryList();
        }
    }
}

function renderRemoteGallerySection(s) {
    if (!remoteSearchQuery()) return;
    const remote = state.remoteGalleries;
    const items = remoteGalleries();
    if (items.length === 0 && !remote.loading && !remote.error) return;

    const section = remoteSection(
        "",
        s.remoteResults,
        remote.meta?.total != null ? String(remote.meta.total) : "",
    );
    for (const gallery of items) {
        section.appendChild(remoteGalleryRow(gallery, s));
    }
    const status = remoteStatus(remote, s);
    if (status) {
        section.appendChild(status);
    } else if (remote.meta && remote.meta.currentPage < remote.meta.lastPage) {
        const more = el("button", "remote-more", s.remoteLoadMore);
        more.type = "button";
        more.dataset.page = String(remote.meta.currentPage + 1);
        section.appendChild(more);
    }

    els.galleryList.appendChild(section);
}

function renderGalleryList() {
    const s = state.strings;
    if (!s) return;
    els.galleryList.replaceChildren();
    const searchingRemote = Boolean(remoteSearchQuery());
    const recentVisible =
        !searchingRemote &&
        state.auth.connected &&
        (state.recentGalleries.loading ||
            Boolean(state.recentGalleries.error) ||
            recentGalleryItems().length > 0);
    els.syncEmpty.classList.toggle(
        "hidden",
        state.galleries.length > 0 || searchingRemote || recentVisible,
    );
    const galleries = filteredGalleries();
    const remote = state.remoteGalleries;
    const remoteHasResults =
        searchingRemote &&
        (remote.loading ||
            Boolean(remote.error) ||
            remoteGalleries().length > 0);
    els.searchEmpty.classList.toggle(
        "hidden",
        state.galleries.length === 0 ||
            galleries.length > 0 ||
            remoteHasResults,
    );

    renderRecentGallerySection(s);

    for (const gallery of galleries) {
        const row = el("button", "gallery-row");
        row.type = "button";
        row.dataset.galleryId = gallery.galleryId;
        row.title = gallery.exists ? gallery.localPath : s.folderMissing;
        row.classList.toggle("selected", gallery.galleryId === state.currentGalleryId);
        row.setAttribute(
            "aria-current",
            gallery.galleryId === state.currentGalleryId ? "true" : "false",
        );

        const info = el("span", "gallery-info");
        info.append(
            el("span", "gallery-name", gallery.name),
            el("span", "gallery-stats", galleryStatsText(gallery)),
            gallery.exists
                ? el(
                      "span",
                      "gallery-meta",
                      `${s.lastSync}: ${formatDateShort(gallery.lastSyncAt)}`,
                  )
                : el("span", "gallery-meta warn", s.folderMissing),
        );
        row.append(el("span", "gallery-avatar", initial(gallery.name)), info);
        els.galleryList.appendChild(row);
    }

    renderRemoteGallerySection(s);
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
        els.btnOpenWebGallery.classList.add("hidden");
        els.detailPathText.textContent = s.syncHint;
        els.detailPath.disabled = true;
        els.detailPath.classList.add("hidden");
        els.detailPath.classList.remove("warn");
        els.pathRow.classList.add("hidden");
        els.mainFooter.classList.add("hidden");
        els.detailStats.textContent = "";
        els.detailLastSync.textContent = "";
        els.welcomeTitle.textContent = s.noGalleryTitle;
        els.welcomeHint.textContent = s.noGalleryHint;
        els.btnSync.disabled = true;
        els.btnFavoritesList.disabled = true;
        els.btnChangeFolder.disabled = true;
        els.btnUnlink.disabled = true;
        return;
    }

    hide(els.welcome);
    const showIdle =
        !state.syncBusy &&
        !state.report &&
        !state.progress;
    els.idleState.classList.toggle("hidden", !showIdle);
    // A missing folder cannot be synced: the idle card offers to locate it
    // instead (keeps the sync history, unlike "Change folder").
    els.idleTitle.textContent = gallery.exists ? s.readyToSync : s.folderMissing;
    els.idleHint.textContent = gallery.exists ? s.syncHint : s.folderMissingHint;
    els.btnIdleSync.textContent = gallery.exists ? s.sync : s.locate;
    els.idleState.classList.toggle("warn", !gallery.exists);
    els.idleState
        .querySelector(".idle-icon use")
        ?.setAttribute("href", gallery.exists ? "#i-sync" : "#i-folder");
    els.detailName.textContent = gallery.name;
    els.btnOpenWebGallery.classList.remove("hidden");
    els.btnOpenWebGallery.title = s.openGalleryInPicstome;
    els.btnOpenWebGallery.setAttribute("aria-label", s.openGalleryInPicstome);
    els.pathRow.classList.remove("hidden");
    els.mainFooter.classList.remove("hidden");
    els.detailPath.classList.remove("hidden");
    els.detailPathText.textContent = gallery.exists ? gallery.localPath : s.folderMissing;
    els.detailPath.classList.toggle("warn", !gallery.exists);
    els.detailPath.disabled = !gallery.exists;
    els.detailPath.title = gallery.exists ? s.openFolder : s.folderMissing;
    els.detailStats.textContent = galleryStatsText(gallery);
    els.detailLastSync.textContent = fmt(s.lastSyncAt, {
        date: formatDate(gallery.lastSyncAt),
    });
    els.btnSync.disabled = state.syncBusy || !gallery.exists;
    renderSyncButton(s);
    // Folder changes and unlinking rewrite the gallery state the running
    // sync is using; the backend refuses them too, so do not offer them.
    els.btnFavoritesList.disabled = false;
    els.btnChangeFolder.disabled = state.syncBusy;
    els.btnUnlink.disabled = state.syncBusy;
}

function syncProgressView(progress, s) {
    let title = s.checkingChanges;
    let ratio = null;
    let detail = "";

    if (progress.phase === "scan" || progress.phase === "scanning") {
        title = s.scanningLocal;
        // The walk reports how many entries it has seen; the total is unknown
        // until it finishes, so the bar stays indeterminate but the counter
        // proves the app is working.
        const seen = progress.fileDone || 0;
        const matched = progress.done || 0;
        detail = seen > 0
            ? fmt(s.scanSeenEntries, { seen, matched })
            : "";
    } else if (progress.phase === "hash") {
        title = fmt(s.hashingLocal, {
            done: progress.done || 0,
            total: progress.total || 0,
        });
        if (progress.total) {
            ratio = Math.min(1, (progress.done || 0) / progress.total);
        }
    } else if (progress.phase === "download" || progress.phase === "upload") {
        title = fmt(
            progress.phase === "download" ? s.downloadingProgress : s.uploadingProgress,
            { done: progress.done || 0, total: progress.total || 0 },
        );
        ratio = transferRatio(progress);
        detail = transferDetail(progress);
    } else if (progress.phase === "done") {
        title = s.progressDone;
        ratio = 1;
    }
    return { title, ratio, detail };
}

/**
 * Fraction of a transfer phase that is complete. Bytes across all concurrent
 * files give a smooth bar; the file count is the fallback when a size is
 * unknown.
 */
function transferRatio(progress) {
    if (progress.bytesTotal) {
        return Math.min(1, (progress.bytesDone || 0) / progress.bytesTotal);
    }
    if (progress.total) {
        return Math.min(1, (progress.done || 0) / progress.total);
    }
    return null;
}

/** "IMG_0001.CR3 · 12 MB / 48 MB" for the file currently moving. */
function transferDetail(progress) {
    const file = progress.file || "";
    if (progress.bytesTotal) {
        return `${file} · ${formatBytes(progress.bytesDone)} / ${formatBytes(progress.bytesTotal)}`;
    }
    return file;
}

/** Apply a progress view to a bar fill and its percentage label. */
function paintBar(fill, percent, ratio) {
    if (typeof ratio === "number") {
        fill.style.width = `${Math.max(2, ratio * 100)}%`;
        percent.textContent = `${Math.round(ratio * 100)}%`;
    } else {
        fill.style.width = "";
        percent.textContent = "";
    }
}

function renderProgress() {
    const s = state.strings;
    if (!s) return;
    if (!state.syncBusy || !state.progress) {
        hide(els.progressWrap);
        return;
    }
    show(els.progressWrap);
    const view = syncProgressView(state.progress, s);
    els.progressTitle.textContent = view.title;
    els.progressBar.classList.toggle("indeterminate", typeof view.ratio !== "number");
    paintBar(els.progressFill, els.progressPercent, view.ratio);
    els.progressText.textContent = view.detail;
}

function renderSyncModal() {
    if (!state.strings) return;
    if (!state.syncBusy) {
        hide(els.syncModal);
        return;
    }
    const s = state.strings;
    const progress = state.progress || { phase: "manifest" };
    const view = syncProgressView(progress, s);
    const name = state.syncTarget?.name;
    show(els.syncModal);
    els.syncModalTitle.textContent = name
        ? fmt(s.syncingGallery, { name })
        : s.syncing;
    els.syncModalPhase.textContent = view.title;
    els.syncModalFile.textContent = view.detail;
    els.syncModalHint.textContent = s.syncModalHint;
    els.syncModal
        .querySelector(".sync-modal")
        ?.classList.toggle("indeterminate", typeof view.ratio !== "number");
    paintBar(els.syncModalFill, els.syncModalPercent, view.ratio);
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

    const failed = (report.errors?.length || 0) > 0;
    const idle = changes === 0 && (report.proposals?.deleteRemote?.length || 0) === 0;
    els.reportSummary.textContent = failed ? s.syncFailed : idle ? s.inSync : s.syncDone;
    els.reportIcon.classList.toggle("error", failed);
    els.reportIconUse.setAttribute("href", failed ? "#i-alert" : "#i-check");
    els.reportSummary.parentElement.className = `report-title ${failed ? "error" : "ok"}`;
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
        const chip = el("span", "count-chip", `${label}: ${value}`);
        chip.classList.toggle("warn", label === s.errors && value > 0);
        chip.classList.toggle("muted", value === 0);
        els.reportCounts.appendChild(chip);
    }

    els.reportErrors.replaceChildren();
    for (const error of report.errors || []) {
        const line = document.createElement("div");
        line.className = "report-error";
        line.textContent = `${error.filename || error.path || "—"} — ${error.message}`;
        els.reportErrors.appendChild(line);
    }
    if ((report.conflicts || []).length > 0) {
        const hint = document.createElement("div");
        hint.className = "report-conflict-hint";
        hint.textContent = s.conflictHint;
        els.reportErrors.appendChild(hint);
    }
    for (const conflict of report.conflicts || []) {
        const line = document.createElement("div");
        line.className = "report-error";
        line.textContent = fmt(s.conflictDetail, {
            filename: conflict.local?.filename || conflict.remote?.filename || "—",
            remote: conflict.remote?.versionCount ?? 1,
            remoteSize: formatBytes(conflict.remote?.size || 0),
            localSize: formatBytes(conflict.local?.size || 0),
        });
        els.reportErrors.appendChild(line);
    }

    els.reportWarnings.replaceChildren();
    const warnings = report.warnings || [];
    if (warnings.length === 0) {
        hide(els.reportWarnings);
    } else {
        show(els.reportWarnings);
        for (const warning of warnings) {
            const line = document.createElement("div");
            line.className = "report-warning";
            line.textContent = warningText(warning);
            els.reportWarnings.appendChild(line);
        }
    }

    els.reportFiles.replaceChildren();
    const fileLines = [];
    for (const item of report.trashedLocal || []) {
        fileLines.push(`${s.deletionLocalDelete}: ${item.filename}`);
    }
    for (const item of report.uploaded || []) {
        const version = item.version_number ? ` (v${item.version_number})` : "";
        const reason =
            item.reason === "modified" ? s.localModifiedReason : s.localNewReason;
        fileLines.push(
            `${s.uploadedFiles}: ${item.filename}${version} — ${reason}`,
        );
    }
    for (const item of report.downloaded || []) {
        fileLines.push(`${s.downloadedFiles}: ${item.filename}`);
    }
    if (fileLines.length === 0) {
        hide(els.reportFiles);
    } else {
        show(els.reportFiles);
        for (const text of fileLines) {
            const line = document.createElement("div");
            line.className = "report-file";
            line.textContent = text;
            els.reportFiles.appendChild(line);
        }
    }

    const ignoredLocal = report.ignoredLocal || [];
    const ignoredRemote = report.ignoredRemote || [];
    if (ignoredLocal.length === 0 && ignoredRemote.length === 0) {
        hide(els.reportIgnored);
    } else {
        show(els.reportIgnored);
        const parts = [];
        if (ignoredLocal.length > 0) {
            parts.push(fmt(s.ignoredLocal, { n: ignoredLocal.length }));
        }
        if (ignoredRemote.length > 0) {
            parts.push(fmt(s.ignoredRemote, { n: ignoredRemote.length }));
        }
        els.reportIgnoredText.textContent = parts.join(" · ");
        els.btnIncludeIgnored.textContent = s.includeIgnored;
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
    const isPlan = state.rating.mode === "plan";
    const favourites = state.rating.favorites || [];
    els.ratingTitle.textContent = isPlan
        ? favourites.length > 0
            ? s.ratingTitle
            : s.ratingPlanTitle
        : state.rating.source === "favorites"
          ? s.ratingTitle
          : s.ratePhotosTitle;
    els.ratingSubtitle.textContent = isPlan
        ? favourites.length > 0
            ? fmt(s.ratingPlanHintStars, {
                  n: favourites.length,
                  total: (state.rating.plan || []).length + favourites.length,
              })
            : fmt(s.ratingPlanHint, { n: (state.rating.plan || []).length })
        : state.rating.source === "favorites"
          ? fmt(s.favoritesFound, { n: state.rating.files.length })
          : s.ratePhotosHint;
    const target = ratingTargetText();
    els.ratingTarget.textContent = target || s.noFolder;
    const needsFolder = !state.rating.galleryId && !target;
    els.btnRatingFolder.classList.toggle("hidden", !needsFolder);

    // The star picker only appears when Picstome has favourites with a heart
    // but no rating: the photographer chooses one value for all of them.
    const askStars = isPlan ? favourites.length > 0 : true;
    els.ratingStars.classList.toggle("hidden", !askStars);

    const plan = state.rating.plan || [];
    if (plan.length > 0) {
        show(els.ratingPlan);
        els.ratingPlan.replaceChildren();
        for (const item of plan) {
            const line = document.createElement("div");
            line.className = "rating-plan-item";
            line.textContent =
                item.rating == null
                    ? fmt(s.ratingPlanItemUnrated, { filename: item.filename })
                    : fmt(s.ratingPlanItem, {
                          filename: item.filename,
                          stars: item.rating,
                      });
            els.ratingPlan.appendChild(line);
        }
    } else {
        hide(els.ratingPlan);
        els.ratingPlan.replaceChildren();
    }

    const value = Number(state.rating.value);
    for (const button of els.starButtons) {
        button.classList.toggle("active", Number(button.dataset.rating) <= value);
    }
    els.ratingValue.textContent = state.rating.value;

    els.btnRatingApply.disabled =
        state.rating.busy ||
        (!isPlan && state.rating.files.length === 0) ||
        (isPlan && plan.length === 0 && favourites.length === 0) ||
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

/** Where a trashed file ended up, for log and result lines. */
function trashTarget(item, s) {
    return (
        item.trashPath ||
        (item.location === "gallery" ? s.deletionTrashGallery : s.deletionTrashSystem)
    );
}

/** Sum of two optional counters (old log entries may lack either). */
function sum(a, b) {
    return (Number(a) || 0) + (Number(b) || 0);
}

/** Header text and body lines for one activity-log entry. */
function logEntryView(entry, s) {
    const at = formatDate(entry.at);
    const lines = [];
    const add = (text, cls = "") => lines.push({ text, cls });

    if (entry.kind === "sync_error") {
        add(entry.message || "", "log-error");
        return { head: `${at} · ${s.syncFailed}`, lines };
    }

    if (entry.kind === "deletion") {
        const actions = {
            remote_delete: s.deletionRemoteDelete,
            delete_local: s.deletionLocalDelete,
            keep_local: s.deletionKeepLocal,
            upload: s.deletionUploadAgain,
        };
        const statuses = {
            deleted: s.deletionStatusDeleted,
            not_found: s.deletionStatusNotFound,
            failed: s.deletionStatusFailed,
        };
        const files = entry.files || [];
        for (const item of files) {
            const status = statuses[item.status];
            add(`${item.filename || item.id}${status ? ` — ${status}` : ""}`);
        }
        for (const item of entry.trashed || []) {
            add(`${item.filename} — ${fmt(s.trashedTo, { path: trashTarget(item, s) })}`);
        }
        for (const error of entry.errors || []) add(`${s.errors}: ${error}`, "log-error");
        return {
            head: `${at} · ${actions[entry.action] || entry.action} · ${files.length}`,
            lines,
        };
    }

    if (entry.kind === "rating") {
        const sources = {
            picstome: s.ratingSourcePicstome,
            favorites: s.ratingSourceFavorites,
        };
        const files = entry.files || [];
        for (const item of files.filter((file) => file.status === "ok")) {
            add(`${s.ratedFiles}: ${item.file || item.name}`);
        }
        for (const item of files.filter((file) => file.status !== "ok")) {
            add(`${item.name} — ${item.message || item.status}`, "log-error");
        }
        const summary = fmt(s.ratingLogLine, {
            stars: entry.rating === "mixed" ? "1–5" : entry.rating,
            n: entry.rated,
        });
        return {
            head: `${at} · ${summary} · ${sources[entry.source] || s.ratingSourceLink}`,
            lines,
        };
    }

    // Regular sync entry.
    const counts = entry.counts || {};
    for (const item of entry.uploaded || []) {
        const reason = item.reason === "modified" ? s.localModifiedReason : s.localNewReason;
        const version = item.versionNumber ? ` (v${item.versionNumber})` : "";
        add(`${s.uploadedFiles}: ${item.filename}${version} — ${reason}`);
    }
    const named = [
        [entry.downloaded, s.downloadedFiles],
        [entry.ignoredLocal, s.ignoredLocalFiles],
        [entry.ignoredRemote, s.ignoredRemoteFiles],
    ];
    for (const [names, label] of named) {
        for (const filename of names || []) add(`${label}: ${filename}`);
    }
    const items = [
        [entry.trashedLocal, s.deletionLocalDelete],
        [entry.localDeletedProposed, s.localDeletionProposed],
        [entry.remoteDeleted, s.remoteDeletionDetected],
    ];
    for (const [list, label] of items) {
        for (const item of list || []) add(`${label}: ${item.filename}`);
    }
    for (const item of entry.conflicts || []) {
        add(`${s.conflicts}: ${item.filename || item.remoteFilename || "—"}`);
    }
    for (const warning of entry.warnings || []) add(warningText(warning), "log-warning");
    for (const error of entry.errors || []) add(`${s.errors}: ${error}`, "log-error");

    const parts = [
        `${s.downloadedFiles}: ${sum(counts.remoteNew, counts.remoteModified)}`,
        `${s.uploadedFiles}: ${sum(counts.localNew, counts.localModified)}`,
        `${s.errors}: ${(entry.errors || []).length}`,
    ];
    return { head: `${at} · ${parts.join(" · ")}`, lines };
}

function renderLog() {
    const s = state.strings;
    if (!s) return;
    if (!state.currentGalleryId) {
        hide(els.syncLog);
        return;
    }
    show(els.syncLog);
    els.syncLogCount.textContent = String(state.syncLog.length);
    els.syncLogEntries.replaceChildren();

    if (state.syncLog.length === 0) {
        els.syncLogEntries.appendChild(el("div", "log-empty", s.noActivity));
        return;
    }

    for (const entry of state.syncLog) {
        const view = logEntryView(entry, s);
        const card = el("div", "log-entry");
        const body = el("div", "log-entry-body");
        for (const line of view.lines) body.appendChild(el("div", line.cls, line.text));
        card.append(el("div", "log-entry-head", view.head), body);
        els.syncLogEntries.appendChild(card);
    }
}

async function clearLog() {
    const galleryId = state.currentGalleryId;
    if (!galleryId) return;
    try {
        await invoke("clear_sync_log", { galleryId });
        state.syncLog = [];
        renderLog();
    } catch {
        // Keep the existing log visible if the file cannot be cleared.
    }
}

async function refreshLog() {
    const galleryId = state.currentGalleryId;
    if (!galleryId) {
        state.syncLog = [];
        renderLog();
        return;
    }
    try {
        state.syncLog = await invoke("get_sync_log", { galleryId });
    } catch {
        state.syncLog = [];
    }
    renderLog();
}

function renderRemoteDeleted() {
    const s = state.strings;
    if (!s) return;
    const modal = state.remoteDeleted;
    if (!modal.open) {
        hide(els.remoteDeletedModal);
        return;
    }
    show(els.remoteDeletedModal);
    els.remoteDeletedHint.textContent = fmt(s.remoteDeletedHint, {
        n: modal.items.length,
    });
    els.remoteDeletedList.replaceChildren();
    for (const item of modal.items) {
        const label = document.createElement("label");
        label.className = "delete-item";
        const checkbox = document.createElement("input");
        checkbox.type = "checkbox";
        checkbox.checked = true;
        checkbox.dataset.remoteId = item.id;
        const text = document.createElement("span");
        text.textContent = item.filename;
        label.appendChild(checkbox);
        label.appendChild(text);
        els.remoteDeletedList.appendChild(label);
    }
    els.btnRemoteDeleteLocal.disabled = modal.busy || modal.items.length === 0;
    els.btnRemoteKeepLocal.disabled = modal.busy || modal.items.length === 0;
    els.btnRemoteUploadAgain.disabled = modal.busy || modal.items.length === 0;

    if (modal.result) {
        show(els.remoteDeletedResult);
        els.remoteDeletedResult.className = `rating-result ${
            modal.result.errors?.length ? "error" : "ok"
        }`;
        const resolved = modal.result.resolved?.length || 0;
        const errors = modal.result.errors || [];
        if (errors.length) {
            els.remoteDeletedResult.textContent = `${s.remoteDeletedError}: ${errors
                .map((error) => error.message)
                .join(" · ")}`;
        } else {
            const trashed = modal.result.trashed || [];
            const details = trashed.map(
                (item) => `${item.filename} — ${fmt(s.trashedTo, { path: trashTarget(item, s) })}`,
            );
            els.remoteDeletedResult.textContent = [
                `${s.remoteDeletedDone} (${resolved})`,
                ...details,
            ].join("\n");
        }
    } else {
        hide(els.remoteDeletedResult);
    }
}

function handlePostSync(report) {
    const remoteDeleted = report?.remoteDeleted || [];
    if (remoteDeleted.length > 0) {
        state.remoteDeleted = {
            open: true,
            galleryId: report.galleryId,
            items: remoteDeleted,
            busy: false,
            result: null,
            pendingFavorites: report,
        };
        renderRemoteDeleted();
        return;
    }
    openFavoritesPrompt(report);
}

function fillFirstSyncList(container, items, s) {
    container.replaceChildren();
    const visible = items.slice(0, 8);
    for (const item of visible) {
        const line = document.createElement("div");
        line.textContent = item.filename;
        container.appendChild(line);
    }
    if (items.length > visible.length) {
        const line = document.createElement("div");
        line.textContent = fmt(s.firstSyncMore, {
            n: items.length - visible.length,
        });
        container.appendChild(line);
    }
}

function renderFirstSync() {
    const s = state.strings;
    if (!s) return;
    const modal = state.firstSync;
    if (!modal.open || !modal.preview) {
        hide(els.firstSyncModal);
        return;
    }
    const preview = modal.preview;
    show(els.firstSyncModal);
    els.firstSyncTitle.textContent = s.firstSyncTitle;
    els.firstSyncHint.textContent = s.firstSyncHint;
    const hasUpload = (preview.localOnly || []).length > 0;
    const hasDownload = (preview.remoteOnly || []).length > 0;
    els.firstSyncUploadSection.classList.toggle("hidden", !hasUpload);
    els.firstSyncDownloadSection.classList.toggle("hidden", !hasDownload);
    els.firstSyncLocalUpload.checked = modal.localAction === "upload";
    els.firstSyncLocalKeep.checked = modal.localAction === "keep";
    els.firstSyncLocalTrash.checked = modal.localAction === "trash";
    els.firstSyncDownload.checked = modal.download;
    els.firstSyncUploadLabel.textContent = fmt(s.firstSyncUpload, {
        n: (preview.localOnly || []).length,
    });
    els.firstSyncUploadHint.textContent = s.firstSyncUploadHint;
    els.firstSyncLocalUploadLabel.textContent = s.firstSyncLocalUpload;
    els.firstSyncLocalKeepLabel.textContent = s.firstSyncLocalKeep;
    els.firstSyncLocalTrashLabel.textContent = s.firstSyncLocalTrash;
    els.firstSyncDownloadLabel.textContent = fmt(s.firstSyncDownload, {
        n: (preview.remoteOnly || []).length,
    });
    els.firstSyncDownloadHint.textContent = s.firstSyncDownloadHint;
    fillFirstSyncList(els.firstSyncUploadList, preview.localOnly || [], s);
    fillFirstSyncList(els.firstSyncDownloadList, preview.remoteOnly || [], s);
    const trashAction = hasUpload && modal.localAction === "trash";
    els.btnFirstSyncConfirm.textContent = trashAction
        ? s.firstSyncConfirmTrash
        : s.firstSyncConfirm;
    els.btnFirstSyncConfirm.className = trashAction ? "danger" : "primary";
    els.btnFirstSyncCancel.textContent = s.cancel;
    els.btnFirstSyncClose.textContent = s.close;
}

function settleFirstSync(confirmed) {
    const modal = state.firstSync;
    const resolve = modal.resolve;
    const preview = modal.preview;
    const localAction = modal.localAction;
    const download = modal.download;
    state.firstSync = {
        open: false,
        preview: null,
        localAction: "upload",
        download: true,
        resolve: null,
    };
    renderFirstSync();
    if (!resolve) return;
    if (!confirmed || !preview) {
        resolve(null);
        return;
    }
    const localOnly = preview.localOnly || [];
    resolve({
        ignoreLocal: localAction === "keep" ? localOnly : [],
        trashLocal: localAction === "trash" ? localOnly : [],
        ignoreRemote: download ? [] : (preview.remoteOnly || []),
    });
}

function askFirstSync(preview) {
    if (state.firstSync.resolve) settleFirstSync(false);
    return new Promise((resolve) => {
        state.firstSync = {
            open: true,
            preview,
            localAction: "upload",
            download: true,
            resolve,
        };
        renderFirstSync();
    });
}

function renderConfirm() {
    const modal = state.confirm;
    if (!modal.open) {
        hide(els.confirmModal);
        return;
    }
    show(els.confirmModal);
    els.confirmTitle.textContent = modal.title;
    els.confirmMessage.textContent = modal.message;
    els.btnConfirmOk.textContent = modal.confirmLabel;
    els.btnConfirmOk.className = modal.danger ? "danger" : "primary";
}

function settleConfirm(value) {
    const resolve = state.confirm.resolve;
    state.confirm = {
        open: false,
        title: "",
        message: "",
        confirmLabel: "",
        danger: true,
        resolve: null,
    };
    renderConfirm();
    if (resolve) resolve(Boolean(value));
}

function askConfirm({ title, message, confirmLabel, danger = true }) {
    if (state.confirm.resolve) settleConfirm(false);
    return new Promise((resolve) => {
        state.confirm = {
            open: true,
            title,
            message,
            confirmLabel,
            danger,
            resolve,
        };
        renderConfirm();
    });
}

function renderAll() {
    applyStrings();
    renderAccount();
    renderRuntimeInfo();
    renderGalleryList();
    renderMain();
    renderProgress();
    renderSyncModal();
    renderReport();
    renderRating();
    renderFavorites();
    renderFirstSync();
    renderRemoteDeleted();
    renderConfirm();
    renderLog();
}

function setState(next) {
    if (!next) return;
    state.lang = next.lang || state.lang;
    state.strings = next.strings || state.strings;
    state.config = next.config || state.config;
    state.mock = Boolean(next.mock);
    state.auth = next.auth || state.auth;
    state.galleries = Array.isArray(next.galleries) ? next.galleries : [];
    maybeLoadRecentGalleries();
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
    if (state.currentGalleryId) {
        refreshLog();
    } else {
        state.syncLog = [];
        renderLog();
    }
}

async function refreshState() {
    const next = await invoke("get_state");
    setState(next);
    return next;
}

function openFavoritesPrompt(report) {
    const favorites = report?.favorites || [];
    const plan = report?.ratingPlan?.items || [];
    if (favorites.length === 0 && plan.length === 0) return;
    state.rating = {
        open: true,
        // Ask once for every favourite without a rating. The exact Picstome
        // ratings of the other photos are written as-is.
        files: favorites.map((photo) => stem(photo.filename)),
        value: "5",
        galleryId: report.galleryId,
        photosDir: null,
        source: "favorites",
        mode: "plan",
        favorites: favorites,
        plan: plan,
        busy: false,
        result: null,
    };
    renderRating();
}

function filteredFavoritePhotos() {
    const query = state.favorites.query.trim().toLowerCase();
    if (!query) return state.favorites.items;
    return state.favorites.items.filter((photo) =>
        String(photo.filename || "")
            .toLowerCase()
            .includes(query),
    );
}

function favoritesTabText(items, tab) {
    const names = items.map((photo) => photo.filename).filter(Boolean);
    const stems = names.map(stem);
    if (tab === "lightroom") return stems.join(", ");
    if (tab === "captureone") return stems.join(" ");
    if (tab === "finder") return stems.join(" OR ");
    return names.join("\n");
}

function favoritesHintKey(tab) {
    if (tab === "lightroom") return "favoritesLightroomHint";
    if (tab === "captureone") return "favoritesCaptureOneHint";
    if (tab === "finder") return "favoritesFinderHint";
    return "favoritesTextHint";
}

function renderFavorites() {
    const s = state.strings;
    if (!s) return;
    if (!state.favorites.open) {
        hide(els.favoritesModal);
        return;
    }

    show(els.favoritesModal);
    const total = state.favorites.items.length;
    const items = filteredFavoritePhotos();
    const busy = state.favorites.busy;
    const text = busy || total === 0
        ? ""
        : favoritesTabText(items, state.favorites.tab);

    els.favoritesSubtitle.textContent = busy
        ? s.favoritesListLoading
        : total === 0
          ? s.favoritesListEmpty
          : fmt(s.favoritesListSubtitle, { n: total });
    els.favoritesFilter.disabled = busy || total === 0;
    els.favoritesText.value = text;
    els.btnFavoritesCopy.disabled = busy || !text;

    for (const button of els.favoritesTabButtons) {
        button.classList.toggle(
            "active",
            button.dataset.tab === state.favorites.tab,
        );
    }

    if (state.favorites.error) {
        show(els.favoritesResult);
        els.favoritesResult.className = "rating-result error";
        els.favoritesResult.textContent = state.favorites.error;
    } else {
        hide(els.favoritesResult);
    }

    els.favoritesHint.textContent = busy
        ? s.favoritesListLoading
        : total === 0
          ? s.favoritesListEmpty
          : items.length === 0
            ? s.favoritesFilterEmpty
            : s[favoritesHintKey(state.favorites.tab)] || "";
}

function closeFavorites() {
    state.favorites.open = false;
    renderFavorites();
}

async function openFavoritesList() {
    const gallery = currentGallery();
    if (!gallery) return;

    state.favorites = {
        open: true,
        galleryId: gallery.galleryId,
        items: [],
        tab: "lightroom",
        query: "",
        busy: true,
        error: null,
    };
    els.favoritesFilter.value = "";
    renderFavorites();

    try {
        const items = await invoke("get_gallery_favorites", {
            galleryId: gallery.galleryId,
        });
        if (
            !state.favorites.open ||
            state.favorites.galleryId !== gallery.galleryId
        ) {
            return;
        }
        state.favorites.items = Array.isArray(items) ? items : [];
    } catch (err) {
        if (!state.favorites.open) return;
        state.favorites.error = `${state.strings?.favoritesListLoadFailed || "Could not load favorites"}: ${
            err?.message || String(err)
        }`;
    } finally {
        if (state.favorites.galleryId === gallery.galleryId) {
            state.favorites.busy = false;
            renderFavorites();
        }
    }
}

function openRateLink({ files, rating, gallery }) {
    const requested = gallery
        ? state.galleries.find((item) => item.galleryId === gallery)
        : null;
    const current = currentGallery();
    const targetGallery = requested || current;
    state.rating = {
        open: true,
        files: Array.isArray(files) ? files : [],
        value: normalizeRating(rating || state.config.lastRating || "1"),
        galleryId: targetGallery?.galleryId || null,
        photosDir: targetGallery ? null : state.config.photosDir || null,
        source: "link",
        favorites: [],
        busy: false,
        result: null,
    };
    renderRating();
}

function isUnknownGalleryError(err) {
    const code = err?.code || err?.error?.code;
    return (
        code === "unknown_gallery" ||
        String(err?.message || err || "").includes("unknown_gallery")
    );
}

async function handleUnknownGallery(gallery) {
    const s = state.strings;
    const remove = await askConfirm({
        title: s.galleryMissingTitle,
        message: fmt(s.galleryMissingMessage, {
            name: gallery.name || gallery.galleryId,
        }),
        confirmLabel: s.galleryMissingRemove,
        danger: false,
    });
    if (remove) {
        await invoke("unlink_gallery", { galleryId: gallery.galleryId });
        if (state.currentGalleryId === gallery.galleryId) {
            state.currentGalleryId = null;
        }
        state.report = null;
        state.syncLog = [];
    }
    await refreshState();
    renderAll();
}

/* Sync lifecycle
 *
 * `state.syncBusy` is owned by these functions only: it turns on before the
 * first backend call and off when the last one returns. Progress events update
 * the bars but never flip it, so a late or early "done" event cannot re-enable
 * the Sync button while the backend is still writing the snapshot.
 */

function beginSync(galleryId, name) {
    state.syncTarget = { galleryId, name: name || "Gallery" };
    state.syncBusy = true;
    state.progress = { phase: "manifest", done: 0, total: 1 };
    state.report = null;
    renderAll();
}

function endSync() {
    state.syncBusy = false;
    state.syncTarget = null;
    state.progress = null;
}

/**
 * Map a failed sync command onto the UI. Returns true when the gallery no
 * longer exists in Picstome (the caller then skips its usual follow-up).
 */
async function handleSyncError(err, gallery) {
    if (isUnknownGalleryError(err)) {
        await handleUnknownGallery(gallery);
        return true;
    }
    state.report = syncErrorReport(err);
    return false;
}

/** Run the actual sync once any first-sync choice has been made. */
async function runSync(galleryId, name, choice = null) {
    if (state.syncBusy) return;
    beginSync(galleryId, name);
    let missing = false;
    try {
        const args = { galleryId };
        if (choice) {
            args.ignoreLocal = choice.ignoreLocal || [];
            args.ignoreRemote = choice.ignoreRemote || [];
            args.trashLocal = choice.trashLocal || [];
        }
        state.report = await invoke("sync_gallery", args);
    } catch (err) {
        missing = await handleSyncError(err, { galleryId, name: name || galleryId });
    } finally {
        endSync();
        if (!missing) await refreshState();
        renderAll();
    }
    if (!missing) handlePostSync(state.report);
}

/** Sync the selected gallery, asking first when this is its first sync. */
async function syncCurrent() {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;

    beginSync(gallery.galleryId, gallery.name);
    let preview = null;
    try {
        preview = await invoke("preview_first_sync", {
            galleryId: gallery.galleryId,
        });
    } catch (err) {
        endSync();
        if (!(await handleSyncError(err, gallery))) renderAll();
        return;
    }
    endSync();
    renderAll();

    let choice = null;
    if (preview?.needed) {
        choice = await askFirstSync(preview);
        if (!choice) return;
    }
    await runSync(gallery.galleryId, gallery.name, choice);
}

/**
 * Link (if needed) and sync a gallery picked from Picstome: asks for a
 * folder, previews a first sync and syncs straight away when no choice is
 * needed.
 */
async function startSync(galleryId, name) {
    if (state.syncBusy) return;
    const label = name || "Gallery";
    beginSync(galleryId, label);

    let result = null;
    let missing = false;
    try {
        result = await invoke("start_sync", { galleryId, name });
        if (!result?.canceled) {
            state.currentGalleryId = result?.report?.galleryId || galleryId;
        }
        if (result?.report) state.report = result.report;
    } catch (err) {
        missing = await handleSyncError(err, { galleryId, name: label });
    } finally {
        endSync();
        if (!missing && !result?.canceled) await refreshState();
        renderAll();
    }

    if (missing || !result || result.canceled) return;
    if (result.preview?.needed) {
        const choice = await askFirstSync(result.preview);
        if (choice) await runSync(galleryId, label, choice);
    } else if (result.report) {
        handlePostSync(result.report);
    }
}

async function applyRating() {
    if (state.rating.busy) return;
    const appliedState = state.rating;
    const isPlan = appliedState.mode === "plan";
    if (!isPlan && appliedState.files.length === 0) return;
    appliedState.busy = true;
    appliedState.result = null;
    renderRating();
    try {
        if (isPlan) {
            // Sync flow: write the exact Picstome ratings, and answer the
            // "favourite without a rating" prompt with the chosen value.
            const favourites = appliedState.favorites || [];
            const result = await invoke("apply_rating_plan", {
                galleryId: appliedState.galleryId,
                favouritesWithoutRating: favourites.length > 0
                    ? Number(appliedState.value)
                    : null,
            });
            appliedState.result = result;
            if (result.errors === 0 && result.rated > 0) {
                setTimeout(() => {
                    if (state.rating === appliedState) {
                        state.rating.open = false;
                        renderRating();
                    }
                }, 1100);
            }
            return;
        }
        const result = await invoke("rate_files", {
            files: appliedState.files,
            rating: appliedState.value,
            photosDir: appliedState.photosDir,
            galleryId: appliedState.galleryId,
            favorites: appliedState.favorites,
        });
        appliedState.result = result;
        state.config.lastRating = appliedState.value;
        if (result.errors === 0 && result.rated > 0) {
            setTimeout(() => {
                if (state.rating === appliedState) {
                    state.rating.open = false;
                    renderRating();
                }
            }, 1100);
        }
    } catch (err) {
        appliedState.result = {
            rated: 0,
            notFound: 0,
            errors: 1,
            details: [{ name: "—", status: "error", msg: err?.message || String(err) }],
        };
    } finally {
        appliedState.busy = false;
        renderRating();
        await refreshLog();
    }
}

async function resolveRemoteDeleted(action) {
    const modal = state.remoteDeleted;
    if (!modal.galleryId || modal.busy) return;
    const ids = Array.from(
        els.remoteDeletedList.querySelectorAll("input[type=checkbox]:checked"),
    ).map((checkbox) => checkbox.dataset.remoteId);
    if (ids.length === 0) return;
    modal.busy = true;
    modal.result = null;
    renderRemoteDeleted();
    try {
        const result = await invoke("resolve_remote_deletions", {
            galleryId: modal.galleryId,
            photoIds: ids,
            action,
        });
        modal.result = result;
        const resolved = new Set(result.resolved || []);
        modal.items = modal.items.filter((item) => !resolved.has(item.id));
        if (state.report?.remoteDeleted) {
            state.report.remoteDeleted = state.report.remoteDeleted.filter(
                (item) => !resolved.has(item.id),
            );
        }
        if (modal.items.length === 0) {
            const pending = modal.pendingFavorites;
            modal.open = false;
            modal.pendingFavorites = null;
            await refreshState();
            renderAll();
            if (pending) openFavoritesPrompt(pending);
        } else {
            await refreshState();
            renderAll();
        }
    } catch (err) {
        modal.result = {
            resolved: [],
            errors: [{ message: err?.message || String(err) }],
        };
    } finally {
        modal.busy = false;
        renderRemoteDeleted();
    }
}

/** Point a gallery whose folder went missing at its new location. */
async function locateCurrent() {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;
    try {
        const result = await invoke("locate_gallery", { galleryId: gallery.galleryId });
        if (result?.ok) await refreshState();
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
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

async function checkForUpdates() {
    try {
        const update = await invoke("check_updates");
        if (!update?.available || !update.url) return;
        const s = state.strings;
        const download = await askConfirm({
            title: s.updateAvailableTitle,
            message: fmt(s.updateAvailableMessage, {
                current: update.currentVersion || state.config.appVersion || "?",
                latest: update.latestVersion || "?",
            }),
            confirmLabel: s.updateDownload,
            danger: false,
        });
        if (download) {
            await invoke("open_update_page", { url: update.url });
        }
    } catch {
        // Update checks are best-effort and must never block startup.
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
        // Only the sync this window started drives the bars; see the note on
        // the sync lifecycle for why `syncBusy` is not touched here.
        if (!state.syncBusy || progress.galleryId !== state.syncTarget?.galleryId) {
            return;
        }
        state.progress = progress;
        renderProgress();
        renderSyncModal();
    });

    await invoke("frontend_ready");
    checkForUpdates();
}

/* UI bindings */

els.galleryList.addEventListener("click", (event) => {
    const more = event.target.closest(".remote-more");
    if (more) {
        if (!state.remoteGalleries.loading) {
            searchRemoteGalleries(Number(more.dataset.page) || 1, false);
        }
        return;
    }
    const row = event.target.closest(".gallery-row");
    if (!row) return;
    if (row.dataset.remote === "1") {
        startSync(row.dataset.galleryId, row.dataset.galleryName);
        return;
    }
    state.currentGalleryId = row.dataset.galleryId;
    state.report = null;
    renderAll();
    refreshLog();
});

els.galleryList.addEventListener("dblclick", (event) => {
    const row = event.target.closest(".gallery-row");
    if (!row || row.dataset.remote === "1") return;
    state.currentGalleryId = row.dataset.galleryId;
    renderAll();
    syncCurrent();
});

els.btnSync.addEventListener("click", syncCurrent);
els.btnFavoritesList.addEventListener("click", openFavoritesList);
els.favoritesFilter.addEventListener("input", () => {
    state.favorites.query = els.favoritesFilter.value;
    renderFavorites();
});
for (const button of els.favoritesTabButtons) {
    button.addEventListener("click", () => {
        state.favorites.tab = button.dataset.tab;
        renderFavorites();
    });
}
els.btnFavoritesCopy.addEventListener("click", async () => {
    const text = els.favoritesText.value;
    if (!(await copyText(text))) {
        els.favoritesHint.textContent =
            state.strings?.favoritesListCopyFailed || "Could not copy";
        return;
    }
    const original = state.strings?.favoritesListCopy || "Copy";
    els.btnFavoritesCopy.textContent =
        state.strings?.favoritesListCopied || "Copied!";
    setTimeout(() => {
        els.btnFavoritesCopy.textContent = original;
    }, 1600);
});
els.btnClearLog.addEventListener("click", clearLog);
els.btnIdleSync.addEventListener("click", () => {
    const gallery = currentGallery();
    if (gallery && !gallery.exists) {
        locateCurrent();
    } else {
        syncCurrent();
    }
});
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

async function toggleTestMode() {
    const testMode = state.config.apiKind === "development";
    const s = state.strings;
    const confirmed = await askConfirm({
        title: testMode ? s.disableTestModeTitle : s.enableTestModeTitle,
        message: testMode ? s.disableTestModeMessage : s.enableTestModeMessage,
        confirmLabel: testMode ? s.disableTestModeAction : s.enableTestModeAction,
        danger: false,
    });
    if (!confirmed) return;
    try {
        await invoke("set_test_mode", { enabled: !testMode });
        hide(els.infoModal);
        await refreshState();
    } catch (err) {
        els.testModeHint.textContent = err?.message || String(err);
    }
}

els.btnInfo.addEventListener("click", () => show(els.infoModal));
els.btnTestMode.addEventListener("click", toggleTestMode);

els.btnChangeFolder.addEventListener("click", async () => {
    if (!state.currentGalleryId || els.btnChangeFolder.disabled) return;
    const s = state.strings;
    const original = s.changeFolder;
    els.btnChangeFolder.disabled = true;
    setLabel(els.btnChangeFolder, s.choosingFolder || original);
    try {
        const result = await invoke("change_folder", {
            galleryId: state.currentGalleryId,
        });
        if (result.ok) {
            state.report = null;
            await refreshState();
        }
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
    } finally {
        els.btnChangeFolder.disabled = state.syncBusy;
        setLabel(els.btnChangeFolder, state.strings?.changeFolder || original);
    }
});

els.btnOpenWebGallery.addEventListener("click", async () => {
    if (!state.currentGalleryId) return;
    try {
        await invoke("open_gallery_in_picstome", {
            galleryId: state.currentGalleryId,
        });
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
    }
});

els.detailPath.addEventListener("click", async () => {
    if (!state.currentGalleryId || els.detailPath.disabled) return;
    await invoke("open_folder", { galleryId: state.currentGalleryId });
});

els.btnUnlink.addEventListener("click", async () => {
    if (!state.currentGalleryId) return;
    const gallery = currentGallery();
    const s = state.strings;
    const confirmed = await askConfirm({
        title: s.confirmUnlinkTitle,
        message: fmt(s.confirmUnlinkMessage, {
            name: gallery?.name || state.currentGalleryId,
        }),
        confirmLabel: s.confirmUnlinkAction,
    });
    if (!confirmed) return;
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
    const confirmed = await askConfirm({
        title: state.strings.confirmRemoteDeleteTitle,
        message: fmt(state.strings.confirmRemoteDeleteMessage, {
            n: ids.length,
        }),
        confirmLabel: state.strings.confirmRemoteDeleteAction,
    });
    if (!confirmed) return;
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
els.btnRatingFolder.addEventListener("click", async () => {
    const folder = await invoke("choose_folder");
    if (folder) {
        state.rating.photosDir = folder;
        state.config.photosDir = folder;
        renderRating();
    }
});

els.btnRemoteDeleteLocal.addEventListener("click", () =>
    resolveRemoteDeleted("delete_local"),
);
els.btnRemoteKeepLocal.addEventListener("click", () =>
    resolveRemoteDeleted("keep_local"),
);
els.btnRemoteUploadAgain.addEventListener("click", () =>
    resolveRemoteDeleted("upload"),
);
for (const radio of [
    els.firstSyncLocalUpload,
    els.firstSyncLocalKeep,
    els.firstSyncLocalTrash,
]) {
    radio.addEventListener("change", () => {
        if (radio.checked) {
            state.firstSync.localAction = radio.value;
            renderFirstSync();
        }
    });
}

els.firstSyncDownload.addEventListener("change", () => {
    state.firstSync.download = els.firstSyncDownload.checked;
});

els.btnFirstSyncConfirm.addEventListener("click", () => settleFirstSync(true));

els.btnIncludeIgnored.addEventListener("click", async () => {
    if (!state.currentGalleryId) return;
    const s = state.strings;
    const local = state.report?.ignoredLocal?.length || 0;
    const remote = state.report?.ignoredRemote?.length || 0;
    if (local === 0 && remote === 0) return;
    const confirmed = await askConfirm({
        title: s.includeIgnoredTitle,
        message: fmt(s.includeIgnoredMessage, { local, remote }),
        confirmLabel: s.includeIgnoredAction,
        danger: false,
    });
    if (!confirmed) return;
    try {
        await invoke("clear_sync_ignores", {
            galleryId: state.currentGalleryId,
        });
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
        return;
    }
    await syncCurrent();
});


els.btnConfirmOk.addEventListener("click", () => settleConfirm(true));

els.gallerySearch.addEventListener("input", () => {
    state.galleryQuery = els.gallerySearch.value;
    renderGalleryList();
    scheduleRemoteSearch();
});

els.gallerySearch.addEventListener("keydown", (event) => {
    if (event.key === "Enter") {
        const first = filteredGalleries()[0];
        if (first) {
            state.currentGalleryId = first.galleryId;
            state.report = null;
            renderAll();
        } else {
            const remote = remoteGalleries()[0];
            if (remote) startSync(remote.id, remote.name);
        }
    } else if (event.key === "Escape") {
        event.stopPropagation();
        els.gallerySearch.value = "";
        state.galleryQuery = "";
        clearRemoteSearch();
        renderGalleryList();
        els.gallerySearch.blur();
    }
});

/* Modals
 *
 * One table describes every dismissible dialog, topmost first. The close
 * button, a click on the backdrop and Escape all go through the same
 * `close`, and a dialog in the middle of an operation (`busy`) stays open.
 */
const MODALS = [
    {
        backdrop: els.confirmModal,
        buttons: [els.btnConfirmCancel],
        isOpen: () => state.confirm.open,
        close: () => settleConfirm(false),
    },
    {
        backdrop: els.firstSyncModal,
        buttons: [els.btnFirstSyncCancel, els.btnFirstSyncClose],
        isOpen: () => state.firstSync.open,
        close: () => settleFirstSync(false),
    },
    {
        backdrop: els.remoteDeletedModal,
        buttons: [els.btnRemoteClose],
        isOpen: () => state.remoteDeleted.open,
        busy: () => state.remoteDeleted.busy,
        close: () => {
            state.remoteDeleted.open = false;
            state.remoteDeleted.pendingFavorites = null;
            renderRemoteDeleted();
        },
    },
    {
        backdrop: els.ratingModal,
        buttons: [els.btnRatingCancel, els.btnRatingClose],
        isOpen: () => state.rating.open,
        busy: () => state.rating.busy,
        close: () => {
            state.rating.open = false;
            renderRating();
        },
    },
    {
        backdrop: els.favoritesModal,
        buttons: [els.btnFavoritesClose],
        isOpen: () => state.favorites.open,
        close: closeFavorites,
    },
    {
        backdrop: els.infoModal,
        buttons: [els.btnInfoClose],
        isOpen: () => !els.infoModal.classList.contains("hidden"),
        close: () => hide(els.infoModal),
    },
];

function dismiss(modal) {
    if (modal.busy?.()) return;
    modal.close();
}

for (const modal of MODALS) {
    for (const button of modal.buttons) {
        button.addEventListener("click", () => dismiss(modal));
    }
    modal.backdrop.addEventListener("click", (event) => {
        if (event.target === modal.backdrop) dismiss(modal);
    });
}

document.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        els.gallerySearch.focus();
        els.gallerySearch.select();
        return;
    }
    if (event.key !== "Escape") return;
    // Close only the topmost dialog, like every desktop app does.
    const top = MODALS.find((modal) => modal.isOpen());
    if (top) {
        event.preventDefault();
        dismiss(top);
    }
});

init();
