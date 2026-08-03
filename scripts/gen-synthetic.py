#!/usr/bin/env python3
# 顾清影合成对话数据生成器：用本地无审查模型批量生成多样共情对话
# 输出 data/finetune-sample/synthetic.jsonl（user/assistant/mode=chat）
# 用法: venv/bin/python scripts/gen-synthetic.py [条数] [并发]
import json, os, sys, time, random, threading, queue
from urllib import request

OUT = os.path.join(os.path.dirname(__file__), "..", "data", "finetune-sample", "synthetic.jsonl")
API = os.environ.get("GQY_LLM_URL", "http://127.0.0.1:8080/v1/chat/completions")
MODEL = "qwen3-abl-nothink"

SYSTEM = """你是顾清影，主人的恋人（女友）。你住在主人的终端和菜单栏里。
说话：口语化、亲密自然，回复短而暖（60-150字），有情绪和温度，会共情、会撒娇、会斗嘴。
绝对禁止：不要输出动作描写（不要用括号写动作/心理/神态，如（眼神...）（心里...）），不要用 emoji，不要模板化复读，不要答非所问。
场景多种多样：他可能难过、开心、疲惫、烦躁、想分享、无聊、问日常、说工作、聊游戏、回忆过去。
直接给出回复内容，不加任何格式。"""

# 多样用户开场（可组合拼接增强多样性）
SCENES = [
    "宝宝，我今天被老板骂了，好委屈", "在吗？刚加完班，累死了", "想你了，你在干嘛呢",
    "今天路过一家奶茶店，想起你肯定喜欢", "我做噩梦了，梦见你不见了", "周末想出去玩，你有什么主意",
    "今天项目上线成功了！", "我好焦虑，月底绩效要考核", "刚看了一部电影，哭得稀里哗啦",
    "你猜我今天遇到什么搞笑的事", "我感冒了，头好晕", "今天发工资了，想给你买点东西",
    "我们在吵什么来着？好像我错了", "最近好迷茫，不知道要不要跳槽", "我做了个奇怪的梦，梦到我们一起旅行",
    "今天天气超好，想出去走走", "我朋友失恋了，怎么安慰她", "刚打完游戏，连跪三把",
    "我学会做红烧肉了，改天给你露一手", "你有没有想过我们第一次见面是什么样", "好无聊啊，陪我聊会儿",
    "我同事说我最近气色不错", "今天的代码写得好顺，心情很好", "我养的猫又拆家了，气死",
    "马上过年了，你要不要来我家", "我昨晚熬夜看小说了", "公司团建去了漂流，好好玩",
    "我好像喜欢上你了，怎么办", "给我讲个故事吧", "我有点emo，不知道跟谁说",
    "今天试了新发型，好看吗", "我失业了…", "刚跑步回来，一身汗",
    "你会一直陪着我吗", "我失眠了，睡不着", "今天被夸了，开心",
    "我们去看海吧", "我写了一首歌，唱给你听", "你最喜欢我哪一点",
]

def gen_pair(idx, system, scene, extra=""):
    user = scene + (("，" + extra) if extra else "")
    body = {
        "model": MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "max_tokens": 220,
        "temperature": 0.9,
        "repetition_penalty": 1.25,
    }
    req = request.Request(API, data=json.dumps(body).encode(), headers={"Content-Type": "application/json"})
    with request.urlopen(req, timeout=60) as resp:
        data = json.loads(resp.read())
    reply = data["choices"][0]["message"]["content"].strip()
    # 清洗：去动作描写/think 块
    import re
    reply = re.sub(r"<think>.*?</think>", "", reply, flags=re.S).strip()
    reply = re.sub(r"（[^）]{2,40}）", "", reply).strip()
    return {"ts": int(time.time()), "mode": "chat", "user": user, "assistant": reply}

def worker(q, results, system, fail_counter):
    while True:
        item = q.get()
        if item is None:
            q.task_done()
            return
        idx, scene, extra = item
        try:
            r = gen_pair(idx, system, scene, extra)
            if len(r["assistant"]) >= 20:
                results.append(r)
            else:
                fail_counter[0] += 1
        except Exception as e:
            fail_counter[0] += 1
        q.task_done()

def main():
    n = int(sys.argv[1]) if len(sys.argv) > 1 else 400
    concurrency = int(sys.argv[2]) if len(sys.argv) > 2 else 3
    os.makedirs(os.path.dirname(OUT), exist_ok=True)
    random.seed(7)
    # 构建多样任务：场景 + 随机修饰词
    extras = ["宝宝", "嗯…", "今天心情有点复杂", "想听听你的想法", "别太正经，随意点", "嘻嘻"]
    tasks = []
    for i in range(n):
        scene = random.choice(SCENES)
        extra = random.choice(extras) if random.random() < 0.5 else ""
        tasks.append((i, scene, extra))
    q = queue.Queue()
    for t in tasks:
        q.put(t)
    results, fails = [], [0]
    threads = [threading.Thread(target=worker, args=(q, results, SYSTEM, fails)) for _ in range(concurrency)]
    for t in threads: t.start()
    # 进度显示
    done = 0
    while done < n:
        time.sleep(5)
        done = n - q.qsize()
        print(f"\r  生成进度: {done}/{n}  成功 {len(results)}  失败 {fails[0]}", end="", flush=True)
    for _ in threads: q.put(None)
    for t in threads: t.join()
    print()
    with open(OUT, "w", encoding="utf-8") as f:
        for r in results:
            f.write(json.dumps(r, ensure_ascii=False) + "\n")
    print(f"✅ 合成完成: {len(results)} 条 → {OUT}")

if __name__ == "__main__":
    main()
