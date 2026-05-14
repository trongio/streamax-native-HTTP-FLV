#!/usr/bin/env bash
# Cross-compile streamax-core for every target platform.
# Run individual sections from the machine that has the right toolchain.
set -euo pipefail

cd "$(dirname "$0")"

usage() {
    cat <<EOF
Usage: $0 <target>

Targets:
  linux        x86_64 Linux  (rustup default toolchain)
  ios          iOS device + simulator XCFramework  (macOS only, needs Xcode)
  android      Android 4 ABIs                       (needs Android NDK, cargo-ndk)
  windows      Windows x86_64                       (cross-compile from Linux OK)
  all          Everything the host can build
EOF
}

build_linux() {
    echo "==> linux x86_64"
    cargo build --release
    ls -lh target/release/libstreamax_core.{a,so}
}

build_ios() {
    echo "==> iOS XCFramework"
    for t in aarch64-apple-ios aarch64-apple-ios-sim x86_64-apple-ios; do
        rustup target add "$t" || true
        cargo build --release --target "$t" --lib
    done

    # Combine simulator slices (arm64-sim + x86_64-sim) into one fat lib
    mkdir -p target/ios-sim-fat
    lipo -create \
        target/aarch64-apple-ios-sim/release/libstreamax_core.a \
        target/x86_64-apple-ios/release/libstreamax_core.a \
        -output target/ios-sim-fat/libstreamax_core.a

    rm -rf target/StreamaxCore.xcframework
    xcodebuild -create-xcframework \
        -library target/aarch64-apple-ios/release/libstreamax_core.a \
        -headers include \
        -library target/ios-sim-fat/libstreamax_core.a \
        -headers include \
        -output target/StreamaxCore.xcframework
    echo "Output: target/StreamaxCore.xcframework"
}

build_android() {
    echo "==> Android (4 ABIs)"
    command -v cargo-ndk >/dev/null || { echo "Install cargo-ndk: cargo install cargo-ndk"; exit 1; }
    : "${ANDROID_NDK_HOME:?Set ANDROID_NDK_HOME}"

    cargo ndk \
        -t arm64-v8a \
        -t armeabi-v7a \
        -t x86_64 \
        -t x86 \
        -o target/android-jniLibs \
        build --release
    echo "Output: target/android-jniLibs/{arm64-v8a,armeabi-v7a,x86_64,x86}/libstreamax_core.so"
}

build_windows() {
    echo "==> Windows x86_64"
    rustup target add x86_64-pc-windows-gnu || true
    cargo build --release --target x86_64-pc-windows-gnu --lib
    ls -lh target/x86_64-pc-windows-gnu/release/streamax_core.dll
}

case "${1:-}" in
    linux)   build_linux ;;
    ios)     build_ios ;;
    android) build_android ;;
    windows) build_windows ;;
    all)
        build_linux
        case "$(uname -s)" in
            Darwin) build_ios ;;
            Linux)  build_windows; [ -n "${ANDROID_NDK_HOME:-}" ] && build_android ;;
        esac
        ;;
    *) usage; exit 1 ;;
esac
