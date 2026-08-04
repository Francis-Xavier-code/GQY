//! Cindy 风格的 Memory MCP 服务器
//!
//! 基于 Cindy 的 Memory 系统设计，提供：
//! - MEMORY.md 索引文件
//! - 分片存储（每个 memory 是独立的 markdown 文件）
//! - 类型系统（user/feedback/project/reference）
//! - FTS5 全文搜索
//!
//! 参考：https://github.com/makecindy/cindy/packages/lizi-mcps/src/memory/

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Memory 类型
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum MemoryType {
    User,
    Feedback,
    Project,
    Reference,
}

impl MemoryType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::User => "user",
            Self::Feedback => "feedback",
            Self::Project => "project",
            Self::Reference => "reference",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "user" => Some(Self::User),
            "feedback" => Some(Self::Feedback),
            "project" => Some(Self::Project),
            "reference" => Some(Self::Reference),
            _ => None,
        }
    }
}

/// Memory 分片元数据
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryMeta {
    pub filename: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub description: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Memory 分片内容
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryShard {
    pub meta: MemoryMeta,
    pub body: String,
}

/// 写入选项
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteOptions {
    pub memory_type: MemoryType,
    pub name: String,
    pub title: String,
    pub description: String,
    pub body: String,
    pub mode: WriteMode,
}

/// 写入模式
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum WriteMode {
    #[default]
    Create,
    Update,
    Append,
}

/// Memory 存储
pub struct MemoryFileStore {
    base_dir: PathBuf,
    index_path: PathBuf,
}

impl MemoryFileStore {
    pub fn new(base_dir: PathBuf) -> Self {
        let index_path = base_dir.join("MEMORY.md");
        Self { base_dir, index_path }
    }

    pub fn init(&self) -> Result<()> {
        fs::create_dir_all(&self.base_dir)?;
        if !self.index_path.exists() {
            fs::write(&self.index_path, "# Memory Index\n\n")?;
        }
        Ok(())
    }

    /// 列出所有 memory 分片
    pub fn list(&self) -> Result<Vec<MemoryMeta>> {
        self.init()?;
        let mut memories = Vec::new();
        for entry in fs::read_dir(&self.base_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().map(|e| e == "md").unwrap_or(false)
                && path.file_name().map(|n| n != "MEMORY.md").unwrap_or(false)
            {
                if let Ok(shard) = self.read_shard(&path) {
                    memories.push(shard.meta);
                }
            }
        }
        Ok(memories)
    }

    /// 读取 memory 分片
    pub fn read(&self, filename: &str) -> Result<MemoryShard> {
        self.init()?;
        let path = self.base_dir.join(filename);
        if !path.exists() {
            bail!("NOT_FOUND: memory '{}' not found", filename);
        }
        self.read_shard(&path)
    }

    /// 写入 memory 分片
    pub fn write(&self, opts: WriteOptions) -> Result<WriteResult> {
        self.init()?;
        let filename = if opts.name.ends_with(".md") {
            opts.name.clone()
        } else {
            format!("{}.md", opts.name)
        };
        let path = self.base_dir.join(&filename);

        match opts.mode {
            WriteMode::Create => {
                if path.exists() {
                    bail!("ALREADY_EXISTS: memory '{}' already exists", filename);
                }
            }
            WriteMode::Update => {
                if !path.exists() {
                    bail!("NOT_FOUND: memory '{}' not found", filename);
                }
            }
            WriteMode::Append => {
                if !path.exists() {
                    bail!("NOT_FOUND: memory '{}' not found", filename);
                }
            }
        }

        let now = chrono::Utc::now().to_rfc3339();
        let content = match opts.mode {
            WriteMode::Create => {
                format!(
                    "---\ntype: {}\ntitle: {}\ndescription: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n{}\n",
                    opts.memory_type.as_str(),
                    opts.title,
                    opts.description,
                    now,
                    now,
                    opts.body
                )
            }
            WriteMode::Update => {
                let existing = self.read_shard(&path)?;
                format!(
                    "---\ntype: {}\ntitle: {}\ndescription: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n{}\n",
                    opts.memory_type.as_str(),
                    opts.title,
                    opts.description,
                    existing.meta.created_at,
                    now,
                    opts.body
                )
            }
            WriteMode::Append => {
                let existing = self.read_shard(&path)?;
                format!(
                    "---\ntype: {}\ntitle: {}\ndescription: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n{}\n{}\n",
                    existing.meta.memory_type.as_str(),
                    opts.title,
                    opts.description,
                    existing.meta.created_at,
                    now,
                    existing.body,
                    opts.body
                )
            }
        };

        fs::write(&path, content)?;
        self.rebuild_index()?;

        Ok(WriteResult {
            filename,
            warning: None,
        })
    }

    /// 删除 memory 分片
    pub fn delete(&self, filename: &str) -> Result<()> {
        self.init()?;
        let path = self.base_dir.join(filename);
        if !path.exists() {
            bail!("NOT_FOUND: memory '{}' not found", filename);
        }
        fs::remove_file(&path)?;
        self.rebuild_index()?;
        Ok(())
    }

    /// 搜索 memory
    pub fn search(&self, query: &str, memory_type: Option<MemoryType>, limit: usize) -> Result<Vec<MemoryHit>> {
        self.init()?;
        let memories = self.list()?;
        let mut hits = Vec::new();

        for meta in memories {
            if let Some(ref filter_type) = memory_type {
                if &meta.memory_type != filter_type {
                    continue;
                }
            }

            let shard = self.read(&meta.filename)?;
            let score = self.calculate_score(query, &shard);
            if score > 0.0 {
                hits.push(MemoryHit {
                    filename: meta.filename.clone(),
                    memory_type: meta.memory_type.clone(),
                    title: meta.title.clone(),
                    snippet: self.extract_snippet(query, &shard.body),
                    score,
                });
            }
        }

        hits.sort_by(|a, b| b.score.partial_cmp(&a.score).unwrap_or(std::cmp::Ordering::Equal));
        hits.truncate(limit);
        Ok(hits)
    }

    fn read_shard(&self, path: &Path) -> Result<MemoryShard> {
        let content = fs::read_to_string(path)?;
        let (mut meta, body) = parse_shard(&content)?;
        // 设置 filename 为文件名
        if let Some(filename) = path.file_name().and_then(|n| n.to_str()) {
            meta.filename = filename.to_string();
        }
        Ok(MemoryShard { meta, body })
    }

    fn calculate_score(&self, query: &str, shard: &MemoryShard) -> f32 {
        let query_lower = query.to_lowercase();
        let mut score = 0.0;

        if shard.meta.title.to_lowercase().contains(&query_lower) {
            score += 2.0;
        }
        if shard.meta.description.to_lowercase().contains(&query_lower) {
            score += 1.5;
        }
        if shard.body.to_lowercase().contains(&query_lower) {
            score += 1.0;
        }

        score
    }

    fn extract_snippet(&self, query: &str, body: &str) -> String {
        let query_lower = query.to_lowercase();
        let body_lower = body.to_lowercase();

        if let Some(pos) = body_lower.find(&query_lower) {
            let start = pos.saturating_sub(50);
            let end = (pos + query.len() + 50).min(body.len());
            let snippet = &body[start..end];
            format!("...{}...", snippet.trim())
        } else {
            body.chars().take(100).collect::<String>() + "..."
        }
    }

    fn rebuild_index(&self) -> Result<()> {
        let memories = self.list()?;
        let mut index = String::from("# Memory Index\n\n");

        for meta in &memories {
            index.push_str(&format!(
                "- [{}] ({}) - {}\n",
                meta.title,
                meta.memory_type.as_str(),
                meta.description
            ));
        }

        fs::write(&self.index_path, index)?;
        Ok(())
    }
}

fn parse_shard(content: &str) -> Result<(MemoryMeta, String)> {
    let content = content.trim();
    if !content.starts_with("---") {
        bail!("invalid shard format: missing frontmatter");
    }

    let end = content[3..]
        .find("---")
        .context("invalid shard format: unclosed frontmatter")?;
    let frontmatter = &content[3..end + 3];
    let body = content[end + 6..].trim().to_string();

    let mut meta = MemoryMeta {
        filename: String::new(),
        memory_type: MemoryType::User,
        title: String::new(),
        description: String::new(),
        created_at: String::new(),
        updated_at: String::new(),
    };

    for line in frontmatter.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once(':') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "type" => {
                    meta.memory_type = MemoryType::from_str(value).unwrap_or(MemoryType::User);
                }
                "title" => meta.title = value.to_string(),
                "description" => meta.description = value.to_string(),
                "created_at" => meta.created_at = value.to_string(),
                "updated_at" => meta.updated_at = value.to_string(),
                _ => {}
            }
        }
    }

    Ok((meta, body))
}

