#!/usr/bin/env bash
set -euo pipefail

pkgname=miyu
pkgver=0.1.0
pkgrel="${1:-5}"
arch=x86_64
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
pkgdir="${TMPDIR:-/tmp}/miyu-pkg-${pkgver}-${pkgrel}"
pkgout="${MIYU_PACKAGE_OUT_DIR:-${XDG_CACHE_HOME:-${HOME}/.cache}/miyu/packages}"
pkgfile="${pkgout}/${pkgname}-${pkgver}-${pkgrel}-${arch}.pkg.tar.zst"
default_kb_dir="${pkgdir}/usr/share/miyu/default-kb"
memes_dir="${pkgdir}/usr/share/miyu/memes"
shorin_guide_repo="${SHORIN_GUIDE_SOURCE:-/home/shorin/Documents/github/ShorinArchExperience-ArchlinuxGuide}"
shorin_wiki_source="${SHORIN_WIKI_SOURCE:-${shorin_guide_repo}/wiki}"

mkdir -p "${pkgout}"
rm -rf "${pkgdir}" "${pkgfile}"
mkdir -p "${pkgdir}/usr/bin" "${default_kb_dir}" "${memes_dir}"
install -Dm755 "${root}/target/release/miyu" "${pkgdir}/usr/bin/miyu"

copy_markdown_tree() {
    local source="$1"
    local dest="$2"
    [[ -d "${source}" ]] || return 0
    while IFS= read -r -d '' file; do
        rel="${file#"${source}/"}"
        case "/${rel}" in
            */.git/*|*/pictures/*|*/legacy/*|*/Legacy/*|*/lagacy/*|*/Lagacy/*|*/Wikis/*) continue ;;
        esac
        install -Dm644 "${file}" "${dest}/${rel}"
    done < <(find "${source}" -type f -name '*.md' -print0 | sort -z)
}

copy_markdown_tree "${root}/kb" "${default_kb_dir}/kb"
copy_markdown_tree "${shorin_wiki_source}" "${default_kb_dir}/shorinwiki"
if [[ -d "${root}/src/memes" ]]; then
    while IFS= read -r -d '' file; do
        rel="${file#"${root}/src/memes/"}"
        install -Dm644 "${file}" "${memes_dir}/${rel}"
    done < <(find "${root}/src/memes" -type f \( -name '*.json' -o -name '*.jpg' -o -name '*.jpeg' -o -name '*.png' -o -name '*.gif' -o -name '*.webp' \) -print0 | sort -z)
fi

mkdir -p "${default_kb_dir}/manifest"
cat > "${default_kb_dir}/manifest/manifest.json" <<EOF
{
  "name": "miyu-default-kb",
  "generated_by": "scripts/package-arch.sh"
}
EOF
if [[ -d "${shorin_guide_repo}/.git" ]]; then
    git -C "${shorin_guide_repo}" rev-parse HEAD > "${default_kb_dir}/manifest/shorinwiki.commit"
elif [[ -d "${shorin_wiki_source}/.git" ]]; then
    git -C "${shorin_wiki_source}" rev-parse HEAD > "${default_kb_dir}/manifest/shorinwiki.commit"
else
    : > "${default_kb_dir}/manifest/shorinwiki.commit"
fi

size="$(du -sb "${pkgdir}/usr" | cut -f1)"
cat > "${pkgdir}/.PKGINFO" <<EOF
pkgname = ${pkgname}
pkgbase = ${pkgname}
pkgver = ${pkgver}-${pkgrel}
pkgdesc = Miyu command-line AI assistant
url = https://github.com/SHORiN-KiWATA/Miyu
builddate = $(date +%s)
packager = Miyu Release <noreply@example.com>
size = ${size}
arch = ${arch}
license = MIT
depend = gcc-libs
depend = glibc
depend = alsa-lib
depend = chafa
depend = ripgrep
optdepend = git: update default Shorin Wiki knowledge base
optdepend = fish: fish shell integration support
optdepend = bash: bash shell integration support
optdepend = zsh: zsh shell integration support
EOF

bsdtar --zstd -cf "${pkgfile}" -C "${pkgdir}" .PKGINFO usr
echo "${pkgfile}"
