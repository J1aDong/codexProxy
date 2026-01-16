#!/bin/bash
set -e

# 获取脚本所在目录
BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
FRONTED_DIR="$BASE_DIR/fronted-tauri"
PACKAGE_JSON="$FRONTED_DIR/package.json"
TAURI_CONF="$FRONTED_DIR/src-tauri/tauri.conf.json"
CARGO_TOML="$FRONTED_DIR/src-tauri/Cargo.toml"

echo "=========================================="
echo "   🚀 Codex Proxy 构建与发布工具 (Tauri)  "
echo "=========================================="

# 1. 检查目录
if [ ! -d "$FRONTED_DIR" ]; then
    echo "❌ 错误: 未找到 'fronted-tauri' 目录。"
    exit 1
fi

# 2. 选择模式
echo "请选择操作类型:"
echo "  1) 仅本地构建 (生成安装包，不推送 Git)"
echo "  2) 发布到 GitHub (提交、打标签并推送，触发 Actions)"
read -p "选择 [1/2, 默认 1]: " MAIN_CHOICE
MAIN_CHOICE="${MAIN_CHOICE:-1}"

# 3. 版本处理逻辑
CURRENT_VERSION=$(node -p "require('$PACKAGE_JSON').version")
V_MAJOR=$(echo $CURRENT_VERSION | cut -d. -f1)
V_MINOR=$(echo $CURRENT_VERSION | cut -d. -f2)
V_PATCH=$(echo $CURRENT_VERSION | cut -d. -f3)
NEXT_PATCH=$((V_PATCH + 1))
DEFAULT_NEXT_VERSION="$V_MAJOR.$V_MINOR.$NEXT_PATCH"

echo ""
echo "📌 当前版本: $CURRENT_VERSION"

if [ "$MAIN_CHOICE" == "2" ]; then
    read -p "🖊️  输入新版本号 (直接回车使用 $DEFAULT_NEXT_VERSION): " INPUT_VERSION
    NEW_VERSION="${INPUT_VERSION:-$DEFAULT_NEXT_VERSION}"
else
    echo "💡 本地构建建议保持版本号不变或仅做本地调整。"
    read -p "🖊️  是否修改版本号? (输入新版本号，直接回车保持 $CURRENT_VERSION): " INPUT_VERSION
    NEW_VERSION="${INPUT_VERSION:-$CURRENT_VERSION}"
fi

# 更新版本号 (package.json, tauri.conf.json, Cargo.toml)
if [ "$NEW_VERSION" != "$CURRENT_VERSION" ]; then
    echo "📝 正在更新版本号..."

    # 更新 package.json
    node -e "
        const fs = require('fs');
        const pkg = require('$PACKAGE_JSON');
        pkg.version = '$NEW_VERSION';
        fs.writeFileSync('$PACKAGE_JSON', JSON.stringify(pkg, null, 2));
    "
    echo "  ✅ package.json"

    # 更新 tauri.conf.json
    node -e "
        const fs = require('fs');
        const conf = JSON.parse(fs.readFileSync('$TAURI_CONF', 'utf8'));
        conf.version = '$NEW_VERSION';
        fs.writeFileSync('$TAURI_CONF', JSON.stringify(conf, null, 2));
    "
    echo "  ✅ tauri.conf.json"

    # 更新 Cargo.toml
    sed -i '' "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
    echo "  ✅ Cargo.toml"

    echo "✅ 版本号已更新为 $NEW_VERSION"
fi

# 4. 执行发布逻辑 (仅模式 2)
if [ "$MAIN_CHOICE" == "2" ]; then
    echo ""
    echo "☁️  准备推送到 GitHub..."
    TAG_NAME="v$NEW_VERSION"

    echo "📦 暂存版本文件..."
    git add "$PACKAGE_JSON" "$TAURI_CONF" "$CARGO_TOML"

    echo "💾 正在提交变更..."
    git commit -m "chore: bump version to $NEW_VERSION" || echo "⚠️  没有需要提交的内容"

    # 处理标签冲突
    if git rev-parse "$TAG_NAME" >/dev/null 2>&1; then
        echo "⚠️  本地已存在标签 '$TAG_NAME'。"
        read -p "🔄 是否删除旧标签并重新创建? (y/N): " DELETE_TAG
        if [[ "$DELETE_TAG" =~ ^[Yy]$ ]]; then
            git tag -d "$TAG_NAME"
            echo "🗑️  本地旧标签已删除。"
            echo "🗑️  尝试删除远程标签 (如果存在)..."
            git push origin :refs/tags/"$TAG_NAME" || true
        else
            echo "❌ 操作终止。请手动处理标签冲突。"
            exit 1
        fi
    fi

    echo "🏷️  创建新标签 $TAG_NAME..."
    git tag "$TAG_NAME"

    echo "🚀 正在推送代码和标签到远程仓库..."
    git push origin main
    git push origin "$TAG_NAME"

    echo "✅ 成功! GitHub Actions 应该已经开始运行。"
    echo "👋 远程发布流程结束。"
    exit 0
fi

# 5. 执行本地构建逻辑 (模式 1)
echo ""
echo "🔨 开始本地构建流程..."
cd "$FRONTED_DIR"

# 检查依赖
if [ ! -d "node_modules" ]; then
    echo "⬇️  正在安装前端依赖..."
    npm install
fi

echo "请选择目标平台:"
echo "  1) 当前系统 (默认)"
echo "  2) macOS (Universal: Intel + Apple Silicon)"
echo "  3) macOS (仅 Apple Silicon)"
echo "  4) macOS (仅 Intel)"
read -p "选择 [1-4, 默认 1]: " PLATFORM_CHOICE

case $PLATFORM_CHOICE in
    2)
        echo "🏗️  构建 macOS Universal..."
        npm run tauri build -- --target universal-apple-darwin
        ;;
    3)
        echo "🏗️  构建 macOS Apple Silicon..."
        npm run tauri build -- --target aarch64-apple-darwin
        ;;
    4)
        echo "🏗️  构建 macOS Intel..."
        npm run tauri build -- --target x86_64-apple-darwin
        ;;
    *)
        echo "🏗️  构建当前系统..."
        npm run tauri build
        ;;
esac

echo ""
echo "✅ 本地构建完成!"
echo "📁 产物目录: $FRONTED_DIR/src-tauri/target/release/bundle"
