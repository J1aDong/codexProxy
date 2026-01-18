const http = require('http');

const API_KEY = 'sk-ant-api03-43mBeNUIHZj3JQVIb3HDf6Yw-a94MKEJgG5emsaKpNoRUwCmG5V46uQcFybxJ1swthN-nMqxBLzuUgcSe-QqHw';
const PROXY_URL = 'http://localhost:8889/v1/messages';

const smallImageData = 'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg==';

const requestData = {
  model: 'claude-sonnet-4-5-20250929',
  max_tokens: 4096,
  messages: [
    {
      role: 'user',
      content: [
        {
          type: 'text',
          text: '这是什么颜色？'
        },
        {
          type: 'image',
          source: {
            type: 'base64',
            media_type: 'image/png',
            data: smallImageData
          }
        }
      ]
    }
  ]
};

console.log('发送到代理的请求:');
console.log(JSON.stringify(requestData, null, 2));

const options = {
  hostname: 'localhost',
  port: 8889,
  path: '/v1/messages',
  method: 'POST',
  headers: {
    'Content-Type': 'application/json',
    'x-api-key': API_KEY
  }
};

const req = http.request(options, (res) => {
  console.log('状态码:', res.statusCode);

  res.on('data', (chunk) => {
    const lines = chunk.toString().split('\n');
    for (const line of lines) {
      if (line.startsWith('data: ')) {
        const dataStr = line.substring(6);
        if (dataStr.trim() === '[DONE]') continue;
        try {
          const data = JSON.parse(dataStr);
          if (data.type === 'content_block_delta' && data.delta.type === 'text_delta') {
            process.stdout.write(data.delta.text);
          }
        } catch (e) {}
      }
    }
  });

  res.on('end', () => {
    console.log('\n');
  });
});

req.on('error', (error) => {
  console.error('请求失败:', error);
});

req.write(JSON.stringify(requestData));
req.end();
