use serde_json::{json, Value};

pub fn language() -> String {
    if let Ok(value) = std::env::var("PICSTOME_LANG") {
        return if value.to_ascii_lowercase().starts_with("es") {
            "es".to_string()
        } else {
            "en".to_string()
        };
    }

    if keyboard_is_spanish() {
        "es".to_string()
    } else {
        "en".to_string()
    }
}

fn keyboard_is_spanish() -> bool {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("defaults")
            .args([
                "read",
                "com.apple.HIToolbox",
                "AppleCurrentKeyboardLayoutInputSourceID",
            ])
            .output();
        if let Ok(output) = output {
            return is_spanish_keyboard_id(&String::from_utf8_lossy(&output.stdout));
        }
    }

    #[cfg(target_os = "linux")]
    {
        if let Ok(layout) = std::env::var("XKB_DEFAULT_LAYOUT") {
            if is_spanish_xkb_layout(&layout) {
                return true;
            }
        }
        let output = std::process::Command::new("setxkbmap")
            .arg("-query")
            .output();
        if let Ok(output) = output {
            let text = String::from_utf8_lossy(&output.stdout);
            if let Some(layout) = text.lines().find_map(|line| line.strip_prefix("layout:")) {
                return is_spanish_xkb_layout(layout);
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        let output = std::process::Command::new("reg")
            .args(["query", r"HKCU\Keyboard Layout\Preload"])
            .output();
        if let Ok(output) = output {
            return is_spanish_windows_layout(&String::from_utf8_lossy(&output.stdout));
        }
    }

    false
}

pub fn is_spanish_keyboard_id(value: &str) -> bool {
    let normalized = value.to_ascii_lowercase();
    normalized.contains("spanish")
        || normalized.contains("español")
        || normalized.contains("es-es")
        || normalized.contains(".es")
}

pub fn is_spanish_xkb_layout(value: &str) -> bool {
    value
        .split([',', ' ', ';'])
        .any(|token| token.trim().eq_ignore_ascii_case("es"))
}

pub fn is_spanish_windows_layout(value: &str) -> bool {
    value
        .lines()
        .filter_map(|line| line.split_whitespace().last())
        .any(|value| {
            let lower = value.to_ascii_lowercase();
            lower.ends_with("0a") && lower.len() >= 4
        })
}

pub fn strings(lang: &str) -> Value {
    if lang == "es" {
        spanish()
    } else {
        english()
    }
}

fn english() -> Value {
    json!({
        "appTitle": "Picstome Sync",
        "syncTab": "Sync",
        "rateTab": "Rate",
        "connect": "Open Picstome",
        "connectedAs": "Connected as",
        "logout": "Log out",
        "mockMode": "Demo mode (fake API)",
        "noGalleries": "No synced galleries yet",
        "noGalleriesHint": "Open a gallery in Picstome and press “Sync / Open in app”, or try the demo gallery.",
        "syncDemo": "Sync demo gallery",
        "update": "Update",
        "openFolder": "Open folder",
        "openGalleryInPicstome": "Open gallery in Picstome",
        "unlink": "Unlink",
        "confirmUnlinkTitle": "Unlink this gallery?",
        "confirmUnlinkMessage": "«{name}» will be removed from the app. Your local folder and photos will not be touched.",
        "confirmUnlinkAction": "Unlink",
        "changeFolder": "Change folder…",
        "choosingFolder": "Choosing folder…",
        "locate": "Locate…",
        "folderMissing": "Folder not found",
        "lastSync": "Last sync",
        "never": "never",
        "photos": "photos",
        "remoteGalleries": "Galleries in Picstome",
        "back": "Back",
        "updateNow": "Update now",
        "updateAvailableTitle": "New version available",
        "updateAvailableMessage": "Picstome Sync {latest} is available. You are using {current}. Download the new version from GitHub?",
        "updateDownload": "Download update",
        "inSync": "Everything is in sync",
        "backToSync": "Back to sync list",
        "downloaded": "Downloaded",
        "uploaded": "Uploaded",
        "conflicts": "Conflicts",
        "warnings": "Warnings",
        "duplicateLocalIgnored": "{filename} won't be uploaded; {preferred} is used for this photo.",
        "conflictDetail": "{filename} — remote v{remote} ({remoteSize}) · local {localSize}",
        "conflictHint": "Both the web copy and the local file changed since the last sync. Sync will not overwrite either one. Keep the version you want and sync again.",
        "errors": "Errors",
        "pendingDeletes": "Deleted locally — delete in Picstome?",
        "deleteRemote": "Delete in Picstome",
        "deleteRemoteHint": "Only the photos you select will be deleted remotely. Local files are never touched.",
        "confirmDelete": "Delete selected photos",
        "confirmRemoteDeleteTitle": "Delete photos from Picstome?",
        "confirmRemoteDeleteMessage": "You are about to delete {n} photo(s) from Picstome. This cannot be undone. Local files will not be touched.",
        "confirmRemoteDeleteAction": "Delete from Picstome",
        "galleryMissingTitle": "Gallery not found in Picstome",
        "galleryMissingMessage": "The gallery “{name}” no longer exists in Picstome. Nothing will be deleted from your local disk. Remove it from the app? Your local files will remain untouched.",
        "galleryMissingRemove": "Remove from app",
        "cancel": "Cancel",
        "close": "Close",
        "selectFolderTitle": "Where should this gallery be synced?",
        "folderError": "Folder not accessible",
        "notFoundDetail": "not found",
        "updateAvailable": "Update available — downloading...",
        "updateReady": "Update ready. Restart the app to install it.",
        "checkUpdates": "Check for updates",
        "restartUpdate": "Restart & Update",
        "exiftoolMissing": "Photo tool unavailable",
        "tooManyFiles": "Too many files in one link (limit {max})",
        "invalidRequest": "Invalid request",
        "noFolder": "No folder selected",
        "waiting": "Waiting for a Picstome link...",
        "needFolder": "Select a photos folder first",
        "needFiles": "Waiting for a Picstome link...",
        "ready": "Ready to rate {n} photo{s} with {r} ★",
        "applying": "Applying rating…",
        "applyBtn": "Apply rating",
        "changeBtn": "Change...",
        "folderLabel": "Photos folder",
        "rating": "Rating",
        "rated": "✓ {n} file{s} rated",
        "notFoundCount": "⚠ {n} not found",
        "errorCount": "✖ {n} error{s}",
        "syncing": "Syncing…",
        "scanning": "Scanning local folder…",
        "downloadingProgress": "Downloading {done}/{total}",
        "uploadingProgress": "Uploading {done}/{total}",
        "syncDone": "Sync finished",
        "syncFailed": "Sync failed",
        "reconnect": "Session expired. Please connect again.",
        "linked": "Linked",
        "unnamedGallery": "Gallery",
        "selectGallery": "Select a gallery",
        "demoHint": "Fake API running locally. Changes you trigger on the demo web appear after Update.",
        "info": "Info",
        "infoTitle": "Build information",
        "infoVersion": "Version",
        "infoProfile": "Build",
        "infoMode": "Mode",
        "infoTarget": "API target",
        "infoClose": "Close",
        "modeProduction": "Production",
        "modeDevelopment": "Development",
        "modeLocal": "Local",
        "modeDemo": "Demo (fake API)",
        "modeCustom": "Custom",
        "apiSwitchTitle": "Test API target",
        "useProduction": "Production",
        "useDevelopment": "Development",
        "apiSwitchHint": "Switching target signs you out. Connect again afterwards.",
        "apiSwitched": "API target changed. Please connect again.",
        "galleries": "Galleries",
        "searchGalleries": "Search galleries…",
        "remoteResults": "Found in Picstome",
        "recentGalleries": "Recent in Picstome",
        "remoteSearching": "Searching Picstome…",
        "remoteSearchError": "Could not search Picstome",
        "remoteLoadMore": "Load more",
        "remoteGalleryHint": "Click to choose a folder and sync",
        "createGallery": "Create gallery in Picstome",
        "noSearchResults": "No galleries match your search",
        "noGalleryTitle": "Select a gallery",
        "noGalleryHint": "Pick a gallery from the sidebar or open Picstome to start.",
        "welcomeTitle": "Welcome to Picstome Sync",
        "welcomeHint": "Open a gallery in Picstome and press “Sync / Open in app”, or try the demo.",
        "sync": "Sync",
        "checkingChanges": "Checking changes in Picstome…",
        "scanningLocal": "Scanning local folder…",
        "progressDone": "Sync complete",
        "favoritesFound": "{n} favorite photo(s) found in Picstome",
        "favoritesHint": "Choose how many stars to write locally. Nothing changes until you press Apply rating.",
        "favoritesList": "Favorites",
        "favoritesListTitle": "Export favorite list",
        "favoritesListSubtitle": "{n} favorite photo(s) in this gallery.",
        "favoritesListEmpty": "No favorites in this gallery yet.",
        "favoritesListSearch": "Filter filenames…",
        "favoritesListCopy": "Copy",
        "favoritesListCopied": "Copied!",
        "favoritesListCopyFailed": "Could not copy",
        "favoritesListLoadFailed": "Could not load favorites",
        "favoritesListLoading": "Loading favorites…",
        "favoritesTabLightroom": "Lightroom",
        "favoritesTabCaptureOne": "Capture One",
        "favoritesTabFinder": "Finder/Explorer",
        "favoritesTabList": "List",
        "favoritesLightroomHint": "Paste this into Lightroom's Library Filter. Then select the photos and use Metadata → Read Metadata from File after applying ratings.",
        "favoritesCaptureOneHint": "Paste this into Capture One's filter/search field.",
        "favoritesFinderHint": "Paste this into Finder or Explorer search.",
        "favoritesTextHint": "One filename per line.",
        "favoritesFilterEmpty": "No filenames match the filter.",
        "ratingTitle": "Favorite photos",
        "ratePhotosTitle": "Rate photos",
        "ratePhotosHint": "Choose how many stars to write to the selected photos.",
        "applyRating": "Apply rating",
        "notNow": "Not now",
        "ratingDone": "Rating finished",
        "ratingFailed": "Rating failed",
        "chooseFolder": "Choose folder…",
        "notConnected": "Not connected",
        "openProfile": "Open account profile",
        "infoAccount": "Account",
        "syncHint": "Fetches remote changes and applies local ones.",
        "readyToSync": "Ready to sync",
        "lastSyncAt": "Last sync: {date}",
        "remoteDeletedTitle": "Deleted in Picstome",
        "remoteDeletedHint": "{n} photo(s) were deleted in Picstome. What do you want to do with the local copies?",
        "deleteLocal": "Move to Trash",
        "keepLocal": "Keep local",
        "uploadAgain": "Upload again",
        "remoteDeletedDone": "Done",
        "remoteDeletedError": "Some photos could not be resolved",
        "uploadedFiles": "Uploaded",
        "downloadedFiles": "Downloaded",
        "activityLog": "Activity log",
        "galleryStats": "{n} photos · {size} on disk",
        "clearLog": "Clear log",
        "noActivity": "No sync activity yet",
        "localNewReason": "new local file",
        "localModifiedReason": "modified locally",
        "ratingLogLine": "Rating {stars}★ · {n} file(s)",
        "ratingSourceLink": "from Picstome link",
        "ratingSourceFavorites": "favorites",
        "ratedFiles": "Rated",
        "deletionRemoteDelete": "Deleted in Picstome",
        "deletionLocalDelete": "Moved local files to trash",
        "deletionKeepLocal": "Kept local copies",
        "deletionUploadAgain": "Uploaded local copies again",
        "deletionTrashSystem": "system trash",
        "deletionTrashGallery": "gallery trash",
        "deletionStatusDeleted": "deleted",
        "deletionStatusNotFound": "not found",
        "deletionStatusFailed": "failed",
        "localDeletionProposed": "Local deletion proposed",
        "remoteDeletionDetected": "Deleted in Picstome",
        "trashedTo": "Trash: {path}",
    })
}

fn spanish() -> Value {
    json!({
        "appTitle": "Picstome Sync",
        "syncTab": "Sincronizar",
        "rateTab": "Valorar",
        "connect": "Abrir Picstome",
        "connectedAs": "Conectado como",
        "logout": "Cerrar sesión",
        "mockMode": "Modo demo (API falsa)",
        "noGalleries": "Todavía no hay galerías sincronizadas",
        "noGalleriesHint": "Abre una galería en Picstome y pulsa «Sincronizar / Abrir en la app», o prueba la galería de demo.",
        "syncDemo": "Sincronizar galería demo",
        "update": "Actualizar",
        "openFolder": "Abrir carpeta",
        "openGalleryInPicstome": "Abrir galería en Picstome",
        "unlink": "Desvincular",
        "confirmUnlinkTitle": "¿Desvincular esta galería?",
        "confirmUnlinkMessage": "«{name}» se quitará de la app. Tu carpeta y tus fotos locales no se tocarán.",
        "confirmUnlinkAction": "Desvincular",
        "changeFolder": "Cambiar carpeta…",
        "choosingFolder": "Eligiendo carpeta…",
        "locate": "Localizar…",
        "folderMissing": "Carpeta no encontrada",
        "lastSync": "Última sync",
        "never": "nunca",
        "photos": "fotos",
        "remoteGalleries": "Galerías en Picstome",
        "back": "Atrás",
        "updateNow": "Actualizar ahora",
        "updateAvailableTitle": "Nueva versión disponible",
        "updateAvailableMessage": "Picstome Sync {latest} está disponible. Estás usando {current}. ¿Quieres descargar la nueva versión desde GitHub?",
        "updateDownload": "Descargar actualización",
        "inSync": "Todo está sincronizado",
        "backToSync": "Volver a la lista",
        "downloaded": "Descargadas",
        "uploaded": "Subidas",
        "conflicts": "Conflictos",
        "warnings": "Avisos",
        "duplicateLocalIgnored": "{filename} no se subirá; se usará {preferred} para esta foto.",
        "conflictDetail": "{filename} — remoto v{remote} ({remoteSize}) · local {localSize}",
        "conflictHint": "La copia web y el archivo local han cambiado desde la última sync. La app no sobrescribirá ninguna. Conserva la versión que quieras y vuelve a sincronizar.",
        "errors": "Errores",
        "pendingDeletes": "Borradas en local — ¿borrar en Picstome?",
        "deleteRemote": "Borrar en Picstome",
        "deleteRemoteHint": "Solo se borrarán en remoto las fotos que selecciones. Los archivos locales nunca se tocan.",
        "confirmDelete": "Borrar fotos seleccionadas",
        "confirmRemoteDeleteTitle": "¿Borrar fotos de Picstome?",
        "confirmRemoteDeleteMessage": "Vas a borrar {n} foto(s) de Picstome. Esto no se puede deshacer. Los archivos locales no se tocan.",
        "confirmRemoteDeleteAction": "Borrar de Picstome",
        "galleryMissingTitle": "Galería no encontrada en Picstome",
        "galleryMissingMessage": "La galería «{name}» ya no existe en Picstome. No se borrará nada de tu disco local. ¿Quitarla de la app? Tus archivos locales no se tocarán.",
        "galleryMissingRemove": "Quitar de la app",
        "cancel": "Cancelar",
        "close": "Cerrar",
        "selectFolderTitle": "¿Dónde quieres sincronizar esta galería?",
        "folderError": "Carpeta no accesible",
        "notFoundDetail": "no encontrado",
        "updateAvailable": "Actualización disponible — descargando...",
        "updateReady": "Actualización lista. Reinicia la app para instalarla.",
        "checkUpdates": "Buscar actualizaciones",
        "restartUpdate": "Reiniciar y actualizar",
        "exiftoolMissing": "Herramienta de fotos no disponible",
        "tooManyFiles": "Demasiados archivos en un enlace (límite {max})",
        "invalidRequest": "Solicitud inválida",
        "noFolder": "Ninguna carpeta seleccionada",
        "waiting": "Esperando enlace de Picstome...",
        "needFolder": "Selecciona primero la carpeta de fotos",
        "needFiles": "Esperando enlace de Picstome...",
        "ready": "Listo para valorar {n} foto{s} con {r} ★",
        "applying": "Aplicando valoración…",
        "applyBtn": "Aplicar valoración",
        "changeBtn": "Cambiar...",
        "folderLabel": "Carpeta de fotos",
        "rating": "Valoración",
        "rated": "✓ {n} archivo{s} valorado{s}",
        "notFoundCount": "⚠ {n} no encontrado{s}",
        "errorCount": "✖ {n} error{s}",
        "syncing": "Sincronizando…",
        "scanning": "Analizando carpeta local…",
        "downloadingProgress": "Descargando {done}/{total}",
        "uploadingProgress": "Subiendo {done}/{total}",
        "syncDone": "Sincronización terminada",
        "syncFailed": "Error de sincronización",
        "reconnect": "La sesión ha caducado. Vuelve a conectar.",
        "linked": "Vinculada",
        "unnamedGallery": "Galería",
        "selectGallery": "Selecciona una galería",
        "demoHint": "La API falsa se ejecuta en local. Los cambios que provoques en la web demo aparecen al pulsar Actualizar.",
        "info": "Info",
        "infoTitle": "Información de la build",
        "infoVersion": "Versión",
        "infoProfile": "Build",
        "infoMode": "Modo",
        "infoTarget": "API utilizada",
        "infoClose": "Cerrar",
        "modeProduction": "Producción",
        "modeDevelopment": "Desarrollo",
        "modeLocal": "Local",
        "modeDemo": "Demo (API falsa)",
        "modeCustom": "Personalizada",
        "apiSwitchTitle": "API de prueba",
        "useProduction": "Producción",
        "useDevelopment": "Desarrollo",
        "apiSwitchHint": "Cambiar de entorno cierra la sesión. Vuelve a conectar después.",
        "apiSwitched": "API cambiada. Vuelve a conectar.",
        "galleries": "Galerías",
        "searchGalleries": "Buscar galerías…",
        "remoteResults": "Encontradas en Picstome",
        "recentGalleries": "Recientes en Picstome",
        "remoteSearching": "Buscando en Picstome…",
        "remoteSearchError": "No se pudo buscar en Picstome",
        "remoteLoadMore": "Cargar más",
        "remoteGalleryHint": "Pulsa para elegir carpeta y sincronizar",
        "createGallery": "Crear galería en Picstome",
        "noSearchResults": "Ninguna galería coincide con la búsqueda",
        "noGalleryTitle": "Selecciona una galería",
        "noGalleryHint": "Elige una galería en la barra lateral o abre Picstome para empezar.",
        "welcomeTitle": "Bienvenido a Picstome Sync",
        "welcomeHint": "Abre una galería en Picstome y pulsa «Sincronizar / Abrir en la app», o prueba la demo.",
        "sync": "Sincronizar",
        "checkingChanges": "Buscando cambios en Picstome…",
        "scanningLocal": "Analizando la carpeta local…",
        "progressDone": "Sincronización completa",
        "favoritesFound": "{n} foto(s) favorita(s) encontradas en Picstome",
        "favoritesHint": "Elige cuántas estrellas escribir en local. No se cambia nada hasta pulsar Aplicar valoración.",
        "favoritesList": "Favoritos",
        "favoritesListTitle": "Exportar lista de favoritos",
        "favoritesListSubtitle": "{n} foto(s) favorita(s) en esta galería.",
        "favoritesListEmpty": "Todavía no hay favoritas en esta galería.",
        "favoritesListSearch": "Filtrar nombres…",
        "favoritesListCopy": "Copiar",
        "favoritesListCopied": "¡Copiado!",
        "favoritesListCopyFailed": "No se pudo copiar",
        "favoritesListLoadFailed": "No se pudieron cargar las favoritas",
        "favoritesListLoading": "Cargando favoritas…",
        "favoritesTabLightroom": "Lightroom",
        "favoritesTabCaptureOne": "Capture One",
        "favoritesTabFinder": "Finder/Explorador",
        "favoritesTabList": "Lista",
        "favoritesLightroomHint": "Pega esto en el filtro de biblioteca de Lightroom. Después de aplicar las valoraciones, selecciona las fotos y usa Metadatos → Leer metadatos del archivo.",
        "favoritesCaptureOneHint": "Pega esto en el campo de filtro/búsqueda de Capture One.",
        "favoritesFinderHint": "Pega esto en la búsqueda de Finder o Explorador.",
        "favoritesTextHint": "Un nombre de archivo por línea.",
        "favoritesFilterEmpty": "Ningún nombre coincide con el filtro.",
        "ratingTitle": "Fotos favoritas",
        "ratePhotosTitle": "Valorar fotos",
        "ratePhotosHint": "Elige cuántas estrellas escribir en las fotos seleccionadas.",
        "applyRating": "Aplicar valoración",
        "notNow": "Ahora no",
        "ratingDone": "Valoración terminada",
        "ratingFailed": "Error al valorar",
        "chooseFolder": "Elegir carpeta…",
        "notConnected": "Sin conexión",
        "openProfile": "Abrir perfil de la cuenta",
        "infoAccount": "Cuenta",
        "syncHint": "Busca cambios remotos y aplica los locales.",
        "readyToSync": "Listo para sincronizar",
        "lastSyncAt": "Última sync: {date}",
        "remoteDeletedTitle": "Borradas en Picstome",
        "remoteDeletedHint": "{n} foto(s) se borraron en Picstome. ¿Qué quieres hacer con las copias locales?",
        "deleteLocal": "Mover a la papelera",
        "keepLocal": "Mantener en local",
        "uploadAgain": "Subir de nuevo",
        "remoteDeletedDone": "Hecho",
        "remoteDeletedError": "Algunas fotos no se pudieron resolver",
        "uploadedFiles": "Subidas",
        "downloadedFiles": "Descargadas",
        "activityLog": "Registro de actividad",
        "galleryStats": "{n} fotos · {size} en disco",
        "clearLog": "Limpiar registro",
        "noActivity": "Todavía no hay actividad de sync",
        "localNewReason": "archivo nuevo en local",
        "localModifiedReason": "modificado en local",
        "ratingLogLine": "Valoración {stars}★ · {n} archivo(s)",
        "ratingSourceLink": "desde enlace de Picstome",
        "ratingSourceFavorites": "favoritas",
        "ratedFiles": "Valoradas",
        "deletionRemoteDelete": "Borradas en Picstome",
        "deletionLocalDelete": "Archivos locales movidos a la papelera",
        "deletionKeepLocal": "Copias locales conservadas",
        "deletionUploadAgain": "Copias locales subidas de nuevo",
        "deletionTrashSystem": "papelera del sistema",
        "deletionTrashGallery": "papelera de la galería",
        "deletionStatusDeleted": "borrada",
        "deletionStatusNotFound": "no encontrada",
        "deletionStatusFailed": "error",
        "localDeletionProposed": "Borrado local propuesto",
        "remoteDeletionDetected": "Borrada en Picstome",
        "trashedTo": "Papelera: {path}",
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn english_and_spanish_have_the_same_keys() {
        let en = english();
        let es = spanish();
        let en_keys: std::collections::BTreeSet<&String> = en.as_object().unwrap().keys().collect();
        let es_keys: std::collections::BTreeSet<&String> = es.as_object().unwrap().keys().collect();
        assert_eq!(en_keys, es_keys, "translation key mismatch");
    }

    #[test]
    fn detects_spanish_keyboards() {
        assert!(is_spanish_keyboard_id("com.apple.keylayout.Spanish"));
        assert!(is_spanish_keyboard_id("com.apple.keylayout.Spanish-ISO"));
        assert!(!is_spanish_keyboard_id("com.apple.keylayout.US"));
        assert!(is_spanish_xkb_layout("es,us"));
        assert!(is_spanish_xkb_layout("us, es"));
        assert!(!is_spanish_xkb_layout("us,de"));
        assert!(is_spanish_windows_layout(
            "HKEY_CURRENT_USER\\Keyboard Layout\\Preload\n    1    REG_SZ    0000040a\n"
        ));
        assert!(!is_spanish_windows_layout(
            "HKEY_CURRENT_USER\\Keyboard Layout\\Preload\n    1    REG_SZ    00000409\n"
        ));
    }

    #[test]
    fn runtime_info_strings_exist() {
        for lang in ["en", "es"] {
            let value = strings(lang);
            for key in [
                "info",
                "infoTitle",
                "infoVersion",
                "infoProfile",
                "infoMode",
                "infoTarget",
                "infoClose",
                "modeProduction",
                "modeDevelopment",
                "modeLocal",
                "modeDemo",
                "modeCustom",
                "apiSwitchTitle",
                "useProduction",
                "useDevelopment",
                "apiSwitchHint",
                "apiSwitched",
                "galleries",
                "searchGalleries",
                "noSearchResults",
                "sync",
                "ratingTitle",
                "applyRating",
                "notNow",
            ] {
                assert!(
                    value.get(key).and_then(|value| value.as_str()).is_some(),
                    "missing {key} in {lang}"
                );
            }
        }
    }
}
