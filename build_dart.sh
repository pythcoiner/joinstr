#!/bin/bash

# Exit if any command fails
set -e

# Variables
RUST_LIB_NAME="joinstr"
ANDROID_OUTPUT_DIR="./dart/android"
IOS_OUTPUT_DIR="./dart/ios/Frameworks"

# Set default ANDROID_NDK_HOME if not provided
ANDROID_NDK_HOME="${ANDROID_NDK_HOME:-/opt/android-ndk}"

# Check if Android NDK exists
if [ ! -d "$ANDROID_NDK_HOME" ]; then
  echo "Android NDK not found."
  echo "Please set ANDROID_NDK_HOME or install the NDK."
  exit 1
fi

export ANDROID_NDK_HOME

echo "Using Android NDK at: $ANDROID_NDK_HOME"

# Create output directories
mkdir -p $ANDROID_OUTPUT_DIR/armeabi-v7a
mkdir -p $ANDROID_OUTPUT_DIR/arm64-v8a
mkdir -p $IOS_OUTPUT_DIR

echo "Building Rust library for Android..."

# Add Android targets if not already added
rustup target add aarch64-linux-android armv7-linux-androideabi || true

# Install cargo-ndk if not installed
if ! command -v cargo-ndk &> /dev/null
then
    echo "Installing cargo-ndk..."
    cargo install cargo-ndk
fi

# Build for Android (32-bit and 64-bit)
cargo ndk -t armeabi-v7a -t arm64-v8a -o $ANDROID_OUTPUT_DIR build --release

echo "Android .so files built and placed in $ANDROID_OUTPUT_DIR"

# echo "Building Rust library for iOS..."
#
# # Add iOS targets if not already added
# rustup target add aarch64-apple-ios x86_64-apple-ios aarch64-apple-ios-sim || true
#
# # Install cargo-lipo if not installed
# if ! command -v cargo-lipo &> /dev/null
# then
#     echo "Installing cargo-lipo..."
#     cargo install cargo-lipo
# fi
#
# # Build universal static library for iOS
# cargo lipo --release --allow-run-on-non-macos
#
# # Copy .a file to iOS output directory
# cp target/aarch64-apple-ios/release/lib${RUST_LIB_NAME}.a $IOS_OUTPUT_DIR/
#
# echo "iOS .a file built and placed in $IOS_OUTPUT_DIR"

echo "Build complete!"
