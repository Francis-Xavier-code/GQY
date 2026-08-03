"""
检索层 —— 加载倒排索引，对查询做 BM25 打分，返回 Top-K 结果 + 摘要。

BM25 公式（Robertson/Zaragoza 2009）：
  score(D,Q) = Σ_{t∈Q} IDF(t) · (f(t,D)·(k1+1)) / (f(t,D) + k1·(1-b + b·|D|/avgdl))
  IDF(t) = ln(1 + (N - df(t) + 0.5) / (df(t) + 0.5))

这是搜索引擎管线的「检索/排序」层。
"""
import html
import json
import os
import re

from tokenizer import tokenize

K1 = 1.5
B = 0.75


def _strip_html(raw: str) -> str:
    raw = re.sub(r"(?is)<(script|style|noscript)[^>]*>.*?</\1>", " ", raw)
    raw = re.sub(r"<[^>]+>", " ", raw)
    raw = html.unescape(raw)
    return re.sub(r"\s+", " ", raw).strip()


def load_index(index_dir: str) -> dict:
    with open(os.path.join(index_dir, "postings.json"), "r", encoding="utf-8") as f:
        postings = json.load(f)
    with open(os.path.join(index_dir, "stats.json"), "r", encoding="utf-8") as f:
        stats = json.load(f)
    docs = {}
    with open(os.path.join(index_dir, "docs.jsonl"), "r", encoding="utf-8") as f:
        for line in f:
            line = line.strip()
            if line:
                d = json.loads(line)
                # postings.json 的键经 JSON 化为字符串，统一用 str 键对齐
                docs[str(d["doc_id"])] = d
    return {"postings": postings, "stats": stats, "docs": docs}


def _idf(df: int, N: int) -> float:
    return max(0.0, (N - df + 0.5) / (df + 0.5))


def search(idx: dict, query: str, top_k: int = 10) -> list[dict]:
    postings = idx["postings"]
    docs = idx["docs"]
    N = idx["stats"]["N"]
    avg_len = idx["stats"]["avg_len"]

    q_terms = tokenize(query)
    if not q_terms:
        return []
    # 查询词频（同一词重复出现加权）
    qtf: dict[str, int] = {}
    for t in q_terms:
        qtf[t] = qtf.get(t, 0) + 1

    scores: dict[int, float] = {}
    for term, qf in qtf.items():
        if term not in postings:
            continue
        plist = postings[term]
        df = len(plist)
        idf = _idf(df, N)
        for doc_id, tf in plist.items():
            doc_len = docs[doc_id]["length"]
            denom = tf + K1 * (1 - B + B * doc_len / avg_len)
            s = idf * (tf * (K1 + 1)) / denom
            # 查询词重复出现额外加权（弱）
            scores[doc_id] = scores.get(doc_id, 0.0) + s * (1 + 0.1 * (qf - 1))

    ranked = sorted(scores.items(), key=lambda x: x[1], reverse=True)[:top_k]

    results = []
    for doc_id, score in ranked:
        d = docs[doc_id]
        snippet = _make_snippet(d, q_terms)
        results.append({
            "doc_id": doc_id,
            "url": d["url"],
            "title": d["title"],
            "score": round(score, 4),
            "snippet": snippet,
        })
    return results


def _make_snippet(doc: dict, q_terms: list[str], window: int = 80) -> str:
    """从原始 HTML 正文里，定位首个命中词附近截取摘要，并高亮。"""
    try:
        with open(doc["file"], "r", encoding="utf-8") as f:
            text = _strip_html(f.read())
    except Exception:
        return ""
    text_l = text.lower()
    hit_pos = -1
    for t in q_terms:
        p = text_l.find(t.lower())
        if p >= 0 and (hit_pos < 0 or p < hit_pos):
            hit_pos = p
    if hit_pos < 0:
        return text[:160]
    start = max(0, hit_pos - window)
    end = min(len(text), hit_pos + window)
    snip = text[start:end]
    if start > 0:
        snip = "…" + snip
    if end < len(text):
        snip = snip + "…"
    # 高亮查询词（HTML 转义后包 <mark>）
    for t in q_terms:
        if len(t) < 1:
            continue
        snip = re.sub(r"(?i)(" + re.escape(t) + r")", r"<mark>\1</mark>", snip)
    return snip


if __name__ == "__main__":
    import sys
    idx = load_index("data/index")
    q = sys.argv[1] if len(sys.argv) > 1 else "example"
    for r in search(idx, q, top_k=5):
        print(f"[{r['score']}] {r['title']}\n    {r['url']}\n    {r['snippet'][:120]}\n")
