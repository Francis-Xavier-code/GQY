# Homebrew formula: gqy (CLI)
#
# 用法（发布 tag 后）：
#   brew tap Francis-Xavier-code/GQY
#   brew install gqy
#
# 发布流程：
#   1. git tag v0.4.0 && git push origin v0.4.0
#   2. 计算源码 tarball 的 sha256：
#        curl -Ls https://github.com/Francis-Xavier-code/GQY/archive/refs/tags/v0.4.0.tar.gz | shasum -a 256
#   3. 把结果填入下面 sha256 并提交本文件
#   4. brew install gqy 验证
class Gqy < Formula
  desc "顾清影 —— 活在终端与菜单栏里的 AI 助理"
  homepage "https://github.com/Francis-Xavier-code/GQY"
  url "https://github.com/Francis-Xavier-code/GQY/archive/refs/tags/v0.4.0.tar.gz"
  sha256 "" # TODO: 发布 v0.4.0 后填入 tarball sha256
  license "GPL-3.0"

  depends_on "rust" => :build
  # 终端图片显示依赖 chafa；不需要图片功能时可移除
  depends_on "chafa"

  def install
    system "cargo", "build", "--release", "--locked"
    bin.install "target/release/gqy"
  end

  test do
    assert_match "gqy", shell_output("#{bin}/gqy --version")
  end
end
