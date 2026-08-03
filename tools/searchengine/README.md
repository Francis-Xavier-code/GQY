# 迷你搜索引擎（tools/searchengine）

从零实现、**零依赖**（仅 Python 标准库）的网页搜索引擎原型：
BFS 爬虫 → 倒排索引 → BM25 排序 → 网页 UI。

特性：**英文优先**（不索引中文，避开中文农场噪声）+ **农场黑名单**（双层拦截已知垃圾站）。

## 快速上手

```bash
cd tools/searchengine
python3 cli.py crawl https://example.com --pages 50 --depth 2
python3 cli.py index
python3 cli.py search "example domain" --top 10
python3 cli.py serve --port 8080   # 打开 http://127.0.0.1:8080
```

## 文档

完整说明见 [wiki/](wiki/)：
[Home](wiki/Home.md) · [快速开始](wiki/快速开始.md) · [架构说明](wiki/架构说明.md) ·
[设计决策](wiki/设计决策.md) · [CLI 参考](wiki/CLI参考.md) · [扩展指南](wiki/扩展指南.md)

## 目录

```
tools/searchengine/
├── cli.py          入口（crawl / index / search / serve）
├── crawler.py      BFS 爬虫 + 域名黑名单
├── indexer.py      倒排索引 + 标题加权
├── query.py        BM25 检索 + 摘要高亮
├── tokenizer.py    英文优先分词器
├── server.py       网页 UI
└── wiki/           文档
```

> 运行时数据（`data/`）不进版本库，首次运行 `crawl` 后自动生成。
