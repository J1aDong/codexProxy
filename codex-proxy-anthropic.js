#!/usr/bin/env node

/**
 * Codex Proxy - Anthropic Messages API 适配器
 * 
 * 将 Anthropic Messages API 格式转换为 Codex Responses API 格式
 * 适配 @ai-sdk/anthropic provider
 * 
 * 格式差异：
 * - Anthropic: messages[], system, tools[]
 * - Codex: input[], instructions, tools[]
 */

const http = require("http");
const https = require("https");
const { Transform } = require("stream");
const { randomUUID } = require("crypto");
const fs = require("fs");
const path = require("path");

process.env.NODE_TLS_REJECT_UNAUTHORIZED = "0";

const PORT = 8889;

// 加载模板文件
const TEMPLATE_PATH = path.resolve(__dirname, "codex-request.json");
const TEMPLATE = JSON.parse(fs.readFileSync(TEMPLATE_PATH, "utf8"));

// Anthropic 工具转换为 Codex 工具
function transformTools(anthropicTools) {
  if (!anthropicTools || anthropicTools.length === 0) return [];
  
  console.log("📨 Tools received:", {
    count: anthropicTools.length,
    firstTool: JSON.stringify(anthropicTools[0])?.substring(0, 200)
  });
  
  return anthropicTools.map(tool => {
    console.log("📨 Tool type:", tool.type, "Tool keys:", Object.keys(tool));
    
    // Claude Code 格式: { name, description, input_schema }
    if (tool.name && !tool.type) {
      return {
        type: "function",
        name: tool.name,
        description: tool.description || "",
        strict: false,
        parameters: tool.input_schema || {}
      };
    }
    
    // Anthropic 格式: { type: "tool", name, ... }
    if (tool.type === "tool") {
      return {
        type: "function",
        name: tool.name,
        description: tool.description || "",
        strict: false,
        parameters: tool.input_schema || {}
      };
    }
    
    // OpenAI 格式: { type: "function", function: {...} }
    if (tool.type === "function") {
      return {
        type: "function",
        name: tool.function?.name || tool.name,
        description: tool.function?.description || tool.description || "",
        strict: false,
        parameters: tool.function?.parameters || tool.input_schema || {}
      };
    }
    
    // 未知格式，返回通用格式
    return {
      type: "function",
      name: tool.name || tool.function?.name || "unknown",
      description: tool.description || tool.function?.description || "",
      strict: false,
      parameters: tool.input_schema || tool.function?.parameters || {}
    };
  });
}

// Anthropic 消息转换为 Codex 输入格式（支持 content blocks 与 tool_result）
// 解析图片 URL，支持多种格式
function resolveImageUrl(block) {
  if (!block || typeof block !== "object") return "";

  // OpenAI 格式: image_url 字符串或对象
  if (typeof block.image_url === "string") return block.image_url;
  if (block.image_url && typeof block.image_url === "object") {
    return block.image_url.url || block.image_url.uri || "";
  }

  // Anthropic 格式: source 对象
  const source = block.source;
  if (source && typeof source === "object") {
    // URL 类型
    if (source.type === "url" && source.url) return source.url;
    // 直接 URL/URI 字段
    if (typeof source.url === "string") return source.url;
    if (typeof source.uri === "string") return source.uri;
    // Base64 类型
    if (source.type === "base64" && typeof source.data === "string") {
      if (source.data.startsWith("data:")) return source.data;
      const mediaType = source.media_type || "image/png";
      return `data:${mediaType};base64,${source.data}`;
    }
    // 兼容：data 字段直接存在
    if (typeof source.data === "string") {
      if (source.data.startsWith("data:")) return source.data;
      const mediaType = source.media_type || "image/png";
      return `data:${mediaType};base64,${source.data}`;
    }
  }

  return "";
}

