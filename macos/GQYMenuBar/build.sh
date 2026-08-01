#!/bin/zsh
set -euo pipefail

project_dir="${0:A:h}"
app_dir="$project_dir/.build/顾清影.app"
contents_dir="$app_dir/Contents"
binary_dir="$contents_dir/MacOS"
resources_dir="$contents_dir/Resources"
module_cache="$project_dir/.build/module-cache"
repo_dir="${project_dir:h:h}"
icon_source="$repo_dir/pics/GQY-icon.png"

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

# 用 GQY-icon.png 生成标准 .icns（iconutil 需要 iconset 目录）
iconset_dir="$project_dir/.build/AppIcon.iconset"
rm -rf "$iconset_dir"
mkdir -p "$iconset_dir"
# 尺寸对：16/32/64/128/256/512/1024（含 @2x）
sips -z 16 16   "$icon_source" --out "$iconset_dir/icon_16x16.png"      >/dev/null
sips -z 32 32   "$icon_source" --out "$iconset_dir/icon_16x16@2x.png"   >/dev/null
sips -z 32 32   "$icon_source" --out "$iconset_dir/icon_32x32.png"      >/dev/null
sips -z 64 64   "$icon_source" --out "$iconset_dir/icon_32x32@2x.png"   >/dev/null
sips -z 128 128 "$icon_source" --out "$iconset_dir/icon_128x128.png"    >/dev/null
sips -z 256 256 "$icon_source" --out "$iconset_dir/icon_128x128@2x.png" >/dev/null
sips -z 256 256 "$icon_source" --out "$iconset_dir/icon_256x256.png"    >/dev/null
sips -z 512 512 "$icon_source" --out "$iconset_dir/icon_256x256@2x.png" >/dev/null
sips -z 512 512 "$icon_source" --out "$iconset_dir/icon_512x512.png"    >/dev/null
sips -z 1024 1024 "$icon_source" --out "$iconset_dir/icon_512x512@2x.png" >/dev/null
iconutil -c icns "$iconset_dir" -o "$resources_dir/AppIcon.icns"

# 版本号跟随 Cargo.toml
app_version="$(sed -n 's/^version = "\([0-9][0-9.]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -1)"
if [[ -n "$app_version" ]]; then
  /usr/libexec/PlistBuddy -c "Set :CFBundleShortVersionString $app_version" \
    -c "Set :CFBundleVersion $app_version" "$contents_dir/Info.plist"
fi

backend_bin="${GQY_BIN:-}"
if [[ -z "$backend_bin" && -x "$repo_dir/target/release/gqy" ]]; then
  backend_bin="$repo_dir/target/release/gqy"
fi
if [[ -z "$backend_bin" && -x "$repo_dir/target/debug/gqy" ]]; then
  backend_bin="$repo_dir/target/debug/gqy"
fi
if [[ -n "$backend_bin" ]]; then
  cp "$backend_bin" "$resources_dir/gqy"
fi

# 默认 ad-hoc 签名；设置 CODESIGN_IDENTITY 可用 Developer ID 正式签名
codesign --force --deep --sign "${CODESIGN_IDENTITY:--}" "$app_dir"

echo "$app_dir"
