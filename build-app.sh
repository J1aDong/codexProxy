#!/bin/bash
set -e

# 获取脚本所在目录
BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
FRONTED_DIR="$BASE_DIR/fronted-tauri"
PACKAGE_JSON="$FRONTED_DIR/package.json"
TAURI_CONF="$FRONTED_DIR/src-tauri/tauri.conf.json"
CARGO_TOML="$FRONTED_DIR/src-tauri/Cargo.toml"
OS_NAME="$(uname -s)"

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

    # 更新 Cargo.toml (跨平台，兼容 macOS/Linux)
    sed -i.bak "s/^version = \".*\"/version = \"$NEW_VERSION\"/" "$CARGO_TOML"
    rm -f "${CARGO_TOML}.bak"
    echo "  ✅ Cargo.toml"

    echo "✅ 版本号已更新为 $NEW_VERSION"
fi

# 4. 执行发布逻辑 (仅模式 2)
if [ "$MAIN_CHOICE" == "2" ]; then
    echo ""
    echo "☁️  准备推送到 GitHub..."
    TAG_NAME="v$NEW_VERSION"

    # 输入本次更新的 changelog
    echo ""
    echo "📝 请输入本次更新的主要内容 (用于 GitHub Release Notes):"
    echo "   - 每行一个更新点，以 '-' 开头"
    echo "   - 直接回车跳过 (将仅基于 commit 历史自动生成)"
    echo "   - 输入 'END' 结束输入"
    CHANGELOG_INPUT=""
    while true; do
        read -p "> " changelog_line
        if [ "$changelog_line" = "END" ] || [ -z "$changelog_line" ]; then
            break
        fi
        if [ -n "$CHANGELOG_INPUT" ]; then
            CHANGELOG_INPUT="${CHANGELOG_INPUT}"$'\n'"${changelog_line}"
        else
            CHANGELOG_INPUT="${changelog_line}"
        fi
    done

    echo "📦 暂存全部变更..."
    git add -A

    echo "💾 正在提交变更..."
    # 构建 commit message：如果有手动输入的 changelog，则添加到 commit body
    if [ -n "$CHANGELOG_INPUT" ]; then
        git commit -m "chore: bump version to $NEW_VERSION" -m "## 更新说明"$'\n'"${CHANGELOG_INPUT}" || echo "⚠️  没有需要提交的内容"
    else
        git commit -m "chore: bump version to $NEW_VERSION" || echo "⚠️  没有需要提交的内容"
    fi

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
if [ "$OS_NAME" = "Darwin" ]; then
    echo "  2) macOS (Universal: Intel + Apple Silicon)"
    echo "  3) macOS (仅 Apple Silicon)"
    echo "  4) macOS (仅 Intel)"
    echo "  5) Linux (AppImage + DEB + RPM) [需要 Linux 环境]"
    read -p "选择 [1-5, 默认 1]: " PLATFORM_CHOICE
elif [ "$OS_NAME" = "Linux" ]; then
    echo "  2) Linux (AppImage + DEB + RPM)"
    read -p "选择 [1-2, 默认 1]: " PLATFORM_CHOICE
else
    read -p "选择 [1, 默认 1]: " PLATFORM_CHOICE
fi

case $PLATFORM_CHOICE in
    2)
        if [ "$OS_NAME" = "Darwin" ]; then
            echo "🏗️  构建 macOS Universal..."
            npm run tauri build -- --target universal-apple-darwin
        elif [ "$OS_NAME" = "Linux" ]; then
            echo "🏗️  构建 Linux (AppImage + DEB + RPM)..."
            npm run tauri build -- --bundles appimage,deb,rpm
        else
            echo "⚠️  当前系统不支持该选项，改为构建当前系统。"
            npm run tauri build
        fi
        ;;
    3)
        if [ "$OS_NAME" = "Darwin" ]; then
            echo "🏗️  构建 macOS Apple Silicon..."
            npm run tauri build -- --target aarch64-apple-darwin
        else
            echo "❌ 当前系统不支持该选项。"
            exit 1
        fi
        ;;
    4)
        if [ "$OS_NAME" = "Darwin" ]; then
            echo "🏗️  构建 macOS Intel..."
            npm run tauri build -- --target x86_64-apple-darwin
        else
            echo "❌ 当前系统不支持该选项。"
            exit 1
        fi
        ;;
    5)
        if [ "$OS_NAME" = "Darwin" ]; then
            echo "❌ 不建议在 macOS 上直接交叉打包 Linux。请在 Linux 主机或 GitHub Ubuntu Runner 上构建。"
            exit 1
        else
            echo "❌ 当前系统不支持该选项。"
            exit 1
        fi
        ;;
    *)
        echo "🏗️  构建当前系统..."
        npm run tauri build
        ;;
esac

echo ""
echo "✅ 本地构建完成!"
echo "📁 产物目录: $FRONTED_DIR/src-tauri/target/release/bundle"
