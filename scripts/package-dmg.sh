#!/usr/bin/env bash
# Builds a universal app and packs it into a drag-to-Applications DMG.
# Env: NIMK_SIGN_IDENTITY (Developer ID Application; ad hoc if unset),
#      NIMK_NOTARY_PROFILE (notarytool keychain profile; skip notarization if unset).
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

ARCHS="arm64 x86_64" ./scripts/build-app.sh
APP="build/NeuralIMK.app"
VERSION="$(plutil -extract CFBundleShortVersionString raw "$APP/Contents/Info.plist")"
DMG="build/NeuralIMK-$VERSION.dmg"
STAGE="build/dmg-stage"
rm -rf "$STAGE" "$DMG"
mkdir -p "$STAGE"
cp -R "$APP" "$STAGE/"
swift scripts/make-dmg-background.swift build/dmg-background.png

if command -v create-dmg >/dev/null; then
  # create-dmg exits 2 when it only failed to set a custom icon.
  create-dmg --volname "NeuralIMK" --background build/dmg-background.png \
    --window-size 640 400 --icon-size 112 --icon "NeuralIMK.app" 160 190 \
    --app-drop-link 480 190 --hide-extension "NeuralIMK.app" "$DMG" "$STAGE" \
    || [[ $? -eq 2 ]]
else
  ln -s /Applications "$STAGE/Applications"
  hdiutil create -volname "NeuralIMK" -srcfolder "$STAGE" -ov -format UDZO "$DMG"
  echo "note: create-dmg not installed; DMG has no background layout (brew install create-dmg)"
fi

if [[ -n "${NIMK_SIGN_IDENTITY:-}" ]]; then
  codesign --force --timestamp -s "$NIMK_SIGN_IDENTITY" "$DMG"
fi
if [[ -n "${NIMK_NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$NIMK_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
elif [[ -z "${NIMK_SIGN_IDENTITY:-}" ]]; then
  echo "warning: ad hoc signed, not notarized: other Macs need System Settings > Privacy & Security > Open Anyway"
fi
echo "built $DMG"
