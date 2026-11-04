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

    report: document.getElementById("report"),
    reportSummary: document.getElementById("reportSummary"),
    reportTime: document.getElementById("reportTime"),
    reportCounts: document.getElementById("reportCounts"),
    reportErrors: document.getElementById("reportErrors"),
    reportWarnings: document.getElementById("reportWarnings"),
    reportFiles: document.getElementById("reportFiles"),
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
    if (warning.kind === "duplicate_local_ignored" && s?.duplicateLocalIgnored) {
        return fmt(s.duplicateLocalIgnored, {
            filename: warning.filename || "—",
            preferred: warning.preferred || "—",
        });
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

function syncButtonLabel(s, busy) {
    return busy ? `⏳ ${s.syncing}` : `🔄 ${s.sync}`;
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
    els.btnSync.textContent = syncButtonLabel(s, state.syncBusy);
    els.btnIdleSync.textContent = s.sync;
    els.idleTitle.textContent = s.readyToSync;
    els.idleHint.textContent = s.syncHint;
    els.btnChangeFolder.textContent = s.changeFolder;
    els.btnChangeFolder.title = s.changeFolder;
    els.btnChangeFolder.setAttribute("aria-label", s.changeFolder);
    els.btnUnlink.textContent = s.unlink;
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

    els.ratingTitle.textContent =
        state.rating.source === "favorites" ? s.ratingTitle : s.ratePhotosTitle;
    els.ratingSubtitle.textContent =
        state.rating.source === "favorites" ? s.favoritesHint : s.ratePhotosHint;
    els.btnFavoritesList.textContent = `♥ ${s.favoritesList}`;
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
    const row = document.createElement("button");
    row.type = "button";
    row.className = "gallery-row remote-gallery-row";
    row.dataset.remote = "1";
    row.dataset.galleryId = gallery.id;
    row.dataset.galleryName = gallery.name || "Gallery";
    row.title = s.remoteGalleryHint;

    const avatar = document.createElement("span");
    avatar.className = "gallery-avatar remote-gallery-avatar";
    avatar.textContent = "+";
    row.appendChild(avatar);

    const info = document.createElement("span");
    info.className = "gallery-info";
    const name = document.createElement("span");
    name.className = "gallery-name";
    name.textContent = gallery.name || "Gallery";
    const hint = document.createElement("span");
    hint.className = "gallery-meta";
    hint.textContent = s.remoteGalleryHint;
    info.appendChild(name);
    info.appendChild(hint);
    row.appendChild(info);
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

    const section = document.createElement("div");
    section.className = "remote-gallery-section recent-gallery-section";

    const head = document.createElement("div");
    head.className = "remote-gallery-head";
    const title = document.createElement("span");
    title.textContent = s.recentGalleries;
    head.appendChild(title);
    section.appendChild(head);

    for (const gallery of items) {
        section.appendChild(remoteGalleryRow(gallery, s));
    }

    if (recent.loading) {
        const status = document.createElement("div");
        status.className = "remote-gallery-status";
        status.textContent = s.remoteSearching;
        section.appendChild(status);
    } else if (recent.error) {
        const status = document.createElement("div");
        status.className = "remote-gallery-status error";
        status.textContent = `${s.remoteSearchError}: ${recent.error}`;
        section.appendChild(status);
    }

    els.galleryList.appendChild(section);
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

    const section = document.createElement("div");
    section.className = "remote-gallery-section";

    const head = document.createElement("div");
    head.className = "remote-gallery-head";
    const title = document.createElement("span");
    title.textContent = s.remoteResults;
    const count = document.createElement("span");
    count.textContent = remote.meta?.total != null ? String(remote.meta.total) : "";
    head.appendChild(title);
    head.appendChild(count);
    section.appendChild(head);

    for (const gallery of items) {
        section.appendChild(remoteGalleryRow(gallery, s));
    }

    if (remote.loading) {
        const status = document.createElement("div");
        status.className = "remote-gallery-status";
        status.textContent = s.remoteSearching;
        section.appendChild(status);
    } else if (remote.error) {
        const status = document.createElement("div");
        status.className = "remote-gallery-status error";
        status.textContent = `${s.remoteSearchError}: ${remote.error}`;
        section.appendChild(status);
    } else if (remote.meta && remote.meta.currentPage < remote.meta.lastPage) {
        const more = document.createElement("button");
        more.type = "button";
        more.className = "remote-more";
        more.textContent = s.remoteLoadMore;
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
        const stats = document.createElement("span");
        stats.className = "gallery-stats";
        stats.textContent = galleryStatsText(gallery);
        const meta = document.createElement("span");
        meta.className = "gallery-meta";
        if (gallery.exists) {
            meta.textContent = `${s.lastSync}: ${formatDateShort(gallery.lastSyncAt)}`;
        } else {
            meta.textContent = s.folderMissing;
            meta.classList.add("warn");
        }
        info.appendChild(name);
        info.appendChild(stats);
        info.appendChild(meta);
        row.appendChild(info);
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
        els.welcomeTitle.textContent =
            state.galleries.length === 0 ? s.noGalleryTitle : s.noGalleryTitle;
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
    els.btnSync.textContent = syncButtonLabel(s, state.syncBusy);
    els.btnFavoritesList.disabled = false;
    els.btnChangeFolder.disabled = false;
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
        const empty = document.createElement("div");
        empty.className = "log-empty";
        empty.textContent = s.noActivity;
        els.syncLogEntries.appendChild(empty);
        return;
    }

    for (const entry of state.syncLog) {
        const card = document.createElement("div");
        card.className = "log-entry";

        const head = document.createElement("div");
        head.className = "log-entry-head";
        const body = document.createElement("div");
        body.className = "log-entry-body";

        if (entry.kind === "sync_error") {
            head.textContent = `${formatDate(entry.at)} · ${s.syncFailed}`;
            card.appendChild(head);
            const line = document.createElement("div");
            line.className = "log-error";
            line.textContent = entry.message || "";
            body.appendChild(line);
            card.appendChild(body);
            els.syncLogEntries.appendChild(card);
            continue;
        }

        if (entry.kind === "deletion") {
            const labels = {
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
            head.textContent = `${formatDate(entry.at)} · ${
                labels[entry.action] || entry.action
            } · ${files.length}`;
            card.appendChild(head);
            for (const item of files) {
                const line = document.createElement("div");
                const status = statuses[item.status];
                line.textContent = `${item.filename || item.id}${
                    status ? ` — ${status}` : ""
                }`;
                body.appendChild(line);
            }
            for (const item of entry.trashed || []) {
                const line = document.createElement("div");
                const target =
                    item.trashPath ||
                    (item.location === "gallery"
                        ? s.deletionTrashGallery
                        : s.deletionTrashSystem);
                line.textContent = `${item.filename} — ${fmt(s.trashedTo, {
                    path: target,
                })}`;
                body.appendChild(line);
            }
            for (const error of entry.errors || []) {
                const line = document.createElement("div");
                line.className = "log-error";
                line.textContent = `${s.errors}: ${error}`;
                body.appendChild(line);
            }
            card.appendChild(body);
            els.syncLogEntries.appendChild(card);
            continue;
        }

        if (entry.kind === "rating") {
            const source =
                entry.source === "favorites"
                    ? s.ratingSourceFavorites
                    : s.ratingSourceLink;
            head.textContent = `${formatDate(entry.at)} · ${fmt(
                s.ratingLogLine,
                { stars: entry.rating, n: entry.rated },
            )} · ${source}`;
            card.appendChild(head);
            for (const item of entry.files || []) {
                if (item.status !== "ok") continue;
                const line = document.createElement("div");
                line.textContent = `${s.ratedFiles}: ${item.file || item.name}`;
                body.appendChild(line);
            }
            for (const item of entry.files || []) {
                if (item.status === "ok") continue;
                const line = document.createElement("div");
                line.className = "log-error";
                line.textContent = `${item.name} — ${item.message || item.status}`;
                body.appendChild(line);
            }
            card.appendChild(body);
            els.syncLogEntries.appendChild(card);
            continue;
        }

        const counts = entry.counts || {};
        const parts = [
            `${s.downloadedFiles}: ${counts.remoteNew + counts.remoteModified || 0}`,
            `${s.uploadedFiles}: ${counts.localNew + counts.localModified || 0}`,
            `${s.errors}: ${(entry.errors || []).length}`,
        ];
        head.textContent = `${formatDate(entry.at)} · ${parts.join(" · ")}`;
        card.appendChild(head);

        for (const item of entry.uploaded || []) {
            const line = document.createElement("div");
            const reason =
                item.reason === "modified"
                    ? s.localModifiedReason
                    : s.localNewReason;
            const version = item.versionNumber ? ` (v${item.versionNumber})` : "";
            line.textContent = `${s.uploadedFiles}: ${item.filename}${version} — ${reason}`;
            body.appendChild(line);
        }
        for (const filename of entry.downloaded || []) {
            const line = document.createElement("div");
            line.textContent = `${s.downloadedFiles}: ${filename}`;
            body.appendChild(line);
        }
        for (const item of entry.localDeletedProposed || []) {
            const line = document.createElement("div");
            line.textContent = `${s.localDeletionProposed}: ${item.filename}`;
            body.appendChild(line);
        }
        for (const item of entry.remoteDeleted || []) {
            const line = document.createElement("div");
            line.textContent = `${s.remoteDeletionDetected}: ${item.filename}`;
            body.appendChild(line);
        }
        for (const item of entry.conflicts || []) {
            const line = document.createElement("div");
            line.textContent = `${s.conflicts}: ${
                item.filename || item.remoteFilename || "—"
            }`;
            body.appendChild(line);
        }
        for (const warning of entry.warnings || []) {
            const line = document.createElement("div");
            line.className = "log-warning";
            line.textContent = warningText(warning);
            body.appendChild(line);
        }
        for (const error of entry.errors || []) {
            const line = document.createElement("div");
            line.className = "log-error";
            line.textContent = `${s.errors}: ${error}`;
            body.appendChild(line);
        }
        card.appendChild(body);
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
            const details = trashed.map((item) => {
                const target =
                    item.trashPath ||
                    (item.location === "gallery"
                        ? s.deletionTrashGallery
                        : s.deletionTrashSystem);
                return `${item.filename} — ${fmt(s.trashedTo, { path: target })}`;
            });
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
    renderReport();
    renderRating();
    renderFavorites();
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
    if (favorites.length === 0) return;
    state.rating = {
        open: true,
        files: favorites.map((photo) => stem(photo.filename)),
        value: "5",
        galleryId: report.galleryId,
        photosDir: null,
        source: "favorites",
        favorites: favorites,
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

async function syncCurrent() {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;
    state.syncBusy = true;
    state.progress = { phase: "manifest", done: 0, total: 1 };
    state.report = null;
    renderAll();
    let missing = false;
    try {
        const report = await invoke("sync_gallery", { galleryId: gallery.galleryId });
        state.report = report;
    } catch (err) {
        if (isUnknownGalleryError(err)) {
            missing = true;
            await handleUnknownGallery(gallery);
        } else {
            state.report = syncErrorReport(err);
        }
    } finally {
        state.syncBusy = false;
        state.progress = null;
        if (!missing) {
            await refreshState();
        }
        renderAll();
        if (!missing) handlePostSync(state.report);
    }
}

async function startSync(galleryId, name) {
    if (state.syncBusy) return;
    state.syncBusy = true;
    state.progress = { phase: "manifest", done: 0, total: 1 };
    state.report = null;
    renderAll();
    let missing = false;
    try {
        const result = await invoke("start_sync", { galleryId, name });
        if (result?.report) {
            state.currentGalleryId = result.report.galleryId;
            state.report = result.report;
        }
    } catch (err) {
        if (isUnknownGalleryError(err)) {
            missing = true;
            await handleUnknownGallery({
                galleryId,
                name: name || galleryId,
            });
        } else {
            state.report = syncErrorReport(err);
        }
    } finally {
        state.syncBusy = false;
        state.progress = null;
        if (!missing) {
            await refreshState();
        }
        renderAll();
        if (!missing) handlePostSync(state.report);
    }
}

async function applyRating() {
    if (state.rating.busy || state.rating.files.length === 0) return;
    const appliedState = state.rating;
    appliedState.busy = true;
    appliedState.result = null;
    renderRating();
    try {
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
els.btnFavoritesClose.addEventListener("click", closeFavorites);
els.favoritesModal.addEventListener("click", (event) => {
    if (event.target === els.favoritesModal) closeFavorites();
});
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

els.btnChangeFolder.addEventListener("click", async () => {
    if (!state.currentGalleryId || els.btnChangeFolder.disabled) return;
    const s = state.strings;
    const original = s.changeFolder;
    els.btnChangeFolder.disabled = true;
    els.btnChangeFolder.textContent = s.choosingFolder || original;
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
        els.btnChangeFolder.disabled = false;
        els.btnChangeFolder.textContent = state.strings?.changeFolder || original;
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

els.btnRemoteDeleteLocal.addEventListener("click", () =>
    resolveRemoteDeleted("delete_local"),
);
els.btnRemoteKeepLocal.addEventListener("click", () =>
    resolveRemoteDeleted("keep_local"),
);
els.btnRemoteUploadAgain.addEventListener("click", () =>
    resolveRemoteDeleted("upload"),
);
els.btnRemoteClose.addEventListener("click", () => {
    state.remoteDeleted.open = false;
    state.remoteDeleted.pendingFavorites = null;
    renderRemoteDeleted();
});
els.remoteDeletedModal.addEventListener("click", (event) => {
    if (event.target === els.remoteDeletedModal) {
        state.remoteDeleted.open = false;
        state.remoteDeleted.pendingFavorites = null;
        renderRemoteDeleted();
    }
});

els.btnConfirmOk.addEventListener("click", () => settleConfirm(true));
els.btnConfirmCancel.addEventListener("click", () => settleConfirm(false));
els.confirmModal.addEventListener("click", (event) => {
    if (event.target === els.confirmModal) settleConfirm(false);
});

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

document.addEventListener("keydown", (event) => {
    if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === "f") {
        event.preventDefault();
        els.gallerySearch.focus();
        els.gallerySearch.select();
        return;
    }
    if (event.key !== "Escape") return;
    hide(els.infoModal);
    state.rating.open = false;
    state.remoteDeleted.open = false;
    if (state.confirm.open) settleConfirm(false);
    renderRating();
    renderRemoteDeleted();
});

init();
