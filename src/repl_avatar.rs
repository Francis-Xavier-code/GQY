use crate::tools::kitty_image;
use crate::i18n::text as t;
use std::io::Write;

/// GQY-icon.png 嵌入二进制
const ICON_PNG: &[u8] = include_bytes!("../pics/GQY-icon.png");

/// 5x5 块字：G / Q / Y（每行 5 字符，1 表示实心，0 表示留白）
const LETTERS: [(&str, [&str; 5]); 3] = [
    ("G", ["01110", "10001", "10000", "10001", "01111"]),
    ("Q", ["01110", "10001", "10001", "10101", "01010"]),
    ("Y", ["10001", "01010", "00100", "00100", "00100"]),
];

/// 渐变主题：月夜清影（深蓝夜空 → 冷蓝 → 月白 → 淡紫 → 银灰）
const GRADIENT: [[u8; 3]; 5] = [
    [30, 58, 138],   // #1e3a8a 深蓝夜空
    [59, 130, 246],  // #3b82f6 冷蓝
    [147, 197, 253], // #93c5fd 月白
    [167, 139, 250], // #a78bfa 淡紫
    [203, 213, 225], // #cbd5e1 银灰
];

const SLOGAN: &str = "顾清影 · 活在终端里的二次元少女";

/// 启动 banner：居中图片 + 状态信息。
/// 图片显示优先级：kitty → iTerm2 → chafa → ANSI 文字 logo。
pub fn print_startup_banner(
    out: &mut impl Write,
    provider: &str,
    model: &str,
    mode: &str,
    memory_count: usize,
    kb_count: usize,
) {
    let _ = print_banner_inner(out, provider, model, mode, memory_count, kb_count);
}

fn print_banner_inner(
    out: &mut impl Write,
    provider: &str,
    model: &str,
    mode: &str,
    memory_count: usize,
    kb_count: usize,
) -> std::io::Result<()> {
    let (terminal_cols, terminal_rows) = crossterm::terminal::size().unwrap_or((80, 24));

    // 尝试显示图片（目标 8 行高度）
    let image_shown = kitty_image::print_icon_centered(ICON_PNG, 8);

    if !image_shown {
        // 降级：ANSI 彩色文字 logo
        print_text_logo(out, terminal_cols)?;
    }

    writeln!(out)?;

    // 状态信息（居中）
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("顾清影 · GQY  v{}", version);
    let status_line1 = format!(
        "{} / {}",
        provider, model
    );
    let status_line2 = format!(
        "{} · {} {} · {} {}",
        mode,
        memory_count,
        t("memories", "条记忆"),
        kb_count,
        t("knowledge articles", "篇知识库"),
    );
    let help_hint = t("Type /help for commands", "输入 /help 查看命令");

    // 居中打印
    print_centered(out, &title, terminal_cols, "\x1b[38;2;147;197;253m")?;  // 月白
    print_centered(out, &status_line1, terminal_cols, "\x1b[38;2;100;116;139m")?;  // 暗灰
    print_centered(out, &status_line2, terminal_cols, "\x1b[38;2;100;116;139m")?;
    writeln!(out)?;
    print_centered(out, help_hint, terminal_cols, "\x1b[38;2;71;85;105m")?;  // 更暗的灰

    // 分隔线
    let separator = "─".repeat(terminal_cols.min(60) as usize);
    let pad = (terminal_cols as usize).saturating_sub(separator.len()) / 2;
    writeln!(out, "\x1b[38;2;51;65;85m{:width$}{}\x1b[0m", "", separator, width = pad)?;

    Ok(())
}

/// ANSI 彩色文字 logo（降级方案）
fn print_text_logo(out: &mut impl Write, terminal_cols: u16) -> std::io::Result<()> {
    for row in 0..5 {
        let [r, g, b] = GRADIENT[row];
        let mut line = String::new();
        for (letter_index, (_, letter_rows)) in LETTERS.iter().enumerate() {
            if letter_index > 0 {
                line.push(' ');
            }
            for ch in letter_rows[row].chars() {
                if ch == '1' {
                    line.push_str("\u{2588}\u{2588}");
                } else {
                    line.push_str("  ");
                }
            }
        }
        let pad = (terminal_cols as usize).saturating_sub(line.chars().count() * 2) / 2;
        write!(out, "\x1b[38;2;{r};{g};{b}m{:width$}{}\x1b[0m\n", "", line, width = pad)?;
    }
    Ok(())
}

/// 居中打印一行带颜色的文本
fn print_centered(
    out: &mut impl Write,
    text: &str,
    terminal_cols: u16,
    color_escape: &str,
) -> std::io::Result<()> {
    let text_width = unicode_width::UnicodeWidthStr::width(text);
    let pad = (terminal_cols as usize).saturating_sub(text_width) / 2;
    writeln!(out, "{}{:width$}{}\x1b[0m", color_escape, "", text, width = pad)
}
