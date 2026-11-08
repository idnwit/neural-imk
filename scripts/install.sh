#!/usr/bin/env bash
# Installs build/NeuralIMK.app into ~/Library/Input Methods (run build-app.sh first).
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
APP="$ROOT/build/NeuralIMK.app"
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

cat <<EOF
Installed to: $DEST/NeuralIMK.app

First time only:
  1. Open System Settings > Keyboard > Input Sources > Edit...
  2. Click "+", pick Vietnamese (or Others) > NeuralIMK, click Add.
     If it is not listed, log out and back in so macOS rescans Input Methods.
  3. Keep "ABC" enabled as a fallback input source.
  4. Select NeuralIMK from the input menu in the menu bar.

After an update: re-select NeuralIMK in the input menu (the old process was stopped).
Debug log:  ~/Library/Logs/NeuralIMK.log
Data:       ~/Library/Application Support/NeuralIMK/
Uninstall:  rm -rf "$DEST/NeuralIMK.app" and remove it from Input Sources.
EOF
