use anyhow::Result;
use crate::cli::{ToolsArgs, ToolsCommand};
use crate::paths::GqyPaths;

pub fn run_tools(paths: &GqyPaths, args: ToolsArgs) -> Result<()> {
    match args.command.unwrap_or(ToolsCommand::List) {
        ToolsCommand::Inspect { source } => {
            let candidates = crate::tools::import::inspect_source(&source)?;
            if candidates.is_empty() {
                println!("{} 里没有找到可执行脚本（可能有清单，直接 import 即可）", source);
                return Ok(());
            }
            println!("{} 的候选脚本（{} 个）：", source, candidates.len());
            println!();
            for (path, header) in &candidates {
                println!("  {path}");
                if !header.is_empty() {
                    println!("      {header}");
                }
            }
            println!();
            println!("判断核心功能后导入：gqy tools import {} --only <候选名,…>", source);
            Ok(())
        }
        ToolsCommand::Import { source, name, only } => {
            let only_list = only
                .as_deref()
                .map(|value| value.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_string).collect::<Vec<_>>());
            let result =
                crate::tools::import::import_tools(paths, &source, name.as_deref(), only_list.as_deref())?;
            if let Some(license) = &result.license {
                println!("许可证：{license}（已随包保留 LICENSE）");
            }
            println!("已导入 {} 个工具：{}", result.tools.len(), result.tools.join(", "));
            println!(
                "工具已安装到 {}，下轮对话即可使用，长期有效。",
                paths.config_dir.join("scripts").display()
            );
            if !result.skills.is_empty() {
                println!(
                    "同时导入 {} 个技能（skills/）：{}（已装入 {}，对话里说「加载技能」即可使用）",
                    result.skills.len(),
                    result.skills.join(", "),
                    paths.skills_dir.display()
                );
            }
            Ok(())
        }
        ToolsCommand::Remove { name } => {
            let removed = crate::tools::import::remove_tools(paths, &name)?;
            if crate::i18n::is_zh() {
                println!("已删除工具包 {name}（移除 {removed} 个工具注册）");
            } else {
                println!("removed tool package: {name} ({removed} tool registrations)");
            }
            Ok(())
        }
        ToolsCommand::Show { name } => {
            let tools = crate::tools::import::show_tools(paths, &name)?;
            println!("工具包 {name}：");
            for (id, display, description, disabled) in tools {
                let state = if disabled { "[已禁用] " } else { "" };
                println!("  {state}{id}（{display}）");
                if !description.is_empty() {
                    println!("      {description}");
                }
            }
            Ok(())
        }
        ToolsCommand::Disable { id } => {
            crate::tools::import::disable_tool(paths, &id)?;
            if crate::i18n::is_zh() {
                println!("已禁用工具 {id}（下一轮扫描生效）");
            } else {
                println!("disabled tool: {id}");
            }
            Ok(())
        }
        ToolsCommand::Enable { id } => {
            crate::tools::import::enable_tool(paths, &id)?;
            if crate::i18n::is_zh() {
                println!("已重新启用工具 {id}（下一轮扫描生效）");
            } else {
                println!("enabled tool: {id}");
            }
            Ok(())
        }
        ToolsCommand::List => {
            let packages = crate::tools::import::list_tools(paths)?;
            if packages.is_empty() {
                println!("暂无已导入的用户工具包（gqy tools import <目录或仓库>）");
            }
            for (name, count, license) in packages {
                println!("{name}: {count} 个工具（{license}）");
            }
            Ok(())
        }
    }
}
