#!/usr/bin/env bash
# GQY (顾清影) 安装脚本
# 用法: curl -fsSL https://raw.githubusercontent.com/GQYTeam/GQY/main/install.sh | bash
#
# 支持: macOS (Apple Silicon / Intel)、Linux (x86_64 / aarch64)
# 安装位置: ~/.local/bin/gqy + ~/.local/share/gqy/ (用户级)
#           或 /usr/local/bin/gqy + /usr/local/share/gqy/ (系统级)

set -euo pipefail

REPO="GQYTeam/GQY"
BINARY="gqy"
VERSION="${GQY_VERSION:-latest}"

# 颜色
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

info()  { echo -e "${CYAN}[info]${NC} $*"; }
ok()    { echo -e "${GREEN}  ✓${NC} $*"; }
warn()  { echo -e "${YELLOW}[warn]${NC} $*"; }
error() { echo -e "${RED}[error]${NC} $*" >&2; exit 1; }

# 旋转动画
spinner() {
    local pid=$1
    local delay=0.1
    local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
    local i=0
    while kill -0 "$pid" 2>/dev/null; do
        echo -ne "\r${CYAN}${frames[$i]}${NC} $2"
        i=$(( (i + 1) % ${#frames[@]} ))
        sleep "$delay"
    done
    echo -ne "\r\033[K"
}

# 带进度的下载（显示大小）
download_with_progress() {
    local url="$1"
    local output="$2"
    local label="$3"

    # 先获取文件大小
    local total_size
    total_size=$(curl -sI "$url" 2>/dev/null | grep -i content-length | awk '{print $2}' | tr -d '\r')
    local total_display=""
    if [ -n "$total_size" ] && [ "$total_size" -gt 0 ] 2>/dev/null; then
        if [ "$total_size" -gt 1048576 ]; then
            total_display=$(printf "%.1fMB" "$(echo "scale=1; $total_size / 1048576" | bc 2>/dev/null || echo "?")")
        elif [ "$total_size" -gt 1024 ]; then
            total_display=$(printf "%.0fKB" "$(echo "scale=0; $total_size / 1024" | bc 2>/dev/null || echo "?")")
        else
            total_display="${total_size}B"
        fi
    fi

    # 下载（带 curl 进度条）
    echo -ne "  ${DIM}↓${NC} ${label}"
    if [ -n "$total_display" ]; then
        echo -ne " ${DIM}(${total_display})${NC}"
    fi
    echo -ne " ... "

    if curl -fL --progress-bar "$url" -o "$output" 2>/dev/null; then
        local file_size
        file_size=$(wc -c < "$output" 2>/dev/null | tr -d ' ')
        if [ "$file_size" -gt 1048576 ]; then
            echo -e "${GREEN}✓${NC} ${DIM}$(printf "%.1fMB" "$(echo "scale=1; $file_size / 1048576" | bc 2>/dev/null || echo "?")")${NC}"
        elif [ "$file_size" -gt 1024 ]; then
            echo -e "${GREEN}✓${NC} ${DIM}$(printf "%.0fKB" "$(echo "scale=0; $file_size / 1024" | bc 2>/dev/null || echo "?")")${NC}"
        else
            echo -e "${GREEN}✓${NC} ${DIM}${file_size}B${NC}"
        fi
        return 0
    else
        echo -e "${RED}✗${NC}"
        return 1
    fi
}

# 检测平台
detect_platform() {
    local os arch
    os="$(uname -s)"
    arch="$(uname -m)"

    case "$os" in
        Darwin)
            case "$arch" in
                arm64)  echo "aarch64-apple-darwin" ;;
                x86_64) echo "x86_64-apple-darwin" ;;
                *)      error "不支持的架构: $arch" ;;
            esac
            ;;
        Linux)
            case "$arch" in
                x86_64)  echo "x86_64-unknown-linux-gnu" ;;
                aarch64) echo "aarch64-unknown-linux-gnu" ;;
                *)       error "不支持的架构: $arch" ;;
            esac
            ;;
        *)
            error "不支持的操作系统: $os (仅支持 macOS 和 Linux)"
            ;;
    esac
}

