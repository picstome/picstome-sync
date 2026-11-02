use serde_json::{json, Value};

pub fn language() -> String {
    let locale = sys_locale::get_locale().unwrap_or_else(|| "en".to_string());
    if locale.to_ascii_lowercase().starts_with("es") {
        "es".to_string()
    } else {
        "en".to_string()
    }
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
        "apiSwitched": "API target changed. Please connect again."
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
        "apiSwitched": "API cambiada. Vuelve a conectar."
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
            ] {
                assert!(
                    value.get(key).and_then(|value| value.as_str()).is_some(),
                    "missing {key} in {lang}"
                );
            }
        }
    }
}
