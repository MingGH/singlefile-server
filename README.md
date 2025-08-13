# SingleFile Server

一个基于 Docker 的 HTTP 服务，使用 [SingleFile](https://github.com/gildas-lormeau/SingleFile) 工具将网页保存为完整的 HTML 文件，包含所有 CSS、JavaScript、图片等资源。

## 功能特性

- 🌐 **完整网页保存** - 保存包含所有资源的单个 HTML 文件
- 🚀 **简单 HTTP API** - 通过简单的 GET 请求即可使用
- 🐳 **Docker 容器化** - 开箱即用，无需复杂配置
- ⚡ **高性能** - 基于 Rust 和 Axum 框架构建
- 🔧 **轻量级** - 最小化的依赖和资源占用

## 快速开始

### 1. 构建镜像

```bash
git clone <your-repo>
cd singlefile-server
docker build -t singlefile-server .
```

### 2. 运行服务

```bash
# 前台运行
docker run -p 8080:8080 singlefile-server

# 后台运行（推荐）
docker run -d --name singlefile-server -p 8080:8080 singlefile-server
```

### 3. 使用服务

#### 保存网页
GET /save?url=<网页URL>

```bash
curl "http://localhost:8080/save?url=https://example.com"
```

响应：
```json
{
	"file_url":null,
	"message":"Task queued successfully",
	"queue_position":1,
	"status":"queued",
	"task_id":"task_1755086347679"
}
```

#### 查询任务状态
GET /status/<task_id>

```bash
curl "http://localhost:8080/status/task_1755086347679"
```

响应：
```json
{
  "message":"Task completed successfully",
  "queue_position":null,
  "status":"completed",
  "task_id":"task_1755086347679"
}
```

#### 获取保存的文件
GET /files/<file_id>

```bash
curl "http://localhost:8080/files/task_1755086347679"
```

响应： 为对应的html文件


## 项目结构

```
.
├── Dockerfile          # Docker 构建配置
├── Cargo.toml          # Rust 项目配置
├── src/
│   └── main.rs         # 主要服务代码
└── README.md           # 项目文档
```

## 技术栈

- **后端框架**: [Axum](https://github.com/tokio-rs/axum) (Rust)
- **网页保存**: [SingleFile CLI](https://github.com/gildas-lormeau/single-file-cli)
- **浏览器引擎**: Chromium (无头模式)
- **容器**: Docker

## 开发

### 本地开发

```bash
# 安装依赖
cargo build

# 运行服务
cargo run

# 测试
curl "http://localhost:8080/save?url=https://httpbin.org/html"
```

## 使用场景

- **网页备份**: 完整保存重要网页内容
- **离线浏览**: 保存网页供离线查看
- **内容归档**: 长期保存网页快照
- **爬虫辅助**: 获取完整渲染后的网页内容
- **测试数据**: 为测试环境提供稳定的网页数据

## 注意事项

- 某些网站可能有反爬虫机制，可能无法正常保存
- 大型网页或包含大量媒体资源的页面可能需要较长时间
- 服务目前将临时文件保存在 `/tmp` 目录，重启后会清空
- 建议在生产环境中添加适当的访问控制和限流机制

## 许可证

MIT License

## 贡献

欢迎提交 Issue 和 Pull Request！

## 更新日志

### v0.1.0
- 初始版本发布
- 支持基本的网页保存功能
- Docker 容器化部署
