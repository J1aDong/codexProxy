#!/usr/bin/env node

/**
 * Claude Proxy Debug - 直接访问 Claude API 并记录详细日志
 * 
 * 用于调试 Claude Code 如何调用 skill
 * 日志格式：curl 命令 + JSON 响应 + 时间戳
 */

const http = require("http");
const https = require("https");
const fs = require("fs");
const path = require("path");

process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

const PORT = 8890;
const LOG_DIR = "/Users/mr.j/myRoom/code/ai/codexProxy/logs";
const BASE_URL = "api.aicodemirror.com";
const BASE_PATH = "/api/claudecode";

if (!fs.existsSync(LOG_DIR)) {
  fs.mkdirSync(LOG_DIR, { recursive: true });
}

const SESSION_LOG_FILE = path.join(LOG_DIR, `claude_skill_${new Date().toISOString().replace(/[:.]/g, "-")}.log`);

function timestamp() {
  return new Date().toISOString();
}

// 格式化为 curl 命令
function formatCurl(method, url, headers, body) {
  let curl = `curl -X ${method} '${url}'`;
  
  for (const [key, value] of Object.entries(headers)) {
    if (key.toLowerCase() === "content-length") continue;
    // 隐藏敏感信息
    let displayValue = value;
    if (key.toLowerCase().includes("key") || key.toLowerCase().includes("auth")) {
      displayValue = value.substring(0, 20) + "..." + value.substring(value.length - 5);
    }
    curl += ` \\\n  -H '${key}: ${displayValue}'`;
  }
  
  if (body) {
    // 美化 JSON
    try {
      const parsed = JSON.parse(body);
      curl += ` \\\n  -d '${JSON.stringify(parsed, null, 2)}'`;
    } catch {
      curl += ` \\\n  -d '${body}'`;
    }
  }
  
  return curl;
}

// 格式化 JSON 响应
function formatJson(data) {
  try {
    if (typeof data === "string") {
      return JSON.stringify(JSON.parse(data), null, 2);
    }
    return JSON.stringify(data, null, 2);
  } catch {
    return data;
  }
}

function writeLog(content) {
  fs.appendFileSync(SESSION_LOG_FILE, content + "\n");
}

