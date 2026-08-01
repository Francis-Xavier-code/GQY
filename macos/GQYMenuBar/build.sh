#!/bin/zsh
set -euo pipefail

project_dir="${0:A:h}"
app_dir="$project_dir/.build/顾清影.app"
contents_dir="$app_dir/Contents"
binary_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
module_cache="$project_dir/.build/module-cache"
repo_dir="${project_dir:h:h}"

mkdir -p "$binary_dir" "$resources_dir" "$module_cache"
xcrun clang \
  -fobjc-arc \
  -fmodules \
  -fmodules-cache-path="$module_cache" \
  -framework AppKit \
  -framework Foundation \
  -mmacosx-version-min=13.0 \
  "$project_dir/main.m" \
  -o "$binary_dir/GQYMenuBar"
cp "$project_dir/Info.plist" "$contents_dir/Info.plist"

backend_bin="${GQY_BIN:-}"
if [[ -z "$backend_bin" && -x "$repo_dir/target/release/miyu" ]]; then
  backend_bin="$repo_dir/target/release/miyu"
fi
if [[ -z "$backend_bin" && -x "$repo_dir/target/debug/miyu" ]]; then
  backend_bin="$repo_dir/target/debug/miyu"
fi
if [[ -n "$backend_bin" ]]; then
  cp "$backend_bin" "$resources_dir/miyu"
fi

codesign --force --deep --sign - "$app_dir"

echo "$app_dir"