// 提取 tool_result 的内容文本
function extractToolResultContent(content) {
  if (typeof content === "string") return content;
  if (!content) return "";

  // 数组格式：提取所有文本
  if (Array.isArray(content)) {
    return content.map(block => {
      if (typeof block === "string") return block;
      if (block.type === "text" && block.text) return block.text;
      if (block.type === "image") return "[image]";
      return JSON.stringify(block);
    }).join("\n");
  }

  // 对象格式
  if (typeof content === "object") {
    if (content.type === "text" && content.text) return content.text;
    return JSON.stringify(content);
  }

  return String(content);
}

function transformMessages(messages) {
  const input = [];
  
  for (const msg of messages) {
    if (!msg || !msg.role) continue;
    
    if (msg.role === "system") {
      continue;
    }
    
    if (msg.role !== "user" && msg.role !== "assistant") continue;
    
    const role = msg.role;
    const content = msg.content;
    const textType = role === "user" ? "input_text" : "output_text";
    
    if (typeof content === "string") {
      input.push({
        type: "message",
        role,
        content: [{
          type: textType,
          text: content
        }]
      });
      continue;
    }
    
    const blocks = Array.isArray(content)
      ? content
      : (content && typeof content === "object" ? [content] : []);
    
    let currentMessage = null;
    const ensureMessage = () => {
      if (!currentMessage) {
        currentMessage = { type: "message", role, content: [] };
        input.push(currentMessage);
      }
    };
    
    for (const block of blocks) {
      if (!block) continue;

      // 调试日志：打印 block 摘要
      if (typeof block === "string") {
        console.log("📦 Block:", {
          type: "string",
          length: block.length
        });
        ensureMessage();
        currentMessage.content.push({
          type: textType,
          text: block
        });
        continue;
      }

      console.log("📦 Block:", {
        type: block.type,
        hasSource: !!block.source,
        sourceType: block.source?.type,
        hasImageUrl: !!block.image_url,
        keys: Object.keys(block)
      });

      if (block.type === "text") {
        ensureMessage();
        currentMessage.content.push({
          type: textType,
          text: block.text || ""
        });
        continue;
      }

      if (block.type === "image" || block.type === "image_url" || block.type === "input_image") {
        const source = block.source;
        const imageUrl = resolveImageUrl(block);
        const isDataUrl = typeof imageUrl === "string" && imageUrl.startsWith("data:");

        console.log("🖼️ Image block:", {
          type: block.type,
          sourceType: source?.type,
          mediaType: source?.media_type,
          hasData: typeof source?.data === "string",
          isDataUrl
        });

        if (imageUrl && role === "user") {
          // OpenAI Responses API 格式：input_image 作为 message content 的一部分
          ensureMessage();
          currentMessage.content.push({
            type: "input_image",
            image_url: imageUrl,
            detail: "auto"
          });
          if (isDataUrl) {
            console.log("📤 Using input_image format with data URL");
          }
        } else {
          ensureMessage();
          currentMessage.content.push({
            type: textType,
            text: imageUrl || JSON.stringify(block)
          });
        }
        continue;
      }
      
      if (block.type === "tool_result") {
        currentMessage = null;
        const resultText = extractToolResultContent(block.content);
        input.push({
          type: "function_call_output",
          call_id: block.tool_use_id || block.id || `tool_${Date.now()}`,
          output: resultText
        });
        continue;
      }

      if (block.type === "tool_use") {
        // assistant 消息中的 tool_use 转换为 Codex function_call
        currentMessage = null;
        input.push({
          type: "function_call",
          call_id: block.id || `call_${Date.now()}`,
          name: block.name || "unknown",
          arguments: typeof block.input === "string" ? block.input : JSON.stringify(block.input || {})
        });
        continue;
      }
      
      ensureMessage();
      currentMessage.content.push({
        type: textType,
        text: JSON.stringify(block)
      });
    }
  }
  
  return input;
}

