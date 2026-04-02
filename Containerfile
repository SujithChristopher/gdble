# Containerfile for Android GDExtension Build
FROM rust:latest

# Install build dependencies
RUN apt-get update && apt-get install -y \
    python3 \
    cmake \
    ninja-build \
    unzip \
    curl \
    git \
    pkg-config \
    libdbus-1-dev \
    libudev-dev \
    && rm -rf /var/lib/apt/lists/*

# Android NDK version and URL
ENV NDK_VERSION=r25c
ENV NDK_URL=https://dl.google.com/android/repository/android-ndk-${NDK_VERSION}-linux.zip

# Download and install NDK
RUN curl -L ${NDK_URL} -o ndk.zip && \
    unzip ndk.zip && \
    mv android-ndk-${NDK_VERSION} /opt/android-ndk && \
    rm ndk.zip

ENV ANDROID_NDK_ROOT=/opt/android-ndk
ENV PATH=$PATH:$ANDROID_NDK_ROOT/toolchains/llvm/prebuilt/linux-x86_64/bin

# Install Rust Android target
RUN rustup target add aarch64-linux-android

# Set up environment variables for Rust cross-compilation
ENV CC_aarch64_linux_android=aarch64-linux-android31-clang
ENV CXX_aarch64_linux_android=aarch64-linux-android31-clang++
ENV AR_aarch64_linux_android=llvm-ar
ENV CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER=aarch64-linux-android31-clang

# Force CMake to recognize Android and skip Linux-specific checks (e.g. dbus)
ENV CMAKE_TOOLCHAIN_FILE_aarch64_linux_android=/opt/android-ndk/build/cmake/android.toolchain.cmake
ENV ANDROID_ABI=arm64-v8a
ENV ANDROID_PLATFORM=android-31
ENV CMAKE_SYSTEM_NAME=Android
ENV CMAKE_SYSTEM_VERSION=31
ENV ANDROID=1
ENV SIMPLEBLE_BACKEND=Android  

WORKDIR /workspace
