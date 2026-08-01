use crate::i18n::text as t;
use crate::paths::GqyPaths;
use anyhow::Result;
use std::io::Write;
use std::path::Path;

const BEGIN_MARKER: &str = "# >>> gqy zsh hook >>>";
const END_MARKER: &str = "# <<< gqy zsh hook <<<";

pub fn hook() -> &'static str {
    r#"# 拦截「命令未找到」：明显是命令/拼写错误 → 交给系统报错；自然语言 → 交给 GQY。
command_not_found_handler() {
    [[ -o interactive ]] || return 127

    local text="$*"
    [[ -n "$text" ]] || return 127
    [[ "$text" != *$'\n'* && "$text" != *$'\r'* ]] || return 127

    # 明显是命令（含路径/内置命令/环境变量前缀）→ 系统报错，不打扰 GQY
    gqy --shell-classify --shell zsh -- "$@" 2>/dev/null && return 127

    gqy --shell-intercept --shell zsh -- "$@" 2>/dev/null
    return 127
}

# ---- 多行自然语言：Enter 时整块交给 GQY（与 fish 的拦截策略一致） ----
__gqy_pending_buffer=""

# 缓冲内是否含「未知命令」行：有则视为自然语言块，交给 GQY
__gqy_zsh_multiline_has_unknown() {
    local buffer="$1" line token kind
    while IFS= read -r line; do
        line="${line#"${line%%[![:space:]]*}"}"
        [[ -n "$line" ]] || continue
        [[ "$line" != \#* ]] || continue
        # 跳过 env 赋值前缀（VAR=... cmd）
        while [[ "$line" =~ ^[A-Za-z_][A-Za-z0-9_]*= ]]; do
            line="${line#*=}"
            line="${line#"${line%%[![:space:]]*}"}"
        done
        token="${line%%[[:space:]]*}"
        [[ -n "$token" ]] || continue
        if ! (( $+commands[$token] )); then
            kind="${$(whence -w "$token" 2>/dev/null)#*: }"
            case "$kind" in
                builtin|reserved|alias|function|autoload|hashed) ;;
                *) return 0 ;;
            esac
        fi
    done <<< "$buffer"
    return 1
}

__gqy_zsh_accept_line() {
    local buffer="$BUFFER"
    local trimmed="${buffer#"${buffer%%[![:space:]]*}"}"
    if [[ -n "$trimmed" && "$buffer" == *$'\n'* ]]; then
        if __gqy_zsh_multiline_has_unknown "$buffer"; then
            # 整块交给 GQY：隐藏光标、清空缓冲，下一提示符发送
            __gqy_pending_buffer="$buffer"
            print -s -- "$buffer"
            print -n '\e[?25l'
            BUFFER=""
            zle accept-line
            return
        fi
    elif [[ "$trimmed" == https://* || "$trimmed" == http://* || "$trimmed" == *://* ]]; then
        # URL 开头的单行输入：zsh 对含 / 的命令不触发 command_not_found_handler
        # （按路径查找直接报错），这里提前交给 GQY；纯 URL 命令如 curl 不受影响
        # （它们是已知命令，不会走到这里；这里只接 URL 开头的未知命令）
        print -s -- "$buffer"
        print -rn -- '\e[?25l'
        print -rn -- $'\e[36m'"${buffer}"$'\e[0m\n'
        printf '%s' "$buffer" | gqy --shell-intercept --shell zsh --stdin 2>/dev/null
        print -rn -- '\e[?25h'
        BUFFER=""
        zle accept-line
        return
    fi
    zle accept-line
}
zle -N __gqy_zsh_accept_line
bindkey -M emacs '^M' __gqy_zsh_accept_line
bindkey -M viins '^M' __gqy_zsh_accept_line
bindkey -M vicmd '^M' __gqy_zsh_accept_line

__gqy_zsh_precmd() {
    if [[ -n "$__gqy_pending_buffer" ]]; then
        local buffer="$__gqy_pending_buffer"
        __gqy_pending_buffer=""
        print -rn -- '\e[?25h'
        print -rn -- $'\e[31m'"${buffer}"$'\e[0m\n'
        printf '%s' "$buffer" | gqy --shell-intercept --shell zsh --stdin 2>/dev/null
        print -rn -- '\e[?25h'
    fi
}
autoload -Uz add-zsh-hook
add-zsh-hook precmd __gqy_zsh_precmd
"#
}

pub fn install(paths: &GqyPaths) -> Result<()> {
    if let Some(parent) = paths.zsh_hook_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&paths.zsh_hook_file, hook())?;
    let rc_path = home_file(".zshrc");
    append_source_block(&rc_path, BEGIN_MARKER, END_MARKER, &paths.zsh_hook_file)?;
    println!(
        "{}: {}",
        t("installed zsh hook", "已安装 zsh hook"),
        paths.zsh_hook_file.display()
    );
    println!("{}: {}", t("updated", "已更新"), rc_path.display());
    super::print_reload_hint("zsh", &paths.zsh_hook_file);
    Ok(())
}

