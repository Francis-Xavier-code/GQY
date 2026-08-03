"""
网页爬虫 —— 纯标准库 BFS 爬虫。

特性：
- 广度优先抓取，从种子 URL 出发，按 max_pages / max_depth 限制。
- 尊重 robots.txt（解析 disallow 规则），不抓取禁止路径。
- 软限速（默认 1 请求/秒）， polite 抓取。
- URL 去重（已访问集合）。
- 同域优先 + 限域可配（默认不跨出种子域，避免爬到全互联网）。
- 原始页面落盘到 data/raw/<hash>.html，并登记到 manifest.jsonl。

这是搜索引擎管线的「数据采集」层。
"""
import hashlib
import json
import os
import re
import ssl
import time
import urllib.parse
import urllib.request
from dataclasses import dataclass, field
from html.parser import HTMLParser

DEFAULT_UA = "MiniSearchBot/0.1 (+local educational crawler)"
POLITE_DELAY = 1.0  # 秒/请求

# 链接提取：在 HTML 里找 <a href> 等
_HREF_RE = re.compile(
    r"""(?i)<a[^>]+href\s*=\s*["']([^"']+)["']""", re.IGNORECASE
)

# —— 域名黑名单：已知内容农场 / SEO 垃圾站 / 低质聚合站 ——
# 匹配规则：后缀匹配——只要 host 以列表中任一项结尾即视为农场站（含子域）。
# 这里收录的是英文搜索生态里典型、长期被诟病的噪声源。可按需增删。
DEFAULT_FARM_DOMAINS = frozenset([
    # 内容农场 / 私有博客网络（PBN）
    "ezinearticles.com",
    "articlesbase.com",
    "goarticles.com",
    "selfgrowth.com",
    "sooperarticles.com",
    "articlecity.com",
    "busytrade.com",
    "articlesfactory.com",
    "amazines.com",
    # 问答农场（低质伪原创、关键词堆砌）
    "ehow.com",
    "hubpages.com",
    "ezine.com",
    "lifewire.com",  # 视情况：部分内容农场化
    "wisegeek.com",
    "brighthub.com",
    # 目录 / 链接农场
    "dmoz.org",
    "dir.yahoo.com",
    "bestoftheweb.com",
    "jayde.com",
    "business.com",
    # 伪原创 / -spinner 站
    "spinrewriter.com",
    "thefreelibrary.com",
    # 中文农场（即便开了英文优先也可能漏网）
    "csdn.net",        # 采集严重、SEO 泛滥
    "cnblogs.com",     # 部分采集
    "51cto.com",
    "oschina.net",
    "zhihu.com",       # 非农场但是 UGC 噪声，英文检索无价值
    "baidu.com",
    "360doc.com",
    "doc88.com",
    "wenku.baidu.com",
    # 通用低质聚合
    "yumpu.com",
    "scribd.com",      # 付费墙 + 采集
    "slideshare.net",
])


class DomainBlocklist:
    """后缀匹配的域名黑名单。支持默认农场清单 + 用户自定义清单。"""

    def __init__(self, extra: "set[str] | None" = None, use_default: bool = True):
        self._rules: set[str] = set()
        if use_default:
            self._rules |= DEFAULT_FARM_DOMAINS
        if extra:
            self._rules |= {d.lower().strip().lstrip("*.") for d in extra}

    @classmethod
    def from_file(cls, path: str, use_default: bool = True) -> "DomainBlocklist":
        extra = set()
        try:
            with open(path, "r", encoding="utf-8") as f:
                for line in f:
                    line = line.strip()
                    if not line or line.startswith(("#", "//")):
                        continue
                    extra.add(line)
        except FileNotFoundError:
            pass
        return cls(extra=extra, use_default=use_default)

    def blocked(self, host: str) -> bool:
        host = host.lower()
        for rule in self._rules:
            if host == rule or host.endswith("." + rule):
                return True
        return False

    def __len__(self):
        return len(self._rules)


@dataclass
class CrawlResult:
    url: str
    status: int
    html: str
    fetched_at: float


class RobotsCache:
    """极简 robots.txt 解析：仅支持 Disallow 与通配符 *。"""

    def __init__(self):
        self._cache: dict[str, set] = {}

    def _fetch_disallows(self, origin: str) -> set:
        if origin in self._cache:
            return self._cache[origin]
        disallows = set()
        try:
            url = urllib.parse.urljoin(origin, "/robots.txt")
            req = urllib.request.Request(url, headers={"User-Agent": DEFAULT_UA})
            ctx = ssl.create_default_context()
            with urllib.request.urlopen(req, timeout=8, context=ctx) as r:
                for line in r.read().decode("utf-8", "ignore").splitlines():
                    line = line.strip()
                    if line.lower().startswith("disallow:"):
                        path = line.split(":", 1)[1].strip()
                        if path:
                            disallows.add(path)
        except Exception:
            pass
        self._cache[origin] = disallows
        return disallows

    def allowed(self, url: str) -> bool:
        origin = "{0.scheme}://{0.netloc}".format(urllib.parse.urlparse(url))
        for rule in self._fetch_disallows(origin):
            if rule == "*" or rule == "/":
                if rule == "*" or url[len(origin):].startswith(rule):
                    return False
            if url[len(origin):].startswith(rule):
                return False
        return True


