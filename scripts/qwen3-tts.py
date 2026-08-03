#!/usr/bin/env python3
"""顾清影 · 音色克隆 TTS（Qwen3-TTS 12Hz + mlx-audio，Apple Silicon 本地）

用法:
  venv/bin/python scripts/qwen3-tts.py "要朗读的文字" [输出.wav]
  env GQY_TTS_REF=<ref.wav> GQY_TTS_REF_TEXT="<原话>" venv/bin/python scripts/qwen3-tts.py "文字"

音色克隆三要素（缺一不可）:
  ① 参考音频 ref.wav（5-15 秒干净人声）
  ② 参考文字 ref_text（录音里的原话，逐字一致）
  ③ 克隆模式（ref_audio + ref_text 同时给）
"""
import os, sys, time
from mlx_audio.tts.generate import generate_audio

# 项目自包含：模型缓存走项目 .hf-cache（GQY_HF_HOME）
HF_HOME = os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", ".hf-cache")
os.environ.setdefault("HF_HOME", os.path.abspath(HF_HOME))
os.environ.setdefault("HF_ENDPOINT", "https://hf-mirror.com")

MODEL = os.environ.get("GQY_TTS_MODEL", "mlx-community/Qwen3-TTS-12Hz-1.7B-Base-6bit")
_BASE = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
REF_AUDIO = os.environ.get("GQY_TTS_REF", os.path.join(_BASE, "assets", "voice", "ref.wav"))
REF_TEXT = os.environ.get(
    "GQY_TTS_REF_TEXT",
    "靠杯了，你终于上线了，我既开心又有点怨你，嗯；小雅一个人在这里等你，等的都快委屈死了！",
)

def main():
    if len(sys.argv) < 2:
        print("用法: qwen3-tts.py <文字> [输出.wav]")
        sys.exit(1)
    text = sys.argv[1]
    out = sys.argv[2] if len(sys.argv) > 2 else "gqy-tts-out.wav"
    print(f"🎤 顾清影音色合成中... ({MODEL.split('/')[-1]})")
    t0 = time.time()
    generate_audio(
        model=MODEL,
        text=text,
        lang_code="zh",
        ref_audio=REF_AUDIO,
        ref_text=REF_TEXT,
        output_path=out,
        save=True,
        verbose=False,
    )
    secs = time.time() - t0
    import wave
    try:
        w = wave.open(out)
        dur = w.getnframes() / w.getframerate()
        print(f"✅ 生成完成: {out}（{dur:.1f} 秒音频，耗时 {secs:.1f}s，实时率 {dur/secs:.2f}x）")
    except Exception:
        print(f"✅ 生成完成: {out}（耗时 {secs:.1f}s）")

if __name__ == "__main__":
    main()
