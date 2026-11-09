#!/usr/bin/env bash
# Dev install: copies the IME nested in build/NeuralIMK.app into the per-user
# "Input Methods" folder, the location Apple defines for third-party input
# methods (run build-app.sh first). With INSTALL_APP=1 it also copies the
# companion app to /Applications (end users get both from the DMG instead).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUTER="$ROOT/build/NeuralIMK.app"
APP="$OUTER/Contents/Library/InputMethods/NeuralIMK.app"
DEST="$HOME/Library/Input Methods"

if [[ ! -x "$APP/Contents/MacOS/NeuralIMK" ]]; then
  echo "error: $APP not found; run ./scripts/build-app.sh (or make app) first" >&2
  exit 1
fi

mkdir -p "$DEST"
# Stop the running instance (SIGTERM: it flushes the engine before exiting).
killall NeuralIMK 2>/dev/null || true
rm -rf "$DEST/NeuralIMK.app"
cp -R "$APP" "$DEST/"

if [[ "${INSTALL_APP:-0}" == "1" ]]; then
  osascript -e 'quit app id "com.giangndm.NeuralIMK"' 2>/dev/null || true
  rm -rf "/Applications/NeuralIMK.app"
  cp -R "$OUTER" "/Applications/"
  echo "Installed companion app to: /Applications/NeuralIMK.app"
fi

cat <<EOF
Installed to: $DEST/NeuralIMK.app

First time only:
  1. Open System Settings > Keyboard > Input Sources > Edit...
  2. Click "+", pick Vietnamese (or Others) > NeuralIMK, click Add.
     If it is not listed, log out and back in so macOS rescans Input Methods.
  3. Keep "ABC" enabled as a fallback input source.
  4. Select NeuralIMK from the input menu in the menu bar.

After an update: re-select NeuralIMK in the input menu (the old process was stopped).
Data and log locations are chosen by the app through the macOS SDK
(Application Support and Logs, named after the bundle identifier); use the
input menu items "Open data folder" and "Show diagnostics log".
Uninstall:  rm -rf "$DEST/NeuralIMK.app" and remove it from Input Sources.
EOF
