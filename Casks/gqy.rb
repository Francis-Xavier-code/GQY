# Homebrew cask: gqy (menu bar app)
#
# 用法（发布 tag + 上传 dmg 后）：
#   brew tap Francis-Xavier-code/GQY
#   brew install --cask gqy
#
# 发布流程：
#   1. zsh macos/GQYMenuBar/build.sh && zsh macos/GQYMenuBar/make-dmg.sh
#   2. gh release create v0.4.3 macos/GQYMenuBar/.build/GQY-0.4.3.dmg
#   3. 计算 dmg 的 sha256：
#        shasum -a 256 macos/GQYMenuBar/.build/GQY-0.4.3.dmg
#   4. 把结果填入下面 sha256 并提交本文件
#   5. 同步到 homebrew-GQY tap 仓库
cask "gqy" do
  version "0.8.4"
  sha256 "c86902e0e56bbfe204067df963351d626ccd9fc5ad94c2b1d4554a9b23dcec14"

  url "https://github.com/Francis-Xavier-code/GQY/releases/download/v#{version}/GQY-#{version}.dmg"
  name "顾清影"
  desc "活在终端与菜单栏里的 AI 助理（菜单栏入口）"
  homepage "https://github.com/Francis-Xavier-code/GQY"

  app "顾清影.app"

  # ad-hoc 签名未公证：安装后移除 quarantine，避免 Gatekeeper 静默拦截启动
  postflight do
    system_command "xattr",
                   args: ["-dr", "com.apple.quarantine", "#{appdir}/顾清影.app"],
                   sudo: false
  end

  # 卸载只清自启项；GQY_HOME（对话/记忆/知识库/备份仓库）是用户数据，绝不随卸载删除
  zap trash: [
    "~/Library/LaunchAgents/dev.gqy.menubar.plist",
  ]
end
