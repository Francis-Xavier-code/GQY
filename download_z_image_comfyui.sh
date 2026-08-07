#!/bin/zsh

# ==============================================================================
# Z-Image-Turbo (FP8 量化版) + ComfyUI 在 Mac M2 上的一键部署与下载脚本
# ==============================================================================

set -e

BASE_DIR="/Users/mac/Desktop/GQY"
COMFY_DIR="$BASE_DIR/ComfyUI"

echo "=== 1. 检查并克隆 ComfyUI 仓库 ==="
if [ ! -d "$COMFY_DIR" ]; then
    echo "正在克隆 ComfyUI 到 $COMFY_DIR ..."
    git clone https://github.com/comfyanonymous/ComfyUI.git "$COMFY_DIR"
else
    echo "ComfyUI 目录已存在，跳过克隆。"
fi

cd "$COMFY_DIR"

echo ""
echo "=== 2. 安装 Python 依赖环境 ==="
python3 -m pip install -r requirements.txt

echo ""
echo "=== 3. 创建模型文件夹结构 ==="
mkdir -p "$COMFY_DIR/models/text_encoders"
mkdir -p "$COMFY_DIR/models/diffusion_models"
mkdir -p "$COMFY_DIR/models/vae"

echo ""
echo "=== 4. 下载量化模型文件 ==="

# 4.1 下载 Text Encoder (Qwen 3.4B FP8 混合量化版，约 5.6 GB)
TEXT_ENCODER_PATH="$COMFY_DIR/models/text_encoders/qwen_3_4b_fp8_mixed.safetensors"
if [ ! -f "$TEXT_ENCODER_PATH" ]; then
    echo "[下载中 1/3] Text Encoder (qwen_3_4b_fp8_mixed.safetensors)..."
    curl -L -C - --progress-bar \
        "https://huggingface.co/Comfy-Org/z_image/resolve/main/split_files/text_encoders/qwen_3_4b_fp8_mixed.safetensors" \
        -o "$TEXT_ENCODER_PATH"
else
    echo "[已存在 1/3] Text Encoder 模型已存在，跳过下载。"
fi

# 4.2 下载 Diffusion Model (Z-Image-Turbo FP8, 约 6.2 GB)
DIFFUSION_MODEL_PATH="$COMFY_DIR/models/diffusion_models/z-image-turbo-fp8-e4m3fn.safetensors"
if [ ! -f "$DIFFUSION_MODEL_PATH" ]; then
    echo "[下载中 2/3] Diffusion Model (z-image-turbo-fp8-e4m3fn.safetensors)..."
    curl -L -C - --progress-bar \
        "https://huggingface.co/drbaph/Z-Image-Turbo-FP8/resolve/main/z-image-turbo-fp8-e4m3fn.safetensors" \
        -o "$DIFFUSION_MODEL_PATH"
else
    echo "[已存在 2/3] Diffusion Model 已存在，跳过下载。"
fi

# 4.3 下载 VAE (Flux 1 VAE, 约 335 MB)
VAE_PATH="$COMFY_DIR/models/vae/ae.safetensors"
if [ ! -f "$VAE_PATH" ]; then
    echo "[下载中 3/3] VAE (ae.safetensors)..."
    curl -L -C - --progress-bar \
        "https://huggingface.co/Comfy-Org/z_image/resolve/main/split_files/vae/ae.safetensors" \
        -o "$VAE_PATH"
else
    echo "[已存在 3/3] VAE 模型已存在，跳过下载。"
fi

echo ""
echo "=============================================================================="
echo "所有量化模型下载与配置完成！"
echo "请在终端运行以下命令启动 ComfyUI："
echo "  cd $COMFY_DIR"
echo "  python3 main.py --lowvram"
echo "=============================================================================="
