/**
 * GQY ↔ pi 工具桥接扩展（pi 底座模式）
 *
 * GQY 主进程在 pi 模式下会：
 *   1. 启动本地 HTTP API（127.0.0.1 随机端口），地址注入 `GQY_PI_TOOL_API`；
 *   2. 把暴露工具清单（JSON）写入 `GQY_PI_TOOL_LIST` 指向的文件。
 *
 * 本扩展在**加载时同步读取**工具清单并 `pi.registerTool`（工具名加 `gqy_` 前缀），
 * 保证首轮 model request 的 system prompt 里就有 gqy_* 工具；
 * 模型调用工具时 `POST /tool` 回调 GQY 主进程执行，结果作为文本返回。
 *
 * 未设置 `GQY_PI_TOOL_LIST` / `GQY_PI_TOOL_API` 时扩展静默退出。
 */
import { readFileSync } from "node:fs";
import type { ExtensionAPI } from "@earendil-works/pi-coding-agent";
import { Type } from "typebox";

interface GqyToolInfo {
	name: string;
	display_name?: string;
	description?: string;
	parameters?: unknown;
	prompt_snippet?: string;
	prompt_guidelines?: string[];
}

export default function gqyBridge(pi: ExtensionAPI) {
	const base = process.env.GQY_PI_TOOL_API;
	const listFile = process.env.GQY_PI_TOOL_LIST;
	if (!base || !listFile) {
		return;
	}

	// session_start 在 bind 完成后、第一个 prompt 处理之前触发：
	// 这里同步读取工具清单并注册，保证首轮 model request 的 system prompt 里就有 gqy_* 工具
	pi.on("session_start", () => {
		registerToolsSync(pi, base, listFile);
	});
}

function registerToolsSync(pi: ExtensionAPI, base: string, listFile: string): void {
	const debug = !!process.env.GQY_PI_DEBUG;

	let tools: GqyToolInfo[];
	try {
		tools = JSON.parse(readFileSync(listFile, "utf8")) as GqyToolInfo[];
	} catch (err) {
		console.error(`[gqy-bridge] 读取工具清单失败: ${String(err)}`);
		return;
	}

	let registered = 0;
	for (const tool of tools) {
		// 规范化工具名：连字符等非法字符转下划线（如 battery-care → battery_care）。
		// execute 仍用原始 name 回调 GQY 桥，映射保持一致。
		const rawName = tool.name;
		const name = String(rawName).toLowerCase().replace(/[^a-z0-9_]/g, "_");
		if (!/^[a-z0-9_]+$/.test(name) || name.length === 0) {
			continue;
		}
		if (name !== rawName && process.env.GQY_PI_DEBUG) {
			console.error(`[gqy-bridge] tool name normalized: ${rawName} -> ${name}`);
		}
		try {
			pi.registerTool({
				name: `gqy_${name}`,
				label: tool.display_name ?? name,
				description: tool.description ?? "",
				promptSnippet: tool.prompt_snippet,
				promptGuidelines: tool.prompt_guidelines,
				parameters: jsonSchemaToTypeBox(tool.parameters),
				async execute(_toolCallId, params) {
					const res = await fetch(`${base}/tool`, {
						method: "POST",
						headers: { "content-type": "application/json" },
						body: JSON.stringify({ name: rawName, arguments: params ?? {} }),
					});
					const data = (await res.json()) as {
						ok: boolean;
						output?: string;
						error?: string;
					};
					if (!data.ok || !res.ok) {
						throw new Error(
							data.error ?? `GQY 工具 ${name} 执行失败 (HTTP ${res.status})`,
						);
					}
					return {
						content: [{ type: "text" as const, text: data.output ?? "" }],
						details: { gqyTool: name },
					};
				},
			});
			registered += 1;
		} catch (err) {
			console.error(`[gqy-bridge] failed to register gqy_${name}: ${String(err)}`);
		}
	}
	if (debug) {
		console.error(
			`[gqy-bridge] registered ${registered}/${tools.length} tools; total in pi: ${pi.getAllTools().length}`,
		);
	}
}

/** 把 GQY 工具参数的 JSON Schema（子集）转换成 TypeBox schema。 */
function jsonSchemaToTypeBox(schema: unknown): any {
	if (typeof schema !== "object" || schema === null) {
		return Type.Unknown();
	}
	const s = schema as Record<string, any>;
	const type = s.type as string | undefined;

	switch (type) {
		case "string": {
			if (Array.isArray(s.enum)) {
				return Type.Union(
					s.enum.map((v) => (typeof v === "string" ? Type.Literal(v) : Type.String())),
				);
			}
			return Type.String({ description: s.description });
		}
		case "integer":
			return Type.Integer({ description: s.description });
		case "number":
			return Type.Number({ description: s.description });
		case "boolean":
			return Type.Boolean({ description: s.description });
		case "array":
			return Type.Array(jsonSchemaToTypeBox(s.items), { description: s.description });
		case "object": {
			const required = new Set<string>(
				Array.isArray(s.required) ? s.required : Object.keys(s.properties ?? {}),
			);
			const props: Record<string, any> = {};
			for (const [key, value] of Object.entries((s.properties ?? {}) as Record<string, any>)) {
				const converted = jsonSchemaToTypeBox(value);
				props[key] = required.has(key) ? converted : Type.Optional(converted);
			}
			return Type.Object(props, {
				additionalProperties: false,
				description: s.description,
			});
		}
		default:
			return Type.Unknown();
	}
}
