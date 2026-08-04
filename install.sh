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

# ── 颜色定义 ──────────────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
CYAN='\033[0;36m'
MAGENTA='\033[0;35m'
DIM='\033[2m'
BOLD='\033[1m'
NC='\033[0m'

# ── 符号定义 ──────────────────────────────────────────────────────────────────
ARROW="${CYAN}========>${NC}"
ARROW_LIGHT="${DIM}-------->${NC}"
CHECK="${GREEN}✓${NC}"
CROSS="${RED}✗${NC}"
WARN="${YELLOW}!${NC}"
INFO="${BLUE}i${NC}"
STAR="${MAGENTA}★${NC}"

# ── 输出函数 ──────────────────────────────────────────────────────────────────
info()  { echo -e "  ${ARROW} $*"; }
ok()    { echo -e "  ${CHECK} ${GREEN}$*${NC}"; }
warn()  { echo -e "  ${WARN} ${YELLOW}$*${NC}"; }
error() { echo -e "  ${CROSS} ${RED}$*${NC}" >&2; exit 1; }
step()  { echo -e "\n${BOLD}$1${NC}"; }
arrow() { echo -e "  ${ARROW_LIGHT} $*"; }

# ── 进度条 ────────────────────────────────────────────────────────────────────
# 显示一个漂亮的进度条
# 用法: progress_bar <当前> <总计> <宽度>
progress_bar() {
    local current=$1
    local total=$2
    local width=${3:-40}
    local percent=0
    local filled=0
    local empty=0

    if [ "$total" -gt 0 ]; then
        percent=$((current * 100 / total))
        filled=$((current * width / total))
        empty=$((width - filled))
    fi

    # 构建进度条
    local bar="${GREEN}"
    for ((i = 0; i < filled; i++)); do
        bar+="█"
    done
    for ((i = 0; i < empty; i++)); do
        bar+="${DIM}░${NC}"
    done

    # 显示进度条
    printf "\r  ${ARROW_LIGHT} [%b${NC}] %3d%%" "$bar" "$percent"
}

