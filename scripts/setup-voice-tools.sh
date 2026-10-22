#!/usr/bin/env bash
# Downloads the CMake that whisper.cpp needs at build time into src-tauri/.tools.
set -euo pipefail
cd "$(dirname "$0")/../src-tauri"
[ -x .tools/cmake/bin/cmake ] && { echo "cmake already present"; exit 0; }
mkdir -p .tools
curl -sL https://github.com/Kitware/CMake/releases/download/v3.31.6/cmake-3.31.6-linux-x86_64.tar.gz | tar xz -C .tools
mv .tools/cmake-3.31.6-linux-x86_64 .tools/cmake
echo "cmake ready: $(.tools/cmake/bin/cmake --version | head -1)"
