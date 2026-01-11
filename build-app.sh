#!/bin/bash
set -e

# 获取脚本所在目录的绝对路径，确保在任何地方运行都能找到 fronted
BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
FRONTED_DIR="$BASE_DIR/fronted"

echo "📦 Starting build process for Codex Proxy Desktop App..."

# 检查 fronted 目录是否存在
if [ ! -d "$FRONTED_DIR" ]; then
    echo "❌ Error: 'fronted' directory not found at $FRONTED_DIR"
    exit 1
fi

cd "$FRONTED_DIR"

# 检查是否安装了依赖
if [ ! -d "node_modules" ]; then
    echo "⬇️ Installing dependencies..."
    npm install
else
    echo "ℹ️ Dependencies already installed."
fi

# 创建构建资源目录 (electron-builder 默认查找位置)
if [ ! -d "build" ]; then
    echo "ℹ️ Creating 'build' directory for icons..."
    mkdir -p build
    echo "💡 Tip: Place your icon.icns, icon.ico, or icon.png (1024x1024) in '$FRONTED_DIR/build/' for custom icons."
fi

echo "🚀 Building Electron app (Vue + TypeScript + Electron)..."
echo "   Output directory: $FRONTED_DIR/release"

# 运行构建
echo "🎯 Target: ${1:-current OS}"

if [ "$1" == "win" ]; then
    npm run build -- --win
elif [ "$1" == "mac" ]; then
    npm run build -- --mac
elif [ "$1" == "all" ]; then
    npm run build -- --mac --win
else
    # 默认只打当前系统
    npm run build
fi

echo "✅ Build completed successfully!"
echo "📁 Artifacts are located in: $FRONTED_DIR/release/"