// HTTP 服务器
const server = http.createServer((req, res) => {
  // 支持 /v1/messages 和 /messages
  if (req.method === "POST" && (req.url.includes("/messages") || req.url.includes("/v1/messages"))) {
    let body = "";
    req.on("data", chunk => { body += chunk.toString(); });
    req.on("end", () => {
      const startTime = Date.now();
      
      try {
        // 构建目标 URL
        const targetUrl = `https://${BASE_URL}${BASE_PATH}${req.url}`;
        
        // 收集请求头
        const forwardHeaders = {};
        const headersToCopy = [
          "content-type",
          "authorization",
          "x-api-key",
          "api-key",
          "anthropic-version",
          "x-anthropic-version",
          "anthropic-beta",
          "accept"
        ];
        
        for (const h of headersToCopy) {
          if (req.headers[h]) {
            forwardHeaders[h] = req.headers[h];
          }
        }
        
        // 确保有 content-type
        if (!forwardHeaders["content-type"]) {
          forwardHeaders["content-type"] = "application/json";
        }
        
        // 记录请求日志
        let logContent = "";
        logContent += `${"=".repeat(80)}\n`;
        logContent += `[${timestamp()}] REQUEST\n`;
        logContent += `${"=".repeat(80)}\n\n`;
        logContent += `# CURL Command:\n`;
        logContent += formatCurl("POST", targetUrl, forwardHeaders, body);
        logContent += `\n\n`;
        logContent += `# Request Body (JSON):\n`;
        logContent += formatJson(body);
        logContent += `\n\n`;
        
        writeLog(logContent);
        
        // 解析请求体
        let requestBody;
        try {
          requestBody = JSON.parse(body);
        } catch {
          requestBody = {};
        }
        
        console.log(`\n[${timestamp()}] 📥 Incoming Request`);
        console.log(`  URL: ${req.url}`);
        console.log(`  Model: ${requestBody.model || "unknown"}`);
        console.log(`  Messages: ${requestBody.messages?.length || 0}`);
        console.log(`  Tools: ${requestBody.tools?.length || 0}`);
        console.log(`  Stream: ${requestBody.stream}`);
        
        // 转发请求
        const urlObj = new URL(targetUrl);
        const options = {
          hostname: urlObj.hostname,
          port: 443,
          path: urlObj.pathname + urlObj.search,
          method: "POST",
          headers: {
            ...forwardHeaders,
            "Content-Length": Buffer.byteLength(body)
          }
        };
        
        const proxyReq = https.request(options, (proxyRes) => {
          console.log(`[${timestamp()}] 📤 Response Status: ${proxyRes.statusCode}`);
          
          // 记录响应头
          let responseLog = "";
          responseLog += `${"=".repeat(80)}\n`;
          responseLog += `[${timestamp()}] RESPONSE (Status: ${proxyRes.statusCode})\n`;
          responseLog += `${"=".repeat(80)}\n\n`;
          responseLog += `# Response Headers:\n`;
          responseLog += JSON.stringify(proxyRes.headers, null, 2);
          responseLog += `\n\n`;
          responseLog += `# Response Body:\n`;
          
          writeLog(responseLog);
          
          // 设置响应头
          const responseHeaders = {
            "Content-Type": proxyRes.headers["content-type"] || "text/event-stream",
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "Access-Control-Allow-Origin": "*"
          };
          
          res.writeHead(proxyRes.statusCode, responseHeaders);
          
          // 收集并转发响应
          let responseBuffer = "";
          let eventCount = 0;
          
          proxyRes.on("data", chunk => {
            const chunkStr = chunk.toString();
            responseBuffer += chunkStr;
            
            // 解析 SSE 事件
            const lines = chunkStr.split("\n");
            for (const line of lines) {
              if (line.startsWith("event:")) {
                eventCount++;
                const eventType = line.substring(6).trim();
                console.log(`  [Event ${eventCount}] ${eventType}`);
              }
              if (line.startsWith("data:")) {
                try {
                  const data = JSON.parse(line.substring(5).trim());
                  // 记录关键事件
                  if (data.type) {
                    const logLine = `[${timestamp()}] Event: ${data.type}\n${formatJson(data)}\n\n`;
                    fs.appendFileSync(SESSION_LOG_FILE, logLine);
                    
                    // 特别关注 tool_use 相关事件
                    if (data.type.includes("tool") || 
                        (data.content_block && data.content_block.type === "tool_use") ||
                        (data.delta && data.delta.type === "input_json_delta")) {
                      console.log(`  🔧 Tool Event: ${data.type}`);
                      if (data.content_block) {
                        console.log(`     Name: ${data.content_block.name}`);
                        console.log(`     ID: ${data.content_block.id}`);
                      }
                    }
                  }
                } catch {}
              }
            }
            
            // 转发给客户端
            res.write(chunk);
          });
          
          proxyRes.on("end", () => {
            const duration = Date.now() - startTime;
            
            // 记录完成信息
            let endLog = "";
            endLog += `\n${"=".repeat(80)}\n`;
            endLog += `[${timestamp()}] COMPLETED\n`;
            endLog += `${"=".repeat(80)}\n`;
            endLog += `Duration: ${duration}ms\n`;
            endLog += `Events: ${eventCount}\n`;
            endLog += `Response Size: ${responseBuffer.length} bytes\n`;
            
            writeLog(endLog);
            
            console.log(`[${timestamp()}] ✅ Request completed in ${duration}ms`);
            console.log(`  Events: ${eventCount}`);
            
            res.end();
          });
        });
        
        proxyReq.on("error", (error) => {
          const errorLog = `\n[${timestamp()}] ERROR: ${error.message}\n`;
          writeLog(errorLog);
          
          console.error(`[${timestamp()}] ❌ Error: ${error.message}`);
          res.writeHead(500, { "Content-Type": "application/json" });
          res.end(JSON.stringify({ error: { message: error.message } }));
        });
        
        proxyReq.write(body);
        proxyReq.end();
        
      } catch (error) {
        const errorLog = `\n[${timestamp()}] PARSE ERROR: ${error.message}\n`;
        writeLog(errorLog);
        
        console.error(`[${timestamp()}] ❌ Parse Error: ${error.message}`);
        res.writeHead(400, { "Content-Type": "application/json" });
        res.end(JSON.stringify({ error: { message: error.message } }));
      }
    });
  } else if (req.method === "OPTIONS") {
    // CORS preflight
    res.writeHead(200, {
      "Access-Control-Allow-Origin": "*",
      "Access-Control-Allow-Methods": "POST, OPTIONS",
      "Access-Control-Allow-Headers": "Content-Type, Authorization, x-api-key, anthropic-version, x-anthropic-version, anthropic-beta"
    });
    res.end();
  } else {
    res.writeHead(404, { "Content-Type": "application/json" });
    res.end(JSON.stringify({ error: { type: "not_found", message: "Not found" } }));
  }
});

server.listen(PORT, () => {
  console.log(`\n${"=".repeat(60)}`);
  console.log(`🔍 Claude Proxy Debug Server`);
  console.log(`${"=".repeat(60)}`);
  console.log(`\n📡 Listening: http://localhost:${PORT}/v1/messages`);
  console.log(`🎯 Target: https://${BASE_URL}${BASE_PATH}`);
  console.log(`📁 Log: ${SESSION_LOG_FILE}`);
  console.log(`\n📝 配置示例:`);
  console.log(`   ANTHROPIC_BASE_URL=http://localhost:${PORT}`);
  console.log(`\n`);
});
