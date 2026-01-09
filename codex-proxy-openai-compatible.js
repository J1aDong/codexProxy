#!/usr/bin/env node

const http = require("http");
const https = require("https");
const { Transform, PassThrough } = require("stream");
const { randomUUID } = require("crypto");
const fs = require("fs");
const path = require("path");

process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

const PORT = 8889;

// 加载模板文件（支持相对路径和绝对路径）
const TEMPLATE_PATH = path.resolve(__dirname, "codex-request.json");
const TEMPLATE = JSON.parse(fs.readFileSync(TEMPLATE_PATH, "utf8"));

// 转换 OpenAI 工具格式为 Codex 格式
function transformTools(openaiTools) {
  if (!openaiTools || openaiTools.length === 0) return [];
  
  return openaiTools.map(tool => {
    if (tool.type === "function") {
      return {
        type: "function",
        name: tool.function?.name || tool.name,
        description: tool.function?.description || tool.description,
        strict: false,
        parameters: tool.function?.parameters || tool.parameters
      };
    }
    return tool;
  });
}

// 转换消息（包括工具调用、工具结果和图片）
function transformMessages(messages) {
  return messages.map(m => {
    // 用户/助手消息
    if (m.role === "user" || m.role === "assistant") {
      let content = m.content;
      
      // 处理内容中的图片 - 多种格式
      if (typeof content === "object") {
        // 格式1: { type: "image_url", image_url: { url: "..." } }
        if (content.type === "image_url" && content.image_url) {
          content = [{
            type: "input_image",
            image_url: content.image_url.url || content.image_url
          }];
        }
        // 格式2: OpenCode 可能直接发送图片对象
        else if (content.type === "image" || content.image) {
          const imgData = content.image || content;
          content = [{
            type: "input_image",
            image_url: imgData.url || imgData.data || JSON.stringify(imgData)
          }];
        }
        // 格式3: 已经是数组
        else if (Array.isArray(content)) {
          content = content.map(item => {
            if (typeof item === "object" && item.type === "image_url") {
              return {
                type: "input_image",
                image_url: item.image_url?.url || item.image_url
              };
            }
            return item;
          });
        }
      }
      
      return {
        type: "message",
        role: m.role,
        content: Array.isArray(content) ? content : [{
          type: m.role === "user" ? "input_text" : "output_text",
          text: typeof content === "string" ? content : JSON.stringify(content)
        }]
      };
    }
    
    // 工具消息
    if (m.role === "tool") {
      return {
        type: "function_call_result",
        id: m.tool_call_id,
        content: [{
          type: "output_text",
          text: m.content
        }]
      };
    }
    
    return m;
  });
}

function transformRequest(chatBody) {
  const { model, messages = [], stream = true, tools, tool_choice = "auto", reasoning_effort } = chatBody;
  const sessionId = randomUUID();
  
  // 转换消息
  const input = transformMessages(messages);
  
  // 转换工具
  const transformedTools = transformTools(tools);
  
  // 构建 reasoning 对象（支持 reasoning_effort 参数）
  let reasoning = TEMPLATE.reasoning;
  if (reasoning_effort) {
    // mapping: low/medium/high/xhigh/xlow -> Codex effort values
    const effortMap = {
      "xlow": "low",
      "low": "low", 
      "medium": "medium",
      "high": "high",
      "xhigh": "high"
    };
    reasoning = {
      effort: effortMap[reasoning_effort] || reasoning_effort,
      summary: TEMPLATE.reasoning?.summary || "auto"
    };
  }
  
  return {
    body: {
      model: model || TEMPLATE.model,
      instructions: TEMPLATE.instructions,
      input,
      tools: transformedTools.length > 0 ? transformedTools : TEMPLATE.tools,
      tool_choice,
      parallel_tool_calls: true,
      reasoning,
      store: false,
      stream,
      include: TEMPLATE.include,
      prompt_cache_key: sessionId
    },
    sessionId
  };
}

// SSE 转换：处理所有事件类型
class CodexToCompletionsTransform extends Transform {
  constructor() {
    super();
    this.buffer = "";
    this.messageId = "chatcmpl-" + Date.now();
    this.created = Math.floor(Date.now() / 1000);
    this.toolCallId = null;
    this.toolName = null;
    this.toolArguments = "";
    this.currentFunctionCall = null;
  }
  