# 获取最新版本
get_latest_version() {
    local url="https://api.github.com/repos/$REPO/releases/latest"
    local version
    version=$(curl -fsSL "$url" 2>/dev/null | grep '"tag_name"' | head -1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    if [ -z "$version" ]; then
        error "无法获取最新版本，请手动指定: GQY_VERSION=v0.8.6 bash install.sh"
    fi
    echo "$version"
}

# 检查依赖
check_deps() {
    local missing=()
    for cmd in curl tar; do
        if ! command -v "$cmd" &>/dev/null; then
            missing+=("$cmd")
        fi
    done
    if [ ${#missing[@]} -gt 0 ]; then
        error "缺少依赖: ${missing[*]}"
    fi
}

# 确定安装前缀 (prefix)
# 二进制 -> $prefix/bin/gqy
# 资源   -> $prefix/share/gqy/{scripts,memes,kb,...}
# resolve_share_base() 从 exe 位置向上查找 share/gqy，这样就能自动发现
choose_install_prefix() {
    # 用户级 (无需 sudo)
    if [ -d "$HOME/.local" ] || [ -n "${XDG_HOME:-}" ]; then
        echo "${XDG_HOME:-$HOME/.local}"
        return
    fi

    # 系统级 (需要 sudo)
    if [ -w /usr/local/bin ] && [ -w /usr/local/share 2>/dev/null ]; then
        echo "/usr/local"
        return
    fi

    # 创建 ~/.local
    mkdir -p "$HOME/.local"
    echo "$HOME/.local"
}

# 下载并安装二进制
install_binary() {
    local platform="$1"
    local version="$2"
    local prefix="$3"

    local archive_name="${BINARY}-${platform}.tar.gz"
    local download_url="https://github.com/$REPO/releases/download/$version/$archive_name"

    echo -e "\n${BOLD}▸ 安装二进制${NC}"
    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap "rm -rf '$tmp_dir'" EXIT

    if ! download_with_progress "$download_url" "$tmp_dir/$archive_name" "$BINARY $version ($platform)"; then
        error "下载失败，请检查网络连接或手动下载:\n  https://github.com/$REPO/releases"
    fi

    echo -ne "  ${DIM}↓${NC} 解压中 ... "
    tar -xzf "$tmp_dir/$archive_name" -C "$tmp_dir"
    echo -e "${GREEN}✓${NC}"

    local binary_path="$tmp_dir/$BINARY"
    if [ ! -f "$binary_path" ]; then
        error "压缩包中未找到 $BINARY 二进制"
    fi

    chmod +x "$binary_path"

    local bin_dir="$prefix/bin"
    echo -ne "  ${DIM}↓${NC} 安装到 $bin_dir ... "
    if [ ! -w "$bin_dir" ] && [ ! -w "$prefix" ]; then
        sudo mkdir -p "$bin_dir"
        sudo mv "$binary_path" "$bin_dir/$BINARY"
    else
        mkdir -p "$bin_dir"
        mv "$binary_path" "$bin_dir/$BINARY"
    fi
    echo -e "${GREEN}✓${NC}"
}

# 安装共享资源
install_share_resources() {
    local version="$1"
    local prefix="$2"

    local share_dir="$prefix/share/gqy"

    echo -e "\n${BOLD}▸ 安装共享资源${NC}"

    local tmp_dir
    tmp_dir=$(mktemp -d)

    local tarball_url="https://github.com/$REPO/archive/refs/tags/$version.tar.gz"
    if ! download_with_progress "$tarball_url" "$tmp_dir/src.tar.gz" "资源包"; then
        warn "共享资源下载失败，部分功能可能不可用"
        warn "可稍后手动安装: gqy kb add <目录>"
        rm -rf "$tmp_dir"
        return
    fi

    echo -ne "  ${DIM}↓${NC} 解压资源 ... "
    local src_dir="$tmp_dir/src"
    mkdir -p "$src_dir"
    tar -xzf "$tmp_dir/src.tar.gz" -C "$src_dir" --strip-components=1
    echo -e "${GREEN}✓${NC}"

    # 确定目标目录可写
    local need_sudo=""
    if [ ! -w "$(dirname "$share_dir")" ] 2>/dev/null; then
        need_sudo="sudo"
    fi

    $need_sudo mkdir -p "$share_dir"

    echo -ne "  ${DIM}↓${NC} 复制到 $share_dir ... "
    local extracted="$src_dir"
    for item in src/scripts src/memes; do
        if [ -d "$extracted/$item" ]; then
            $need_sudo cp -r "$extracted/$item" "$share_dir/${item#src/}"
        fi
    done

    for item in kb communication; do
        if [ -d "$extracted/$item" ]; then
            local name
            case "$item" in
                kb) name="kb" ;;
                communication) name="bridges" ;;
            esac
            $need_sudo cp -r "$extracted/$item" "$share_dir/$name"
        fi
    done

    if [ -f "$extracted/pics/GQY-icon.png" ]; then
        $need_sudo cp "$extracted/pics/GQY-icon.png" "$share_dir/"
    fi
    echo -e "${GREEN}✓${NC}"

    rm -rf "$tmp_dir"
}

# 检查 PATH
check_path() {
    local bin_dir="$1"
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$bin_dir"; then
        warn "$bin_dir 不在 PATH 中"
        echo ""
        echo "  请将以下内容添加到 ~/.zshrc 或 ~/.bashrc:"
        echo ""
        echo "    export PATH=\"$bin_dir:\$PATH\""
        echo ""
    fi
}

# 提示后续步骤
post_install() {
    local prefix="$1"
    local bin_dir="$prefix/bin"

    echo ""
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo -e "${GREEN}  ✓ 顾清影 (GQY) 安装完成！${NC}"
    echo -e "${GREEN}━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━${NC}"
    echo ""
    echo -e "  ${BOLD}快速开始:${NC}"
    echo "    gqy                    # 进入对话"
    echo "    gqy config             # 配置 TUI"
    echo "    gqy web                # 启动 Web 面板"
    echo ""
    echo -e "  ${BOLD}可选:${NC}"
    echo "    gqy zsh-init           # 安装 Shell 集成"
    echo "    gqy kb add $prefix/share/gqy/kb  # 导入知识库"
    echo ""
    echo -e "  ${DIM}资源目录: $prefix/share/gqy/${NC}"
    echo -e "  ${DIM}文档: https://github.com/$REPO${NC}"
    echo ""
}

# 主流程
main() {
    echo ""
    echo -e "${CYAN}${BOLD}  顾清影 (GQY) 安装程序${NC}"
    echo -e "${DIM}  ────────────────────────────────${NC}"
    echo ""

    check_deps

    local platform
    platform=$(detect_platform)
    echo -e "  ${DIM}平台${NC}    $platform"

    local version
    if [ "$VERSION" = "latest" ]; then
        version=$(get_latest_version)
    else
        version="$VERSION"
    fi
    echo -e "  ${DIM}版本${NC}    $version"

    local prefix
    prefix=$(choose_install_prefix)
    echo -e "  ${DIM}安装到${NC}  $prefix"

    # 安装二进制
    install_binary "$platform" "$version" "$prefix"

    # 安装共享资源
    install_share_resources "$version" "$prefix"

    # 检查 PATH
    check_path "$prefix/bin"

    # 提示后续步骤
    post_install "$prefix"
}

main "$@"
