#!/bin/sh
# Builds "FM-1 Big Screen.app" (DEBUGGER=1: "FM-1 Big Screen Debugger.app", with the MIDI debugger) (Apple silicon + Intel in one) and installs it in /Applications.
set -e
cd "$(dirname "$0")"
export PATH="$HOME/.cargo/bin:$PATH"
rustup target add aarch64-apple-darwin x86_64-apple-darwin >/dev/null
# LITE=1: the lite version, without the Mellotron's built-in sounds (it can download them)
FEAT=""; [ "$LITE" = 1 ] && FEAT="--no-default-features"
[ "$DEBUGGER" = 1 ] && FEAT="$FEAT --features debugger"
cargo build --release $FEAT --target aarch64-apple-darwin
cargo build --release $FEAT --target x86_64-apple-darwin

VER=$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)
NAME="FM-1 Big Screen"; ID="com.pfkellogg.fm1-mirror"
[ "$DEBUGGER" = 1 ] && NAME="FM-1 Big Screen Debugger" && ID="com.pfkellogg.fm1-mirror.debugger"   # beside the normal app
APP="dist/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp icon/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"   # (tools/make_icon.py draws it)
[ "$LITE" = 1 ] || cp -R assets/mellotron "$APP/Contents/Resources/mellotron"   # the Mellotron's strips (src/mellotron.rs)
lipo -create -output "$APP/Contents/MacOS/fm1-mirror" \
    target/aarch64-apple-darwin/release/fm1-mirror target/x86_64-apple-darwin/release/fm1-mirror
cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>CFBundleName</key><string>$NAME</string>
  <key>CFBundleDisplayName</key><string>$NAME</string>
  <key>CFBundleIdentifier</key><string>$ID</string>
  <key>CFBundleExecutable</key><string>fm1-mirror</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>CFBundleIconFile</key><string>AppIcon</string>
  <key>CFBundleVersion</key><string>$VER</string>
  <key>CFBundleShortVersionString</key><string>$VER</string>
  <key>LSMinimumSystemVersion</key><string>11.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>The spyglass shows the waveform of the FM-1's own sound, which arrives over its USB cable as an audio input. FM-1 Big Screen listens only to the FM-1.</string>
</dict>
</plist>
PLIST
codesign --force --deep -s - "$APP"
rm -rf "/Applications/$NAME.app"
cp -R "$APP" /Applications/
echo "Installed /Applications/$NAME.app ($VER)"
