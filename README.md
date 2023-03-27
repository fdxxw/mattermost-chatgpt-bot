# mattermost-chatgpt-bot
为 mattermost 创建 ChatGPT 机器人，使用 gpt-3.5-turbo 模型

## 环境变量
1. MATTERMOST_TOKEN     机器人Token,必要
2. MATTERMOST_URL       必要
3. OPENAI_API_KEY       必要
4. MATTERMOST_BOT_NAME  非必要，机器人名称，默认 chatgpt
5. OPENAI_API_PROXY     非必要，访问 openai 使用的代理，

## 快速开始

```bash
git clone https://github.com/fdxxw/mattermost-chatgpt-bot
# 编辑 docker-compose.yml，修改环境变量
docker-compose up -d
```
## License

[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](https://opensource.org/licenses/Apache-2.0)