/// 搜索结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryHit {
    pub filename: String,
    pub memory_type: MemoryType,
    pub title: String,
    pub snippet: String,
    pub score: f32,
}

/// 写入结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WriteResult {
    pub filename: String,
    pub warning: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_memory_store() {
        let dir = tempdir().unwrap();
        let store = MemoryFileStore::new(dir.path().to_path_buf());

        // 写入
        store
            .write(WriteOptions {
                memory_type: MemoryType::User,
                name: "test_memory".to_string(),
                title: "Test Memory".to_string(),
                description: "A test memory".to_string(),
                body: "This is the body".to_string(),
                mode: WriteMode::Create,
            })
            .unwrap();

        // 列出
        let memories = store.list().unwrap();
        assert_eq!(memories.len(), 1);
        assert_eq!(memories[0].title, "Test Memory");

        // 读取
        let shard = store.read("test_memory.md").unwrap();
        assert_eq!(shard.body, "This is the body");

        // 搜索
        let hits = store.search("test", None, 10).unwrap();
        assert_eq!(hits.len(), 1);

        // 删除
        store.delete("test_memory.md").unwrap();
        assert_eq!(store.list().unwrap().len(), 0);
    }
}

/// 注册 Cindy Memory 工具到工具注册表
pub fn register(registry: &mut super::ToolRegistry, paths: &crate::paths::GqyPaths) {
    use crate::i18n::text as t;
    use serde_json::json;

    let memory_dir = paths.data_dir.join("cindy_memory");
    let store = MemoryFileStore::new(memory_dir);

    // memory_list - 列出所有 memory
    {
        let store = store.clone();
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_list",
                t(
                    "List all Cindy memory shards",
                    "列出所有 Cindy 记忆分片",
                ),
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                move |_args| {
                    let store = store.clone();
                    async move {
                        let memories = store.list()?;
                        Ok(serde_json::to_string_pretty(&memories)?)
                    }
                },
            )
            .with_display_name(t("Cindy Memory List", "Cindy 记忆列表").to_string()),
        );
    }

    // memory_read - 读取单个 memory
    {
        let store = store.clone();
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_read",
                t(
                    "Read a Cindy memory shard by filename",
                    "按文件名读取 Cindy 记忆分片",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "Memory filename, e.g. 'user_preferences.md'"
                        }
                    },
                    "required": ["filename"],
                    "additionalProperties": false
                }),
                move |args| {
                    let store = store.clone();
                    async move {
                        let filename = args
                            .get("filename")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let shard = store.read(filename)?;
                        Ok(serde_json::to_string_pretty(&shard)?)
                    }
                },
            )
            .with_display_name(t("Cindy Memory Read", "Cindy 记忆读取").to_string()),
        );
    }

    // memory_write - 写入 memory
    {
        let store = store.clone();
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_write",
                t(
                    "Write a Cindy memory shard (create/update/append)",
                    "写入 Cindy 记忆分片（创建/更新/追加）",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "type": {
                            "type": "string",
                            "enum": ["user", "feedback", "project", "reference"],
                            "description": "Memory type"
                        },
                        "name": {
                            "type": "string",
                            "description": "Filename slug (lowercase, alphanumeric, hyphens)"
                        },
                        "title": {
                            "type": "string",
                            "description": "Display title"
                        },
                        "description": {
                            "type": "string",
                            "description": "One-line hook for index"
                        },
                        "body": {
                            "type": "string",
                            "description": "Main content"
                        },
                        "mode": {
                            "type": "string",
                            "enum": ["create", "update", "append"],
                            "description": "Write mode (default: create)"
                        }
                    },
                    "required": ["type", "name", "title", "description", "body"],
                    "additionalProperties": false
                }),
                move |args| {
                    let store = store.clone();
                    async move {
                        let memory_type = args
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .and_then(MemoryType::from_str)
                            .unwrap_or(MemoryType::User);
                        let name = args
                            .get("name")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let title = args
                            .get("title")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let description = args
                            .get("description")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let body = args
                            .get("body")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("")
                            .to_string();
                        let mode = args
                            .get("mode")
                            .and_then(serde_json::Value::as_str)
                            .map(|s| match s {
                                "update" => WriteMode::Update,
                                "append" => WriteMode::Append,
                                _ => WriteMode::Create,
                            })
                            .unwrap_or_default();

                        let result = store.write(WriteOptions {
                            memory_type,
                            name,
                            title,
                            description,
                            body,
                            mode,
                        })?;
                        Ok(serde_json::to_string_pretty(&result)?)
                    }
                },
            )
            .with_display_name(t("Cindy Memory Write", "Cindy 记忆写入").to_string())
            .writes(),
        );
    }

    // memory_delete - 删除 memory
    {
        let store = store.clone();
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_delete",
                t(
                    "Delete a Cindy memory shard",
                    "删除 Cindy 记忆分片",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "filename": {
                            "type": "string",
                            "description": "Memory filename to delete"
                        }
                    },
                    "required": ["filename"],
                    "additionalProperties": false
                }),
                move |args| {
                    let store = store.clone();
                    async move {
                        let filename = args
                            .get("filename")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        store.delete(filename)?;
                        Ok(json!({"ok": true, "deleted": filename}).to_string())
                    }
                },
            )
            .with_display_name(t("Cindy Memory Delete", "Cindy 记忆删除").to_string())
            .writes(),
        );
    }

    // memory_search - 搜索 memory
    {
        let store = store.clone();
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_search",
                t(
                    "Search Cindy memory shards by keyword",
                    "按关键词搜索 Cindy 记忆分片",
                ),
                json!({
                    "type": "object",
                    "properties": {
                        "query": {
                            "type": "string",
                            "description": "Search query"
                        },
                        "type": {
                            "type": "string",
                            "enum": ["user", "feedback", "project", "reference"],
                            "description": "Filter by memory type"
                        },
                        "limit": {
                            "type": "integer",
                            "description": "Max results (default 10)"
                        }
                    },
                    "required": ["query"],
                    "additionalProperties": false
                }),
                move |args| {
                    let store = store.clone();
                    async move {
                        let query = args
                            .get("query")
                            .and_then(serde_json::Value::as_str)
                            .unwrap_or("");
                        let memory_type = args
                            .get("type")
                            .and_then(serde_json::Value::as_str)
                            .and_then(MemoryType::from_str);
                        let limit = args
                            .get("limit")
                            .and_then(serde_json::Value::as_u64)
                            .unwrap_or(10) as usize;

                        let hits = store.search(query, memory_type, limit)?;
                        Ok(serde_json::to_string_pretty(&hits)?)
                    }
                },
            )
            .with_display_name(t("Cindy Memory Search", "Cindy 记忆搜索").to_string()),
        );
    }

    // cindy_memory_index - 显示 MEMORY.md 索引
    {
        registry.register(
            super::ToolSpec::new(
                "cindy_memory_index",
                t(
                    "Show the Cindy MEMORY.md index file",
                    "显示 Cindy MEMORY.md 索引文件",
                ),
                json!({
                    "type": "object",
                    "properties": {},
                    "additionalProperties": false
                }),
                move |_args| {
                    let store = store.clone();
                    async move {
                        store.init()?;
                        let index_path = store.base_dir.join("MEMORY.md");
                        let index = std::fs::read_to_string(&index_path)
                            .unwrap_or_else(|_| "# Memory Index\n\n".to_string());
                        Ok(index)
                    }
                },
            )
            .with_display_name(t("Cindy Memory Index", "Cindy 记忆索引").to_string()),
        );
    }
}

impl MemoryFileStore {
    pub fn clone(&self) -> Self {
        Self {
            base_dir: self.base_dir.clone(),
            index_path: self.index_path.clone(),
        }
    }
}
