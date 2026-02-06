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

const PORT = parseInt(process.env.PORT || "8889", 10);
const DEFAULT_TARGET = "https://api.aicodemirror.com/api/codex/backend-api/codex/responses";
const targetUrl = new URL(process.env.CODEX_PROXY_TARGET || DEFAULT_TARGET);

// 支持的 Codex 模型列表
const SUPPORTED_CODEX_MODELS = {
  "gpt-5.2-codex": "gpt-5.2-codex",
  "gpt-5.3-codex": "gpt-5.3-codex"
};

// 默认模型（gpt-5.3-codex 作为新默认）
const DEFAULT_CODEX_MODEL = "gpt-5.3-codex";

// 从环境变量或配置获取默认模型
const getDefaultModel = () => {
  return process.env.CODEX_DEFAULT_MODEL || DEFAULT_CODEX_MODEL;
};

// 加载模板文件
const TEMPLATE_PATH = path.resolve(__dirname, "codex-request.json");
const TEMPLATE = JSON.parse(fs.readFileSync(TEMPLATE_PATH, "utf8"));

// ============================================================
// Skill 转换模块 - Claude Code Skill → Codex CLI Skill
// ============================================================

const SKILL_SEARCH_PATHS = [
  path.join(process.env.HOME || "", ".claude", "skills"),
  path.join(process.env.HOME || "", ".codex", "skills"),
];

function findSkillPath(skillName, cwd) {
  if (!skillName) return null;
  
  const parts = skillName.split(":");
  const namespace = parts.length > 1 ? parts[0] : null;
  const name = parts.length > 1 ? parts.slice(1).join(":") : skillName;
  
  const searchPaths = [...SKILL_SEARCH_PATHS];
  if (cwd) {
    searchPaths.push(path.join(cwd, ".claude", "skills"));
    searchPaths.push(path.join(cwd, ".codex", "skills"));
  }
  
  for (const basePath of searchPaths) {
    const candidates = namespace
      ? [
          path.join(basePath, namespace, name, "SKILL.md"),
          path.join(basePath, `${namespace}-${name}`, "SKILL.md"),
        ]
      : [
          path.join(basePath, name, "SKILL.md"),
        ];
    
    for (const candidate of candidates) {
      try {
        if (fs.existsSync(candidate)) return candidate;
      } catch (e) {}
    }
  }
  return null;
}

