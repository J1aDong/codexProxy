# Codex Proxy Test Plan (TDD)

This document is the single source of truth for proxy tests.
Update this document first, then change code to satisfy it.

## Key and Config
Key file (ignored by git):
- docs/key.json

Required fields in docs/key.json:
{
  "api_key": "YOUR_TEST_KEY",
  "base_url": "http://localhost:8889",
  "messages_endpoint": "/v1/messages"
}

Optional fields:
- log_dirs: array of directories to scan for latest proxy log

## Scope
- Anthropic Messages input parsing
- Anthropic -> Codex request transform
- Codex SSE -> Anthropic SSE mapping
- Image content handling
- Tool call mapping

## Test Data
Use a 1x1 PNG (black pixel) for deterministic image tests.

BLACK_PIXEL_PNG_BASE64
```
iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==
```

DATA_URL
```
data:image/png;base64,<BLACK_PIXEL_PNG_BASE64>
```

## Base Requests (Anthropic)

Text-only:
```json
{
  "model": "claude-sonnet-4-5-20250929",
  "max_tokens": 256,
  "messages": [
    { "role": "user", "content": "Say hello" }
  ],
  "stream": true
}
```

Image (base64):
```json
{
  "model": "claude-sonnet-4-5-20250929",
  "max_tokens": 256,
  "messages": [
    {
      "role": "user",
      "content": [
        { "type": "text", "text": "What color is this?" },
        {
          "type": "image",
          "source": {
            "type": "base64",
            "media_type": "image/png",
            "data": "<BLACK_PIXEL_PNG_BASE64>"
          }
        }
      ]
    }
  ],
  "stream": true
}
```

## Test Cases

CORE-001 Server Health
- Send: GET {base_url}/
- Expect: any HTTP response (server is up)

CORE-002 Text Streaming SSE
- Send: Text-only request
- Expect:
  - SSE includes message_start
  - SSE includes content_block_start (text)
  - SSE includes content_block_delta (text_delta)
  - SSE ends with message_stop

CORE-003 Tool Call Mapping
- Send: request with tools[] and prompt "use shell_command"
- Expect:
  - SSE contains content_block_start with tool_use
  - message_stop stop_reason is tool_use

IMG-001 Base64 Image -> input_image
- Send: Image (base64) request
- Expect:
  - Outgoing Codex request contains:
    { "type": "input_image", "image_url": "data:image/png;base64,..." }
  - Response describes the image (not a request for local path)

IMG-002 Data URL Passthrough
- Send: image.source.data = DATA_URL
- Expect:
  - image_url preserved as DATA_URL

IMG-003 image_url Object Accepted
- Send: content block:
  { "type": "image_url", "image_url": { "url": "<DATA_URL>" } }
- Expect:
  - image_url resolved to DATA_URL

IMG-004 Mixed Content Order
- Send: text -> image -> text
- Expect:
  - Codex content keeps ordering
  - No image block dropped

IMG-005 Multiple Images
- Send: two image blocks in one message
- Expect:
  - Two input_image blocks in outgoing request

IMG-006 file:// Path Handling
- Send: image.source.url = "file:///absolute/path/to/image.png"
- Expect:
  - Either convert to data URL OR return a clear error explaining unsupported file://
  - If implemented, conversion must preserve media_type
  - If upstream times out, log-based verification of data URL conversion is acceptable

IMG-007 No Local-Path Fallback
- Send: Image (base64) request
- Expect:
  - Response should not ask for local file path
  - If it does, treat as failure of image delivery

IMG-008 path Field Handling
- Send: image.source = { type: "file", path: "/absolute/path/to/image.png" }
- Expect:
  - Either convert to data URL OR return a clear error explaining unsupported file path
  - If upstream times out, log-based verification of data URL conversion is acceptable

## Verification Checklist
- Outgoing Codex request includes input_image with data URL
- No blank image_url is emitted
- SSE events follow expected ordering
- Logs include the image trace line:
  "Image: data:image/png;base64,..."

## Notes
- Keep this document as the single source of truth for proxy tests.
- Update IMG-* cases before changing image handling code.
