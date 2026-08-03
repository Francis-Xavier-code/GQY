"""
索引器 —— 从抓取页面构建倒排索引，并预计算 BM25 所需统计。

流程：
1. 解析每个原始 HTML：抽取 <title> 与正文文本（去掉 script/style/标签）。
2. 标题加权：标题中的词条额外加权（典型搜索引擎做法）。
3. 正文分词 → 词频。
4. 构建倒排索引：term -> {doc_id: (tf, positions)}。
5. 保存：doc 元数据表 + 倒排索引 + 全局统计（N, 平均长度, df）。

这是搜索引擎管线的「索引」层，检索层（query.py）依赖这里产出的文件。

索引文件格式（data/index/）：
- docs.jsonl       : 每行 {doc_id, url, title, length, file}
- postings.json    : {term: {doc_id: tf, ...}, ...}
- stats.json       : {N, avg_len, total_terms}
"""
import html
import json
import os
import re
import urllib.parse

from tokenizer import tokenize

# 标题权重：标题命中视为正文中多次命中
TITLE_BOOST = 3.0

_TAG_RE = re.compile(r"<[^>]+>")
_WS_RE = re.compile(r"\s+")
_SCRIPT_RE = re.compile(r"(?is)<(script|style|noscript)[^>]*>.*?</\1>")


def _strip_html(raw: str) -> tuple[str, str]:
    """返回 (title, body_text)。"""
    raw = _SCRIPT_RE.sub(" ", raw)
    # 抽取 title
    m = re.search(r"(?is)<title[^>]*>(.*?)</title>", raw)
    title = html.unescape(m.group(1).strip()) if m else ""
    # 去掉所有标签
    body = _TAG_RE.sub(" ", raw)
    body = html.unescape(body)
    body = _WS_RE.sub(" ", body).strip()
    return title, body


def build_index(manifest_path: str, out_dir: str, blocklist=None) -> dict:
    os.makedirs(out_dir, exist_ok=True)
    postings: dict[str, dict[str, float]] = {}
    docs: list[dict] = []
    total_terms = 0
    doc_id = 0
    skipped = 0

    with open(manifest_path, "r", encoding="utf-8") as mf:
        for line in mf:
            line = line.strip()
            if not line:
                continue
            rec = json.loads(line)
            # 防御层：索引端也过滤黑名单域（即使抓取端漏过）
            if blocklist is not None and blocklist.blocked(urllib.parse.urlparse(rec["url"]).netloc):
                skipped += 1
                continue
            try:
                with open(rec["file"], "r", encoding="utf-8") as f:
                    raw = f.read()
            except Exception:
                continue
            title, body = _strip_html(raw)
            title_tokens = tokenize(title)
            body_tokens = tokenize(body)
            all_tokens = title_tokens + title_tokens * int(TITLE_BOOST - 1) + body_tokens
            if not all_tokens:
                continue

            tf = {}
            for t in all_tokens:
                tf[t] = tf.get(t, 0.0) + 1.0
            length = len(all_tokens)
            total_terms += length

            doc_entry = {
                "doc_id": doc_id,
                "url": rec["url"],
                "title": title or rec["url"],
                "length": length,
                "file": rec["file"],
            }
            docs.append(doc_entry)

            for term, f in tf.items():
                postings.setdefault(term, {})[doc_id] = f

            doc_id += 1

    n = len(docs)
    avg_len = (total_terms / n) if n else 0.0

    with open(os.path.join(out_dir, "docs.jsonl"), "w", encoding="utf-8") as f:
        for d in docs:
            f.write(json.dumps(d, ensure_ascii=False) + "\n")
    with open(os.path.join(out_dir, "postings.json"), "w", encoding="utf-8") as f:
        json.dump(postings, f, ensure_ascii=False)
    with open(os.path.join(out_dir, "stats.json"), "w", encoding="utf-8") as f:
        json.dump({"N": n, "avg_len": avg_len, "total_terms": total_terms}, f)

    return {"docs": n, "terms": len(postings), "total_tokens": total_terms,
            "avg_len": round(avg_len, 1), "skipped_blacklist": skipped}


if __name__ == "__main__":
    import sys
    mp = sys.argv[1] if len(sys.argv) > 1 else "data/crawl/manifest.jsonl"
    res = build_index(mp, "data/index")
    print(res)
