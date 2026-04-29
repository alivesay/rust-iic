#!/bin/bash
# Build a macOS .app bundle for rust-iic
set -euo pipefail

APP_NAME="RustIIc"
BUNDLE_ID="com.rustiic.emulator"
BINARY="rust-iic"
ICON="assets/icon.icns"

# Build release binary
cargo build --release

# Create .app bundle structure
APP_DIR="target/${APP_NAME}.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

# Copy binary
cp "target/release/${BINARY}" "$APP_DIR/Contents/MacOS/"

# Copy icon
cp "$ICON" "$APP_DIR/Contents/Resources/AppIcon.icns"

# Create Info.plist
cat > "$APP_DIR/Contents/Info.plist" << EOF
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleName</key>
    <string>${APP_NAME}</string>
    <key>CFBundleDisplayName</key>
    <string>Apple //c Emulator</string>
    <key>CFBundleIdentifier</key>
    <string>${BUNDLE_ID}</string>
    <key>CFBundleVersion</key>
    <string>0.1.0</string>
    <key>CFBundleShortVersionString</key>
    <string>0.1.0</string>
    <key>CFBundleExecutable</key>
    <string>${BINARY}</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>LSMinimumSystemVersion</key>
    <string>11.0</string>
</dict>
</plist>
EOF

echo "Built: ${APP_DIR}"
echo "Run with: open ${APP_DIR}"
