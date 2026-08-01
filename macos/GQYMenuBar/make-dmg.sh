#!/bin/zsh
set -euo pipefail

# 把构建好的 顾清影.app 打包成可分发的 DMG。
# 用法：先运行 build.sh 产出 .app，再运行本脚本：
#   zsh macos/GQYMenuBar/build.sh
#   zsh macos/GQYMenuBar/make-dmg.sh
# 产物：macos/GQYMenuBar/.build/GQY-<版本>.dmg

project_dir="${0:A:h}"
app_dir="$project_dir/.build/顾清影.app"
staging_dir="$project_dir/.build/dmg-staging"
repo_dir="${project_dir:h:h}"
app_version="$(sed -n 's/^version = "\([0-9][0-9.]*\)"/\1/p' "$repo_dir/Cargo.toml" | head -1)"
dmg_path="$project_dir/.build/GQY-${app_version:-0.0.0}.dmg"

if [[ ! -d "$app_dir" ]]; then
  echo "找不到 $app_dir，请先运行 build.sh" >&2
  exit 1
fi

rm -rf "$staging_dir"
mkdir -p "$staging_dir"
cp -R "$app_dir" "$staging_dir/"
# Applications 快捷方式：用户挂载 DMG 后把 app 拖进去即可安装
ln -s /Applications "$staging_dir/Applications"

rm -f "$dmg_path"
hdiutil create \
  -volname "顾清影" \
  -srcfolder "$staging_dir" \
  -ov \
  -format UDZO \
  -fs HFS+ \
  "$dmg_path" >/dev/null

rm -rf "$staging_dir"
echo "$dmg_path"
