use super::{tool_descriptions, ToolRegistry, ToolSpec};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

pub fn register(registry: &mut ToolRegistry) {
    let allowed_tools = registry.tool_names().into_iter().collect::<BTreeSet<_>>();
    let loadable_tools = loadable_tools(&allowed_tools);
    let description = format!(
        "按需加载部分内置工具的完整说明和参数 schema。重要：只有下面 <available_tools> 列表里的内置工具需要、也可以通过 load_tools 加载；未列出的工具已经在顶层工具列表中可直接调用，尤其是用户脚本工具和系统脚本工具，不要对脚本工具调用 load_tools。如果要使用列表中的某个工具，请先调用 load_tools，参数为 {{\"names\":[\"工具名\"]}}。加载成功后，后续轮次可以直接调用对应工具。\n\n{}",
        available_tools_xml(&loadable_tools)
    );
    registry.register(
        ToolSpec::new(
            "load_tools",
            description,
            json!({
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要加载的内置工具名称列表。只允许填写 available_tools 列表里的工具；脚本工具不需要 load_tools，应该直接调用。"
                    }
                },
                "required": ["names"]
            }),
            move |args| {
                let loadable_tools = loadable_tools.clone();
                async move { load_tools(args, &loadable_tools) }
            },
        )
        .with_display_name("加载工具"),
    );
}

fn load_tools(args: Value, allowed_tools: &BTreeSet<String>) -> Result<String> {
    let names = args
        .get("names")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("names array is required"))?;
    if names.is_empty() {
        bail!("names must not be empty");
    }

    let mut loaded = Vec::new();
    for value in names {
        let name = value.as_str().unwrap_or_default().trim();
        if name.is_empty() {
            continue;
        }
        if !allowed_tools.contains(name) {
            bail!(
                "tool cannot be loaded with load_tools: {name}. Only tools listed in available_tools can be loaded; script tools and top-level tools should be called directly."
            );
        }
        let Some(desc) = tool_descriptions::get(name) else {
            bail!("unknown tool: {name}");
        };
        loaded.push(json!({
            "name": desc.name,
            "display_name": desc.display_name,
            "description": desc.description,
            "parameters": desc.parameters,
            "permission": desc.permission,
        }));
    }

    Ok(serde_json::to_string_pretty(&json!({
        "loaded_tools": loaded,
        "note": "这些工具的完整定义已加载到当前对话上下文；后续可以直接按对应 name 调用。"
    }))?)
}

fn loadable_tools(allowed_tools: &BTreeSet<String>) -> BTreeSet<String> {
    tool_descriptions::on_demand_descriptions()
        .iter()
        .filter(|desc| allowed_tools.contains(&desc.name))
        .map(|desc| desc.name.clone())
        .collect()
}

fn available_tools_xml(allowed_tools: &BTreeSet<String>) -> String {
    let items = tool_descriptions::on_demand_descriptions()
        .iter()
        .filter(|desc| allowed_tools.contains(&desc.name))
        .map(|desc| {
            format!(
                "  <tool>\n    <name>{}</name>\n    <display_name>{}</display_name>\n    <description>{}</description>\n  </tool>",
                xml_escape(&desc.name),
                xml_escape(&desc.display_name),
                xml_escape(&desc.description)
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("<available_tools>\n{items}\n</available_tools>")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loadable_tools_excludes_registered_scripts() {
        let allowed = BTreeSet::from(["get_weather".to_string(), "battery-care".to_string()]);
        let loadable = loadable_tools(&allowed);

        assert!(loadable.contains("get_weather"));
        assert!(!loadable.contains("battery-care"));
    }

    #[test]
    fn load_tools_rejects_script_like_unlisted_tool() {
        let allowed = BTreeSet::from(["read_file".to_string()]);
        let err = load_tools(json!({"names": ["battery-care"]}), &allowed).unwrap_err();

        assert!(err.to_string().contains("script tools"));
    }
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}
