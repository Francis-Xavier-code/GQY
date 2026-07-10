use super::{ToolPermission, ToolRegistry, ToolSpec};
use anyhow::{bail, Result};
use serde_json::{json, Value};
use std::collections::BTreeSet;

const BASE_DESCRIPTION: &str = "按需加载工具或脚本的完整说明和参数 schema。<available_tools> 是可加载的内置工具，<available_scripts> 是可加载的脚本工具；请使用 {\"names\":[\"名称\"]} 加载。<unregistered_scripts> 中的文件尚未注册为工具，不能直接加载或调用；需要先读取对应路径并使用 register_script 注册。";

pub fn register(registry: &mut ToolRegistry) {
    registry.register(
        ToolSpec::new(
            "load_tools",
            BASE_DESCRIPTION,
            json!({
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": { "type": "string" },
                        "description": "要加载的内置工具或脚本工具名称列表。只允许填写 available_tools 或 available_scripts 中的名称。"
                    }
                },
                "required": ["names"],
                "additionalProperties": false
            }),
            |_args| async {
                bail!("load_tools must be executed through the active tool registry")
            },
        )
        .with_display_name("加载工具"),
    );
}

pub(super) fn dynamic_description(registry: &ToolRegistry, loaded: &BTreeSet<String>) -> String {
    let loadable = registry.loadable_tools(loaded);
    let builtins = loadable
        .iter()
        .copied()
        .filter(|tool| !tool.is_script)
        .collect::<Vec<_>>();
    let scripts = loadable
        .iter()
        .copied()
        .filter(|tool| tool.is_script)
        .collect::<Vec<_>>();

    format!(
        "{BASE_DESCRIPTION}\n\n{}\n{}\n{}",
        tools_xml("available_tools", "tool", &builtins),
        tools_xml("available_scripts", "script", &scripts),
        unregistered_scripts_xml(registry),
    )
}

pub(super) fn execute(args: Value, registry: &ToolRegistry) -> Result<String> {
    let names = args
        .get("names")
        .and_then(Value::as_array)
        .ok_or_else(|| anyhow::anyhow!("names array is required"))?;
    if names.is_empty() {
        bail!("names must not be empty");
    }

    let mut loaded = Vec::new();
    let mut seen = BTreeSet::new();
    for value in names {
        let name = value.as_str().unwrap_or_default().trim();
        if name.is_empty() || !seen.insert(name.to_string()) {
            continue;
        }
        let Some(tool) = registry.get(name) else {
            bail!("unknown tool or script: {name}");
        };
        if tool.name == "load_tools" || tool.always_loaded {
            bail!(
                "tool cannot be loaded with load_tools: {name}. Only names listed in available_tools or available_scripts can be loaded."
            );
        }
        loaded.push(json!({
            "name": tool.name,
            "display_name": tool.display_name.as_deref().unwrap_or(&tool.name),
            "description": tool.description,
            "parameters": tool.parameters,
            "permission": match tool.permission {
                ToolPermission::ReadOnly => "readonly",
                ToolPermission::Writes => "writes",
            },
            "kind": if tool.is_script { "script" } else { "tool" },
        }));
    }

    if loaded.is_empty() {
        bail!("names must contain at least one loadable tool or script");
    }

    Ok(serde_json::to_string_pretty(&json!({
        "loaded_tools": loaded,
        "note": "完整定义将在下一轮工具列表中生效，可直接按 name 调用。"
    }))?)
}

fn tools_xml(tag: &str, item_tag: &str, tools: &[&ToolSpec]) -> String {
    let items = tools
        .iter()
        .map(|tool| {
            format!(
                "  <{item_tag}>\n    <name>{}</name>\n    <display_name>{}</display_name>\n    <description>{}</description>\n  </{item_tag}>",
                xml_escape(&tool.name),
                xml_escape(tool.display_name.as_deref().unwrap_or(&tool.name)),
                xml_escape(&tool.description),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("<{tag}>\n{items}\n</{tag}>")
}

fn unregistered_scripts_xml(registry: &ToolRegistry) -> String {
    let items = registry
        .unregistered_scripts()
        .iter()
        .map(|script| {
            format!(
                "  <script>\n    <name>{}</name>\n    <path>{}</path>\n  </script>",
                xml_escape(&script.name),
                xml_escape(&script.path),
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("<unregistered_scripts>\n{items}\n</unregistered_scripts>")
}

fn xml_escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn description_separates_builtin_scripts_and_unregistered_files() {
        let mut registry = ToolRegistry::new();
        register(&mut registry);
        registry.register(
            ToolSpec::new(
                "custom_builtin",
                "Built in",
                json!({"type":"object","properties":{}}),
                |_| async { Ok(String::new()) },
            )
            .with_always_loaded(false),
        );
        registry
            .replace_script_tools(
                vec![ToolSpec::new(
                    "lazy_script",
                    "Lazy script",
                    json!({"type":"object","properties":{"query":{"type":"string"}}}),
                    |_| async { Ok(String::new()) },
                )
                .script()
                .with_always_loaded(false)],
                vec![super::super::registry::UnregisteredScript {
                    name: "unknown_script".to_string(),
                    path: "unknown-script".to_string(),
                }],
            )
            .unwrap();

        let description = dynamic_description(&registry, &BTreeSet::new());
        assert!(description.contains("<available_tools>"));
        assert!(description.contains("custom_builtin"));
        assert!(description.contains("<available_scripts>"));
        assert!(description.contains("lazy_script"));
        assert!(description.contains("<unregistered_scripts>"));
        assert!(description.contains("unknown-script"));
    }

    #[tokio::test]
    async fn registry_loads_dynamic_script_definition() {
        let mut registry = ToolRegistry::new();
        register(&mut registry);
        registry
            .replace_script_tools(
                vec![ToolSpec::new(
                    "lazy_script",
                    "Lazy script",
                    json!({"type":"object","properties":{}}),
                    |_| async { Ok(String::new()) },
                )
                .script()
                .with_always_loaded(false)],
                Vec::new(),
            )
            .unwrap();

        let output = registry
            .call("load_tools", r#"{"names":["lazy_script"]}"#)
            .await
            .unwrap();
        assert!(output.contains("lazy_script"));
        assert!(output.contains("\"kind\": \"script\""));
    }
}
