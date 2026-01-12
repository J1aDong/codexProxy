#!/bin/bash
set -e

# 获取脚本所在目录
BASE_DIR="$(cd "$(dirname "$0")" && pwd)"
FRONTED_DIR="$BASE_DIR/fronted"
PACKAGE_JSON="$FRONTED_DIR/package.json"

echo "=========================================="
echo "   🚀 Codex Proxy Build & Release Tool    "
echo "=========================================="

# 1. 检查目录
if [ ! -d "$FRONTED_DIR" ]; then
    echo "❌ Error: 'fronted' directory not found."
    exit 1
fi

# 2. 读取当前版本
CURRENT_VERSION=$(node -p "require('$PACKAGE_JSON').version")

# 计算建议的下一个版本 (Patch + 1)
V_MAJOR=$(echo $CURRENT_VERSION | cut -d. -f1)
V_MINOR=$(echo $CURRENT_VERSION | cut -d. -f2)
V_PATCH=$(echo $CURRENT_VERSION | cut -d. -f3)
NEXT_PATCH=$((V_PATCH + 1))
DEFAULT_NEXT_VERSION="$V_MAJOR.$V_MINOR.$NEXT_PATCH"

echo "📌 Current Version: $CURRENT_VERSION"
read -p "🖊️  Enter new version (Press Enter for $DEFAULT_NEXT_VERSION): " INPUT_VERSION
NEW_VERSION="${INPUT_VERSION:-$DEFAULT_NEXT_VERSION}"

echo "🎯 Target Version: $NEW_VERSION"
echo ""

# 3. 更新 package.json
if [ "$NEW_VERSION" != "$CURRENT_VERSION" ]; then
    echo "📝 Updating package.json..."
    # 使用 node 更新文件以保持格式
    node -e "
        const fs = require('fs');
        const pkg = require('$PACKAGE_JSON');
        pkg.version = '$NEW_VERSION';
        fs.writeFileSync('$PACKAGE_JSON', JSON.stringify(pkg, null, 2));
    "
    echo "✅ Version updated in package.json"
else
    echo "ℹ️  Version unchanged."
fi

echo ""

# 4. Git 操作 (Tag & Push)
read -p "☁️  Do you want to commit, tag 'v$NEW_VERSION' and push to trigger GitHub Actions? (y/N) " DO_GIT

if [[ "$DO_GIT" =~ ^[Yy]$ ]]; then
    TAG_NAME="v$NEW_VERSION"
    
    echo "📦 Staging package.json..."
    git add "$PACKAGE_JSON"
    
    # 提交 (如果版本没变，commit 可能会空，允许失败)
    echo "💾 Committing..."
    git commit -m "chore: bump version to $NEW_VERSION" || echo "⚠️  Nothing to commit"

    # 处理 Tag 冲突
    if git rev-parse "$TAG_NAME" >/dev/null 2>&1; then
        echo "⚠️  Tag '$TAG_NAME' already exists locally."
        read -p "🔄 Delete old tag and recreate? (y/N) " DELETE_TAG
        if [[ "$DELETE_TAG" =~ ^[Yy]$ ]]; then
            git tag -d "$TAG_NAME"
            echo "🗑️  Old local tag deleted."
            
            # 尝试删除远程 tag (忽略错误，因为可能远程不存在)
            echo "🗑️  Attempting to delete remote tag (if exists)..."
            git push origin :refs/tags/"$TAG_NAME" || true
        else
            echo "❌ Aborted. Please handle tag conflict manually."
            exit 1
        fi
    fi

    echo "🏷️  Creating tag $TAG_NAME..."
    git tag "$TAG_NAME"

    echo "🚀 Pushing code and tags to remote..."
    git push origin main
    git push origin "$TAG_NAME"

    echo "✅ Done! GitHub Actions should be running now."
else
    echo "⏭️  Skipping Git operations."
fi

echo ""

# 5. 本地构建 (可选)
read -p "🔨 Do you also want to build locally? (y/N) " DO_BUILD

if [[ "$DO_BUILD" =~ ^[Yy]$ ]]; then
    cd "$FRONTED_DIR"
    
    echo "Select target platform:"
    echo "  1) Current OS (default)"
    echo "  2) macOS Only (mac)"
    echo "  3) Windows Only (win)"
    echo "  4) All Platforms (mac + win)"
    read -p "Choice [1]: " PLATFORM_CHOICE

    ARGS=""
    case $PLATFORM_CHOICE in
        2) ARGS="--mac";;
        3) ARGS="--win";;
        4) ARGS="--mac --win";;
        *) ARGS="";;
    esac

    echo "🏗️  Starting build with args: $ARGS"
    npm run build -- $ARGS
    
    echo "✅ Local build completed!"
    echo "📁 Output: $FRONTED_DIR/release"
else
    echo "👋 Bye!"
fi