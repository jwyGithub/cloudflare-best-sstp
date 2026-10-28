# syntax=docker/dockerfile:1

# ============================================================
# 构建阶段：在 Rust 官方镜像里编译出 release 二进制
# ============================================================
FROM rust:1-bookworm AS builder

WORKDIR /app

# 先只拷贝清单文件，利用 Docker 层缓存预编译依赖：
# 只要 Cargo.toml / Cargo.lock 不变，依赖层就能命中缓存，加速后续构建。
COPY Cargo.toml Cargo.lock ./
RUN mkdir -p src \
    && echo 'fn main() {}' > src/main.rs \
    && cargo build --release --locked \
    && rm -rf src

# 再拷贝真实源码并构建。touch 确保 cargo 感知到源码变更而重新编译本 crate。
COPY . .
RUN touch src/main.rs \
    && cargo build --release --locked

# ============================================================
# 运行阶段：最小化 Debian + CA 证书，仅携带二进制
# ============================================================
FROM debian:bookworm-slim AS runtime

# ca-certificates: TLS 根证书（兼容自定义/企业代理场景）
# tini: 作为 PID 1 正确转发信号并回收僵尸进程
RUN apt-get update \
    && apt-get install -y --no-install-recommends ca-certificates tini \
    && rm -rf /var/lib/apt/lists/*

# 以非 root 用户运行，降低攻击面
RUN useradd --system --uid 10001 --create-home --home-dir /app appuser
WORKDIR /app

# 输出目录（可被 volume 覆盖挂载）
RUN mkdir -p /app/output && chown -R appuser:appuser /app

COPY --from=builder /app/target/release/cloudflare-best-sstp /usr/local/bin/cloudflare-best-sstp

USER appuser

# 默认输出路径指向挂载目录，便于持久化
ENV OUTPUT_PATH=/app/output/sstp.txt \
    LOG_LEVEL=info

ENTRYPOINT ["/usr/bin/tini", "--", "/usr/local/bin/cloudflare-best-sstp"]
