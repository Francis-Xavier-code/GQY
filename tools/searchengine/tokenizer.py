"""
搜索引擎分词器 —— 英文 / 数字优先。

设计决策：
- 只处理拉丁字母与数字（英文词、URL 片段、版本号、代码标识符等）。
- 刻意不处理 CJK。原因：中文搜索生态信噪比低（SEO 农场、采集站、
  营销权重泛滥），纯英文检索（DuckDuckGo / Google）召回质量高一个量级。
  对面向个人 / 技术用途的搜索后端，英文优先是性价比最高的方案。
- 零依赖、转小写、按词切分。

这是搜索引擎管线里最底层、最影响召回质量的模块。
"""
import re

# 英文 / 数字词（含点、下划线、加号、横杠，方便匹配版本号与标识符）
_LATIN_RE = re.compile(r"[a-zA-Z0-9][a-zA-Z0-9._+#-]*")

_MIN_TERM = 1
_MAX_TERM = 64  # 防止异常超长 token


def tokenize(text: str) -> list[str]:
    """把文本切分为小写词条列表。非拉丁内容直接忽略。"""
    if not text:
        return []
    return [
        m.group(0).lower()
        for m in _LATIN_RE.finditer(text)
        if _MIN_TERM <= len(m.group(0)) <= _MAX_TERM
    ]


def term_freq(tokens: list[str]) -> dict[str, int]:
    """统计词频。"""
    tf: dict[str, int] = {}
    for t in tokens:
        tf[t] = tf.get(t, 0) + 1
    return tf


if __name__ == "__main__":
    samples = [
        "Python crawler search engine implementation",
        "BM25 ranking algorithm for web search",
        "苹果公司发布了新的iPhone 16手机",
        "部署 Kubernetes v1.30 的最佳实践 best practices",
    ]
    for s in samples:
        print(f"\nIN : {s}")
        print(f"OUT: {tokenize(s)}")
