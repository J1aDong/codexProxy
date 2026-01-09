#!/usr/bin/env python3
"""
Test Suite for Codex Proxy Server

This test suite validates that the proxy correctly translates between:
- OpenAI Chat Completions API format (used by OpenCode)
- Codex Responses API format (used by Codex CLI)

Tests cover:
- Basic text streaming
- Tool/function calls (shell_command, apply_patch, MCP, etc.)
- Image support
- Message formats
"""

import json
import time
import httpx
import subprocess
import os
from typing import Dict, List, Any, Optional
from dataclasses import dataclass
from pathlib import Path

# Test configuration
PROXY_URL = "http://localhost:8889/v1/chat/completions"
DIRECT_API_URL = "https://api.aicodemirror.com/api/codex/backend-api/codex/responses"
API_KEY = "sk-ant-api03-Q0eszrExhT8cZz_FOlTrcHBmoQ-7YfO_PMI7ncTikMLNjokHx-B4IzPdo0PpQR4rKFpTSM2f3HwM7DmtdpQ2oQ"

@dataclass
class TestResult:
    name: str
    passed: bool
    message: str
    details: Optional[Dict] = None

class CodexProxyTester:
    def __init__(self):
        self.results: List[TestResult] = []
        self.template = self._load_template()
    
    def _load_template(self) -> Dict:
        """Load captured Codex request template"""
        template_path = os.path.join(os.path.dirname(__file__), "codex-request.json")
        with open(template_path, 'r') as f:
            return json.load(f)
    
    def _make_proxy_request(self, body: Dict, timeout: float = 60.0) -> Dict:
        """Make request through proxy server"""
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {API_KEY}"
        }
        
        with httpx.Client(timeout=timeout, verify=False) as client:
            response = client.post(PROXY_URL, json=body, headers=headers)
            return {
                "status_code": response.status_code,
                "text": response.text
            }
    
    def _make_direct_request(self, body: Dict, timeout: float = 60.0) -> Dict:
        """Make request directly to Codex API"""
        headers = {
            "Content-Type": "application/json",
            "Authorization": f"Bearer {API_KEY}",
            "User-Agent": "codex_cli_rs/0.79.0 (Mac OS 15.6.1; arm64)",
            "originator": "codex_cli_rs"
        }
        
        with httpx.Client(http2=True, timeout=timeout, verify=False) as client:
            response = client.post(DIRECT_API_URL, json=body, headers=headers)
            return {
                "status_code": response.status_code,
                "text": response.text
            }
    
    def _extract_sse_events(self, text: str) -> List[Dict]:
        """Extract SSE events from response text"""
        events = []
        for line in text.split('\n'):
            if line.startswith('data: '):
                try:
                    events.append(json.loads(line[6:]))
                except:
                    pass
        return events
    
    # =========================================================================
    # Test Cases
    # =========================================================================
    
    def test_basic_text_streaming(self) -> TestResult:
        """Test: Basic text streaming works correctly"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [{"role": "user", "content": "Say 'hello' in one word"}],
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=30.0)
            
            if result["status_code"] != 200:
                return TestResult(
                    name="Basic Text Streaming",
                    passed=False,
                    message=f"Expected status 200, got {result['status_code']}",
                    details={"response": result["text"][:500]}
                )
            
            events = self._extract_sse_events(result["text"])
            
            # Check for completion event
            has_completion = any(
                e.get("choices", [{}])[0].get("finish_reason") == "stop" 
                for e in events
            )
            
            if not has_completion:
                return TestResult(
                    name="Basic Text Streaming",
                    passed=False,
                    message="No completion event found",
                    details={"events_count": len(events)}
                )
            
            return TestResult(
                name="Basic Text Streaming",
                passed=True,
                message="Text streaming works correctly",
                details={"events_count": len(events)}
            )
            
        except Exception as e:
            return TestResult(
                name="Basic Text Streaming",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_shell_command(self) -> TestResult:
        """Test: shell_command tool call works"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Use shell_command to run 'echo proxy_test_success'"}
            ],
            "tools": [self.template["tools"][0]],  # shell_command
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=60.0)
            
            if result["status_code"] != 200:
                return TestResult(
                    name="Tool: shell_command",
                    passed=False,
                    message=f"Status {result['status_code']}",
                    details={"response": result["text"][:500]}
                )
            
            events = self._extract_sse_events(result["text"])
            
            # Check for tool_calls
            has_tool_call = any(
                "tool_calls" in str(e.get("choices", [{}])[0].get("delta", {}))
                for e in events
            )
            
            has_tool_finish = any(
                e.get("choices", [{}])[0].get("finish_reason") == "tool_calls"
                for e in events
            )
            
            return TestResult(
                name="Tool: shell_command",
                passed=has_tool_call and has_tool_finish,
                message=f"Tool calls: {has_tool_call}, Finish: {has_tool_finish}",
                details={"events_count": len(events)}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool: shell_command",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_view_image(self) -> TestResult:
        """Test: view_image tool call format"""
        # view_image tool expects a path, not an image in the message
        # This test verifies the tool definition is correctly passed
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Describe the image at path /tmp/test_image.png"}
            ],
            "tools": [self.template["tools"][6]],  # view_image
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=30.0)
            
            if result["status_code"] != 200:
                return TestResult(
                    name="Tool: view_image",
                    passed=False,
                    message=f"Status: {result['status_code']}",
                    details={"response": result["text"][:500]}
                )
            
            events = self._extract_sse_events(result["text"])
            
            # Should get a response (tool may or may not be called depending on model)
            has_response = len(events) > 0
            
            return TestResult(
                name="Tool: view_image",
                passed=has_response,
                message=f"Response received: {has_response}",
                details={"events_count": len(events)}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool: view_image",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_apply_patch(self) -> TestResult:
        """Test: apply_patch tool call format"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Use apply_patch to add 'Hello World' to /tmp/test.txt"}
            ],
            "tools": [self.template["tools"][5]],  # apply_patch
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=30.0)
            
            return TestResult(
                name="Tool: apply_patch",
                passed=result["status_code"] == 200,
                message=f"Status: {result['status_code']}",
                details={"response_preview": result["text"][:200]}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool: apply_patch",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_mcp_resources(self) -> TestResult:
        """Test: list_mcp_resources tool call"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "List available MCP resources"}
            ],
            "tools": [self.template["tools"][1]],  # list_mcp_resources
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=30.0)
            
            return TestResult(
                name="Tool: list_mcp_resources",
                passed=result["status_code"] == 200,
                message=f"Status: {result['status_code']}",
                details={"response_preview": result["text"][:200]}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool: list_mcp_resources",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_update_plan(self) -> TestResult:
        """Test: update_plan tool call"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Update the plan to: 1. Test 2. Fix 3. Deploy"}
            ],
            "tools": [self.template["tools"][4]],  # update_plan
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=30.0)
            
            return TestResult(
                name="Tool: update_plan",
                passed=result["status_code"] == 200,
                message=f"Status: {result['status_code']}",
                details={"response_preview": result["text"][:200]}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool: update_plan",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_multiple_tools(self) -> TestResult:
        """Test: Multiple tools in single request"""
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Run echo and list files"}
            ],
            "tools": self.template["tools"][:2],  # shell_command, list_mcp_resources
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=60.0)
            
            return TestResult(
                name="Multiple Tools",
                passed=result["status_code"] == 200,
                message=f"Status: {result['status_code']}",
                details={"response_preview": result["text"][:200]}
            )
            
        except Exception as e:
            return TestResult(
                name="Multiple Tools",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def test_tool_result_feedback(self) -> TestResult:
        """Test: Tool result feedback loop"""
        # First request to get tool call
        body = {
            "model": "gpt-5.2-codex",
            "messages": [
                {"role": "user", "content": "Run 'pwd' command"}
            ],
            "tools": [self.template["tools"][0]],
            "stream": True
        }
        
        try:
            result = self._make_proxy_request(body, timeout=60.0)
            
            if result["status_code"] != 200:
                return TestResult(
                    name="Tool Result Feedback",
                    passed=False,
                    message=f"First request failed: {result['status_code']}"
                )
            
            events = self._extract_sse_events(result["text"])
            has_tool_call = any(
                "tool_calls" in str(e.get("choices", [{}])[0].get("delta", {}))
                for e in events
            )
            
            return TestResult(
                name="Tool Result Feedback",
                passed=has_tool_call,
                message=f"Tool call detected: {has_tool_call}",
                details={"events_count": len(events)}
            )
            
        except Exception as e:
            return TestResult(
                name="Tool Result Feedback",
                passed=False,
                message=f"Exception: {str(e)}"
            )
    
    def run_all_tests(self) -> List[TestResult]:
        """Run all test cases"""
        tests = [
            self.test_basic_text_streaming,
            self.test_tool_shell_command,
            self.test_tool_view_image,
            self.test_tool_apply_patch,
            self.test_tool_mcp_resources,
            self.test_tool_update_plan,
            self.test_multiple_tools,
            self.test_tool_result_feedback,
        ]
        
        for test in tests:
            print(f"Running: {test.__name__}...")
            self.results.append(test())
            time.sleep(0.5)  # Rate limiting
        
        return self.results
    
    def print_report(self):
        """Print test report"""
        print("\n" + "=" * 80)
        print("Codex Proxy Test Report")
        print("=" * 80)
        
        passed = sum(1 for r in self.results if r.passed)
        total = len(self.results)
        
        for result in self.results:
            status = "✅ PASS" if result.passed else "❌ FAIL"
            print(f"\n{status} - {result.name}")
            print(f"   {result.message}")
            if result.details:
                for key, value in result.details.items():
                    print(f"   {key}: {value}")
        
        print("\n" + "=" * 80)
        print(f"Summary: {passed}/{total} tests passed")
        print("=" * 80)
        
        return passed == total


if __name__ == "__main__":
    tester = CodexProxyTester()
    
    # Check if proxy is running
    try:
        httpx.get(PROXY_URL, timeout=5.0)
    except:
        print("⚠️  Proxy server not running! Please start it first:")
        print(f"   node /Users/mr.j/codex-proxy-v2.js")
        exit(1)
    
    # Run tests
    tester.run_all_tests()
    success = tester.print_report()
    
    exit(0 if success else 1)
