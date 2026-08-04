# Homebrew formula: gqy (CLI)
#
# 用法（发布 tag 后）：
#   brew tap GQYTeam/GQY
#   brew install gqy
#
# 发布流程：
#   1. git tag v0.4.5 && git push origin v0.4.5
#   2. 计算源码 tarball 的 sha256：
#        curl -Ls https://github.com/GQYTeam/GQY/archive/refs/tags/v0.4.5.tar.gz | shasum -a 256
#   3. 把结果填入下面 sha256 并提交本文件
#   4. 同步到 homebrew-GQY tap 仓库
#   5. brew install gqy 验证
class Gqy < Formula
  desc "顾清影 —— 活在终端与菜单栏里的 AI 助理"
  homepage "https://github.com/GQYTeam/GQY"
  url "https://github.com/GQYTeam/GQY/archive/refs/tags/v0.8.6.tar.gz"
  sha256 "94a1bc6235c6ef6fb069484f20ab241ea21d29d09260d1e770b817ffb6ec65e8"
  license "GPL-3.0"

  depends_on "rust" => :build
  # 终端图片显示依赖 chafa；不需要图片功能时可移除
  depends_on "chafa"

  def install
    system "cargo", "install", *std_cargo_args
    # 只读共享资源统一装进 $(brew --prefix)/share/gqy 一个目录：
    # scripts（脚本工具）、memes（内置表情库）、kb（知识库源）、
    # bridges（napcat/tg 桥接脚本，gqy napcat / gqy tg 管理）。
    # 运行时从可执行文件位置自动解析该目录。
    pkgshare.install "src/scripts"
    pkgshare.install "src/memes"
    pkgshare.install "kb"
    pkgshare.install "communication" => "bridges"
    # 菜单栏壳源码（gqy menubar --install 用 clang 现场编译）
    pkgshare.install "macos/GQYMenuBar" => "menubar"
    pkgshare.install "pics/GQY-icon.png"
  end

  test do
    assert_match "gqy", shell_output("#{bin}/gqy --version")
  end
end
