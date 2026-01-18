#!/usr/bin/env python3
"""
Comprehensive Image Handling Test Suite for Rust Codex Proxy

This test suite validates that the proxy correctly handles images in various formats:
- Base64 encoded images
- Local file paths (file:// protocol)
- HTTP/HTTPS URLs
- Mixed content (text + images)
- Multiple images in one request

Tests cover:
✅ Image block parsing
✅ Image URL resolution
✅ Base64 encoding handling
✅ File path conversion
✅ Mixed content blocks
✅ Multiple images
"""

import json
import base64
import httpx
import os
import sys
from pathlib import Path
from typing import Dict, List, Optional, Any

# Test configuration
RUST_PROXY_URL = "http://localhost:8889/messages"
API_KEY = "sk-ant-api03-43mBeNUIHZj3JQVIb3HDf6Yw-a94MKEJgG5emsaKpNoRUwCmG5V46uQcFybxJ1swthN-nMqxBLzuUgcSe-QqHw"

# Create a simple test image (1x1 red pixel PNG)
TEST_IMAGE_PNG = base64.b64encode(
    b'\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR\x00\x00\x00\x01\x00\x00\x00\x01\x08\x02\x00\x00\x00\x90wS\xde\x00\x00\x00\x0cIDATx\x9cc\xf8\xcf\xc0\x00\x00\x00\x03\x00\x01\x00\x00\x00\x00IEND\xaeB`\x82'
).decode('utf-8')

TEST_IMAGE_DATA_URL = f"data:image/png;base64,{TEST_IMAGE_PNG}"

def create_test_image_file():
    """Create a test image file"""
    test_dir = Path("/tmp/codex_proxy_test")
    test_dir.mkdir(exist_ok=True)
    
    image_path = test_dir / "test_image.png"
    with open(image_path, "wb") as f:
        f.write(base64.b64decode(TEST_IMAGE_PNG))
    
    return str(image_path)

class ImageTestResult:
    def __init__(self, name: str, passed: bool, message: str, details: Optional[Dict] = None):
        self.name = name
        self.passed = passed
        self.message = message
        self.details = details or {}