// 主转换函数
function transformRequest(anthropicBody) {
  const { 
    model, 
    messages = [], 
    tools, 
    stream = true 
  } = anthropicBody;
  
  const sessionId = randomUUID();
  
  // 转换对话消息
  const chatMessages = transformMessages(messages);
  
  // 构建 Codex 要求的 input 数组结构
  // 1. 必须以 TEMPLATE.input[0] 开头 (包含 # AGENTS.md 签名)，否则后端校验失败
  const finalInput = [TEMPLATE.input[0]];
  
  // 2. 如果有用户提供的 system prompt (Claude Skills)，将其作为上下文注入
  if (anthropicBody.system) {
    console.log("📝 Injecting Claude system context (" + anthropicBody.system.length + " chars)");
    finalInput.push({
      type: "message",
      role: "user",
      content: [{
        type: "input_text",
        text: `<system_context>\n${anthropicBody.system}\n</system_context>`
      }]
    });
  }
  
  // 3. 追加实际对话历史
  finalInput.push(...chatMessages);
  
  // 获取 instructions (必须与模板完全一致)
  const instructions = TEMPLATE.instructions;
  
  // 转换工具
  const transformedTools = transformTools(tools);
  
  // 自动转换 Claude 模型名为 Codex 模型名
  let codexModel = model || TEMPLATE.model;
  if (model && /claude|sonnet|opus|haiku/i.test(model)) {
    console.log(`🔄 Auto-converting model: ${model} → gpt-5.2-codex`);
    codexModel = "gpt-5.2-codex";
  }
  
  return {
    body: {
      model: codexModel,
      instructions,
      input: finalInput,
      tools: transformedTools.length > 0 ? transformedTools : TEMPLATE.tools,
      tool_choice: "auto",
      parallel_tool_calls: true,
      reasoning: TEMPLATE.reasoning,
      store: false,
      stream,
      include: TEMPLATE.include,
      prompt_cache_key: sessionId
    },
    sessionId
  };
}

// SSE 转换器（Codex → Anthropic）
class CodexToAnthropicTransform extends Transform {
  constructor(model) {
    super();
    this.buffer = "";
    this.messageId = "msg_" + Date.now();
    this.created = Math.floor(Date.now() / 1000);
    this.sentMessageStart = false;  // 标记是否已发送 message_start
    this.model = model || "gpt-5.2-codex";
    this.contentIndex = 0;
    this.openTextIndex = null;
    this.openToolIndex = null;
    this.toolCallId = null;
    this.toolName = null;
    this.currentFunctionCall = null;
    this.sawToolCall = false;
  }
  
  _sendMessageStart() {
    if (this.sentMessageStart) return;
    this.sentMessageStart = true;
    this.push("event: message_start\n" +
      "data: " + JSON.stringify({
        type: "message_start",
        message: {
          id: this.messageId,
          type: "message",
          role: "assistant",
          content: [],
          model: this.model,
          stop_reason: null,
          usage: { input_tokens: 0, output_tokens: 0 }
        }
      }) + "\n\n");
  }
  
  _openTextBlock() {
    if (this.openTextIndex !== null) return;
    this.openTextIndex = this.contentIndex++;
    this.push("event: content_block_start\n" +
      "data: " + JSON.stringify({
        type: "content_block_start",
        index: this.openTextIndex,
        content_block: {
          type: "text",
          text: ""
        }
      }) + "\n\n");
  }
  
  _closeTextBlock() {
    if (this.openTextIndex === null) return;
    this.push("event: content_block_stop\n" +
      "data: " + JSON.stringify({
        type: "content_block_stop",
        index: this.openTextIndex
      }) + "\n\n");
    this.openTextIndex = null;
  }
  
  _openToolBlock(toolCallId, toolName) {
    if (this.openToolIndex !== null) return;
    this.openToolIndex = this.contentIndex++;
    this.push("event: content_block_start\n" +
      "data: " + JSON.stringify({
        type: "content_block_start",
        index: this.openToolIndex,
        content_block: {
          type: "tool_use",
          id: toolCallId,
          name: toolName,
          input: {}
        }
      }) + "\n\n");
  }
  
  _closeToolBlock() {
    if (this.openToolIndex === null) return;
    this.push("event: content_block_stop\n" +
      "data: " + JSON.stringify({
        type: "content_block_stop",
        index: this.openToolIndex
      }) + "\n\n");
    this.openToolIndex = null;
  }
  
