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
        "unlink": "Unlink",
        "changeFolder": "Change folder…",
        "locate": "Locate…",
        "folderMissing": "Folder not found",
        "lastSync": "Last sync",
        "never": "never",
        "photos": "photos",
        "remoteGalleries": "Galleries in Picstome",
        "back": "Back",
        "updateNow": "Update now",
        "inSync": "Everything is in sync",
        "backToSync": "Back to sync list",
        "downloaded": "Downloaded",
        "uploaded": "Uploaded",
        "conflicts": "Conflicts",
        "errors": "Errors",
        "pendingDeletes": "Deleted locally — delete in Picstome?",
        "deleteRemote": "Delete in Picstome",
        "deleteRemoteHint": "Only the photos you select will be deleted remotely. Local files are never touched.",
        "confirmDelete": "Delete selected photos",
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
        "deleteLocal": "Delete locally",
        "keepLocal": "Keep local",
        "uploadAgain": "Upload again",
        "remoteDeletedDone": "Done",
        "remoteDeletedError": "Some photos could not be resolved",
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
        "unlink": "Desvincular",
        "changeFolder": "Cambiar carpeta…",
        "locate": "Localizar…",
        "folderMissing": "Carpeta no encontrada",
        "lastSync": "Última sync",
        "never": "nunca",
        "photos": "fotos",
        "remoteGalleries": "Galerías en Picstome",
        "back": "Atrás",
        "updateNow": "Actualizar ahora",
        "inSync": "Todo está sincronizado",
        "backToSync": "Volver a la lista",
        "downloaded": "Descargadas",
        "uploaded": "Subidas",
        "conflicts": "Conflictos",
        "errors": "Errores",
        "pendingDeletes": "Borradas en local — ¿borrar en Picstome?",
        "deleteRemote": "Borrar en Picstome",
        "deleteRemoteHint": "Solo se borrarán en remoto las fotos que selecciones. Los archivos locales nunca se tocan.",
        "confirmDelete": "Borrar fotos seleccionadas",
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
        "deleteLocal": "Borrar en local",
        "keepLocal": "Mantener en local",
        "uploadAgain": "Subir de nuevo",
        "remoteDeletedDone": "Hecho",
        "remoteDeletedError": "Algunas fotos no se pudieron resolver",
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
