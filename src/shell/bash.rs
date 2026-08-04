use crate::i18n::text as t;
use crate::paths::GqyPaths;
use anyhow::Result;
use std::io::Write;
use std::path::Path;

const BEGIN_MARKER: &str = "# >>> gqy bash hook >>>";
const END_MARKER: &str = "# <<< gqy bash hook <<<";

pub fn hook() -> &'static str {
    r#"# 拦截「命令未找到」：明显是命令/拼写错误 → 交给系统报错；自然语言 → 交给 GQY。
command_not_found_handle() {
    [[ $- == *i* ]] || return 127

    local text="$*"
    [[ -n "$text" ]] || return 127
    [[ "$text" != *$'\n'* && "$text" != *$'\r'* ]] || return 127

    # classify: exit 0=命令, 1=自然语言, 2=不确定
    gqy --shell-classify --shell bash -- "$@" 2>/dev/null
    local rc=$?

    if [[ $rc -eq 0 ]]; then
        return 127  # 是命令，走系统报错
    elif [[ $rc -eq 2 ]]; then
        # 不确定：询问用户（3 秒超时默认走命令）
        printf '\e[33m❓ 命令还是对话？[c=命令 / n=对话] (3s 超时走命令)\e[0m '
        local reply=""
        read -n 1 -t 3 reply 2>/dev/null
        echo ""
        if [[ "$reply" == "n" || "$reply" == "N" ]]; then
            gqy --shell-intercept --shell bash -- "$@" 2>/dev/null
            return 127
        fi
        return 127  # 默认或 c → 走系统报错
    fi

    # rc=1 → 高置信自然语言，发给 GQY
    gqy --shell-intercept --shell bash -- "$@" 2>/dev/null
    return 127
}

# ---- 多行自然语言：Enter 时整块交给 GQY（bind -x 读取 READLINE_LINE） ----
__gqy_pending_buffer=""

# 缓冲内是否含「未知命令」行：有则视为自然语言块，交给 GQY
__gqy_multiline_has_unknown() {
    local buffer="$1" line token
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
        if ! command -v "$token" >/dev/null 2>&1 && ! type -t "$token" >/dev/null 2>&1; then
            return 0
        fi
    done <<< "$buffer"
    return 1
}

__gqy_enter() {
    local buffer="${READLINE_LINE:-}"
    if [[ -n "$buffer" && "$buffer" == *$'\n'* ]] && __gqy_multiline_has_unknown "$buffer"; then
        # 整块交给 GQY：清空行缓冲，下一提示符发送
        __gqy_pending_buffer="$buffer"
        READLINE_LINE=""
        READLINE_POINT=0
    fi
}
if [[ $- == *i* ]]; then
    bind -x '"\C-m": __gqy_enter' 2>/dev/null || true
fi

__gqy_prompt() {
    if [[ -n "$__gqy_pending_buffer" ]]; then
        local buffer="$__gqy_pending_buffer"
        __gqy_pending_buffer=""
        printf '\033[31m%s\033[0m\n' "$buffer"
        printf '%s' "$buffer" | gqy --shell-intercept --shell bash --stdin 2>/dev/null
    fi
}
if [[ -z "${__gqy_prompt_hooked:-}" ]]; then
    __gqy_prompt_hooked=1
    PROMPT_COMMAND="__gqy_prompt${PROMPT_COMMAND:+; $PROMPT_COMMAND}"
fi
"#
}

pub fn install(paths: &GqyPaths) -> Result<()> {
    if let Some(parent) = paths.bash_hook_file.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(&paths.bash_hook_file, hook())?;
    let rc_path = home_file(".bashrc");
    append_source_block(&rc_path, BEGIN_MARKER, END_MARKER, &paths.bash_hook_file)?;
    println!(
        "{}: {}",
        t("installed bash hook", "已安装 bash hook"),
        paths.bash_hook_file.display()
    );
    println!("{}: {}", t("updated", "已更新"), rc_path.display());
    super::print_reload_hint("bash", &paths.bash_hook_file);
    Ok(())
}

pub fn uninstall(paths: &GqyPaths) -> Result<bool> {
    let removed_file = remove_file_if_exists(&paths.bash_hook_file)?;
    let rc_path = home_file(".bashrc");
    let removed_block = remove_source_block(&rc_path, BEGIN_MARKER, END_MARKER)?;
    let removed = removed_file || removed_block;
    if removed {
        println!(
            "{}: bash",
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
    fn bash_hook_defines_command_not_found_handler() {
        let hook = hook();
        assert!(hook.contains("command_not_found_handle"));
        assert!(hook.contains("--shell bash"));
        assert!(hook.contains("return 127"));
    }

    #[test]
    fn bash_hook_wires_classifier_and_multiline_intercept() {
        let hook = hook();
        assert!(hook.contains("--shell-classify"));
        assert!(hook.contains("__gqy_enter"));
        assert!(hook.contains("bind -x"));
        assert!(hook.contains("--stdin"));
    }

    #[test]
    fn bash_hook_does_not_filter_natural_language_symbols() {
        let hook = hook();
        assert!(!hook.contains("${#text} <= 120"));
        assert!(!hook.contains("gqy_shell_syntax_pattern"));
        assert!(!hook.contains("gqy_leading_pattern"));
    }

    #[test]
    fn remove_file_if_exists_reports_whether_file_was_removed() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("hook.sh");

        assert!(!remove_file_if_exists(&path).unwrap());
        std::fs::write(&path, hook()).unwrap();
        assert!(remove_file_if_exists(&path).unwrap());
        assert!(!remove_file_if_exists(&path).unwrap());
    }

    #[test]
    fn remove_source_block_reports_whether_block_was_removed() {
        let temp = tempfile::tempdir().unwrap();
        let rc_path = temp.path().join(".bashrc");
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
