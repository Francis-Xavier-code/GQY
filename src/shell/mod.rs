pub mod bash;
pub mod fish;
pub mod zsh;

use crate::i18n::text as t;
use std::env;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

/// 分类结果：命令、自然语言、不确定。
/// exit codes: 0=Command, 1=NaturalLang, 2=Uncertain
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClassifyResult {
    Command,
    NaturalLang,
    Uncertain,
}

impl ClassifyResult {
    pub fn exit_code(self) -> i32 {
        match self {
            Self::Command => 0,
            Self::NaturalLang => 1,
            Self::Uncertain => 2,
        }
    }
}

pub fn print_reload_hint(shell: &str, hook_file: &Path) {
    let source = match shell {
        "fish" => format!("source {}", fish_quote(hook_file)),
        "bash" | "zsh" => format!("source {}", shell_quote(hook_file)),
        _ => return,
    };
    if current_parent_shell().as_deref() == Some(shell) {
        println!(
            "{}: {}",
            t(
                "run this in the current terminal to load it now",
                "在当前终端运行此命令可立即加载"
            ),
            source
        );
    } else {
        println!(
            "{}",
            t(
                "open a new matching shell session for the hook to take effect",
                "新开对应 shell 会话后 hook 将生效"
            )
        );
    }
}

pub fn current_parent_shell() -> Option<String> {
    let mut pid = std::process::id();
    for _ in 0..8 {
        let parent = parent_pid(pid)?;
        let name = process_name(parent)?;
        if matches!(name.as_str(), "fish" | "bash" | "zsh") {
            return Some(name);
        }
        pid = parent;
    }
    None
}

fn parent_pid(pid: u32) -> Option<u32> {
    if let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) {
        let after_name = stat.rsplit_once(") ")?.1;
        return after_name.split_whitespace().nth(1)?.parse().ok();
    }

    ps_field(pid, "ppid")?.parse().ok()
}

fn process_name(pid: u32) -> Option<String> {
    let name = std::fs::read_to_string(format!("/proc/{pid}/comm"))
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .or_else(|| ps_field(pid, "comm"))?;
    std::path::Path::new(&name)
        .file_name()
        .and_then(|value| value.to_str())
        .map(str::to_string)
}

