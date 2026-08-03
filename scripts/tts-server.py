#!/usr/bin/env python3
"""顾清影 · TTS 常驻服务（Qwen3-TTS 音色克隆，模型常驻内存）
启动: venv/bin/python scripts/tts-server.py [端口, 默认 8091]
接口: GET /tts?text=... → wav 音频流
"""
import os, sys, io, wave, time
from http.server import HTTPServer, BaseHTTPRequestHandler
from urllib.parse import urlparse, parse_qs, unquote

BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
os.environ.setdefault("HF_HOME", os.path.join(BASE, ".hf-cache"))
os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")

MODEL = os.environ.get("GQY_TTS_MODEL", "mlx-community/Qwen3-TTS-12Hz-1.7B-Base-6bit")
REF_AUDIO = os.environ.get("GQY_TTS_REF", os.path.join(BASE, "assets", "voice", "ref.wav"))
REF_TEXT = os.environ.get("GQY_TTS_REF_TEXT", "靠杯了，你终于上线了，我既开心又有点怨你，嗯；小雅一个人在这里等你，等的都快委屈死了！")

from mlx_audio.tts.generate import generate_audio

class Handler(BaseHTTPRequestHandler):
    def do_GET(self):
        if self.path.startswith("/health"):
            self.send_response(200); self.end_headers(); self.wfile.write(b"ok"); return
        if not self.path.startswith("/tts"):
            self.send_response(404); self.end_headers(); return
        q = parse_qs(urlparse(self.path).query)
        text = q.get("text", [""])[0]
        if not text.strip():
            self.send_response(400); self.end_headers(); return
        out = f"/tmp/gqy-tts-{int(time.time())}.wav"
        generate_audio(model=MODEL, text=text, lang_code="zh",
                       ref_audio=REF_AUDIO, ref_text=REF_TEXT,
                       output_path=out, save=True, verbose=False)
        # mlx-audio 输出是目录形式（out/audio_000.wav），兼容两种
        import glob
        if os.path.isdir(out):
            files = sorted(glob.glob(out + "/audio_*.wav"))
            data = open(files[0], "rb").read() if files else b""
        elif os.path.isfile(out):
            data = open(out, "rb").read()
        else:
            data = b""
        if data:
            self.send_response(200)
            self.send_header("Content-Type", "audio/wav")
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)
        else:
            self.send_response(500); self.end_headers()
    def log_message(self, *a): pass

if __name__ == "__main__":
    port = int(sys.argv[1]) if len(sys.argv) > 1 else 8091
    print(f"✅ TTS 服务启动: http://127.0.0.1:{port}（模型加载中...）")
    # 预热：先加载模型
    generate_audio(model=MODEL, text="预热", lang_code="zh", ref_audio=REF_AUDIO,
                   ref_text=REF_TEXT, output_path="/tmp/gqy-tts-warmup.wav", save=True, verbose=False)
    print("✅ 模型就绪，接受请求")
    HTTPServer(("127.0.0.1", port), Handler).serve_forever()
