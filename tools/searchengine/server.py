"""
网页搜索 UI —— 纯标准库 http.server，单文件、无依赖。

启动：python3 cli.py serve --port 8080
然后浏览器打开 http://127.0.0.1:8080
"""
import html
import os
from http.server import BaseHTTPRequestHandler, HTTPServer
from urllib.parse import urlparse, parse_qs

import query

PAGE = """<!doctype html><html lang="zh"><head><meta charset="utf-8">
<title>迷你搜索引擎</title>
<style>
 body{{font-family:-apple-system,system-ui,"PingFang SC",sans-serif;max-width:760px;margin:40px auto;padding:0 16px;color:#1a1a1a}}
 h1{{font-weight:800;letter-spacing:-.02em}}
 form{{display:flex;gap:8px;margin:24px 0}}
 input{{flex:1;padding:12px 14px;font-size:16px;border:1px solid #ddd;border-radius:10px}}
 button{{padding:12px 20px;font-size:16px;border:0;border-radius:10px;background:#2563eb;color:#fff;cursor:pointer}}
 .meta{{color:#888;font-size:13px;margin-bottom:16px}}
 .r{{margin:18px 0;padding-bottom:14px;border-bottom:1px solid #f0f0f0}}
 .r a{{color:#1a0dab;text-decoration:none;font-size:18px;font-weight:500}}
 .r a:hover{{text-decoration:underline}}
 .url{{color:#0a8a3f;font-size:13px;margin:2px 0}}
 .snip{{color:#444;font-size:14px;line-height:1.5}}
 mark{{background:#fff3a3;padding:0 2px}}
 .empty{{color:#888}}
</style></head><body>
<h1>🔍 迷你搜索引擎</h1>
<form method="get"><input name="q" value="{q}" placeholder="输入查询…" autofocus><button>搜索</button></form>
{meta}
{results}
</body></html>"""

RESULT_TPL = """
<div class="r">
  <a href="{url}">{title}</a>
  <div class="url">{url}</div>
  <div class="snip">{snip}</div>
</div>"""


def render(idx, q: str) -> str:
    if not q:
        return "", ""
    results = query.search(idx, q, top_k=15)
    meta = f'<div class="meta">约 {len(results)} 条结果（BM25 排序）</div>'
    if not results:
        body = '<div class="empty">没有找到相关结果。试试更短的关键词，或先爬取更多页面。</div>'
        return meta, body
    body = "".join(
        RESULT_TPL.format(
            url=html.escape(r["url"]),
            title=html.escape(r["title"]),
            snip=r["snippet"],
        )
        for r in results
    )
    return meta, body


def run(index_dir: str, host: str = "127.0.0.1", port: int = 8080):
    idx = query.load_index(index_dir)
    print(f"索引已加载：{idx['stats']['N']} 篇文档。UI 在 http://{host}:{port}")

    class H(BaseHTTPRequestHandler):
        def do_GET(self):
            qs = parse_qs(urlparse(self.path).query)
            q = qs.get("q", [""])[0]
            meta, results = render(idx, q)
            page = PAGE.format(q=html.escape(q), meta=meta, results=results)
            self.send_response(200)
            self.send_header("Content-Type", "text/html; charset=utf-8")
            self.end_headers()
            self.wfile.write(page.encode("utf-8"))

        def log_message(self, *a):
            pass

    HTTPServer((host, port), H).serve_forever()


if __name__ == "__main__":
    run(os.path.join(os.path.dirname(os.path.abspath(__file__)), "data", "index"))
