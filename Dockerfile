# 多阶段构建 Dockerfile
# 第一阶段：构建 Rust 应用
FROM rust:1.79-slim as builder

WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# 第二阶段：运行时环境
FROM node:20-slim

# 安装必要的依赖和 Chrome 依赖
RUN apt-get update && apt-get install -y \
    wget \
    gnupg \
    ca-certificates \
    fonts-liberation \
    libatk-bridge2.0-0 \
    libatk1.0-0 \
    libcups2 \
    libdbus-1-3 \
    libdrm2 \
    libx11-xcb1 \
    libxcomposite1 \
    libxdamage1 \
    libxrandr2 \
    libgbm1 \
    libpango-1.0-0 \
    libpangocairo-1.0-0 \
    libxss1 \
    libasound2 \
    libnss3 \
    libxshmfence1 \
    libxcb1 \
    libxfixes3 \
    libxrender1 \
    libxext6 \
    libatspi2.0-0 \
    --no-install-recommends && rm -rf /var/lib/apt/lists/*

# 安装 Chromium
RUN apt-get update && apt-get install -y chromium --no-install-recommends && rm -rf /var/lib/apt/lists/*

# 设置 Puppeteer 可执行路径
ENV PUPPETEER_EXECUTABLE_PATH=/usr/bin/chromium

# 安装 single-file-cli
RUN npm install -g single-file-cli

# 从构建阶段复制编译好的二进制文件
COPY --from=builder /app/target/release/singlefile-server /usr/local/bin/singlefile-server

# 创建工作目录
WORKDIR /app

# 创建临时目录
RUN mkdir -p /tmp

# 暴露端口
EXPOSE 8080

# 运行服务
CMD ["singlefile-server"]