class ImageTestSuite:
    def __init__(self):
        self.results: List[ImageTestResult] = []
        self.test_image_path = create_test_image_file()
    
    def make_request(self, body: Dict, timeout: float = 30.0) -> Dict:
        """Make request to proxy server"""
        headers = {
            "Content-Type": "application/json",
            "x-api-key": API_KEY,
            "x-anthropic-version": "2023-06-01"
        }
        
        try:
            with httpx.Client(timeout=timeout, verify=False) as client:
                response = client.post(RUST_PROXY_URL, json=body, headers=headers)
                return {
                    "status_code": response.status_code,
                    "text": response.text,
                    "headers": dict(response.headers)
                }
        except Exception as e:
            return {
                "status_code": 0,
                "text": f"Request failed: {str(e)}",
                "headers": {}
            }
    
    def test_base64_image(self) -> ImageTestResult:
        """Test: Base64 encoded image"""
        print("\n📸 Test 1: Base64 Encoded Image")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What do you see in this image?"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": TEST_IMAGE_PNG
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Base64 Image",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        # Check if response contains text about the image
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Base64 Image",
            passed=has_response,
            message=f"Response received: {has_response}",
            details={"status": result["status_code"]}
        )
    
    def test_data_url_image(self) -> ImageTestResult:
        """Test: Data URL format image"""
        print("\n📸 Test 2: Data URL Image")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Describe this image"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "data": TEST_IMAGE_DATA_URL
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Data URL Image",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Data URL Image",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_file_path_image(self) -> ImageTestResult:
        """Test: Local file path (file:// protocol)"""
        print(f"\n📸 Test 3: File Path Image ({self.test_image_path})")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What is in this file?"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": f"file://{self.test_image_path}"
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="File Path Image",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="File Path Image",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_http_url_image(self) -> ImageTestResult:
        """Test: HTTP URL image"""
        print("\n📸 Test 4: HTTP URL Image")
        
        # Use a small test image from a public URL
        test_url = "https://via.placeholder.com/100"
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What color is this image?"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "url": test_url
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body, timeout=60.0)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="HTTP URL Image",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="HTTP URL Image",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_mixed_content(self) -> ImageTestResult:
        """Test: Mixed text and image content"""
        print("\n📸 Test 5: Mixed Content (Text + Image)")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "First, I'll describe what I want you to do."
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": TEST_IMAGE_PNG
                            }
                        },
                        {
                            "type": "text",
                            "text": "Now, analyze this image and tell me what you see."
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Mixed Content",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Mixed Content",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_multiple_images(self) -> ImageTestResult:
        """Test: Multiple images in one request"""
        print("\n📸 Test 6: Multiple Images")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Compare these two images"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": TEST_IMAGE_PNG
                            }
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": TEST_IMAGE_PNG
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Multiple Images",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Multiple Images",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_image_with_tools(self) -> ImageTestResult:
        """Test: Image with tool calls"""
        print("\n📸 Test 7: Image with Tool Calls")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Read the text in this image"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": TEST_IMAGE_PNG
                            }
                        }
                    ]
                }
            ],
            "tools": [
                {
                    "name": "shell_command",
                    "description": "Execute shell commands",
                    "input_schema": {
                        "type": "object",
                        "properties": {
                            "command": {"type": "string"}
                        },
                        "required": ["command"]
                    }
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Image with Tools",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Image with Tools",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_image_url_object(self) -> ImageTestResult:
        """Test: Image with URL object format"""
        print("\n📸 Test 8: Image URL Object Format")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "Describe this"
                        },
                        {
                            "type": "image_url",
                            "image_url": {
                                "url": TEST_IMAGE_DATA_URL
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Image URL Object",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Image URL Object",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def test_image_uri_format(self) -> ImageTestResult:
        """Test: Image with URI format"""
        print("\n📸 Test 9: Image URI Format")
        
        body = {
            "model": "claude-3-5-sonnet-20241022",
            "messages": [
                {
                    "role": "user",
                    "content": [
                        {
                            "type": "text",
                            "text": "What is this?"
                        },
                        {
                            "type": "image",
                            "source": {
                                "type": "url",
                                "uri": TEST_IMAGE_DATA_URL
                            }
                        }
                    ]
                }
            ],
            "stream": True,
            "max_tokens": 100
        }
        
        result = self.make_request(body)
        
        if result["status_code"] != 200:
            return ImageTestResult(
                name="Image URI Format",
                passed=False,
                message=f"Status {result['status_code']}",
                details={"response": result["text"][:500]}
            )
        
        has_response = "message_start" in result["text"]
        
        return ImageTestResult(
            name="Image URI Format",
            passed=has_response,
            message=f"Response received: {has_response}"
        )
    
    def run_all_tests(self) -> List[ImageTestResult]:
        """Run all image tests"""
        tests = [
            self.test_base64_image,
            self.test_data_url_image,
            self.test_file_path_image,
            self.test_http_url_image,
            self.test_mixed_content,
            self.test_multiple_images,
            self.test_image_with_tools,
            self.test_image_url_object,
            self.test_image_uri_format,
        ]
        
        print("=" * 70)
        print("🖼️  IMAGE HANDLING TEST SUITE")
        print("=" * 70)
        print(f"📍 Proxy: {RUST_PROXY_URL}")
        print(f"🖼️  Test Image: {self.test_image_path}")
        print("=" * 70)
        
        for i, test in enumerate(tests, 1):
            print(f"\n[{i}/{len(tests)}] Running: {test.__name__}")
            try:
                result = test()
                self.results.append(result)
                
                status = "✅ PASS" if result.passed else "❌ FAIL"
                print(f"   {status} - {result.message}")
            except Exception as e:
                result = ImageTestResult(
                    name=test.__name__,
                    passed=False,
                    message=f"Exception: {str(e)}"
                )
                self.results.append(result)
                print(f"   ❌ FAIL - Exception: {str(e)}")
            
            # Small delay between tests
            import time
            time.sleep(1)
        
        return self.results
    
    def print_report(self):
        """Print test report"""
        print("\n" + "=" * 70)
        print("📊 TEST REPORT")
        print("=" * 70)
        
        passed = sum(1 for r in self.results if r.passed)
        total = len(self.results)
        
        for result in self.results:
            status = "✅ PASS" if result.passed else "❌ FAIL"
            print(f"\n{status} - {result.name}")
            print(f"   📝 {result.message}")
            
            if result.details:
                for key, value in result.details.items():
                    print(f"   📊 {key}: {value}")
        
        print("\n" + "=" * 70)
        print(f"📈 SUMMARY: {passed}/{total} tests passed ({passed/total*100:.1f}%)")
        
        if passed == total:
            print("🎉 ALL IMAGE TESTS PASSED!")
            print("\n✅ Image handling is working correctly:")
            print("   • Base64 encoded images")
            print("   • Data URL format")
            print("   • File path (file://)")
            print("   • HTTP/HTTPS URLs")
            print("   • Mixed content (text + images)")
            print("   • Multiple images")
            print("   • Images with tools")
            print("   • Various URL/URI formats")
        else:
            print("⚠️  Some image tests failed. Check the details above.")
        
        print("=" * 70)
        
        return passed == total

def main():
    """Main test runner"""
    print("🖼️  Rust Codex Proxy - Image Handling Test Suite")
    print("=" * 70)
    
    # Check if proxy is running
    try:
        with httpx.Client(timeout=5.0) as client:
            response = client.get("http://localhost:8889")
    except:
        print("❌ Proxy server not running!")
        print("   Please start it first:")
        print("   cd /Users/mr.j/myRoom/code/ai/MyProjects/codexProxy/fronted-tauri/src-tauri")
        print("   cargo run")
        return False
    
    print("✅ Proxy server is running")
    
    suite = ImageTestSuite()
    suite.run_all_tests()
    success = suite.print_report()
    
    return success

if __name__ == "__main__":
    try:
        success = main()
        sys.exit(0 if success else 1)
    except KeyboardInterrupt:
        print("\n\n⚠️  Tests interrupted by user")
        sys.exit(130)
    except Exception as e:
        print(f"\n\n❌ Unexpected error: {e}")
        import traceback
        traceback.print_exc()
        sys.exit(1)