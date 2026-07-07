use std::env;
use std::fs;
use std::path::Path;

const PROMPT_MASK: &[u8] = b"MiyuPromptMask";

fn main() {
    println!("cargo:rerun-if-changed=src/prompts/miyu.md");
    println!("cargo:rerun-if-changed=src/prompts/plan.md");

    let prompt = fs::read("src/prompts/miyu.md").expect("read src/prompts/miyu.md");
    let encoded = prompt
        .into_iter()
        .enumerate()
        .map(|(index, byte)| byte ^ PROMPT_MASK[index % PROMPT_MASK.len()])
        .collect::<Vec<_>>();
    let encoded = base64_encode(&encoded);
    let out_dir = env::var("OUT_DIR").expect("OUT_DIR is set by cargo");
    let dest = Path::new(&out_dir).join("default_miyu_prompt.rs");
    fs::write(
        dest,
        format!(
            "const PROMPT_MASK: &[u8] = b\"MiyuPromptMask\";\nconst OBFUSCATED_DEFAULT_SYSTEM_PROMPT: &str = \"{encoded}\";\n"
        ),
    )
    .expect("write generated prompt asset");
}

fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut output = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let first = chunk[0];
        let second = chunk.get(1).copied().unwrap_or(0);
        let third = chunk.get(2).copied().unwrap_or(0);
        output.push(TABLE[(first >> 2) as usize] as char);
        output.push(TABLE[(((first & 0b0000_0011) << 4) | (second >> 4)) as usize] as char);
        if chunk.len() > 1 {
            output.push(TABLE[(((second & 0b0000_1111) << 2) | (third >> 6)) as usize] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(TABLE[(third & 0b0011_1111) as usize] as char);
        } else {
            output.push('=');
        }
    }
    output
}
