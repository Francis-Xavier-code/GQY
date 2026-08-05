use anyhow::Result;
use crate::cli::{MemesArgs, MemesCommand};
use crate::i18n::text as t;
use crate::paths::GqyPaths;

pub fn run_memes(paths: &GqyPaths, args: MemesArgs) -> Result<()> {
    match args.command.unwrap_or(MemesCommand::List) {
        MemesCommand::List => {
            let libs = meme_library_summaries(paths);
            if libs.is_empty() {
                println!("{}", t("No meme libraries found.", "未找到表情库。"));
                return Ok(());
            }
            for (library, count) in libs {
                println!("{library}: {count} 个表情");
            }
            println!();
            println!(
                "{}",
                t(
                    "Add more with `add_meme` in conversation (give it an image path).",
                    "对话中用 add_meme 工具（提供图片路径）即可添加更多表情。"
                )
            );
            Ok(())
        }
        MemesCommand::Stats => {
            let (total, formats) = meme_format_stats(paths);
            if crate::i18n::is_zh() {
                println!("共 {total} 个表情；格式分布：{formats}");
            } else {
                println!("{total} memes total; formats: {formats}");
            }
            Ok(())
        }
    }
}

/// 扫描表情库目录（内置 + 用户覆盖层），返回 (库名, 表情数)。
fn meme_library_summaries(paths: &GqyPaths) -> Vec<(String, usize)> {
    let mut result = Vec::new();
    let mut scan = |root: &std::path::Path| {
        let Ok(entries) = std::fs::read_dir(root) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let index = path.join("index.json");
            if !index.is_file() {
                continue;
            }
            let count = std::fs::read_to_string(&index)
                .ok()
                .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
                .and_then(|value| {
                    value
                        .get("memes")
                        .and_then(serde_json::Value::as_array)
                        .map(Vec::len)
                })
                .unwrap_or(0);
            result.push((entry.file_name().to_string_lossy().to_string(), count));
        }
    };
    // 三种布局都扫：brew/app 的 share/memes、源码树的 share/src/memes、用户覆盖层
    scan(&paths.share_dir.join("memes"));
    scan(&paths.share_dir.join("src/memes"));
    scan(&paths.data_dir.join("memes"));
    result
}

/// 表情格式分布统计（jpg/gif/png/webp…）。
fn meme_format_stats(paths: &GqyPaths) -> (usize, String) {
    use std::collections::BTreeMap;
    let mut formats: BTreeMap<String, usize> = BTreeMap::new();
    let mut total = 0usize;
    for root in [
        paths.share_dir.join("memes"),
        paths.share_dir.join("src/memes"),
        paths.data_dir.join("memes"),
    ] {
        let Ok(entries) = std::fs::read_dir(&root) else {
            continue;
        };
        for entry in entries.flatten() {
            let images = entry.path().join("images");
            let Ok(files) = std::fs::read_dir(&images) else {
                continue;
            };
            for file in files.flatten() {
                let name = file.file_name().to_string_lossy().to_string();
                let ext = name.rsplit('.').next().unwrap_or("?").to_lowercase();
                *formats.entry(ext).or_insert(0) += 1;
                total += 1;
            }
        }
    }
    let summary = formats
        .into_iter()
        .map(|(ext, count)| format!("{ext}×{count}"))
        .collect::<Vec<_>>()
        .join(" ");
    (total, summary)
}
