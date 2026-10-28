//! cloudflare-best-sstp
//!
//! 检测 SSTP 节点联通性，将可用节点输出到文件并可选同步到 GitHub 仓库。
//! 全部配置由环境变量驱动，便于在 docker-compose 中集中管理。

mod config;
mod error;
mod http;
mod output;
mod sstp;

use anyhow::Result;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::Config;
use crate::http::HttpClient;
use crate::sstp::SstpChecker;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    // 本地开发自动加载 .env；容器中使用真实环境变量，无 .env 亦不影响。
    let _ = dotenvy::dotenv();
    init_logging();

    match run().await {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            error!(error = ?err, "程序执行失败");
            std::process::ExitCode::FAILURE
        }
    }
}

/// 初始化日志：优先 RUST_LOG，其次 LOG_LEVEL，默认 info。
fn init_logging() {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| {
        let level = std::env::var("LOG_LEVEL").unwrap_or_else(|_| "info".to_string());
        EnvFilter::new(level)
    });

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .init();
}

/// 主流程编排。
async fn run() -> Result<()> {
    let cfg = Config::from_env()?;
    info!(
        concurrency = cfg.http.concurrency,
        proxy = cfg.http.proxy.is_some(),
        github_sync = cfg.github.enabled,
        "配置加载完成"
    );

    let http = HttpClient::new(&cfg.http)?;

    // 1~4：登录 -> 从 VPN Gate 拉取列表 -> 并发检测 -> 过滤可用节点
    let checker = SstpChecker::new(&http, &cfg.request, &cfg.source, cfg.http.concurrency);
    let nodes = checker.run().await?;

    // 渲染输出内容
    let content = nodes
        .iter()
        .map(|n| n.render(&cfg.request.output_template, &cfg.request.server_host))
        .collect::<Vec<_>>()
        .join("\n");

    // 结果为空（上游列表为空 / 被限流返回空，或全部节点检测不通）时，
    // 绝不写出——否则会用空内容覆盖掉上一次的有效订阅，破坏可用性。
    // 保留上次的 sstp.txt（本地与 GitHub），本次安静跳过。
    if nodes.is_empty() {
        warn!("结果为空（上游列表为空/被限流，或无可用节点），为避免覆盖上次的有效结果，本次跳过写入");
        return Ok(());
    }

    // 输出到所有启用的目标（文件 / GitHub）
    let sinks = output::build_sinks(&cfg, &http);
    output::write_all(&sinks, &content).await?;

    info!(count = nodes.len(), "全部完成");
    Ok(())
}