  _transform(chunk, encoding, callback) {
    this.buffer += chunk.toString();
    const lines = this.buffer.split("\n");
    this.buffer = lines.pop() || "";
    
    this._sendMessageStart();
    
    for (const line of lines) {
      if (!line.startsWith("data: ")) continue;
      
      try {
        const data = JSON.parse(line.slice(6));
        const etype = data.type || "";
        
        // 文本输出 -> Anthropic text delta
        if (etype === "response.output_text.delta") {
          this._openTextBlock();
          this.push("event: content_block_delta\n" +
            "data: " + JSON.stringify({
              type: "content_block_delta",
              index: this.openTextIndex,
              delta: {
                type: "text_delta",
                text: data.delta || ""
              }
            }) + "\n\n");
        }
        
        // 工具调用开始
        if (etype === "response.output_item.added") {
          const item = data.item || {};
          if (item.type === "function_call") {
            this.currentFunctionCall = item;
            this.toolCallId = item.call_id || "tool_" + Date.now();
            this.toolName = item.name || "unknown";
            this.sawToolCall = true;
            this._closeTextBlock();
            
            // Anthropic 格式的工具调用开始
            this._openToolBlock(this.toolCallId, this.toolName);
          }
        }
        
        // 工具调用参数
        if (etype === "response.function_call_arguments.delta" || etype === "response.function_call_arguments_delta") {
          const delta = data.delta || data.arguments || "";
          if (this.openToolIndex === null) {
            this.sawToolCall = true;
            this._closeTextBlock();
            this.toolCallId = this.toolCallId || "tool_" + Date.now();
            this.toolName = this.toolName || "unknown";
            this._openToolBlock(this.toolCallId, this.toolName);
          }
          
          this.push("event: content_block_delta\n" +
            "data: " + JSON.stringify({
              type: "content_block_delta",
              index: this.openToolIndex,
              delta: {
                type: "input_json_delta",
                partial_json: typeof delta === "string" ? delta : JSON.stringify(delta)
              }
            }) + "\n\n");
        }
        
        // 工具调用停止
        if (etype === "response.output_item.done") {
          if (this.currentFunctionCall && this.currentFunctionCall.type === "function_call") {
            this.currentFunctionCall = null;
            this._closeToolBlock();
          }
        }
        
        // 响应完成
        if (etype === "response.completed") {
          this._closeTextBlock();
          this._closeToolBlock();
          const stopReason = this.sawToolCall ? "tool_use" : "end_turn";
          // 发送 message_delta（usage 和 delta 是平级的）
          if (data.response?.usage) {
            this.push("event: message_delta\n" +
              "data: " + JSON.stringify({
                type: "message_delta",
                delta: {
                  stop_reason: stopReason
                },
                usage: {
                  input_tokens: data.response.usage.input_tokens,
                  output_tokens: data.response.usage.output_tokens
                }
              }) + "\n\n");
          } else {
            this.push("event: message_delta\n" +
              "data: " + JSON.stringify({
                type: "message_delta",
                delta: {
                  stop_reason: stopReason
                }
              }) + "\n\n");
          }
          
          // 发送消息结束
          this.push("event: message_stop\n" +
            "data: " + JSON.stringify({
              type: "message_stop",
              stop_reason: stopReason
            }) + "\n\n");
        }
      } catch (e) {}
    }
    callback();
  }
  
  _flush(callback) { callback(); }
}

