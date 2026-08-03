"""
CLI —— 把爬取/索引/检索串起来。

用法：
  python3 cli.py crawl   <url> [--pages N] [--depth D]
  python3 cli.py index
  python3 cli.py search  <query> [--top K]
  python3 cli.py serve   [--port 8080]     # 启动网页搜索 UI

数据放在 ./data 下：
  data/crawl/manifest.jsonl + raw/
  data/index/{docs.jsonl, postings.json, stats.json}
"""
import argparse
import os
import sys

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import crawler
import indexer
import query

DATA = os.path.join(os.path.dirname(os.path.abspath(__file__)), "data")
CRAWL_DIR = os.path.join(DATA, "crawl")
INDEX_DIR = os.path.join(DATA, "index")
DEFAULT_BLACKLIST_FILE = os.path.join(DATA, "blacklist.txt")


def _build_blocklist(args) -> "crawler.DomainBlocklist":
    if args.no_blacklist:
        return crawler.DomainBlocklist(use_default=False)
    path = args.blacklist_file or DEFAULT_BLACKLIST_FILE
    return crawler.DomainBlocklist.from_file(path, use_default=True)


def cmd_crawl(args):
    bl = _build_blocklist(args)
    print(f"→ 从 {args.urls} 抓取 (pages≤{args.pages}, depth≤{args.depth}); 黑名单 {len(bl)} 域")
    res = crawler.crawl(args.urls, CRAWL_DIR, max_pages=args.pages, max_depth=args.depth,
                        english_only=args.english_only, blocklist=bl)
    print(f"  抓到 {res['pages']} 页, {res['bytes']} 字节 -> {res['manifest_path']}")


def cmd_index(args):
    mp = os.path.join(CRAWL_DIR, "manifest.jsonl")
    if not os.path.exists(mp):
        print("! 先跑 crawl 生成 manifest"); sys.exit(1)
    bl = _build_blocklist(args)
    print(f"→ 构建倒排索引 ... (黑名单 {len(bl)} 域)")
    res = indexer.build_index(mp, INDEX_DIR, blocklist=bl)
    print(f"  文档 {res['docs']} 篇, 词条 {res['terms']} 个, 平均长度 {res['avg_len']}"
          + (f", 黑名单剔除 {res['skipped_blacklist']} 篇" if res.get('skipped_blacklist') else ""))


def cmd_search(args):
    idx = query.load_index(INDEX_DIR)
    print(f"→ 查询: {args.query!r}\n")
    for r in query.search(idx, args.query, top_k=args.top):
        print(f"[{r['score']}] {r['title']}")
        print(f"    {r['url']}")
        print(f"    {r['snippet'][:140]}\n")


def cmd_serve(args):
    from server import run
    run(INDEX_DIR, host="127.0.0.1", port=args.port)


def main():
    p = argparse.ArgumentParser(description="从零实现的迷你搜索引擎")
    sub = p.add_subparsers(dest="cmd", required=True)

    pc = sub.add_parser("crawl")
    pc.add_argument("urls", nargs="+")
    pc.add_argument("--pages", type=int, default=50)
    pc.add_argument("--depth", type=int, default=2)
    pc.add_argument("--english-only", dest="english_only", action="store_true", default=True,
                    help="只抓英文页（默认开，过滤中文/日文噪声源）")
    pc.add_argument("--no-english-only", dest="english_only", action="store_false")
    pc.add_argument("--blacklist-file", default=None,
                    help="自定义黑名单文件（每行一个域名，# 开头为注释；默认读 data/blacklist.txt）")
    pc.add_argument("--no-blacklist", dest="no_blacklist", action="store_true", default=False,
                    help="关闭域名黑名单（含默认农场清单）")
    pc.set_defaults(func=cmd_crawl)

    pi = sub.add_parser("index")
    pi.add_argument("--blacklist-file", default=None)
    pi.add_argument("--no-blacklist", dest="no_blacklist", action="store_true", default=False)
    pi.set_defaults(func=cmd_index)

    ps = sub.add_parser("search")
    ps.add_argument("query")
    ps.add_argument("--top", type=int, default=10)
    ps.set_defaults(func=cmd_search)

    pv = sub.add_parser("serve")
    pv.add_argument("--port", type=int, default=8080)
    pv.set_defaults(func=cmd_serve)

    args = p.parse_args()
    args.func(args)


if __name__ == "__main__":
    main()