# ── 旋转动画 ──────────────────────────────────────────────────────────────────
spinner() {
    local pid=$1
    local delay=0.1
    local frames=('⠋' '⠙' '⠹' '⠸' '⠼' '⠴' '⠦' '⠧' '⠇' '⠏')
    local i=0
    while kill -0 "$pid" 2>/dev/null; do
        echo -ne "\r  ${CYAN}${frames[$i]}${NC} $2"
        i=$(( (i + 1) % ${#frames[@]} ))
        sleep "$delay"
    done
    echo -ne "\r\033[K"
}

# ── 带进度的下载 ──────────────────────────────────────────────────────────────
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

    # 显示下载信息
    echo -e "  ${ARROW_LIGHT} ${DIM}下载${NC} ${BOLD}$label${NC}"
    if [ -n "$total_display" ]; then
        echo -e "  ${ARROW_LIGHT} ${DIM}大小${NC} ${total_display}"
    fi

    # 下载（带 curl 进度条）
    if curl -fL --progress-bar "$url" -o "$output" 2>/dev/null; then
        local file_size
        file_size=$(wc -c < "$output" 2>/dev/null | tr -d ' ')
        local size_display=""
        if [ "$file_size" -gt 1048576 ]; then
            size_display=$(printf "%.1fMB" "$(echo "scale=1; $file_size / 1048576" | bc 2>/dev/null || echo "?")")
        elif [ "$file_size" -gt 1024 ]; then
            size_display=$(printf "%.0fKB" "$(echo "scale=0; $file_size / 1024" | bc 2>/dev/null || echo "?")")
        else
            size_display="${file_size}B"
        fi
        echo -e "  ${CHECK} ${GREEN}下载完成${NC} ${DIM}(${size_display})${NC}"
        return 0
    else
        echo -e "  ${CROSS} ${RED}下载失败${NC}"
        return 1
    fi
}

# ── 平台检测 ──────────────────────────────────────────────────────────────────
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

# ── 获取最新版本 ──────────────────────────────────────────────────────────────
get_latest_version() {
    local url="https://api.github.com/repos/$REPO/releases/latest"
    local version
    version=$(curl -fsSL "$url" 2>/dev/null | grep '"tag_name"' | head -1 | sed -E 's/.*"tag_name": *"([^"]+)".*/\1/')
    if [ -z "$version" ]; then
        error "无法获取最新版本，请手动指定: GQY_VERSION=v0.8.7 bash install.sh"
    fi
    echo "$version"
}

# ── 检查依赖 ──────────────────────────────────────────────────────────────────
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

# ── 确定安装前缀 ──────────────────────────────────────────────────────────────
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

# ── 安装二进制 ────────────────────────────────────────────────────────────────
install_binary() {
    local platform="$1"
    local version="$2"
    local prefix="$3"

    local archive_name="${BINARY}-${platform}.tar.gz"
    local download_url="https://github.com/$REPO/releases/download/$version/$archive_name"

    step "▸ 安装二进制"
    echo ""

    local tmp_dir
    tmp_dir=$(mktemp -d)
    trap "rm -rf '$tmp_dir'" EXIT

    if ! download_with_progress "$download_url" "$tmp_dir/$archive_name" "$BINARY $version ($platform)"; then
        error "下载失败，请检查网络连接或手动下载:\n  https://github.com/$REPO/releases"
    fi

    echo ""
    arrow "解压中 ..."
    tar -xzf "$tmp_dir/$archive_name" -C "$tmp_dir"
    ok "解压完成"

    local binary_path="$tmp_dir/$BINARY"
    if [ ! -f "$binary_path" ]; then
        error "压缩包中未找到 $BINARY 二进制"
    fi

    chmod +x "$binary_path"

    local bin_dir="$prefix/bin"
    arrow "安装到 ${BOLD}$bin_dir${NC} ..."
    if [ ! -w "$bin_dir" ] && [ ! -w "$prefix" ]; then
        sudo mkdir -p "$bin_dir"
        sudo mv "$binary_path" "$bin_dir/$BINARY"
    else
        mkdir -p "$bin_dir"
        mv "$binary_path" "$bin_dir/$BINARY"
    fi
    ok "二进制安装完成"
}

# ── 安装共享资源 ──────────────────────────────────────────────────────────────
install_share_resources() {
    local version="$1"
    local prefix="$2"

    local share_dir="$prefix/share/gqy"

    step "▸ 安装共享资源"
    echo ""

    local tmp_dir
    tmp_dir=$(mktemp -d)

    local tarball_url="https://github.com/$REPO/archive/refs/tags/$version.tar.gz"
    if ! download_with_progress "$tarball_url" "$tmp_dir/src.tar.gz" "资源包"; then
        warn "共享资源下载失败，部分功能可能不可用"
        warn "可稍后手动安装: gqy kb add <目录>"
        rm -rf "$tmp_dir"
        return
    fi

    echo ""
    arrow "解压资源 ..."
    local src_dir="$tmp_dir/src"
    mkdir -p "$src_dir"
    tar -xzf "$tmp_dir/src.tar.gz" -C "$src_dir" --strip-components=1
    ok "解压完成"

    # 确定目标目录可写
    local need_sudo=""
    if [ ! -w "$(dirname "$share_dir")" ] 2>/dev/null; then
        need_sudo="sudo"
    fi

    $need_sudo mkdir -p "$share_dir"

    arrow "复制到 ${BOLD}$share_dir${NC} ..."
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
    ok "共享资源安装完成"

    rm -rf "$tmp_dir"
}

# ── 检查 PATH ─────────────────────────────────────────────────────────────────
check_path() {
    local bin_dir="$1"
    if ! echo "$PATH" | tr ':' '\n' | grep -qx "$bin_dir"; then
        echo ""
        warn "${BOLD}$bin_dir${NC} 不在 PATH 中"
        echo ""
        echo -e "  ${DIM}请将以下内容添加到 ~/.zshrc 或 ~/.bashrc:${NC}"
        echo ""
        echo -e "    ${CYAN}export PATH=\"$bin_dir:\$PATH\"${NC}"
        echo ""
    fi
}

# ── 安装成功提示 ──────────────────────────────────────────────────────────────
post_install() {
    local prefix="$1"
    local bin_dir="$prefix/bin"

    echo ""
    echo -e "${GREEN}╔══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${GREEN}║${NC}  ${STAR} ${BOLD}顾清影 (GQY) 安装完成！${NC}                              ${GREEN}║${NC}"
    echo -e "${GREEN}╚══════════════════════════════════════════════════════════════╝${NC}"
    echo ""
    echo -e "  ${BOLD}快速开始:${NC}"
    echo ""
    echo -e "    ${ARROW} ${CYAN}gqy${NC}                    # 进入对话"
    echo -e "    ${ARROW} ${CYAN}gqy config${NC}             # 配置 TUI"
    echo -e "    ${ARROW} ${CYAN}gqy web${NC}                # 启动 Web 面板"
    echo ""
    echo -e "  ${BOLD}可选:${NC}"
    echo ""
    echo -e "    ${ARROW_LIGHT} ${DIM}gqy zsh-init${NC}           # 安装 Shell 集成"
    echo -e "    ${ARROW_LIGHT} ${DIM}gqy kb add $prefix/share/gqy/kb${NC}  # 导入知识库"
    echo ""
    echo -e "  ${DIM}资源目录: $prefix/share/gqy/${NC}"
    echo -e "  ${DIM}文档: https://github.com/$REPO${NC}"
    echo ""
}

# ── 主流程 ────────────────────────────────────────────────────────────────────
main() {
    echo ""
    echo -e "${CYAN}╔══════════════════════════════════════════════════════════════╗${NC}"
    echo -e "${CYAN}║${NC}  ${BOLD}顾清影 (GQY) 安装程序${NC}                                  ${CYAN}║${NC}"
    echo -e "${CYAN}╚══════════════════════════════════════════════════════════════╝${NC}"
    echo ""

    check_deps

    local platform
    platform=$(detect_platform)
    echo -e "  ${INFO} ${DIM}平台${NC}    ${BOLD}$platform${NC}"

    local version
    if [ "$VERSION" = "latest" ]; then
        version=$(get_latest_version)
    else
        version="$VERSION"
    fi
    echo -e "  ${INFO} ${DIM}版本${NC}    ${BOLD}$version${NC}"

    local prefix
    prefix=$(choose_install_prefix)
    echo -e "  ${INFO} ${DIM}安装到${NC}  ${BOLD}$prefix${NC}"

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
