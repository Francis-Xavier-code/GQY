# Homebrew cask: gqy (menu bar app)
#
# 用法（发布 tag + 上传 dmg 后）：
#   brew tap Francis-Xavier-code/GQY
#   brew install --cask gqy
#
# 发布流程：
#   1. zsh macos/GQYMenuBar/build.sh && zsh macos/GQYMenuBar/make-dmg.sh
#   2. 上传 macos/GQYMenuBar/.build/GQY-0.4.0.dmg 到 GitHub Release v0.4.0 资产
#   3. 计算 dmg 的 sha256：
#        shasum -a 256 macos/GQYMenuBar/.build/GQY-0.4.0.dmg
#   4. 把结果填入下面 sha256 并提交本文件
cask "gqy" do
  version "0.4.1"
  sha256 "b0743523d1e1c4936cf18c7a38f814a1ee43fe595d73815f0021f7eaf7b355bc"

  url "https://github.com/Francis-Xavier-code/GQY/releases/download/v#{version}/GQY-#{version}.dmg"
  name "顾清影"
  desc "活在终端与菜单栏里的 AI 助理（菜单栏入口）"
  homepage "https://github.com/Francis-Xavier-code/GQY"
  license "GPL-3.0"

  app "顾清影.app"

  zap trash: [
    "~/Library/Application Support/GQY",
    "~/Library/LaunchAgents/dev.gqy.menubar.plist",
  ]
end