pub fn uninstall(paths: &GqyPaths) -> Result<bool> {
    let removed_file = remove_file_if_exists(&paths.zsh_hook_file)?;
    let rc_path = home_file(".zshrc");
    let removed_block = remove_source_block(&rc_path, BEGIN_MARKER, END_MARKER)?;
    let removed = removed_file || removed_block;
    if removed {
        println!(
            "{}: zsh",
            t("removed GQY shell hook", "已移除 GQY shell hook")
        );
    }
    Ok(removed)
}

fn home_file(name: &str) -> std::path::PathBuf {
    directories::BaseDirs::new()
        .map(|dirs| dirs.home_dir().join(name))
        .unwrap_or_else(|| std::path::PathBuf::from(name))
}

fn append_source_block(rc_path: &Path, begin: &str, end: &str, hook_file: &Path) -> Result<()> {
    let existing = std::fs::read_to_string(rc_path).unwrap_or_default();
    if existing.contains(begin) && existing.contains(end) {
        return Ok(());
    }
    if let Some(parent) = rc_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(rc_path)?;
    if !existing.ends_with('\n') && !existing.is_empty() {
        writeln!(file)?;
    }
    writeln!(file, "{begin}")?;
    writeln!(file, "[ -r {:?} ] && source {:?}", hook_file, hook_file)?;
    writeln!(file, "{end}")?;
    Ok(())
}

fn remove_source_block(rc_path: &Path, begin: &str, end: &str) -> Result<bool> {
    let Ok(existing) = std::fs::read_to_string(rc_path) else {
        return Ok(false);
    };
    let Some(begin_index) = existing.find(begin) else {
        return Ok(false);
    };
    let Some(end_relative) = existing[begin_index..].find(end) else {
        return Ok(false);
    };
    let mut end_index = begin_index + end_relative + end.len();
    if existing.as_bytes().get(end_index) == Some(&b'\r') {
        end_index += 1;
    }
    if existing.as_bytes().get(end_index) == Some(&b'\n') {
        end_index += 1;
    }
    let mut updated = String::new();
    updated.push_str(&existing[..begin_index]);
    updated.push_str(&existing[end_index..]);
    std::fs::write(rc_path, updated)?;
    Ok(true)
}

fn remove_file_if_exists(path: &Path) -> Result<bool> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn zsh_hook_defines_command_not_found_handler() {
        let hook = hook();
        assert!(hook.contains("command_not_found_handler"));
        assert!(hook.contains("--shell zsh"));
        assert!(hook.contains("return 127"));
    }

    #[test]
    fn zsh_hook_wires_classifier_and_multiline_intercept() {
        let hook = hook();
        assert!(hook.contains("--shell-classify"));
        assert!(hook.contains("gqy_zsh_accept_line"));
        assert!(hook.contains("bindkey -M emacs '^M'"));
        assert!(hook.contains("precmd"));
        assert!(hook.contains("--stdin"));
    }

    #[test]
    fn zsh_hook_does_not_filter_natural_language_symbols() {
        let hook = hook();
        assert!(!hook.contains("${#text} <= 120"));
        assert!(!hook.contains("gqy_shell_syntax_pattern"));
        assert!(!hook.contains("gqy_leading_pattern"));
    }

    #[test]
    fn remove_file_if_exists_reports_whether_file_was_removed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hook.zsh");

        assert!(!remove_file_if_exists(&path).unwrap());
        std::fs::write(&path, hook()).unwrap();
        assert!(remove_file_if_exists(&path).unwrap());
        assert!(!remove_file_if_exists(&path).unwrap());
    }

    #[test]
    fn remove_source_block_reports_whether_block_was_removed() {
        let temp = tempfile::tempdir().unwrap();
        let rc_path = temp.path().join(".zshrc");
        std::fs::write(
            &rc_path,
            format!("before\n{BEGIN_MARKER}\nsource hook\n{END_MARKER}\nafter\n"),
        )
        .unwrap();

        assert!(remove_source_block(&rc_path, BEGIN_MARKER, END_MARKER).unwrap());
        assert_eq!(
            std::fs::read_to_string(&rc_path).unwrap(),
            "before\nafter\n"
        );
        assert!(!remove_source_block(&rc_path, BEGIN_MARKER, END_MARKER).unwrap());
    }
}
