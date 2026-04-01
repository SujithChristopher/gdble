#!/bin/bash
# Build GdBLE and deploy to both the submodule addons/ and the parent project addons/.

set -e

OS=$(uname -s)
ARCH=$(uname -m)
echo "Platform: $OS $ARCH"

if [ "$OS" = "Linux" ]; then
    if [ "$ARCH" = "x86_64" ]; then
        TARGET="x86_64-unknown-linux-gnu"
        BIN_SUBDIR="linux-x86_64"
        LIB_NAME="libgdble.so"
    elif [ "$ARCH" = "aarch64" ]; then
        TARGET="aarch64-unknown-linux-gnu"
        BIN_SUBDIR="linux-arm64"
        LIB_NAME="libgdble.so"
    else
        echo "Unsupported Linux architecture: $ARCH" && exit 1
    fi
elif [ "$OS" = "Darwin" ]; then
    if [ "$ARCH" = "x86_64" ]; then
        TARGET="x86_64-apple-darwin"
        BIN_SUBDIR="macos-x86_64"
        LIB_NAME="libgdble.dylib"
    elif [ "$ARCH" = "arm64" ]; then
        TARGET="aarch64-apple-darwin"
        BIN_SUBDIR="macos-arm64"
        LIB_NAME="libgdble.dylib"
    else
        echo "Unsupported macOS architecture: $ARCH" && exit 1
    fi
else
    echo "Unsupported OS: $OS" && exit 1
fi

echo "Building for $TARGET..."
cargo build --release --target "$TARGET"

SRC="target/$TARGET/release/$LIB_NAME"
LOCAL_BIN="addons/gdble/bin/$BIN_SUBDIR"
PROJECT_BIN="../addons/gdble/bin/$BIN_SUBDIR"

echo "Copying to submodule addons..."
mkdir -p "$LOCAL_BIN"
cp "$SRC" "$LOCAL_BIN/$LIB_NAME"

echo "Copying to main project addons..."
mkdir -p "$PROJECT_BIN"
cp "$SRC" "$PROJECT_BIN/$LIB_NAME"

echo ""
echo "Build complete!"
echo "  Submodule : $LOCAL_BIN/$LIB_NAME"
echo "  Project   : $PROJECT_BIN/$LIB_NAME"
