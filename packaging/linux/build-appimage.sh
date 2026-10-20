#!/usr/bin/env bash
# Build the Linux AppImage for Kala Studio.
#
# The script is intentionally explicit: Vite UI -> native Rust kala-studio
# stage (binary + studio_dist + OCCT lib/ + FreeCAD adapter) -> electron-builder
# AppImage -> headless smoke test. It never publishes and never uses sudo.
#
# Run from anywhere:
#   packaging/linux/build-appimage.sh
#   packaging/linux/build-appimage.sh --no-smoke
#
# Environment:
#   RUSTUP_HOME / CARGO_HOME   Rust toolchain locations (inherit your shell)
#   KALA_OCCT_LIB_DIR          optional explicit OCCT lib directory for staging
#                              (Linux normally resolves the exact TK*.so files
#                              with ldd automatically)
#   KALA_FREECAD_PYTHON        optional explicit FreeCADCmd path. Its Python is
#                              probed and the adapter imports are verified in
#                              that interpreter; all supported ABIs (cp311-cp314)
#                              are vendored either way. Default: FreeCADCmd on PATH.
#   KALA_REQUIRE_FREECAD_VENDOR=1
#                              fail unless FreeCAD is detected and its ABI is
#                              supported and verified; without it the four ABI
#                              sets are still vendored without verification

set -euo pipefail

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd -- "$SCRIPT_DIR/../.." && pwd)"
STUDIO_DIR="$REPO_ROOT/studio"
RUN_SMOKE=1

for argument in "$@"; do
  case "$argument" in
    --no-smoke) RUN_SMOKE=0 ;;
    -h|--help)
      sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
      exit 0
      ;;
    *)
      echo "[build-appimage] unknown argument: $argument" >&2
      exit 2
      ;;
  esac
done

require_command() {
  if ! command -v "$1" >/dev/null 2>&1; then
    echo "[build-appimage] missing required command: $1" >&2
    exit 1
  fi
}

require_command node
require_command npm
require_command cargo
require_command rustc
require_command ldd

cd "$STUDIO_DIR"

echo "[build-appimage] Vite UI"
npm run build

echo "[build-appimage] native kala-studio stage (OCCT + FreeCAD adapter + vendored FreeCAD Python deps)"
NATIVE_ARGS=()
if [[ -n "${KALA_OCCT_LIB_DIR:-}" ]]; then
  NATIVE_ARGS+=(--occt-lib-dir "$KALA_OCCT_LIB_DIR")
fi
if [[ -n "${KALA_FREECAD_PYTHON:-}" ]]; then
  NATIVE_ARGS+=(--freecad-python "$KALA_FREECAD_PYTHON")
fi
if [[ "${KALA_REQUIRE_FREECAD_VENDOR:-0}" == "1" ]]; then
  NATIVE_ARGS+=(--require-freecad-vendor)
fi
node scripts/native-backend.cjs "${NATIVE_ARGS[@]}"

echo "[build-appimage] electron-builder AppImage"
node scripts/desktop-pack.cjs --linux AppImage --x64 --publish never

APPIMAGE="$(
  find "$STUDIO_DIR/release" -maxdepth 1 -type f -name '*.AppImage' -printf '%T@ %p\n' \
    | sort -nr \
    | head -n 1 \
    | cut -d' ' -f2-
)"
if [[ -z "$APPIMAGE" || ! -f "$APPIMAGE" ]]; then
  echo "[build-appimage] electron-builder did not produce an AppImage under studio/release" >&2
  exit 1
fi

echo "[build-appimage] AppImage: $APPIMAGE"
echo "[build-appimage] size: $(stat -c '%s' "$APPIMAGE") bytes"
echo "[build-appimage] sha256: $(sha256sum "$APPIMAGE" | cut -d' ' -f1)"

if [[ "$RUN_SMOKE" == "1" ]]; then
  echo "[build-appimage] headless smoke (extraction path; no X server)"
  node "$SCRIPT_DIR/smoke-appimage.mjs" --appimage "$APPIMAGE"
fi