fn ps_field(pid: u32, field: &str) -> Option<String> {
    let output = std::process::Command::new("ps")
        .args(["-o", &format!("{field}="), "-p", &pid.to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8(output.stdout)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn shell_quote(path: &Path) -> String {
    format!("'{}'", path.display().to_string().replace('\'', "'\\''"))
}

fn fish_quote(path: &Path) -> String {
    format!(
        "'{}'",
        path.display()
            .to_string()
            .replace('\\', "\\\\")
            .replace('\'', "\\'")
    )
}

#[cfg(test)]
pub fn looks_like_natural_language(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return false;
    }
    !trimmed.contains('\n') && !trimmed.contains('\r')
}

/// History expansion 语法（`!!`、`!$`、`!^`、`!n`、`!string`、`!?pattern?`）。
/// 这些输入应直接透传给 shell 处理，不进 GQY 分类器。
pub fn is_history_expansion(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.len() < 2 || !trimmed.starts_with('!') {
        return false;
    }
    let second = trimmed.as_bytes()[1];
    // !!  上一条命令
    // !$   上一条的最后一个参数
    // !^   上一条的第一个参数
    // !n   第 n 条历史
    // !-n  倒数第 n 条
    // !str 以 str 开头的最近命令
    // !?pattern?  包含 pattern 的最近命令
    second == b'!'
        || second == b'$'
        || second == b'^'
        || second == b'?'
        || second.is_ascii_digit()
        || second == b'-'
        || second.is_ascii_alphabetic()
}

/// heredoc 开头（`<<` 或 `<<-`），应透传给 shell 处理多行内容。
pub fn is_heredoc_start(input: &str) -> bool {
    let trimmed = input.trim();
    // <<WORD 或 <<-WORD（允许引号包裹定界符）
    if let Some(after) = trimmed.strip_prefix("<<") {
        let rest = after.trim_start_matches('-').trim_start();
        return !rest.is_empty() && !rest.starts_with(' ');
    }
    // 管道/序列链中的 heredoc：`cmd <<EOF`
    if input.contains("<<") {
        let parts: Vec<&str> = input.splitn(2, "<<").collect();
        if parts.len() == 2 {
            let after = parts[1].trim_start_matches('-').trim_start();
            if !after.is_empty() && !after.starts_with(' ') {
                return true;
            }
        }
    }
    false
}

/// 管道链（含 `|` 但不是以 `|` 开头），整条应透传给 shell。
pub fn is_pipe_chain(input: &str) -> bool {
    let trimmed = input.trim();
    if trimmed.is_empty() || trimmed.starts_with('|') {
        return false;
    }
    // 检查是否有未引号包裹的 `|`
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    for ch in trimmed.chars() {
        if escaped {
            escaped = false;
            continue;
        }
        if ch == '\\' {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if ch == '|' && !in_single && !in_double {
            return true;
        }
    }
    false
}

#[allow(dead_code)]
pub fn is_shell_command(input: &str, shell_name: &str) -> bool {
    classify_with_confidence(input, shell_name) == ClassifyResult::Command
}

/// 三态分类：Command / NaturalLang / Uncertain。
///
/// 评分规则：
/// - PATH 命中 / builtin / keyword → Command
/// - 显式路径（/、./、../、~/）→ Command
/// - 歧义命令 + CJK/问号尾巴 → NaturalLang
/// - 以中文/日文/韩文开头 → NaturalLang
/// - 含 ? / ？ / 吗 / 呢 / 吧 → NaturalLang
/// - 全英文、无特殊字符、不在 PATH → Uncertain
/// - 其他 → NaturalLang
pub fn classify_with_confidence(input: &str, shell_name: &str) -> ClassifyResult {
    let Some((command, rest)) = first_command_token_with_rest(input) else {
        return ClassifyResult::NaturalLang;
    };

    // 歧义命令 + CJK/问号 → 自然语言
    if ambiguous_command_tail_looks_like_message(&command, rest) {
        return ClassifyResult::NaturalLang;
    }

    // PATH / builtin / keyword / 显式路径 → 命令
    if is_shell_keyword_or_builtin(&command, shell_name)
        || is_explicit_command_path(&command)
        || command_exists_in_path(&command)
    {
        return ClassifyResult::Command;
    }

    // 以下规则用于判断是 NaturalLang 还是 Uncertain

    // 以 CJK 字符开头 → 高置信自然语言
    if let Some(first_char) = input.trim().chars().next() {
        if is_cjk_char(first_char) {
            return ClassifyResult::NaturalLang;
        }
    }

    // 含问号/语气词 → 高置信自然语言
    let trimmed = input.trim();
    if trimmed.contains('?')
        || trimmed.contains('？')
        || trimmed.contains("吗")
        || trimmed.contains("呢")
        || trimmed.contains("吧")
        || trimmed.contains("么")
        || trimmed.contains("呀")
        || trimmed.contains("哦")
        || trimmed.contains("啊")
    {
        return ClassifyResult::NaturalLang;
    }

    // 含 CJK 字符（不只是开头）→ 自然语言
    if trimmed.chars().any(is_cjk_char) {
        return ClassifyResult::NaturalLang;
    }

    // 全英文、不在 PATH → 不确定
    ClassifyResult::Uncertain
}

fn first_command_token_with_rest(input: &str) -> Option<(String, &str)> {
    let mut offset = 0;
    while let Some(token) = next_fish_like_token(input, &mut offset) {
        if is_env_assignment(&token) {
            continue;
        }
        return Some((token, input.get(offset..).unwrap_or("")));
    }
    None
}

/// 歧义命令（既是合法命令又常被当自然语言开头）：
/// 尾巴含中文/问号/语气词 → 判为自然语言。
/// 扩展版：新增常见聊天开头词（帮/请/帮我/怎么/为什么/如何/能不能/今天/明天/等
/// 下/写/查/搜/翻译/推荐/解释/什么是…），这些词后接中文时几乎总是对话。
fn ambiguous_command_tail_looks_like_message(command: &str, rest: &str) -> bool {
    const AMBIGUOUS: &[&str] = &[
        "time", "test", "date", "which", "type", "command", "history", "help", "man",
    ];
    const CHAT_OPENERS: &[&str] = &[
        "帮", "请", "帮我", "怎么", "为什么", "如何", "能不能", "可以", "今天", "明天",
        "昨天", "等下", "写", "查", "搜", "翻译", "推荐", "解释", "什么是", "告诉我",
        "你说", "你觉得", "介绍一下", "点评", "总结", "分析",
    ];
    let rest = rest.trim();
    if rest.is_empty() {
        return false;
    }
    let has_chat_signal = rest
        .chars()
        .any(|ch| ch == '?' || ch == '？' || is_cjk_char(ch));
    if AMBIGUOUS.contains(&command) {
        return has_chat_signal;
    }
    // 聊天开场词：即使命令存在于 PATH（如 `写`、`查` 很少是命令），
    // 只要它是首词且后接中文，判为自然语言
    if CHAT_OPENERS.iter().any(|opener| command.starts_with(opener)) {
        return has_chat_signal;
    }
    false
}

fn is_cjk_char(ch: char) -> bool {
    matches!(
        ch,
        '\u{3400}'..='\u{4DBF}'
            | '\u{4E00}'..='\u{9FFF}'
            | '\u{F900}'..='\u{FAFF}'
            | '\u{20000}'..='\u{2A6DF}'
            | '\u{2A700}'..='\u{2B73F}'
            | '\u{2B740}'..='\u{2B81F}'
            | '\u{2B820}'..='\u{2CEAF}'
    )
}

fn next_fish_like_token(input: &str, offset: &mut usize) -> Option<String> {
    let mut index = *offset;
    loop {
        let rest = input.get(index..)?;
        let Some(ch) = rest.chars().next() else {
            *offset = input.len();
            return None;
        };
        if ch.is_whitespace() {
            index += ch.len_utf8();
            continue;
        }
        if ch == '#' {
            index += ch.len_utf8();
            while let Some(next) = input.get(index..).and_then(|rest| rest.chars().next()) {
                index += next.len_utf8();
                if next == '\n' || next == '\r' {
                    break;
                }
            }
            continue;
        }
        break;
    }

    let mut token = String::new();
    let mut in_single = false;
    let mut in_double = false;
    let mut escaped = false;
    let mut consumed = input.len();

    for (relative, ch) in input[index..].char_indices() {
        let absolute = index + relative;
        if escaped {
            token.push(ch);
            escaped = false;
            continue;
        }
        if ch == '\\' && !in_single {
            escaped = true;
            continue;
        }
        if ch == '\'' && !in_double {
            in_single = !in_single;
            continue;
        }
        if ch == '"' && !in_single {
            in_double = !in_double;
            continue;
        }
        if !in_single
            && !in_double
            && (ch.is_whitespace() || matches!(ch, ';' | '|' | '&' | '<' | '>'))
        {
            consumed = absolute + ch.len_utf8();
            if token.is_empty() {
                token.push(ch);
            }
            break;
        }
        token.push(ch);
    }

    *offset = consumed;
    if token.is_empty() {
        None
    } else {
        Some(token)
    }
}

fn is_env_assignment(token: &str) -> bool {
    let Some((name, _)) = token.split_once('=') else {
        return false;
    };
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
}

fn is_shell_keyword_or_builtin(command: &str, shell_name: &str) -> bool {
    let common = matches!(
        command,
        "alias"
            | "bg"
            | "break"
            | "builtin"
            | "case"
            | "cd"
            | "command"
            | "continue"
            | "else"
            | "end"
            | "exec"
            | "exit"
            | "false"
            | "fg"
            | "for"
            | "function"
            | "functions"
            | "history"
            | "if"
            | "jobs"
            | "not"
            | "or"
            | "and"
            | "read"
            | "return"
            | "set"
            | "source"
            | "status"
            | "switch"
            | "test"
            | "time"
            | "true"
            | "while"
    );
    common
        || (shell_name == "fish"
            && matches!(
                command,
                "abbr"
                    | "argparse"
                    | "begin"
                    | "bind"
                    | "block"
                    | "contains"
                    | "count"
                    | "disown"
                    | "emit"
                    | "eval"
                    | "math"
                    | "random"
                    | "string"
                    | "type"
                    | "ulimit"
            ))
}

fn is_explicit_command_path(command: &str) -> bool {
    command.starts_with('/')
        || command.starts_with("./")
        || command.starts_with("../")
        || command.starts_with("~/")
}

fn command_exists_in_path(command: &str) -> bool {
    if command.is_empty() || command.contains('/') {
        return false;
    }
    let Some(paths) = env::var_os("PATH") else {
        return false;
    };
    env::split_paths(&paths).any(|dir| is_executable_file(&dir.join(command)))
}

fn is_executable_file(path: &Path) -> bool {
    let Ok(metadata) = fs::metadata(path) else {
        return false;
    };
    metadata.is_file() && metadata.permissions().mode() & 0o111 != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_safe_natural_language() {
        assert!(looks_like_natural_language("帮我查一下 niri 输入法"));
        assert!(looks_like_natural_language(
            "why is fcitx candidate window small"
        ));
    }

    #[test]
    fn accepts_command_not_found_text_without_syntax_filtering() {
        assert!(looks_like_natural_language(
            "这样写可以吗？假设我们输入一个字母`x`"
        ));
        assert!(looks_like_natural_language(
            "我好像在输入里加一个左斜杠就会导致输入不被传给gqy/对吗？"
        ));
        assert!(looks_like_natural_language(
            "软件需要适配 Wayland 的 `text-input` 协议，输入法要支持 $GTK_IM_MODULE 吗？"
        ));
        assert!(looks_like_natural_language(
            "GTK_IM_MODULE=fcitx 是什么意思？"
        ));
        assert!(looks_like_natural_language(
            "./target/release/gqy 查询为什么失败？"
        ));
    }

    #[test]
    fn rejects_empty_or_multiline_text() {
        assert!(!looks_like_natural_language(""));
        assert!(!looks_like_natural_language("   "));
        assert!(!looks_like_natural_language("第一行\n第二行"));
    }

    #[test]
    fn classifies_commands_as_shell() {
        assert!(is_shell_command("echo hi", "fish"));
        assert!(is_shell_command("cd /tmp", "fish"));
        assert!(is_shell_command("FOO=bar cargo check", "fish"));
        assert!(is_shell_command("# comment\nls", "fish"));
        assert!(is_shell_command("./target/release/gqy hi", "fish"));
        assert!(is_shell_command("for item in a b", "fish"));
        assert!(is_shell_command("time cargo check", "fish"));
    }

    #[test]
    fn classifies_messages_as_gqy() {
        assert!(!is_shell_command("你觉得 a;b 是什么意思", "fish"));
        assert!(!is_shell_command("解释 <tag> 是什么", "fish"));
        assert!(!is_shell_command("第一行\n第二行", "fish"));
        assert!(!is_shell_command("# note\n解释一下这个问题", "fish"));
        assert!(!is_shell_command("time 是什么命令？", "fish"));
        assert!(!is_shell_command(
            "this-command-probably-does-not-exist",
            "fish"
        ));
        assert!(!is_shell_command(
            "GTK_IM_MODULE=fcitx 是什么意思？",
            "fish"
        ));
    }

    #[test]
    fn detects_history_expansion() {
        // 正例
        assert!(is_history_expansion("!!"));
        assert!(is_history_expansion("!$"));
        assert!(is_history_expansion("!^"));
        assert!(is_history_expansion("!?pattern?"));
        assert!(is_history_expansion("!42"));
        assert!(is_history_expansion("!-3"));
        assert!(is_history_expansion("!git"));
        assert!(is_history_expansion("!echo hello"));
        assert!(is_history_expansion("  !!  ")); // trim 后是 !!
        // 反例
        assert!(!is_history_expansion("!"));
        assert!(!is_history_expansion("hello"));
        assert!(!is_history_expansion(""));
        assert!(!is_history_expansion("echo !"));
    }

    #[test]
    fn detects_heredoc() {
        // 正例
        assert!(is_heredoc_start("cat <<EOF"));
        assert!(is_heredoc_start("cat <<-EOF"));
        assert!(is_heredoc_start("cat << 'END'"));
        assert!(is_heredoc_start("grep pattern <<DONE"));
        // 反例
        assert!(!is_heredoc_start("echo hello"));
        assert!(!is_heredoc_start("echo <<"));
        assert!(!is_heredoc_start("echo << "));
        assert!(!is_heredoc_start(""));
    }

    #[test]
    fn detects_pipe_chain() {
        // 正例
        assert!(is_pipe_chain("cat file | grep pattern"));
        assert!(is_pipe_chain("ls -la | sort | head"));
        assert!(is_pipe_chain("echo hello | wc -l"));
        // 反例
        assert!(!is_pipe_chain("echo hello"));
        assert!(!is_pipe_chain("| starting with pipe"));
        assert!(!is_pipe_chain(""));
        assert!(!is_pipe_chain("echo 'not|a|pipe'"));
    }

    #[test]
    fn classify_confidence_commands() {
        // PATH 命中 → Command
        assert_eq!(classify_with_confidence("ls -la", "zsh"), ClassifyResult::Command);
        assert_eq!(classify_with_confidence("git status", "zsh"), ClassifyResult::Command);
        assert_eq!(classify_with_confidence("cargo check", "zsh"), ClassifyResult::Command);
        // builtin → Command
        assert_eq!(classify_with_confidence("cd /tmp", "zsh"), ClassifyResult::Command);
        assert_eq!(classify_with_confidence("echo hello", "zsh"), ClassifyResult::Command);
        // 显式路径 → Command
        assert_eq!(classify_with_confidence("./target/release/gqy", "zsh"), ClassifyResult::Command);
        assert_eq!(classify_with_confidence("/usr/bin/ls", "zsh"), ClassifyResult::Command);
        // env prefix → Command
        assert_eq!(classify_with_confidence("FOO=bar cargo check", "zsh"), ClassifyResult::Command);
    }

    #[test]
    fn classify_confidence_natural_language() {
        // CJK 开头 → NaturalLang
        assert_eq!(classify_with_confidence("帮我看看磁盘空间", "zsh"), ClassifyResult::NaturalLang);
        assert_eq!(classify_with_confidence("怎么清理缓存", "zsh"), ClassifyResult::NaturalLang);
        // 问号 → NaturalLang（用不存在的命令避免 PATH 命中）
        assert_eq!(classify_with_confidence("zzyyxx this?", "zsh"), ClassifyResult::NaturalLang);
        assert_eq!(classify_with_confidence("这是什么？", "zsh"), ClassifyResult::NaturalLang);
        // 语气词 → NaturalLang
        assert_eq!(classify_with_confidence("is this okay吗", "zsh"), ClassifyResult::NaturalLang);
        // CJK 在任意位置 → NaturalLang
        assert_eq!(classify_with_confidence("hello 世界", "zsh"), ClassifyResult::NaturalLang);
        // 歧义命令 + CJK → NaturalLang
        assert_eq!(classify_with_confidence("time 是什么命令？", "zsh"), ClassifyResult::NaturalLang);
    }

    #[test]
    fn classify_confidence_uncertain() {
        // 全英文、不在 PATH → Uncertain
        assert_eq!(classify_with_confidence("foobarbaz", "zsh"), ClassifyResult::Uncertain);
        assert_eq!(classify_with_confidence("xyz123", "zsh"), ClassifyResult::Uncertain);
    }
}
