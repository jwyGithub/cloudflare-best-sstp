# cloudflare-best-sstp

用 Rust 重写的 SSTP 节点联通性检测工具：登录换取鉴权 Cookie → 拉取节点订阅列表 →
并发检测每个节点的可用性与延迟 → 将可用节点按模板渲染后输出到**本地文件**并可选**同步到 GitHub 仓库**。

全部配置由**环境变量**驱动，二进制运行在**容器**中，通过 **GitHub Actions** 打包发布与定时执行。

---

## 设计目标与取舍

| 目标 | 做法 |
| --- | --- |
| **高可用** | 瞬时错误（网络错误 / 5xx / 429）指数退避重试；CI 质量门禁保证主干始终可编译可发布 |
| **高性能** | `tokio` 异步 + 可配置并发度检测；连接池复用；release `lto=fat` + `panic=abort` + `strip` |
| **不重复造轮子** | HTTP 用 `reqwest`，重试/并发用成熟 crate；镜像构建只有 `Dockerfile` 一处来源，compose 与 CI 共用 |
| **结构清晰** | 按职责分模块：`config` / `http` / `sstp` / `output`，各司其职 |
| **可扩展** | 输出目标抽象为 `Sink` trait（file / github），新增目标只需实现 trait 并在 `build_sinks` 注册 |

## 项目结构

```
src/
├── main.rs            # 编排：读取配置 → 登录 → 拉表 → 并发检测 → 输出
├── config.rs          # 全部环境变量解析（带默认值 / 校验）
├── http.rs            # reqwest 客户端封装：代理 / 超时 / 连接池 / 重试退避
├── error.rs           # 统一错误类型
├── sstp/
│   ├── mod.rs
│   ├── model.rs       # 节点数据模型
│   └── checker.rs     # 登录鉴权 + 节点并发联通性检测
└── output/
    ├── mod.rs         # Sink trait + build_sinks / write_all
    ├── file.rs        # 本地文件输出目标
    └── github.rs      # GitHub 仓库同步（Contents API，内容无变化则幂等跳过）
```

## 配置（环境变量）

完整清单见 [`.env.example`](./.env.example)。关键项：

### 业务请求
| 变量 | 必填 | 默认 | 说明 |
| --- | --- | --- | --- |
| `SSTP_PASSWORD` | ✅ | — | 登录密码，换取鉴权 Cookie |
| `SSTP_LOGIN_URL` | | `.../login` | 登录地址 |
| `SSTP_LIST_URL` | | `.../vpngate` | 节点订阅列表地址 |
| `SSTP_CHECK_URL` | | `.../admin/check` | 单节点检测地址 |
| `SSTP_OUTPUT_TEMPLATE` | | `{host}#{loc}链式SSTP代理{proxy}` | 输出行模板，占位符 `{host} {loc} {proxy} {ip} {response_time}` |

### HTTP / 代理 / 性能
| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `SSTP_PROXY` | 空 | 代理地址，支持 `http` / `https` / `socks5`；留空回退 `ALL_PROXY` / 系统代理 |
| `SSTP_TIMEOUT_SECS` | `30` | 请求超时（秒） |
| `SSTP_MAX_RETRIES` | `3` | 瞬时错误最大重试次数 |
| `SSTP_RETRY_BACKOFF_MS` | `500` | 重试基准退避（指数 `base * 2^n`） |
| `SSTP_CONCURRENCY` | `16` | 节点检测并发度 |

### 输出
| 变量 | 默认 | 说明 |
| --- | --- | --- |
| `SSTP_OUTPUT_FILE_ENABLED` | `true` | 是否写入本地文件 |
| `OUTPUT_PATH` | `/app/output/sstp.txt` | 输出文件路径 |
| `GITHUB_SYNC_ENABLED` | `false` | 是否同步到 GitHub（`true` 时 TOKEN/REPO 必填） |
| `GITHUB_TOKEN` | — | PAT 或 Actions 内置 token，需 `contents:write` |
| `GITHUB_REPO` | — | 目标仓库 `owner/repo` |
| `GITHUB_BRANCH` | `main` | 目标分支 |
| `GITHUB_FILE_PATH` | `sstp.txt` | 结果在仓库中的路径 |
| `LOG_LEVEL` | `info` | `trace`/`debug`/`info`/`warn`/`error`（或用 `RUST_LOG` 精细控制） |

## 本地运行（Docker Compose）

```bash
cp .env.example .env         # 至少填写 SSTP_PASSWORD
docker compose up --build    # 构建并运行，结果落在 ./output/sstp.txt
```

这是一次性任务：检测完成后容器退出。结果通过 `./output` 卷持久化到宿主机。

## 直接本地运行（Rust）

```bash
cp .env.example .env
cargo run --release
```

## CI / CD（GitHub Actions）

| 工作流 | 触发 | 作用 |
| --- | --- | --- |
| [`ci.yml`](./.github/workflows/ci.yml) | push / PR | `fmt` + `clippy -D warnings` + `test` + release 构建 |
| [`docker-publish.yml`](./.github/workflows/docker-publish.yml) | push main / tag `v*` | 复用 `Dockerfile` 构建 **amd64 + arm64** 镜像并推送到 **GHCR**（带 GHA 层缓存） |
| [`scheduled-run.yml`](./.github/workflows/scheduled-run.yml) | 每 6 小时 / 手动 | 拉取已发布镜像运行检测，由程序经 Contents API 把结果写回 `sstp.txt` |

### 启用定时任务需要
1. 先让 `docker-publish` 成功发布一次 `:latest` 镜像。
2. 在仓库 **Settings → Secrets and variables → Actions** 配置：
   - `SSTP_PASSWORD`（必填）
   - `SSTP_PROXY`（可选）
3. `scheduled-run` 使用 Actions 内置 `GITHUB_TOKEN`（已授予 `contents:write`）提交结果，无需额外 PAT。

### 镜像地址
```
ghcr.io/<owner>/cloudflare-best-sstp:latest
```

## 新增输出目标（扩展示例）

在 `src/output/` 实现 `Sink` trait 并在 `build_sinks` 按配置注册即可，`main.rs` 与其他模块无需改动。
