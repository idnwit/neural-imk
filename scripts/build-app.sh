#!/usr/bin/env bash
# Builds build/NeuralIMK.app: Rust staticlib + cbindgen header + swiftc link.
# Bundles pretrained models from target/models/bundle (runs `make models` if missing).
# Env: ARCHS="arm64 x86_64" for a universal binary (default: host arch only).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

SRC="macos/NeuralIMK"
APP="build/NeuralIMK.app"
BIN_NAME="NeuralIMK"
MIN_MACOS="13.0"
HOST_ARCH="$(uname -m)"
ARCHS="${ARCHS:-$HOST_ARCH}"

export MACOSX_DEPLOYMENT_TARGET="$MIN_MACOS"

rust_triple() {
  case "$1" in
    arm64) echo "aarch64-apple-darwin" ;;
    x86_64) echo "x86_64-apple-darwin" ;;
    *) echo "unsupported arch: $1" >&2; exit 1 ;;
  esac
}

echo "==> header"
mkdir -p target
cbindgen --config crates/nimk-ffi/cbindgen.toml --crate nimk-ffi --output target/nimk_ffi.h

MODELS="target/models/bundle"
if [[ ! -f "$MODELS/model/linear_action/latest" || ! -f "$MODELS/model/gru_action/latest" ]]; then
  echo "==> models (make models)"
  make models
fi

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources" build/obj

SLICES=()
for arch in $ARCHS; do
  echo "==> rust ($arch)"
  if [[ "$arch" == "$HOST_ARCH" ]]; then
    cargo build --release -p nimk-ffi
    LIB="target/release/libnimk_ffi.a"
  else
    triple="$(rust_triple "$arch")"
    cargo build --release -p nimk-ffi --target "$triple"
    LIB="target/$triple/release/libnimk_ffi.a"
  fi

  echo "==> swift ($arch)"
  OUT="build/obj/$BIN_NAME-$arch"
  swiftc -O -swift-version 5 \
    -target "$arch-apple-macos$MIN_MACOS" \
    -module-name NeuralIMK \
    -import-objc-header "$SRC/NeuralIMK-Bridging.h" \
    -Xcc -I -Xcc "$ROOT/target" \
    "$SRC"/*.swift \
    "$LIB" \
    -framework Cocoa -framework InputMethodKit -framework CoreGraphics \
    -o "$OUT"
  SLICES+=("$OUT")
done

if [[ ${#SLICES[@]} -gt 1 ]]; then
  lipo -create "${SLICES[@]}" -output "$APP/Contents/MacOS/$BIN_NAME"
else
  cp "${SLICES[0]}" "$APP/Contents/MacOS/$BIN_NAME"
fi

echo "==> bundled models"
mkdir -p "$APP/Contents/Resources/models"
cp -R "$MODELS/." "$APP/Contents/Resources/models/"

# Reference copy only: the engine embeds its defaults (docs/11-config.md).
cp crates/nimk-engine/config/default.toml "$APP/Contents/Resources/default-config.toml"

cp "$SRC/Info.plist" "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

echo "==> icon"
ICON="$APP/Contents/Resources/NeuralIMK.tiff"
if ! swift scripts/make-icon.swift "$ICON" 2>/dev/null; then
  # Fallback placeholder: the generic app icon, scaled to menu size.
  echo "warning: icon render failed, using placeholder" >&2
  sips -s format tiff -z 16 16 \
    /System/Library/CoreServices/CoreTypes.bundle/Contents/Resources/GenericApplicationIcon.icns \
    --out "$ICON" >/dev/null
fi

echo "==> codesign (ad hoc)"
codesign -s - --force --deep "$APP"

echo "built $APP"
