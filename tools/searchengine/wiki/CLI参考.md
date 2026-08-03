# CLI 参考

入口：`python3 cli.py <子命令> [参数]`

## crawl — 抓取网页

```
python3 cli.py crawl <urls...> [--pages N] [--depth D] [--english-only | --no-english-only]
                         [--blacklist-file PATH] [--no-blacklist]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `urls` | （必填） | 一个或多个种子 URL |
| `--pages` | 50 | 最多抓取页数 |
| `--depth` | 2 | BFS 最大深度 |
| `--english-only` / `--no-english-only` | 开 | 只抓英文页（关则不过滤非英文页） |
| `--blacklist-file PATH` | `data/blacklist.txt` | 自定义黑名单文件 |
| `--no-blacklist` | 关 | 关闭域名黑名单（含默认农场清单） |

输出：`data/crawl/manifest.jsonl` + `data/crawl/raw/*.html`

## index — 构建索引

```
python3 cli.py index [--blacklist-file PATH] [--no-blacklist]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `--blacklist-file PATH` | `data/blacklist.txt` | 自定义黑名单文件 |
| `--no-blacklist` | 关 | 关闭域名黑名单 |

输出：`data/index/{docs.jsonl, postings.json, stats.json}`
（若触发黑名单剔除，会打印 `黑名单剔除 N 篇`）

## search — 命令行检索

```
python3 cli.py search <query> [--top K]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `query` | （必填） | 查询串（英文有效；中文返回零结果） |
| `--top` | 10 | 返回结果条数 |

## serve — 启动网页 UI

```
python3 cli.py serve [--port P]
```

| 参数 | 默认 | 说明 |
|---|---|---|
| `--port` | 8080 | 监听端口 |

打开 http://127.0.0.1:<port> 使用。

## 示例

```bash
# 抓取一个技术文档站全集（同域，depth 3）
python3 cli.py crawl https://docs.python.org/3/ --pages 200 --depth 3

# 重建索引（带自定义黑名单）
python3 cli.py index --blacklist-file my_sites.txt

# 检索
python3 cli.py search "bm25 ranking" --top 5

# 起 UI
python3 cli.py serve --port 8090
```