// HTTP 服务器
const server = http.createServer((req, res) => {
  // Anthropic API 使用 /messages
  if (req.method === "POST" && req.url.includes("/messages")) {
    let body = "";
    req.on("data", chunk => { body += chunk.toString(); });
    req.on("end", () => {
      try {
        const anthropicBody = JSON.parse(body);
        const { body: responsesBody, sessionId } = transformRequest(anthropicBody);
        const postData = JSON.stringify(responsesBody);
        
        console.log("\n📥 Anthropic Request:", {
          model: anthropicBody.model,
          messages: anthropicBody.messages?.length,
          tools: anthropicBody.tools?.length || 0
        });
        
        // 获取 Claude Code 需要的 header
        const anthropicVersion = req.headers["x-anthropic-version"] || req.headers["anthropic-version"] || "2023-06-01";
        
        // 必须使用客户端传入的 API key
        // 支持两种认证方式：Authorization header 或 x-api-key header
        const rawAuthHeader = Array.isArray(req.headers.authorization)
          ? req.headers.authorization[0]
          : req.headers.authorization;
        const rawApiKeyHeader = Array.isArray(req.headers["x-api-key"])
          ? req.headers["x-api-key"][0]
          : req.headers["x-api-key"];
        const rawAltApiKeyHeader = Array.isArray(req.headers["api-key"])
          ? req.headers["api-key"][0]
          : req.headers["api-key"];

        let authHeader = typeof rawAuthHeader === "string" ? rawAuthHeader : "";
        let apiKeyHeader = typeof rawApiKeyHeader === "string" ? rawApiKeyHeader : "";
        if (!apiKeyHeader && typeof rawAltApiKeyHeader === "string") {
          apiKeyHeader = rawAltApiKeyHeader;
        }

        if (!authHeader && apiKeyHeader) {
          authHeader = `Bearer ${apiKeyHeader}`;
        }
        if (!apiKeyHeader && authHeader) {
          const match = authHeader.match(/^Bearer\\s+(.+)$/i);
          apiKeyHeader = match ? match[1] : authHeader;
        }

        if (!authHeader && !apiKeyHeader) {
          res.writeHead(401, {"Content-Type": "application/json"});
          res.end(JSON.stringify({ error: { type: "unauthorized", message: "Missing API key" } }));
          return;
        }

        const options = {
          hostname: "api.aicodemirror.com",
          path: "/api/codex/backend-api/codex/responses",
          method: "POST",
          headers: {
            "Content-Type": "application/json",
            "Content-Length": Buffer.byteLength(postData),
            ...(authHeader ? { "Authorization": authHeader } : {}),
            ...(apiKeyHeader ? { "x-api-key": apiKeyHeader } : {}),
            "User-Agent": "Anthropic-Node/0.3.4",
            "x-anthropic-version": anthropicVersion,
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
              res.writeHead(proxyRes.statusCode, {"Content-Type": "application/json"});
              res.end(errorData);
            });
            return;
          }
          
          res.writeHead(200, {
            "Content-Type": "text/event-stream",
            "Cache-Control": "no-cache",
            "Connection": "keep-alive",
            "Access-Control-Allow-Origin": "*"
          });
          
          const transform = new CodexToAnthropicTransform(anthropicBody.model);
          proxyRes.pipe(transform).pipe(res);
        });
        
        proxyReq.on("error", (error) => {
          res.writeHead(500, {"Content-Type": "application/json"});
          res.end(JSON.stringify({ error: { message: error.message } }));
        });
        
        proxyReq.write(postData);
        proxyReq.end();
        
        console.log("[✅] Anthropic Messages → Codex Responses API");
        
      } catch (error) {
        res.writeHead(400, {"Content-Type": "application/json"});
        res.end(JSON.stringify({ error: { message: error.message } }));
      }
    });
  } else {
    res.writeHead(404, {"Content-Type": "application/json"});
    res.end(JSON.stringify({ error: { type: "not_found", message: "Not found" } }));
  }
});

server.listen(PORT, () => {
  console.log(`\n🚀 Codex Proxy (Anthropic Style)`);
  console.log(`📡 Listening: http://localhost:${PORT}/messages`);
  console.log(`🎯 Target: https://api.aicodemirror.com/api/codex/backend-api/codex/responses`);
  console.log(`\n✨ Features:`);
  console.log(`   - Anthropic Messages API ↔ Codex Responses API`);
  console.log(`   - Tool/Function calls ✅`);
  console.log(`   - Streaming ✅`);
  console.log(`\n📝 OpenCode 配置示例:`);
  console.log(`   provider: @ai-sdk/anthropic`);
  console.log(`   baseURL: http://localhost:${PORT}`);
  console.log(`\n`);
});
