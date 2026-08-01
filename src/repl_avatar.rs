use base64::Engine;
use std::io::Write;

/// 内嵌的顾清影头像（512×512 RGBA），显示时缩放到目标高度。
const AVATAR_PNG: &[u8] = include_bytes!("../pics/GQY-avatar.png");

const DISPLAY_HEIGHT: u32 = 96;

/// 终端支持时打印头像（iTerm2 / kitty 图形协议），
/// 普通终端回退为 ANSI 真彩色块马赛克头像。
pub fn print_if_supported(out: &mut impl Write) {
    if std::env::var("TERM_PROGRAM").as_deref() == Ok("iTerm.app") {
        let _ = print_iterm2(out);
        return;
    }
    let term = std::env::var("TERM").unwrap_or_default();
    let program = std::env::var("TERM_PROGRAM").unwrap_or_default();
    if term == "xterm-kitty" || program == "kitty" {
        let _ = print_kitty(out);
    } else {
        let _ = print_block_art(out);
    }
}

/// ANSI 真彩色块马赛克：所有终端可显示（Terminal.app 也支持）。
const BLOCK_WIDTH: u32 = 36;
const BLOCK_HEIGHT: u32 = 16;

fn print_block_art(out: &mut impl Write) -> std::io::Result<()> {
    let Ok(source) = image::load_from_memory(AVATAR_PNG) else {
        return Ok(());
    };
    let rgba = source.to_rgba8();
    let small =
        image::imageops::resize(&rgba, BLOCK_WIDTH, BLOCK_HEIGHT, image::imageops::FilterType::Triangle);
    for y in 0..BLOCK_HEIGHT {
        for x in 0..BLOCK_WIDTH {
            let pixel = small.get_pixel(x, y);
            let [r, g, b, a] = pixel.0;
            if a < 40 {
                write!(out, "  ")?;
            } else {
                write!(out, "\x1b[48;2;{r};{g};{b}m  \x1b[0m")?;
            }
        }
        writeln!(out)?;
    }
    Ok(())
}

fn resized_png() -> Option<Vec<u8>> {
    let source = image::load_from_memory(AVATAR_PNG).ok()?;
    let width = (source.width() as f32 * (DISPLAY_HEIGHT as f32 / source.height() as f32)) as u32;
    let small = source.resize(width.max(1), DISPLAY_HEIGHT, image::imageops::FilterType::Lanczos3);
    let mut bytes = Vec::new();
    small.write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Png).ok()?;
    Some(bytes)
}

fn print_iterm2(out: &mut impl Write) -> std::io::Result<()> {
    let Some(png) = resized_png() else { return Ok(()) };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    write!(
        out,
        "\x1b]1337;File=inline=1;width={DISPLAY_HEIGHT}px;height={DISPLAY_HEIGHT}px;preserveAspectRatio=1:{encoded}\x07"
    )
}

fn print_kitty(out: &mut impl Write) -> std::io::Result<()> {
    let Some(png) = resized_png() else { return Ok(()) };
    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);
    write!(out, "\x1b_Gf=32,s={},v={},m=1;{}\x1b\\", DISPLAY_HEIGHT, DISPLAY_HEIGHT, encoded)?;
    write!(out, "\x1b_Gm=0\x1b\\")
}