  _transform(chunk, encoding, callback) {
    this.buffer += chunk.toString();
    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() || "";
    
    for (const line of lines) {
      if (!line.startsWith("data: ")) continue;
      
      try {
        const data = JSON.parse(line.slice(6));
        const etype = data.type || "";
        
        // 处理文本输出
        if (etype === "response.output_text.delta") {
          this.push("data: " + JSON.stringify({
            id: this.messageId,
            object: "chat.completion.chunk",
            created: this.created,
            model: "gpt-5.2-codex",
            choices: [{
              index: 0,
              delta: {
                content: [{
                  type: "text",
                  text: data.delta
                }]
              },
              finish_reason: null
            }]
          }) + "\n\n");
        }
        
        // 处理工具调用开始 (Codex: output_item.added with type: function_call)
        if (etype === "response.output_item.added") {
          const item = data.item || {};
          if (item.type === "function_call") {
            this.currentFunctionCall = item;
            this.toolCallId = item.call_id || "tool_" + Date.now();
            this.toolName = item.name;
            this.toolArguments = "";
            
            // 发送 OpenAI 格式的 tool_call
            this.push("data: " + JSON.stringify({
              id: this.messageId,
              object: "chat.completion.chunk",
              created: this.created,
              model: "gpt-5.2-codex",
              choices: [{
                index: 0,
                delta: {
                  role: "assistant",
                  tool_calls: [{
                    id: this.toolCallId,
                    type: "function",
                    function: {
                      name: this.toolName,
                      arguments: ""
                    }
                  }]
                },
                finish_reason: null
              }]
            }) + "\n\n");
          }
        }
        
        // 处理工具调用参数增量
        if (etype === "response.function_call_arguments.delta" || etype === "response.function_call_arguments_delta") {
          const delta = data.delta || data.arguments || "";
          this.toolArguments += delta;
          
          this.push("data: " + JSON.stringify({
            id: this.messageId,
            object: "chat.completion.chunk",
            created: this.created,
            model: "gpt-5.2-codex",
            choices: [{
              index: 0,
              delta: {
                tool_calls: [{
                  id: this.toolCallId,
                  type: "function",
                  function: {
                    name: this.toolName,
                    arguments: delta
                  }
                }]
              },
              finish_reason: null
            }]
          }) + "\n\n");
        }
        
        // 处理工具调用完成 (Codex: output_item.done)
        if (etype === "response.output_item.done") {
          if (this.currentFunctionCall && this.currentFunctionCall.type === "function_call") {
            this.currentFunctionCall = null;
            
            // 发送 finish_reason 为 tool_calls
            this.push("data: " + JSON.stringify({
              id: this.messageId,
              object: "chat.completion.chunk",
              created: this.created,
              model: "gpt-5.2-codex",
              choices: [{
                index: 0,
                delta: {},
                finish_reason: "tool_calls"
              }]
            }) + "\n\n");
          }
        }
        
        // 处理响应完成
        if (etype === "response.completed") {
          this.push("data: " + JSON.stringify({
            id: this.messageId,
            object: "chat.completion.chunk",
            created: this.created,
            model: "gpt-5.2-codex",
            choices: [{
              index: 0,
              delta: {},
              finish_reason: "stop"
            }],
            usage: data.response?.usage
          }) + "\n\n");
          this.push("data: [DONE]\n\n");
        }
      } catch (e) {
        // 静默处理解析错误
      }
    }
    callback();
  }
  
  _flush(callback) { callback(); }
}

const server = http.createServer((req, res) => {
  if (req.method === "POST" && req.url.includes("/chat/completions")) {
    let body = "";
    req.on("data", chunk => { body += chunk.toString(); });
    req.on("end", () => {
      try {
        const chatBody = JSON.parse(body);
        console.log("\n📥 Request:", {
          model: chatBody.model,
          messages: chatBody.messages?.length,
          tools: chatBody.tools?.length || 0
        });
        
        const { body: responsesBody, sessionId } = transformRequest(chatBody);
        const postData = JSON.stringify(responsesBody);
        
        const options = {
          hostname: "api.aicodemirror.com",
          path: "/api/codex/backend-api/codex/responses",
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "Content-Length": Buffer.byteLength(postData),
            "Authorization": (req.headers.authorization || ""),
            "User-Agent": "codex_cli_rs/0.79.0 (macOS; arm64)",
            "originator": "codex_cli_rs",
            "Accept": "text/event-stream",
            "conversation_id": sessionId,
            "session_id": sessionId
          }
        };
        
        const proxyReq = https.request(options, (proxyRes) => {
          if (proxyRes.statusCode !== 200) {
            let errorData = "";
            proxyRes.on("data", chunk => { errorData += chunk.toString(); });
            proxyRes.on("end", () => {
              console.log("❌ Error:", errorData.substring(0, 200));
              res.writeHead(proxyRes.statusCode, {"Content-Type": "application/json"});
              res.end(errorData);
            });
            return;
          }
          
          res.writeHead(200, {
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
            "Connection": "keep-alive"
          });
          
          const transform = new CodexToCompletionsTransform();
          proxyRes.pipe(transform).pipe(res);
        });
        
        proxyReq.on("error", (error) => {
          console.error("❌ Proxy error:", error.message);
          res.writeHead(500, {"Content-Type": "application/json"});
          res.end(JSON.stringify({error: {message: error.message}}));
        });
        
        proxyReq.write(postData);
        proxyReq.end();
        
        console.log("[✅] Chat Completions → Responses API");
      } catch (error) {
        console.error("❌ Parse error:", error.message);
        res.writeHead(400, {"Content-Type": "application/json"});
        res.end(JSON.stringify({error: {message: error.message}}));
      }
    });
  } else {
    res.writeHead(404, {"Content-Type": "application/json"});
    res.end(JSON.stringify({error: "Not found"}));
  }
});

server.listen(PORT, () => {
  console.log(`\n🚀 Codex Proxy (with tool support)`);
  console.log(`📡 Listening: http://localhost:${PORT}/v1/chat/completions`);
  console.log(`🎯 Target: https://api.aicodemirror.com/api/codex/backend-api/codex/responses`);
  console.log(`\n✨ Features:`);
  console.log(`   - Text streaming ✅`);
  console.log(`   - Tool/Function calls ✅`);
  console.log(`   - Image support ✅`);
  console.log(`\n`);
});