def _norm_url(url: str) -> str:
    p = urllib.parse.urlparse(url)
    # 去 fragment，规整结尾斜杠
    path = p.path or "/"
    if path.endswith("/"):
        path = path[:-1]
    return f"{p.scheme}://{p.netloc}{path}" + (f"?{p.query}" if p.query else "")


def looks_english(html: str, min_latin_ratio: float = 0.55) -> bool:
    """
    轻量英文页判断：
    1. 优先看 <html lang> / <meta> 是否声明 en 系语言；
    2. 否则对正文抽样，统计拉丁字母占比，低于阈值视为非英文页（如中文/日文站）。
    用于英文优先搜索后端，从源头过滤中文等噪声语料。
    """
    m = re.search(r'(?i)<html[^>]*\blang\s*=\s*["\']([^"\']+)', html)
    if m and m.group(1).lower().startswith("en"):
        return True
    # 去掉标签后抽样
    body = _TAG_RE.sub(" ", html) if "_TAG_RE" in globals() else re.sub(r"<[^>]+>", " ", html)
    sample = re.sub(r"\s+", "", body)[:4000]
    if not sample:
        return False
    latin = sum(1 for ch in sample if (65 <= ord(ch) <= 90 or 97 <= ord(ch) <= 122))
    return (latin / len(sample)) >= min_latin_ratio


def extract_links(html: str, base: str) -> list[str]:
    links = []
    for m in _HREF_RE.finditer(html):
        href = m.group(1).strip()
        if not href or href.startswith(("javascript:", "mailto:", "tel:", "#")):
            continue
        abs_url = urllib.parse.urljoin(base, href)
        links.append(_norm_url(abs_url))
    return links


def crawl(
    seeds: list[str],
    out_dir: str,
    max_pages: int = 50,
    max_depth: int = 2,
    same_domain_only: bool = True,
    delay: float = POLITE_DELAY,
    max_bytes: int = 2_000_000,
    english_only: bool = True,
    blocklist: "DomainBlocklist | None" = None,
) -> dict:
    """
    返回统计 dict：{pages, bytes, manifest_path, ...}
    """
    os.makedirs(out_dir, exist_ok=True)
    raw_dir = os.path.join(out_dir, "raw")
    os.makedirs(raw_dir, exist_ok=True)
    manifest_path = os.path.join(out_dir, "manifest.jsonl")

    robots = RobotsCache()
    visited: set[str] = set()
    queue: list[tuple[str, int]] = [( _norm_url(s), 0) for s in seeds]
    seen_hosts = {urllib.parse.urlparse(s).netloc for s in seeds}

    ctx = ssl.create_default_context()
    pages = 0
    total_bytes = 0
    manifest_f = open(manifest_path, "w", encoding="utf-8")

    while queue and pages < max_pages:
        url, depth = queue.pop(0)
        if url in visited:
            continue
        visited.add(url)

        # 域名黑名单：跳过已知农场站 / 低质聚合源
        if blocklist is not None and blocklist.blocked(urllib.parse.urlparse(url).netloc):
            continue

        if not robots.allowed(url):
            continue

        # 限域：默认不跨出种子域名
        host = urllib.parse.urlparse(url).netloc
        if same_domain_only and host not in seen_hosts:
            continue

        try:
            req = urllib.request.Request(url, headers={"User-Agent": DEFAULT_UA})
            with urllib.request.urlopen(req, timeout=10, context=ctx) as r:
                ctype = r.headers.get("Content-Type", "")
                if "html" not in ctype.lower():
                    continue
                raw = r.read(max_bytes)
                html = raw.decode("utf-8", "ignore")
                status = r.status
        except Exception:
            continue

        # 英文优先：从源头过滤中文/日文等噪声语料（仍有少数页面会漏过，索引层已不处理 CJK）
        if english_only and not looks_english(html):
            continue

        # 落盘
        h = hashlib.sha1(url.encode("utf-8")).hexdigest()[:16]
        fname = os.path.join(raw_dir, f"{h}.html")
        with open(fname, "w", encoding="utf-8") as f:
            f.write(html)

        rec = {
            "url": url,
            "file": fname,
            "status": status,
            "depth": depth,
            "ts": time.time(),
        }
        manifest_f.write(json.dumps(rec, ensure_ascii=False) + "\n")
        pages += 1
        total_bytes += len(raw)

        if depth < max_depth:
            for link in extract_links(html, url):
                host = urllib.parse.urlparse(link).netloc
                if blocklist is not None and blocklist.blocked(host):
                    continue
                if not same_domain_only and host not in seen_hosts:
                    seen_hosts.add(host)
                if link not in visited:
                    queue.append((link, depth + 1))

        time.sleep(delay)

    manifest_f.close()
    return {
        "pages": pages,
        "bytes": total_bytes,
        "manifest_path": manifest_path,
        "raw_dir": raw_dir,
    }


if __name__ == "__main__":
    import sys
    seeds = sys.argv[1:] or ["https://example.com"]
    res = crawl(seeds, "data/crawl", max_pages=20, max_depth=1)
    print(res)
