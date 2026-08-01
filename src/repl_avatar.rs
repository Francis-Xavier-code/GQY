use std::io::Write;

/// 终端启动横幅：彩色渐变文字版 GQY（oh-my-logo 风格）。
/// 所有终端通用（24bit ANSI 颜色），iTerm2/kitty/Terminal.app 均可显示。
#[allow(dead_code)]
const DISPLAY_HEIGHT: u32 = 96;

/// 5x5 块字：G / Q / Y（每行 5 字符，1 表示实心，0 表示留白）
const LETTERS: [(&str, [&str; 5]); 3] = [
    (
        "G",
        [
            "01110", "10001", "10000", "10001", "01111",
        ],
    ),
    (
        "Q",
        [
            "01110", "10001", "10001", "10101", "01010",
        ],
    ),
    (
        "Y",
        [
            "10001", "01010", "00100", "00100", "00100",
        ],
    ),
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

pub fn print_if_supported(out: &mut impl Write) {
    let _ = print_gqy_logo(out);
}

fn print_gqy_logo(out: &mut impl Write) -> std::io::Result<()> {
    for (row, _pattern) in LETTERS[0].1.iter().enumerate() {
        let [r, g, b] = GRADIENT[row];
        let mut line = String::new();
        for (letter_index, (_, letter_rows)) in LETTERS.iter().enumerate() {
            if letter_index > 0 {
                line.push(' ');
            }
            let letter_row = letter_rows[row];
            for ch in letter_row.chars() {
                if ch == '1' {
                    line.push_str("\u{2588}\u{2588}"); // 实心双宽块
                } else {
                    line.push_str("  ");
                }
            }
        }
        write!(
            out,
            "\x1b[38;2;{r};{g};{b}m{line}\x1b[0m\n"
        )?;
    }
    writeln!(out)?;
    // 标语行：柔和的灰色小字
    writeln!(
        out,
        "\x1b[38;2;148;163;184m{SLOGAN}\x1b[0m\n"
    )?;
    Ok(())
}

/// 保留的图形头像能力（iTerm2 / kitty 图形协议），供未来切换使用。
#[allow(dead_code)]
const AVATAR_PNG: &[u8] = include_bytes!("../pics/GQY-avatar.png");

#[allow(dead_code)]
fn resized_png() -> Option<Vec<u8>> {
    let source = image::load_from_memory(AVATAR_PNG).ok()?;
    let width = (source.width() as f32 * (DISPLAY_HEIGHT as f32 / source.height() as f32)) as u32;
    let small = source.resize(width.max(1), DISPLAY_HEIGHT, image::imageops::FilterType::Lanczos3);
    let mut bytes = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).ok()?;
    Some(bytes)
}