function extractSkillFromToolResult(content) {
  if (!content) return null;
  
  const blocks = Array.isArray(content) ? content : [content];
  const allTexts = blocks
    .map(block => typeof block === "string" ? block : (block.text || ""))
    .filter(Boolean);
  const fullText = allTexts.join("\n");
  
  const nameMatch = fullText.match(/<command-name>([^<]+)<\/command-name>/);
  const skillName = nameMatch ? nameMatch[1].replace(/^\//, "") : null;
  
  const pathMatch = fullText.match(/Base Path:\s*([^\n]+)/);
  const basePath = pathMatch ? pathMatch[1].trim() : null;
  
  let skillContent = null;
  if (fullText.includes("Base Path:")) {
    const idx = fullText.indexOf("\n", fullText.indexOf("Base Path:"));
    if (idx !== -1) skillContent = fullText.substring(idx + 1).trim();
  } else {
    const contentWithoutMeta = fullText
      .replace(/<command-name>[^<]*<\/command-name>/g, "")
      .replace(/<[^>]+>/g, "")
      .trim();
    if (contentWithoutMeta) skillContent = contentWithoutMeta;
  }
  
  return (skillName && skillContent) ? { skillName, skillContent, basePath } : null;
}

function convertToCodexSkillFormat(skillName, skillContent, skillPath) {
  let fullContent = skillContent;
  
  if (skillPath) {
    try {
      const fileContent = fs.readFileSync(skillPath, "utf-8");
      if (fileContent.trim()) fullContent = fileContent;
    } catch (e) {
      console.warn("⚠️ Failed to read skill file:", skillPath, e.message);
    }
  }
  
  return `<skill>
<name>${skillName}</name>
<path>${skillPath || "unknown"}</path>
${fullContent}
</skill>`;
}

function isSkillToolUse(block) {
  return block?.type === "tool_use" && 
         typeof block.name === "string" && 
         block.name.toLowerCase() === "skill";
}

function isSkillToolResult(block, skillToolIds) {
  if (!block || block.type !== "tool_result") return false;
  
  const toolUseId = block.tool_use_id || block.id;
  if (skillToolIds?.has(toolUseId)) return true;
  
  const content = block.content;
  if (!content) return false;
  
  const text = typeof content === "string" 
    ? content 
    : (Array.isArray(content) ? content.map(b => b.text || "").join("\n") : "");
  
  return text.includes("<command-name>") || text.includes("Base Path:");
}

// Anthropic 工具转换为 Codex 工具
function transformTools(anthropicTools) {
  if (!anthropicTools || anthropicTools.length === 0) return [];
  
  const filteredTools = anthropicTools.filter(tool => {
    const name = tool?.name || tool?.function?.name || "";
    return typeof name !== "string" || name.toLowerCase() !== "skill";
  });

  console.log("📨 Tools received:", {
    count: anthropicTools.length,
    filteredCount: filteredTools.length,
    skillToolFiltered: anthropicTools.length !== filteredTools.length
  });
  
  return filteredTools.map(tool => {
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

function summarizeDocumentBlock(block) {
  if (!block || typeof block !== "object") return "[document omitted]";

  const source = block.source && typeof block.source === "object" ? block.source : {};
  const parts = [];
  const sourceType = typeof source.type === "string" ? source.type : "";
  const mediaType = typeof source.media_type === "string"
    ? source.media_type
    : (typeof source.mime_type === "string" ? source.mime_type : "");
  const name = typeof block.name === "string" ? block.name : "";
  const base64Len = typeof source.data === "string" ? source.data.length : 0;

  if (name) parts.push(`name=${name}`);
  if (sourceType) parts.push(`source=${sourceType}`);
  if (mediaType) parts.push(`media=${mediaType}`);
  if (base64Len) parts.push(`base64_len=${base64Len}`);

  return parts.length > 0
    ? `[document omitted: ${parts.join(" ")}]`
    : "[document omitted]";
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
      if (block.type === "document") return summarizeDocumentBlock(block);
      if (block.type === "image") return "[image]";
      return JSON.stringify(block);
    }).join("\n");
  }

  // 对象格式
  if (typeof content === "object") {
    if (content.type === "text" && content.text) return content.text;
    if (content.type === "document") return summarizeDocumentBlock(content);
    return JSON.stringify(content);
  }

  return String(content);
}

function transformMessages(messages, cwd) {
  const input = [];
  const extractedSkills = [];
  const skillToolIds = new Set();
  
  for (const msg of messages) {
    const content = msg?.content;
    if (Array.isArray(content)) {
      for (const block of content) {
        if (isSkillToolUse(block)) {
          skillToolIds.add(block.id);
        }
      }
    }
  }
  
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
      
      if (block.type === "document") {
        ensureMessage();
        currentMessage.content.push({
          type: textType,
          text: summarizeDocumentBlock(block)
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
        
        if (isSkillToolResult(block, skillToolIds)) {
          const skillInfo = extractSkillFromToolResult(block.content);
          if (skillInfo) {
            const skillPath = findSkillPath(skillInfo.skillName, cwd);
            const codexSkill = convertToCodexSkillFormat(
              skillInfo.skillName,
              skillInfo.skillContent,
              skillPath
            );
            extractedSkills.push(codexSkill);
            console.log("🎯 Skill extracted:", skillInfo.skillName);
            continue;
          }
        }
        
        const resultText = extractToolResultContent(block.content);
        input.push({
          type: "function_call_output",
          call_id: block.tool_use_id || block.id || `tool_${Date.now()}`,
          output: resultText
        });
        continue;
      }

      if (block.type === "tool_use") {
        currentMessage = null;
        
        if (isSkillToolUse(block)) {
          console.log("🔧 Skipping Skill tool_use:", block.id);
          continue;
        }
        
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
  
  return { input, skills: extractedSkills };
}

// 从消息中提取 cwd（工作目录）
// Claude Code 可能在消息中包含路径信息
function extractCwdFromMessages(messages) {
  // 尝试从第一条用户消息中提取路径
  for (const msg of messages) {
    if (msg.role === "user") {
      const content = Array.isArray(msg.content)
        ? msg.content.map(c => c.text || "").join(" ")
        : (typeof msg.content === "string" ? msg.content : "");

      // 匹配常见的路径模式
      const pathMatch = content.match(/(?:^|\s)(\/[^\s]+)/);
      if (pathMatch && pathMatch[1].length > 1) {
        return pathMatch[1];
      }
    }
  }
  return null;
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
  
  // 从请求中提取 cwd（如果有），否则使用默认值
  const cwd = extractCwdFromMessages(messages) || process.cwd();
  
  // 转换对话消息（同时提取 Skills）
  const { input: chatMessages, skills: extractedSkills } = transformMessages(messages, cwd);
  
  // 构建 Codex 要求的 input 数组结构
  // 1. 必须以 TEMPLATE.input[0] 开头 (包含 # AGENTS.md 签名)，否则后端校验失败
  const finalInput = [TEMPLATE.input[0]];
  
  // 2. 如果有用户提供的 system prompt (Claude Code)，转换为 Codex 原生格式
  if (anthropicBody.system) {
    console.log("📝 Injecting AGENTS.md context (" + anthropicBody.system.length + " chars)");

    // 2.1 AGENTS.md 格式：# AGENTS.md instructions for {cwd} + <INSTRUCTIONS>
    finalInput.push({
      type: "message",
      role: "user",
      content: [{
        type: "input_text",
        text: `# AGENTS.md instructions for ${cwd}\n\n<INSTRUCTIONS>\n${anthropicBody.system}\n</INSTRUCTIONS>`
      }]
    });

    // 2.2 environment_context：独立的 user message
    finalInput.push({
      type: "message",
      role: "user",
      content: [{
        type: "input_text",
        text: `<environment_context>
  <cwd>${cwd}</cwd>
  <approval_policy>on-request</approval_policy>
  <sandbox_mode>workspace-write</sandbox_mode>
  <network_access>restricted</network_access>
  <shell>${process.env.SHELL || 'bash'}</shell>
</environment_context>`
      }]
    });
  }
  
  // 3. 注入提取的 Skills（Codex 格式）
  if (extractedSkills.length > 0) {
    console.log("🎯 Injecting", extractedSkills.length, "skill(s) into request");
    for (const skill of extractedSkills) {
      finalInput.push({
        type: "message",
        role: "user",
        content: [{
          type: "input_text",
          text: skill
        }]
      });
    }
  }
  
  // 4. 追加实际对话历史
  finalInput.push(...chatMessages);
  
  // 获取 instructions (必须与模板完全一致)
  const instructions = TEMPLATE.instructions;
  
  // 转换工具（仅使用客户端传入的工具，避免返回 Claude Code 不存在的工具）
  const transformedTools = transformTools(tools);
  const toolsForCodex = transformedTools;
  if (toolsForCodex.length === 0) {
    console.log("⚠️ No client tools provided; sending empty tools list to Codex to avoid tool-name mismatch.");
  }
  
  // 智能模型选择和转换
  let codexModel = model || TEMPLATE.model || getDefaultModel();

  // 如果是 Claude 模型，转换为 Codex 模型
  if (model && /claude|sonnet|opus|haiku/i.test(model)) {
    const defaultModel = getDefaultModel();
    console.log(`🔄 Auto-converting model: ${model} → ${defaultModel}`);
    codexModel = defaultModel;
  }

  // 验证模型是否支持，如果不支持则使用默认模型
  if (!SUPPORTED_CODEX_MODELS[codexModel]) {
    const fallbackModel = getDefaultModel();
    console.log(`⚠️ Unsupported model: ${codexModel}, falling back to: ${fallbackModel}`);
    codexModel = fallbackModel;
  }
  
  return {
    body: {
      model: codexModel,
      instructions,
      input: finalInput,
      tools: toolsForCodex,
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
    this.model = model || getDefaultModel();
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

        // 优先使用环境变量配置的 Key (强制覆盖)
        if (process.env.CODEX_API_KEY) {
          apiKeyHeader = process.env.CODEX_API_KEY;
          authHeader = `Bearer ${apiKeyHeader}`;
        } else if (!authHeader && apiKeyHeader) {
          authHeader = `Bearer ${apiKeyHeader}`;
        }
        
        if (!apiKeyHeader && authHeader) {
          const match = authHeader.match(/^Bearer\s+(.+)$/i);
          apiKeyHeader = match ? match[1] : authHeader;
        }

        if (!authHeader && !apiKeyHeader) {
          res.writeHead(401, {"Content-Type": "application/json"});
          res.end(JSON.stringify({ error: { type: "unauthorized", message: "Missing API key" } }));
          return;
        }

        const options = {
          hostname: targetUrl.hostname,
          path: targetUrl.pathname + targetUrl.search,
          protocol: targetUrl.protocol,
          port: targetUrl.port || (targetUrl.protocol === "https:" ? 443 : 80),
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
  console.log(`🎯 Target: ${targetUrl.toString()}`);
  console.log(`\n✨ Features:`);
  console.log(`   - Anthropic Messages API ↔ Codex Responses API`);
  console.log(`   - Tool/Function calls ✅`);
  console.log(`   - Streaming ✅`);
  console.log(`\n📝 OpenCode 配置示例:`);
  console.log(`   provider: @ai-sdk/anthropic`);
  console.log(`   baseURL: http://localhost:${PORT}`);
  console.log(`\n`);
});
