# cloudflare-best-sstp

使用 Rust 检测 SSTP 节点可用性，并由 GitHub Actions 定时运行。Action 直接构建并运行程序，将可用节点写入仓库根目录的 `sstp.txt` 并提交更新，无需 Docker。

## 配置

在仓库 **Settings → Secrets and variables → Actions** 配置：

| 名称 | 类型 | 说明 |
| --- | --- | --- |
| `SSTP_PASSWORD` | Secret | 登录密码 |
| `SSTP_SERVER_HOST` | Variable | 登录与检测服务的主机名，例如 `example.com`；程序据此生成 `https://<host>/login`、`https://<host>/admin/check` 和 Referer |
| `SSTP_LIST_URL` | Variable | SSTP 订阅列表接口地址 |
| `SSTP_OUTPUT_TEMPLATE` | Variable | 输出模板，支持 `{host}`、`{loc}`、`{proxy}`、`{ip}`、`{response_time}` |

生产运行配置全部从 Actions 注入。

## 定时运行

`.github/workflows/scheduled-run.yml` 每天 UTC 00:00 运行，也可以从 Actions 页面手动触发。工作流直接在 GitHub 托管的 Ubuntu Runner 上运行 `cargo run --release`，并用 `GITHUB_TOKEN` 将更新后的 `sstp.txt` 提交到默认分支根目录。需要在仓库 **Settings → Actions → General → Workflow permissions** 允许工作流读写仓库内容。

若本次没有可用节点，工作流保留上次的 `sstp.txt`；若内容没有变化则跳过提交。

## 本地运行

安装 Rust 后在当前 shell 设置上述四个环境变量，再运行：

```sh
cargo run --release
```

程序将结果打印到标准输出。HTTP 请求直连，不读取或使用代理环境变量。

## CI

`.github/workflows/ci.yml` 在分支推送、Pull Request 和手动触发时运行格式检查、Clippy、测试及 Release 构建。
