#!/usr/bin/env python3
"""
测试脚本：对比JavaScript和Rust版本发送的请求格式
"""

import requests
import json

API_KEY = "sk-ant-api03-43mBeNUIHZj3JQVIb3HDf6Yw-a94MKEJgG5emsaKpNoRUwCmG5V46uQcFybxJ1swthN-nMqxBLzuUgcSe-QqHw"

# 测试用例1：小图片（110字节的黑色像素）
small_image_data = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="

def construct_request(image_data):
    """构造发送到Rust代理的请求"""
    return {
        "model": "claude-sonnet-4-5-20250929",
        "max_tokens": 4096,
        "messages": [
            {
                "role": "user",
                "content": [
                    {
                        "type": "text",
                        "text": "这是什么颜色？"
                    },
                    {
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": "image/png",
                            "data": image_data
                        }
                    }
                ]
            }
        ]
    }

def test_rust_proxy():
    """测试Rust代理"""
    url = "http://localhost:8889/v1/messages"
    headers = {
        "Content-Type": "application/json",
        "x-api-key": API_KEY
    }

    request_data = construct_request(small_image_data)

    print("=" * 80)
    print("发送到Rust代理的请求:")
    print("=" * 80)
    print(json.dumps(request_data, indent=2, ensure_ascii=False))
    print("=" * 80)

    try:
        response = requests.post(url, json=request_data, headers=headers, timeout=30)
        print(f"\n状态码: {response.status_code}")

        if response.status_code == 200:
            # 处理流式响应
            for line in response.iter_lines():
                if line:
                    line_str = line.decode('utf-8')
                    if line_str.startswith('data: '):
                        data_str = line_str[6:]  # 去掉 'data: ' 前缀
                        if data_str.strip() == '[DONE]':
                            continue
                        try:
                            data = json.loads(data_str)
                            if data.get('type') == 'content_block_delta':
                                delta = data.get('delta', {})
                                if delta.get('type') == 'text_delta':
                                    print(delta.get('text', ''), end='', flush=True)
                        except json.JSONDecodeError:
                            pass
            print("\n")
        else:
            print(f"错误: {response.text}")
    except Exception as e:
        print(f"请求失败: {e}")

if __name__ == "__main__":
    test_rust_proxy()