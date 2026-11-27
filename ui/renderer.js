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
    /** Cover thumbnails of linked galleries: galleryId -> data: URL. */
    covers: {},
    /** Linked gallery ids the covers were last loaded for. */
    coversKey: null,
    /** Show every linked gallery instead of the most recent ones. */
    showAllLinked: false,
    /** Photographer's profile photo (data: URL) or null when there is none. */
    avatar: null,
    /** Account the avatar was last loaded for. */
    avatarFor: null,
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
        busy: false,
        result: null,
    },
    syncLog: [],
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
        altLabel: "",
        danger: true,
        resolve: null,
    },
    /**
     * Pending review of the selected gallery: the plan from the backend and
     * the choice made for every row. `null` when there is none.
     */
    review: null,
};

const els = {
    envBadge: document.getElementById("envBadge"),
    btnAddGallery: document.getElementById("btnAddGallery"),
    galleryList: document.getElementById("galleryList"),
    btnHome: document.getElementById("btnHome"),
    mainActions: document.getElementById("mainActions"),
    home: document.getElementById("home"),
    homeSyncedSection: document.getElementById("homeSyncedSection"),
    homeSyncedTitle: document.getElementById("homeSyncedTitle"),
    homeSyncedHint: document.getElementById("homeSyncedHint"),
    homeSynced: document.getElementById("homeSynced"),
    homeRemoteSection: document.getElementById("homeRemoteSection"),
    homeRemoteTitle: document.getElementById("homeRemoteTitle"),
    homeRemoteHint: document.getElementById("homeRemoteHint"),
    homeRemote: document.getElementById("homeRemote"),
    homeRemoteStatus: document.getElementById("homeRemoteStatus"),
    homeEmpty: document.getElementById("homeEmpty"),
    homeEmptyTitle: document.getElementById("homeEmptyTitle"),
    homeEmptyHint: document.getElementById("homeEmptyHint"),
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
    accountMenu: document.getElementById("accountMenu"),
    accountCaret: document.getElementById("accountCaret"),
    btnOpenProfile: document.getElementById("btnOpenProfile"),
    btnCheckUpdates: document.getElementById("btnCheckUpdates"),
    updateStatus: document.getElementById("updateStatus"),
    btnConfirmAlt: document.getElementById("btnConfirmAlt"),
    detailRawMode: document.getElementById("detailRawMode"),
    reportRaw: document.getElementById("reportRaw"),
    btnLogout: document.getElementById("btnLogout"),
    btnInfo: document.getElementById("btnInfo"),

    detailName: document.getElementById("detailName"),
    titleLogo: document.getElementById("titleLogo"),
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

    review: document.getElementById("review"),
    reviewTitle: document.getElementById("reviewTitle"),
    reviewHint: document.getElementById("reviewHint"),
    reviewGroups: document.getElementById("reviewGroups"),
    reviewBar: document.getElementById("reviewBar"),
    reviewSummary: document.getElementById("reviewSummary"),
    btnReviewApply: document.getElementById("btnReviewApply"),
    btnReviewDiscard: document.getElementById("btnReviewDiscard"),
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

    infoModal: document.getElementById("infoModal"),
    infoTitle: document.getElementById("infoTitle"),
    infoVersionLabel: document.getElementById("infoVersionLabel"),
    infoAccountLabel: document.getElementById("infoAccountLabel"),
    infoVersion: document.getElementById("infoVersion"),
    infoAccount: document.getElementById("infoAccount"),
    btnInfoClose: document.getElementById("btnInfoClose"),
    btnTestMode: document.getElementById("btnTestMode"),
    testModeBox: document.getElementById("testModeBox"),
    menuLogoutSeparator: document.getElementById("menuLogoutSeparator"),
    testModeHint: document.getElementById("testModeHint"),

    ratingModal: document.getElementById("ratingModal"),
    ratingTitle: document.getElementById("ratingTitle"),
    ratingSubtitle: document.getElementById("ratingSubtitle"),
    ratingTarget: document.getElementById("ratingTarget"),
    btnRatingFolder: document.getElementById("btnRatingFolder"),
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

/** Tooltip and accessible name of an icon-only button. */
function setTooltip(button, text) {
    button.title = text;
    button.setAttribute("aria-label", text);
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
    if (warning.kind === "unsafe_filename" && s?.unsafeFilename) {
        return fmt(s.unsafeFilename, vars);
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
    setLabel(els.btnSync, state.syncBusy ? s.reviewing : s.review);
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

/** Camera RAW formats (DNG included), as in the sync engine. */
const RAW_EXTENSIONS = new Set([
    "dng", "cr3", "cr2", "arw", "nef", "raf", "orf", "rw2", "pef", "srw", "mos", "mrw", "3fr",
]);

function isRawFile(filename) {
    const ext = String(filename || "").split(".").pop().toLowerCase();
    return RAW_EXTENSIONS.has(ext);
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

    setTooltip(els.btnHome, s.home);
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
    setLabel(els.btnLogout, s.logout);
    setLabel(els.btnOpenProfile, s.myProfile);
    setLabel(els.btnCheckUpdates, s.checkUpdates);
    setLabel(els.btnInfo, s.info);
    renderSyncButton(s);
    setLabel(els.btnChangeFolder, s.changeFolder);
    els.btnChangeFolder.title = s.changeFolder;
    setLabel(els.btnUnlink, s.unlink);
    els.detailPath.title = s.openFolder;
    els.btnOpenWebGallery.title = s.openGalleryInPicstome;
    els.btnOpenWebGallery.setAttribute("aria-label", s.openGalleryInPicstome);

    els.infoTitle.textContent = s.infoTitle;
    els.infoVersionLabel.textContent = s.infoVersion;
    els.infoAccountLabel.textContent = s.infoAccount;
    els.btnInfoClose.textContent = s.infoClose;

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
    els.syncLogTitle.textContent = s.activityLog;
    els.btnClearLog.textContent = s.clearLog;
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
    // Only a real profile photo is shown: a letter tile added nothing next
    // to the name, so without a photo the avatar is left out entirely.
    els.accountInitial.replaceChildren();
    if (connected && state.avatar) {
        const img = el("img", "gallery-cover");
        img.src = state.avatar;
        img.alt = "";
        els.accountInitial.appendChild(img);
    }
    els.accountInitial.classList.toggle("hidden", !(connected && state.avatar));
    // The menu is always available (Info lives there); profile and log out
    // only make sense with a session.
    els.btnOpenProfile.classList.toggle("hidden", !connected);
    els.menuLogoutSeparator.classList.toggle("hidden", !connected);

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

    // The build profile only matters when it is not a normal release.
    const profile = config.buildProfile && config.buildProfile !== "release"
        ? ` (${config.buildProfile})`
        : "";
    els.infoVersion.textContent = `${config.appVersion || "?"}${profile}`;
    els.infoAccount.textContent = state.auth.connected
        ? [state.auth.name, state.auth.email].filter(Boolean).join(" · ")
        : "—";
    const testMode = kind === "development";
    // Test mode is hidden: it is entered with three quick clicks on the
    // version number and only shows up here while it is active.
    els.testModeBox.classList.toggle("hidden", !testMode);
    els.btnTestMode.textContent = s.testModeActive;
    els.testModeHint.textContent = s.testModeActiveHint;
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

/* Ordering
 *
 * Linked galleries: most recently synced first, never-synced ones last. Picstome galleries: newest created first.
 * Picstome sorts its list by *last modification*, which put old galleries
 * that were touched recently above new ones; gallery ids are ULIDs, whose
 * first ten characters encode the creation time, so the order is derived
 * from the id and needs no server change.
 */

const ULID_ALPHABET = "0123456789ABCDEFGHJKMNPQRSTVWXYZ";

/** Creation time (ms) encoded in a ULID, or null for other ids. */
function ulidTime(id) {
    const value = String(id || "").toUpperCase();
    if (!/^[0-9A-HJKMNP-TV-Z]{26}$/.test(value)) return null;
    let time = 0;
    for (const char of value.slice(0, 10)) {
        time = time * 32 + ULID_ALPHABET.indexOf(char);
    }
    return time;
}

/** Best available creation time of a Picstome gallery summary. */
function createdAt(gallery) {
    return ulidTime(gallery.id) ?? (Date.parse(gallery.updatedAt || gallery.updated_at) || 0);
}

function byNewestCreated(a, b) {
    return createdAt(b) - createdAt(a);
}

/** Most recently synced first; never-synced galleries last. */
function byLastSync(a, b) {
    return (b.lastSyncAt || 0) - (a.lastSyncAt || 0);
}

/** Linked galleries listed before the "Show all" toggle. */
const LINKED_PREVIEW = 12;

/** Linked galleries in sidebar/home order. */
function sortedGalleries(galleries = state.galleries) {
    return [...galleries].sort(byLastSync);
}

/** Unlinked Picstome galleries, newest first. */
function recentGalleryItems(limit = 5) {
    const linked = new Set(state.galleries.map((gallery) => gallery.galleryId));
    const seen = new Set();
    return state.recentGalleries.items
        .filter((gallery) => {
            if (linked.has(gallery.id) || seen.has(gallery.id)) return false;
            seen.add(gallery.id);
            return true;
        })
        .sort(byNewestCreated)
        .slice(0, limit);
}

/** Load the photographer's profile photo once per signed-in account. */
async function loadAvatar() {
    const key = state.auth.email || state.auth.name || "";
    if (state.avatarFor === key) return;
    state.avatarFor = key;
    try {
        state.avatar = (await invoke("get_account_avatar")) || null;
    } catch {
        state.avatar = null;
    }
    renderAccount();
}

/* Covers */

/** Thumbnail tile for a gallery: its cover when known, else its initial. */
function galleryAvatar(gallery, className = "gallery-avatar") {
    const avatar = el("span", className);
    const cover = state.covers[gallery.galleryId];
    if (cover) {
        const img = el("img", "gallery-cover");
        img.src = cover;
        img.alt = "";
        img.decoding = "async";
        avatar.classList.add("has-cover");
        avatar.appendChild(img);
    } else {
        avatar.textContent = initial(gallery.name);
    }
    return avatar;
}

/** Tile for a Picstome gallery that is not on this computer. */
function remoteAvatar(className = "gallery-avatar") {
    const avatar = el("span", `${className} remote-gallery-avatar`);
    avatar.appendChild(icon("cloud-off"));
    return avatar;
}

/**
 * Load cover thumbnails for the linked galleries. Runs when the set of
 * linked galleries changes or after a sync (`force`); covers are cached on
 * disk by the backend, so this is cheap after the first time.
 */
async function loadCovers(force = false) {
    const key = state.galleries
        .map((gallery) => gallery.galleryId)
        .sort()
        .join(",");
    if (!key || (!force && key === state.coversKey)) return;
    state.coversKey = key;
    try {
        state.covers = (await invoke("get_gallery_covers")) || {};
        renderGalleryList();
        renderHome();
    } catch {
        // Covers are decoration; initials remain when they cannot load.
    }
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
            // Picstome orders by last modification; fetch a wider window so
            // the newest *created* galleries are in it once sorted locally.
            perPage: 50,
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
        renderHome();
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

    const avatar = remoteAvatar();
    const info = el("span", "gallery-info");
    info.append(
        el("span", "gallery-name", gallery.name || "Gallery"),
        el("span", "gallery-meta", remoteGalleryMeta(gallery, s)),
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

/** "Not synced · Created 12 Nov" for a Picstome gallery row. */
function remoteGalleryMeta(gallery, s) {
    const created = createdAt(gallery);
    return created
        ? `${s.notSynced} · ${fmt(s.createdOn, { date: formatDateShort(created) })}`
        : s.notSynced;
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

    els.btnHome.classList.toggle("selected", !state.currentGalleryId);
    if (galleries.length > 0) {
        const head = el("div", "remote-gallery-head local-gallery-head");
        head.append(
            el("span", "", s.onThisComputer),
            el("span", "remote-gallery-count", String(galleries.length)),
        );
        els.galleryList.appendChild(head);
    }

    // With dozens or hundreds of linked galleries the list would push
    // "Recent in Picstome" out of sight: show the most recent ones and a
    // toggle. A search always lists every match.
    const ordered = sortedGalleries(galleries);
    const collapsible = !state.galleryQuery.trim() && ordered.length > LINKED_PREVIEW;
    const visible =
        collapsible && !state.showAllLinked ? ordered.slice(0, LINKED_PREVIEW) : ordered;
    for (const gallery of visible) {
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
        row.append(galleryAvatar(gallery), info);
        els.galleryList.appendChild(row);
    }
    if (collapsible) {
        const toggle = el(
            "button",
            "list-toggle",
            state.showAllLinked ? s.showFewer : fmt(s.showAll, { n: ordered.length }),
        );
        toggle.type = "button";
        toggle.dataset.toggle = "linked";
        els.galleryList.appendChild(toggle);
    }

    renderRecentGallerySection(s);
    renderRemoteGallerySection(s);
}

/** One card on the home page. */
function homeCard({ avatar, title, lines, dataset }) {
    const card = el("button", "home-card");
    card.type = "button";
    Object.assign(card.dataset, dataset);
    const info = el("span", "home-card-info");
    info.appendChild(el("span", "home-card-title", title));
    for (const [text, cls] of lines) info.appendChild(el("span", cls, text));
    card.append(avatar, info);
    return card;
}

/**
 * Home page: recently synced galleries with their covers, then the newest
 * Picstome galleries that are not on this computer yet.
 */
function renderHome() {
    const s = state.strings;
    if (!s || els.home.classList.contains("hidden")) return;

    els.homeSyncedTitle.textContent = s.homeSyncedTitle;
    els.homeSyncedHint.textContent = s.homeSyncedHint;
    els.homeRemoteTitle.textContent = s.homeRemoteTitle;
    els.homeRemoteHint.textContent = s.homeRemoteHint;
    els.homeEmptyTitle.textContent = s.homeEmptyTitle;
    els.homeEmptyHint.textContent = s.homeEmptyHint;

    const synced = sortedGalleries().slice(0, 6);
    els.homeSynced.replaceChildren(
        ...synced.map((gallery) =>
            homeCard({
                avatar: galleryAvatar(gallery, "home-card-cover"),
                title: gallery.name,
                lines: [
                    [galleryStatsText(gallery), "home-card-meta"],
                    gallery.exists
                        ? [`${s.lastSync}: ${formatDateShort(gallery.lastSyncAt)}`, "home-card-meta"]
                        : [s.folderMissing, "home-card-meta warn"],
                ],
                dataset: { galleryId: gallery.galleryId },
            }),
        ),
    );
    els.homeSyncedSection.classList.toggle("hidden", synced.length === 0);

    const recent = state.recentGalleries;
    const remote = state.auth.connected ? recentGalleryItems(6) : [];
    els.homeRemote.replaceChildren(
        ...remote.map((gallery) =>
            homeCard({
                avatar: remoteAvatar("home-card-cover"),
                title: gallery.name || "Gallery",
                lines: [
                    [remoteGalleryMeta(gallery, s), "home-card-meta"],
                    [s.syncGalleryAction, "home-card-action"],
                ],
                dataset: {
                    remote: "1",
                    galleryId: gallery.id,
                    galleryName: gallery.name || "Gallery",
                },
            }),
        ),
    );
    const status = remoteStatus(recent, s);
    els.homeRemoteStatus.textContent = status ? status.textContent : "";
    els.homeRemoteStatus.classList.toggle("hidden", !status);
    const remoteVisible = state.auth.connected && (remote.length > 0 || Boolean(status));
    els.homeRemoteSection.classList.toggle("hidden", !remoteVisible);
    els.homeEmpty.classList.toggle("hidden", synced.length > 0 || remoteVisible);
}

/** Leave the selected gallery and show the home page. */
function goHome() {
    if (state.review) discardReview();
    state.currentGalleryId = null;
    state.report = null;
    renderAll();
    refreshLog();
}

function renderMain() {
    const s = state.strings;
    if (!s) return;
    const gallery = currentGallery();

    els.mainActions.classList.toggle("hidden", !gallery);
    if (!gallery) {
        state.currentGalleryId = null;
        hide(els.progressWrap);
        hide(els.report);
        hide(els.idleState);
        const homeVisible = state.auth.connected || state.galleries.length > 0;
        els.welcome.classList.toggle("hidden", homeVisible);
        els.home.classList.toggle("hidden", !homeVisible);
        renderHome();
        // Home is the app's front page: it carries the app's own name and
        // logo (the sidebar no longer repeats them).
        els.detailName.textContent = s.appTitle;
        show(els.titleLogo);
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
    hide(els.home);
    hide(els.titleLogo);
    const reviewing = state.review?.galleryId === gallery.galleryId;
    const showIdle =
        !state.syncBusy &&
        !state.report &&
        !state.progress &&
        !reviewing;
    els.idleState.classList.toggle("hidden", !showIdle);
    // A missing folder cannot be synced: the idle card offers to locate it
    // instead (keeps the sync history, unlike "Change folder").
    els.idleTitle.textContent = gallery.exists ? s.readyToReview : s.folderMissing;
    els.idleHint.textContent = gallery.exists ? s.reviewIdleHint : s.folderMissingHint;
    els.btnIdleSync.textContent = gallery.exists ? s.review : s.locate;
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
    const rawLabels = { jpeg_only: s.rawModeJpegOnly, jpeg_and_raw: s.rawModeJpegAndRaw };
    els.detailRawMode.textContent = rawLabels[gallery.rawMode] || "";
    els.detailRawMode.title = s.rawChangeTitle;
    els.detailRawMode.classList.toggle("hidden", !gallery.rawMode);
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
    const applying = state.syncTarget?.mode === "apply";
    els.syncModalTitle.textContent = fmt(applying ? s.applyingGallery : s.reviewingGallery, {
        name: name || "",
    });
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
    const failed = (report.errors?.length || 0) > 0;
    const moved =
        (report.downloaded?.length || 0) +
        (report.uploaded?.length || 0) +
        (report.trashedLocal?.length || 0) +
        (report.deletedRemote?.length || 0) +
        (report.remoteDeletedResolved || 0) +
        (report.ratingsResult?.rated || 0);
    const idle = moved === 0;
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
    const skipped = report.skippedChanged || [];
    const warnings = [
        ...(report.warnings || []),
        ...(skipped.length
            ? [{ message: fmt(s.reportSkippedChanged, { n: skipped.length, files: skipped.slice(0, 5).join(", ") }) }]
            : []),
    ];
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
    for (const item of report.deletedRemote || []) {
        fileLines.push(`${s.reportDeletedRemote}: ${item}`);
    }
    for (const item of report.uploadedRaw || []) {
        fileLines.push(`${s.uploadedFiles}: ${item.filename} — ${s.rawEarlierVersion}`);
    }
    for (const item of report.uploaded || []) {
        const version = item.versionNumber ? ` (v${item.versionNumber})` : "";
        const reason =
            item.reason === "modified" ? s.localModifiedReason : s.localNewReason;
        fileLines.push(
            `${s.uploadedFiles}: ${item.filename}${version} — ${reason}`,
        );
    }
    for (const item of report.downloaded || []) {
        fileLines.push(`${s.downloadedFiles}: ${item.filename}`);
    }
    if (report.ratingsResult?.rated) {
        fileLines.push(fmt(s.ratingsResult, { n: report.ratingsResult.rated }));
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

    const rawKept = report.rawKeptLocal || [];
    els.reportRaw.textContent = rawKept.length ? fmt(s.rawKeptLocal, { n: rawKept.length }) : "";
    els.reportRaw.title = rawKept.join("\n");
    els.reportRaw.classList.toggle("hidden", rawKept.length === 0);

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
    els.btnRatingFolder.classList.toggle("hidden", Boolean(state.rating.galleryId || target));

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

/**
 * Ask what to upload for photos with a RAW file. Resolves `"jpeg_only"`,
 * `"jpeg_and_raw"` or `null` (cancelled: asked again next time).
 */
async function askRawMode({ title, message }) {
    const s = state.strings;
    const choice = await askChoice({
        title,
        message,
        confirmLabel: s.rawJpegOnly,
        altLabel: s.rawJpegAndRaw,
        danger: false,
    });
    if (choice === "confirm") return "jpeg_only";
    if (choice === "alt") return "jpeg_and_raw";
    return null;
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
    els.btnConfirmAlt.textContent = modal.altLabel;
    els.btnConfirmAlt.classList.toggle("hidden", !modal.altLabel);
}

/** Close the dialog and answer `"confirm"`, `"alt"` or `null` (cancelled). */
function settleConfirm(choice) {
    const resolve = state.confirm.resolve;
    state.confirm = {
        open: false,
        title: "",
        message: "",
        confirmLabel: "",
        altLabel: "",
        danger: true,
        resolve: null,
    };
    renderConfirm();
    if (resolve) resolve(choice || null);
}

/**
 * Ask the photographer to choose. Resolves `"confirm"` for the main button,
 * `"alt"` for the optional second action and `null` when cancelled.
 */
function askChoice({ title, message, confirmLabel, altLabel = "", danger = true }) {
    if (state.confirm.resolve) settleConfirm(null);
    return new Promise((resolve) => {
        state.confirm = { open: true, title, message, confirmLabel, altLabel, danger, resolve };
        renderConfirm();
    });
}

/** Yes/no confirmation. */
async function askConfirm(options) {
    return (await askChoice(options)) === "confirm";
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
    renderReview();
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
    // No gallery is auto-selected: the app opens on the home page.
    if (state.auth.connected) {
        loadCovers();
        loadAvatar();
    } else {
        state.avatar = null;
        state.avatarFor = null;
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

/**
 * Mark a backend operation on a gallery as running. `mode` is `"review"`
 * (working out the changes) or `"apply"` (moving files); it only changes
 * the wording of the progress dialog.
 */
function beginSync(galleryId, name, mode = "review") {
    state.syncTarget = { galleryId, name: name || "Gallery", mode };
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

/* Review → Apply
 *
 * "Review changes" asks the backend for a plan (nothing moves), shows every
 * change grouped by direction with a choice per row, and applies only what
 * the photographer chose. Rows left on "Later" are proposed again by the
 * next review.
 */

/** Row choices offered per group, the first being the default. */
const REVIEW_OPTIONS = {
    downloads: ["apply", "later", "ignore"],
    uploads: ["apply", "later", "ignore"],
    uploadsFirstSync: ["apply", "later", "ignore", "trash"],
    remoteDeleted: ["later", "keep", "trash", "upload"],
    remoteDeletedWithRaw: ["later", "keep", "trash", "trash_raw", "upload"],
    localDeleted: ["later", "delete", "restore"],
    ratings: ["apply", "later"],
};

/** "★★★★☆" for 4 stars. */
function starsText(stars) {
    const n = Math.max(0, Math.min(5, Number(stars) || 0));
    return "★".repeat(n) + "☆".repeat(5 - n);
}

/** Rows shown per group before "Show all". */
const REVIEW_PREVIEW_ROWS = 8;

/** Label of a row choice. */
function reviewOptionLabel(group, option, s) {
    const labels = {
        apply:
            group === "downloads"
                ? s.optDownload
                : group === "ratings"
                  ? s.optWriteStars
                  : s.optUpload,
        later: s.optLater,
        ignore: s.optIgnore,
        trash: s.optTrash,
        trash_raw: s.optTrashWithRaw,
        keep: s.optKeep,
        upload: s.optUploadAgain,
        delete: s.optDeleteInPicstome,
        restore: s.optDownloadAgain,
    };
    return labels[option] || option;
}

/** Build the review state for a plan, every row on its default. */
function newReview(galleryId, plan) {
    const choices = { downloads: {}, uploads: {}, remoteDeleted: {}, localDeleted: {}, ratings: {} };
    const ratings = plan.ratings || {};
    for (const item of ratings.items || []) choices.ratings[item.id] = "apply";
    for (const item of ratings.needsRating || []) choices.ratings[item.id] = "apply";
    for (const item of plan.downloads) choices.downloads[item.id] = "apply";
    for (const item of plan.uploads) choices.uploads[item.relativePath] = "apply";
    for (const item of plan.remoteDeleted) choices.remoteDeleted[item.id] = "later";
    for (const item of plan.localDeleted) choices.localDeleted[item.id] = "later";
    return {
        galleryId,
        plan,
        choices,
        /** Favourites with a heart and no stars: they need a star value. */
        favouriteIds: new Set((ratings.needsRating || []).map((item) => item.id)),
        favouritesStars: null,
        massConfirm: "",
        expanded: {},
    };
}

/** The groups of a plan, in the order they are shown. */
function reviewGroups(review, s) {
    const plan = review.plan;
    const size = (bytes) => (bytes ? formatBytes(bytes) : "");
    const meta = (...parts) => parts.filter(Boolean).join(" · ");
    return [
        {
            key: "downloads",
            title: s.groupDownloads,
            items: plan.downloads,
            id: (item) => item.id,
            name: (item) => item.filename,
            meta: (item) =>
                meta(item.reason === "modified" ? s.reasonModified : s.reasonNew, size(item.size)),
            options: REVIEW_OPTIONS.downloads,
        },
        {
            key: "uploads",
            title: s.groupUploads,
            items: plan.uploads,
            id: (item) => item.relativePath,
            name: (item) => item.relativePath,
            meta: (item) =>
                meta(
                    item.reason === "modified" ? s.reasonModifiedLocal : s.reasonNew,
                    size(item.size),
                    item.rawFirst?.length ? fmt(s.rawFirstTag, { raw: item.rawFirst.join(", ") }) : "",
                ),
            options: plan.firstSync ? REVIEW_OPTIONS.uploadsFirstSync : REVIEW_OPTIONS.uploads,
        },
        {
            key: "remoteDeleted",
            title: s.groupRemoteDeleted,
            items: plan.remoteDeleted,
            id: (item) => item.id,
            name: (item) => item.relativePath || item.filename,
            meta: (item) => (item.companions?.length ? `+ ${item.companions.join(", ")}` : ""),
            options: plan.remoteDeleted.some((item) => item.companions?.length)
                ? REVIEW_OPTIONS.remoteDeletedWithRaw
                : REVIEW_OPTIONS.remoteDeleted,
            rowOptions: (item) =>
                item.companions?.length
                    ? REVIEW_OPTIONS.remoteDeletedWithRaw
                    : REVIEW_OPTIONS.remoteDeleted,
            tone: "warn",
        },
        {
            key: "localDeleted",
            title: s.groupLocalDeleted,
            items: plan.localDeleted,
            id: (item) => item.id,
            name: (item) => item.filename,
            meta: () => "",
            options: REVIEW_OPTIONS.localDeleted,
            tone: "warn",
        },
        {
            key: "ratings",
            title: s.groupRatings,
            items: [...(plan.ratings?.items || []), ...(plan.ratings?.needsRating || [])],
            id: (item) => item.id,
            name: (item) => item.filename,
            meta: (item) => ratingRowMeta(review, item, s),
            options: REVIEW_OPTIONS.ratings,
        },
    ];
}

/**
 * What a ratings row will write: "★★★★☆ · before ★★☆☆☆", "Remove stars",
 * or, for a favourite without stars, the value chosen above the list.
 */
function ratingRowMeta(review, item, s) {
    if (review.favouriteIds.has(item.id)) {
        return review.favouritesStars
            ? fmt(s.favouriteStarsChosen, { stars: starsText(review.favouritesStars) })
            : s.favouriteStarsMissing;
    }
    const next = item.rating == null ? s.starsClear : starsText(item.rating);
    const before = item.appliedRating == null ? "" : fmt(s.starsBefore, { stars: starsText(item.appliedRating) });
    return [next, before].filter(Boolean).join(" · ");
}

/** Number of actions the photographer chose (everything not "Later"). */
function reviewActionCount(review) {
    let count = 0;
    for (const [group, choices] of Object.entries(review.choices)) {
        if (group === "ratings") continue;
        for (const choice of Object.values(choices)) if (choice !== "later") count += 1;
    }
    return count + reviewRatingIds(review).length;
}

/**
 * Photo ids whose stars will be written: ticked rows, except favourites
 * while no star value has been chosen for them.
 */
function reviewRatingIds(review) {
    return Object.entries(review.choices.ratings)
        .filter(([, choice]) => choice === "apply")
        .map(([id]) => id)
        .filter((id) => !review.favouriteIds.has(id) || review.favouritesStars);
}

/** How many photos the review would delete in Picstome. */
function reviewDeletes(review) {
    return Object.values(review.choices.localDeleted).filter((choice) => choice === "delete").length;
}

/** A `<select>` with the choices of a row (or of a whole group). */
function reviewSelect(group, options, value, s, extraClass = "") {
    const select = el("select", `review-select ${extraClass}`.trim());
    for (const option of options) {
        const node = el("option", "", reviewOptionLabel(group, option, s));
        node.value = option;
        node.selected = option === value;
        select.appendChild(node);
    }
    return select;
}

function renderReviewGroup(review, group, s) {
    const section = el("section", `review-group ${group.tone || ""}`.trim());
    section.dataset.group = group.key;
    const head = el("div", "review-group-head");
    const title = el("h3", "", group.title);
    title.appendChild(el("span", "review-count", String(group.items.length)));
    const all = reviewSelect(group.key, group.options, "", s, "review-all");
    all.insertBefore(el("option", "", s.reviewSetAll), all.firstChild);
    all.firstChild.value = "";
    all.firstChild.selected = true;
    all.dataset.group = group.key;
    head.append(title, all);
    section.appendChild(head);

    if (group.key === "uploads") renderRawChoice(review, section, s);
    if (group.key === "ratings") renderFavouriteStars(review, section, s);

    const expanded = review.expanded[group.key];
    const visible = expanded ? group.items : group.items.slice(0, REVIEW_PREVIEW_ROWS);
    const list = el("div", "review-rows");
    for (const item of visible) {
        const id = group.id(item);
        const choice = review.choices[group.key][id];
        const row = el("div", `review-row choice-${choice}`);
        const text = el("div", "review-row-text");
        text.append(el("span", "review-row-name", group.name(item)));
        const meta = group.meta(item);
        if (meta) text.append(el("span", "review-row-meta", meta));
        const select = reviewSelect(group.key, (group.rowOptions || (() => group.options))(item), choice, s);
        select.dataset.group = group.key;
        select.dataset.id = id;
        row.append(text, select);
        list.appendChild(row);
    }
    section.appendChild(list);
    if (!expanded && group.items.length > visible.length) {
        const more = el("button", "list-toggle", fmt(s.showAll, { n: group.items.length }));
        more.type = "button";
        more.dataset.expand = group.key;
        section.appendChild(more);
    }
    if (group.key === "localDeleted") renderMassDeleteGuard(review, section, s);
    return section;
}

/** "N photos have a RAW file. Upload: JPEG only / JPEG + RAW". */
function renderRawChoice(review, section, s) {
    const plan = review.plan;
    if (plan.rawDecisionNeeded > 0) {
        const box = el("div", "review-note raw-choice");
        box.append(el("span", "", fmt(s.firstSyncRawTitle, { n: plan.rawDecisionNeeded })));
        for (const [mode, label] of [["jpeg_only", s.rawJpegOnly], ["jpeg_and_raw", s.rawJpegAndRaw]]) {
            const button = el("button", "secondary small", label);
            button.type = "button";
            button.dataset.rawMode = mode;
            box.appendChild(button);
        }
        box.appendChild(el("p", "hint", s.firstSyncRawHint));
        section.appendChild(box);
    }
    if (plan.rawKeptLocal?.length) {
        section.appendChild(el("p", "hint review-note", fmt(s.rawKeptLocal, { n: plan.rawKeptLocal.length })));
    }
}

/** Typed confirmation when the review deletes a large share in Picstome. */
function renderMassDeleteGuard(review, section, s) {
    const deletes = reviewDeletes(review);
    if (!review.plan.massDelete || deletes === 0) return;
    const total = review.plan.remoteTotal || 1;
    const box = el("div", "review-note mass-delete");
    box.appendChild(
        el("p", "", fmt(s.massDeleteWarning, { n: deletes, percent: Math.round((deletes / total) * 100) })),
    );
    const input = el("input", "mass-delete-input");
    input.id = "massDeleteConfirm";
    input.inputMode = "numeric";
    input.placeholder = fmt(s.massDeletePlaceholder, { n: deletes });
    input.value = review.massConfirm;
    box.appendChild(input);
    section.appendChild(box);
}

/** Star value for every favourite that has a heart but no stars. */
function renderFavouriteStars(review, section, s) {
    const favourites = review.favouriteIds.size;
    if (favourites === 0) return;
    const box = el("div", "review-note");
    box.appendChild(el("span", "", fmt(s.ratingsFavourites, { n: favourites })));
    const select = el("select", "review-select");
    select.id = "reviewFavouriteStars";
    const skip = el("option", "", s.ratingsFavouritesSkip);
    skip.value = "";
    select.appendChild(skip);
    for (let stars = 5; stars >= 1; stars -= 1) {
        const option = el("option", "", starsText(stars));
        option.value = String(stars);
        option.selected = review.favouritesStars === stars;
        select.appendChild(option);
    }
    box.appendChild(select);
    section.appendChild(box);
}

function renderReviewNotes(plan, s) {
    const lines = [
        ...plan.conflicts.map((item) =>
            fmt(s.conflictDetail, {
                filename: item.local?.filename || item.remote?.filename || "—",
                remote: item.remote?.versionCount ?? 1,
                remoteSize: formatBytes(item.remote?.size || 0),
                localSize: formatBytes(item.local?.size || 0),
            }),
        ),
        ...plan.warnings.map(warningText),
        ...plan.errors.map((error) => `${error.path || error.filename || "—"} — ${error.message}`),
    ];
    if (lines.length === 0) return null;
    const section = el("section", "review-group notes");
    const head = el("div", "review-group-head");
    head.appendChild(el("h3", "", plan.conflicts.length ? s.groupConflicts : s.groupNotes));
    section.appendChild(head);
    if (plan.conflicts.length) section.appendChild(el("p", "hint", s.groupConflictsHint));
    for (const line of lines) section.appendChild(el("div", "review-note-line", line));
    return section;
}

function renderReview() {
    const s = state.strings;
    const review = state.review;
    const visible = Boolean(s && review && review.galleryId === state.currentGalleryId && !state.syncBusy);
    els.review.classList.toggle("hidden", !visible);
    els.reviewBar.classList.toggle("hidden", !visible);
    if (!visible) return;

    const plan = review.plan;
    els.reviewTitle.textContent = s.reviewTitle;
    els.reviewHint.textContent = plan.firstSync ? s.reviewFirstSyncHint : s.reviewHint;
    const nodes = reviewGroups(review, s)
        .filter((group) => group.items.length > 0 || (group.key === "uploads" && plan.rawDecisionNeeded > 0))
        .map((group) => renderReviewGroup(review, group, s));
    const notes = renderReviewNotes(plan, s);
    if (notes) nodes.push(notes);
    els.reviewGroups.replaceChildren(...nodes);

    els.btnReviewDiscard.textContent = s.reviewDiscard;
    updateReviewBar();
}

/**
 * Summary and Apply button. Kept separate from `renderReview` so typing the
 * mass-delete confirmation does not redraw (and unfocus) the input.
 */
function updateReviewBar() {
    const s = state.strings;
    const review = state.review;
    if (!s || !review) return;
    const count = reviewActionCount(review);
    const deletes = reviewDeletes(review);
    const confirmed = !review.plan.massDelete || deletes === 0 || Number(review.massConfirm) === deletes;
    els.reviewSummary.textContent = count ? fmt(s.reviewSelected, { n: count }) : s.reviewNothingSelected;
    els.btnReviewApply.textContent = fmt(s.reviewApply, { n: count });
    els.btnReviewApply.disabled = count === 0 || !confirmed;
}

/** An empty report, shown as "Everything is in sync". */
function inSyncReport(galleryId) {
    return { galleryId, counts: {}, downloaded: [], uploaded: [], conflicts: [], errors: [], warnings: [] };
}

/** Show a fresh plan: the review, or "in sync" when there is nothing to do. */
function showPlan(galleryId, plan) {
    const hasWork =
        plan.downloads.length ||
        plan.uploads.length ||
        plan.rawDecisionNeeded ||
        plan.remoteDeleted.length ||
        plan.localDeleted.length ||
        plan.conflicts.length ||
        plan.errors.length ||
        (plan.ratings?.items || []).length ||
        (plan.ratings?.needsRating || []).length;
    state.review = hasWork ? newReview(galleryId, plan) : null;
    state.report = hasWork ? null : inSyncReport(galleryId);
}

/** Review the selected gallery. */
async function reviewCurrent() {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;
    beginSync(gallery.galleryId, gallery.name);
    try {
        const plan = await invoke("review_gallery", { galleryId: gallery.galleryId });
        endSync();
        showPlan(gallery.galleryId, plan);
    } catch (err) {
        endSync();
        if (await handleSyncError(err, gallery)) return;
    }
    renderAll();
}

/**
 * Link a Picstome gallery (asks for a folder when needed) and review it.
 */
async function linkAndReview(galleryId, name) {
    if (state.syncBusy) return;
    const label = name || "Gallery";
    beginSync(galleryId, label);
    let result = null;
    try {
        result = await invoke("start_sync", { galleryId, name });
    } catch (err) {
        endSync();
        if (await handleSyncError(err, { galleryId, name: label })) return;
        renderAll();
        return;
    }
    endSync();
    if (result?.canceled) {
        renderAll();
        return;
    }
    state.currentGalleryId = galleryId;
    await refreshState();
    if (result?.plan) showPlan(galleryId, result.plan);
    renderAll();
    refreshLog();
}

/** The backend selection for the choices of a review. */
function reviewSelection(review) {
    const pick = (group, value) =>
        Object.entries(review.choices[group])
            .filter(([, choice]) => choice === value)
            .map(([id]) => id);
    const actions = {
        keep: ["keep_local", false],
        trash: ["delete_local", false],
        trash_raw: ["delete_local", true],
        upload: ["upload", false],
    };
    return {
        downloads: pick("downloads", "apply"),
        ignoreRemote: pick("downloads", "ignore"),
        uploads: pick("uploads", "apply"),
        ignoreLocal: pick("uploads", "ignore"),
        trashLocal: pick("uploads", "trash"),
        deleteRemote: pick("localDeleted", "delete"),
        restore: pick("localDeleted", "restore"),
        confirmMassDelete: Number(review.massConfirm) || null,
        remoteDeleted: Object.entries(review.choices.remoteDeleted)
            .filter(([, choice]) => actions[choice])
            .map(([id, choice]) => ({ id, action: actions[choice][0], includeRaw: actions[choice][1] })),
    };
}

/** Apply the review, after a confirmation for anything destructive. */
async function applyReview() {
    const review = state.review;
    if (!review || state.syncBusy) return;
    const s = state.strings;
    const deletes = reviewDeletes(review);
    if (review.plan.massDelete && deletes > 0 && Number(review.massConfirm) !== deletes) {
        document.getElementById("massDeleteConfirm")?.focus();
        return;
    }
    if (deletes > 0) {
        const confirmed = await askConfirm({
            title: s.confirmRemoteDeleteTitle,
            message: fmt(s.confirmRemoteDeleteMessage, { n: deletes }),
            confirmLabel: s.confirmRemoteDeleteAction,
        });
        if (!confirmed) return;
    }
    const gallery = currentGallery();
    beginSync(review.galleryId, gallery?.name || review.plan.name, "apply");
    let applied = null;
    try {
        applied = await invoke("apply_review", {
            galleryId: review.galleryId,
            planId: review.plan.id,
            selection: reviewSelection(review),
            ratings: {
                apply: reviewRatingIds(review).length > 0,
                favouritesStars: review.favouritesStars,
                ids: reviewRatingIds(review),
            },
        });
        state.review = null;
        state.report = { ...applied.report, ratingsResult: applied.ratings || null };
    } catch (err) {
        state.review = null;
        state.report = syncErrorReport(err);
    } finally {
        endSync();
        await refreshState();
        renderAll();
    }
    if (applied) refreshCoverAfterSync(review.galleryId);
}

/** Forget the pending review. */
async function discardReview() {
    const review = state.review;
    state.review = null;
    renderAll();
    if (review) {
        try {
            await invoke("discard_review", { galleryId: review.galleryId });
        } catch {
            // Nothing to undo: the review only lived in memory.
        }
    }
}

/** Answer the RAW question inside a review, then review again. */
async function chooseRawInReview(mode) {
    const review = state.review;
    if (!review) return;
    try {
        await invoke("set_gallery_raw_mode", { galleryId: review.galleryId, mode });
        await refreshState();
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
        return;
    }
    await reviewCurrent();
}

/** A first sync is when a gallery gets its photos, and so its cover. */
function refreshCoverAfterSync(galleryId) {
    if (!state.covers[galleryId]) loadCovers(true);
}

async function applyRating() {
    if (state.rating.busy) return;
    const appliedState = state.rating;
    if (appliedState.files.length === 0) return;
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
        await refreshLog();
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

/**
 * Look for a newer release on GitHub.
 *
 * At startup it runs quietly and at most every 15 days (the backend keeps
 * the throttle). From the Info dialog (`manual`) it always asks GitHub and
 * reports the outcome there, including "you are up to date" and errors.
 */
async function checkForUpdates({ manual = false } = {}) {
    const s = state.strings;
    const status = (text) => {
        if (manual) els.updateStatus.textContent = text;
    };
    if (manual) {
        els.btnCheckUpdates.disabled = true;
        els.btnCheckUpdates.classList.add("busy");
        setLabel(els.btnCheckUpdates, s.checkingUpdates);
        status("");
    }
    try {
        const update = await invoke("check_updates", { force: manual });
        const current = update?.currentVersion || state.config.appVersion || "?";
        if (update?.error) {
            status(fmt(s.updateCheckFailed, { error: update.error }));
            return;
        }
        if (!update?.available || !update.url) {
            status(fmt(s.upToDate, { current }));
            return;
        }
        status("");
        hide(els.infoModal);
        const download = await askConfirm({
            title: s.updateAvailableTitle,
            message: fmt(s.updateAvailableMessage, {
                current,
                latest: update.latestVersion || "?",
            }),
            confirmLabel: s.updateDownload,
            danger: false,
        });
        if (download) await invoke("open_update_page", { url: update.url });
    } catch (err) {
        // The automatic check is best-effort and must never block startup.
        status(fmt(s.updateCheckFailed, { error: err?.message || String(err) }));
    } finally {
        if (manual) {
            els.btnCheckUpdates.disabled = false;
            els.btnCheckUpdates.classList.remove("busy");
            setLabel(els.btnCheckUpdates, s.checkUpdates);
        }
    }
}

/* Gallery selection */

/**
 * Show a linked gallery and check, with a single `stat`, that its folder is
 * still there. The folder may have been moved, renamed, deleted or be on an
 * unplugged drive since the app started; the photographer then chooses to
 * locate it or unlink the gallery.
 */
async function openGallery(galleryId) {
    if (state.review && state.review.galleryId !== galleryId) discardReview();
    state.currentGalleryId = galleryId;
    state.report = null;
    renderAll();
    refreshLog();

    let exists;
    try {
        exists = await invoke("check_gallery_folder", { galleryId });
    } catch {
        return;
    }
    const gallery = state.galleries.find((item) => item.galleryId === galleryId);
    if (!gallery || state.currentGalleryId !== galleryId) return;
    if (gallery.exists !== exists) {
        gallery.exists = exists;
        renderAll();
    }
    if (exists) return;

    const s = state.strings;
    const choice = await askChoice({
        title: s.folderGoneTitle,
        message: fmt(s.folderGoneMessage, { name: gallery.name, path: gallery.localPath }),
        confirmLabel: s.folderGoneLocate,
        altLabel: s.folderGoneUnlink,
        danger: false,
    });
    if (choice === "confirm") await locateCurrent();
    if (choice === "alt") await unlinkGallery(galleryId);
}

/** Remove the link to a gallery (its local files stay where they are). */
async function unlinkGallery(galleryId) {
    await invoke("unlink_gallery", { galleryId });
    if (state.currentGalleryId === galleryId) state.currentGalleryId = null;
    state.report = null;
    state.coversKey = null;
    await refreshState();
}

/* Account menu */

function closeAccountMenu() {
    hide(els.accountMenu);
    els.accountChip.setAttribute("aria-expanded", "false");
}

function toggleAccountMenu() {
    const open = els.accountMenu.classList.contains("hidden");
    els.accountMenu.classList.toggle("hidden", !open);
    els.accountChip.setAttribute("aria-expanded", String(open));
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
            // A link from Picstome opens the review; it never applies.
            openGallery(galleryId).then(reviewCurrent);
        } else {
            linkAndReview(galleryId, name);
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
    if (event.target.closest(".list-toggle")) {
        state.showAllLinked = !state.showAllLinked;
        renderGalleryList();
        return;
    }
    const row = event.target.closest(".gallery-row");
    if (!row) return;
    if (row.dataset.remote === "1") {
        linkAndReview(row.dataset.galleryId, row.dataset.galleryName);
        return;
    }
    openGallery(row.dataset.galleryId);
});

els.btnHome.addEventListener("click", goHome);
els.home.addEventListener("click", (event) => {
    const card = event.target.closest(".home-card");
    if (!card) return;
    if (card.dataset.remote === "1") {
        linkAndReview(card.dataset.galleryId, card.dataset.galleryName);
        return;
    }
    openGallery(card.dataset.galleryId);
});

els.galleryList.addEventListener("dblclick", (event) => {
    const row = event.target.closest(".gallery-row");
    if (!row || row.dataset.remote === "1") return;
    state.currentGalleryId = row.dataset.galleryId;
    renderAll();
    reviewCurrent();
});

els.btnSync.addEventListener("click", reviewCurrent);
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

els.btnReviewApply.addEventListener("click", applyReview);
els.btnReviewDiscard.addEventListener("click", discardReview);
els.reviewGroups.addEventListener("change", (event) => {
    const review = state.review;
    const target = event.target;
    if (!review) return;
    if (target.id === "reviewFavouriteStars") {
        review.favouritesStars = Number(target.value) || null;
    } else if (target.classList.contains("review-all") && target.value) {
        for (const id of Object.keys(review.choices[target.dataset.group])) {
            const row = els.reviewGroups.querySelector(
                `select[data-group="${target.dataset.group}"][data-id="${CSS.escape(id)}"]`,
            );
            // "Trash with its RAW" only applies to rows that have a RAW.
            const allowed = !row || Array.from(row.options).some((option) => option.value === target.value);
            if (allowed) review.choices[target.dataset.group][id] = target.value;
        }
    } else if (target.dataset.group && target.dataset.id) {
        review.choices[target.dataset.group][target.dataset.id] = target.value;
    } else {
        return;
    }
    renderReview();
});
els.reviewGroups.addEventListener("input", (event) => {
    if (event.target.id === "massDeleteConfirm" && state.review) {
        state.review.massConfirm = event.target.value.trim();
        updateReviewBar();
    }
});
els.reviewGroups.addEventListener("click", (event) => {
    const expand = event.target.closest("[data-expand]");
    if (expand && state.review) {
        state.review.expanded[expand.dataset.expand] = true;
        renderReview();
        return;
    }
    const raw = event.target.closest("[data-raw-mode]");
    if (raw) chooseRawInReview(raw.dataset.rawMode);
});
els.btnIdleSync.addEventListener("click", () => {
    const gallery = currentGallery();
    if (gallery && !gallery.exists) {
        locateCurrent();
    } else {
        reviewCurrent();
    }
});
els.btnAddGallery.addEventListener("click", openPicstome);
els.btnConnect.addEventListener("click", openPicstome);
els.btnConnectWelcome.addEventListener("click", openPicstome);
els.btnDemo.addEventListener("click", () => linkAndReview("G123", "MiBoda"));
els.btnDemoWelcome.addEventListener("click", () => linkAndReview("G123", "MiBoda"));

els.accountChip.addEventListener("click", (event) => {
    event.stopPropagation();
    toggleAccountMenu();
});

els.btnOpenProfile.addEventListener("click", async () => {
    closeAccountMenu();
    await invoke("open_account_profile");
});

els.btnLogout.addEventListener("click", async () => {
    closeAccountMenu();
    const s = state.strings;
    const confirmed = await askConfirm({
        title: s.confirmLogoutTitle,
        message: s.confirmLogoutMessage,
        confirmLabel: s.logout,
    });
    if (!confirmed) return;
    await invoke("logout");
    await refreshState();
});

// Any click outside the account menu closes it.
document.addEventListener("click", (event) => {
    if (!event.target.closest(".account-wrap")) closeAccountMenu();
});

els.detailRawMode.addEventListener("click", async () => {
    const gallery = currentGallery();
    if (!gallery || state.syncBusy) return;
    const s = state.strings;
    const mode = await askRawMode({ title: s.rawChangeTitle, message: s.rawChangeMessage });
    if (!mode || mode === gallery.rawMode) return;
    try {
        await invoke("set_gallery_raw_mode", { galleryId: gallery.galleryId, mode });
        await refreshState();
    } catch (err) {
        state.report = syncErrorReport(err);
        renderAll();
    }
});

els.btnCheckUpdates.addEventListener("click", () => checkForUpdates({ manual: true }));

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

els.btnInfo.addEventListener("click", () => {
    closeAccountMenu();
    els.updateStatus.textContent = "";
    show(els.infoModal);
});

/* Hidden test mode: three clicks on the version within a second. */
const TEST_MODE_CLICKS = 3;
const TEST_MODE_WINDOW_MS = 1000;
let versionClicks = [];
els.infoVersion.addEventListener("click", () => {
    const now = Date.now();
    versionClicks = [...versionClicks, now].filter((at) => now - at < TEST_MODE_WINDOW_MS);
    if (versionClicks.length < TEST_MODE_CLICKS) return;
    versionClicks = [];
    toggleTestMode();
});
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
    await unlinkGallery(state.currentGalleryId);
});

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
    await reviewCurrent();
});


els.btnConfirmOk.addEventListener("click", () => settleConfirm("confirm"));
els.btnConfirmAlt.addEventListener("click", () => settleConfirm("alt"));

els.gallerySearch.addEventListener("input", () => {
    state.galleryQuery = els.gallerySearch.value;
    // New query, new list: start reading it from the top.
    els.galleryList.scrollTop = 0;
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
            if (remote) linkAndReview(remote.id, remote.name);
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
        close: () => settleConfirm(null),
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
    if (!els.accountMenu.classList.contains("hidden")) {
        closeAccountMenu();
        return;
    }
    // Close only the topmost dialog, like every desktop app does.
    const top = MODALS.find((modal) => modal.isOpen());
    if (top) {
        event.preventDefault();
        dismiss(top);
    }
});

init();
