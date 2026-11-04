#!/usr/bin/env bash
set -euo pipefail

# Usage: scripts/release-notes.sh <version-or-tag>
VERSION="${1:-}"
VERSION="${VERSION#v}"
if [[ -z "$VERSION" ]]; then
  echo "usage: $0 <version-or-tag>" >&2
  exit 1
fi

NOTES="$(cat <<'EOF'
# Picstome Sync __VERSION__ — Descarga / Download

> Build de prueba: usa galerías de prueba y mantén copias de seguridad.  
> Test build: use test galleries and keep backups.

## 🇪🇸 Español

### ¿Qué archivo descargo?

| Sistema | Archivo recomendado |
|---|---|
| macOS (Apple Silicon o Intel) | `Picstome.Sync___VERSION___universal.dmg` |
| Windows 64-bit | `Picstome.Sync___VERSION___x64-setup.exe` |
| Linux Debian/Ubuntu | `Picstome.Sync___VERSION___amd64.deb` |
| Linux portable | `Picstome.Sync___VERSION___amd64.AppImage` |

- **macOS:** el DMG es universal, sirve tanto para Apple Silicon como para Intel.
- **Windows:** el `.exe` es el instalador recomendado. El `.msi` está pensado para instalaciones gestionadas.
- **Linux:** para el AppImage, hazlo ejecutable con `chmod +x` y ábrelo. El `.deb` se instala con el gestor de paquetes.

> ⚠️ **macOS:** la app no está notarizada. Si aparece como “dañada”, ejecuta:
> ```bash
> xattr -cr "/Applications/Picstome Sync.app"
> ```
> También puedes abrirla con clic derecho → **Abrir**.

> ⚠️ **Windows:** el instalador no está firmado. Si SmartScreen muestra un aviso, pulsa **Más información → Ejecutar de todas formas**.

## 🇬🇧 English

### Which file should I download?

| System | Recommended file |
|---|---|
| macOS (Apple Silicon or Intel) | `Picstome.Sync___VERSION___universal.dmg` |
| Windows 64-bit | `Picstome.Sync___VERSION___x64-setup.exe` |
| Linux Debian/Ubuntu | `Picstome.Sync___VERSION___amd64.deb` |
| Linux portable | `Picstome.Sync___VERSION___amd64.AppImage` |

- **macOS:** the DMG is universal and works on both Apple Silicon and Intel.
- **Windows:** the `.exe` is the recommended installer. The `.msi` is intended for managed installations.
- **Linux:** make the AppImage executable with `chmod +x` and run it. Install the `.deb` with your package manager.

> ⚠️ **macOS:** the app is not notarized. If macOS says it is “damaged”, run:
> ```bash
> xattr -cr "/Applications/Picstome Sync.app"
> ```
> You can also right-click the app → **Open**.

> ⚠️ **Windows:** the installer is unsigned. If SmartScreen warns you, click **More info → Run anyway**.
EOF
)"
printf '%s
' "$NOTES" | sed "s/__VERSION__/${VERSION}/g"
