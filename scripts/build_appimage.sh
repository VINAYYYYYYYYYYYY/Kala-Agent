#!/usr/bin/env bash
# Build a runnable Kala AppImage (x86_64 / aarch64).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DIST="${ROOT}/dist"
APPDIR="${DIST}/Kala.AppDir"
ARCH="$(uname -m)"
OUT="${DIST}/Kala-${ARCH}.AppImage"

cd "${ROOT}"
chmod +x "${ROOT}/packaging/AppRun" "${ROOT}/scripts/build_appimage.sh"

echo "==> sync packaging deps"
uv sync --extra packaging

echo "==> ensure icon"
if [[ ! -f "${ROOT}/packaging/kala.png" ]]; then
  uv run python - <<'PY'
from pathlib import Path
from PySide6.QtGui import QImage, QColor, QPainter
from PySide6.QtCore import Qt

img = QImage(256, 256, QImage.Format.Format_ARGB32)
img.fill(QColor("#0e1116"))
p = QPainter(img)
p.setRenderHint(QPainter.RenderHint.Antialiasing)
p.setBrush(QColor("#3d9a7a"))
p.setPen(Qt.PenStyle.NoPen)
p.drawRoundedRect(40, 40, 176, 176, 36, 36)
p.end()
Path("packaging/kala.png").write_bytes(b"")  # touch path
img.save("packaging/kala.png")
print("wrote packaging/kala.png")
PY
fi

echo "==> pyinstaller"
rm -rf "${DIST}/Kala" "${DIST}/pyi-work" "${APPDIR}"
uv run pyinstaller packaging/kala-gui.spec \
  --distpath "${DIST}" \
  --workpath "${DIST}/pyi-work" \
  --noconfirm

echo "==> assemble AppDir"
mkdir -p "${APPDIR}/usr"
cp -a "${DIST}/Kala/." "${APPDIR}/usr/"
if [[ -x "${APPDIR}/usr/kala-gui" ]]; then
  mkdir -p "${APPDIR}/usr/bin"
  ln -sfn ../kala-gui "${APPDIR}/usr/bin/kala-gui"
fi

cp "${ROOT}/packaging/AppRun" "${APPDIR}/AppRun"
chmod +x "${APPDIR}/AppRun"
cp "${ROOT}/packaging/kala.desktop" "${APPDIR}/kala.desktop"
cp "${ROOT}/packaging/kala.png" "${APPDIR}/kala.png"

# Desktop entry Exec must match AppImage convention (binary name relative to AppDir)
sed -i 's|^Exec=.*|Exec=kala-gui|' "${APPDIR}/kala.desktop"
sed -i 's|^Icon=.*|Icon=kala|' "${APPDIR}/kala.desktop"

APPIMAGETOOL="$(command -v appimagetool || true)"
if [[ -z "${APPIMAGETOOL}" ]]; then
  echo "==> downloading appimagetool"
  mkdir -p "${DIST}/tools"
  TOOL="${DIST}/tools/appimagetool-${ARCH}.AppImage"
  if [[ ! -x "${TOOL}" ]]; then
    curl -fsSL -o "${TOOL}" \
      "https://github.com/AppImage/appimagetool/releases/download/continuous/appimagetool-${ARCH}.AppImage"
    chmod +x "${TOOL}"
  fi
  APPIMAGETOOL="${TOOL}"
fi

echo "==> appimagetool"
# Extracted appimagetool may need FUSE; --appimage-extract-and-run avoids that
if [[ "${APPIMAGETOOL}" == *.AppImage ]]; then
  ARCH="${ARCH}" "${APPIMAGETOOL}" --appimage-extract-and-run "${APPDIR}" "${OUT}"
else
  ARCH="${ARCH}" "${APPIMAGETOOL}" "${APPDIR}" "${OUT}"
fi
chmod +x "${OUT}"
echo "Built: ${OUT}"
