//! cloudflare-best-sstp
//!
//! 检测 SSTP 节点联通性，并将可用节点打印到标准输出供 GitHub Actions 保存。

mod config;
mod error;
mod http;
mod sstp;

use anyhow::Result;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::config::RequestConfig;
use crate::http::HttpClient;
use crate::sstp::SstpChecker;

#[tokio::main]
async fn main() -> std::process::ExitCode {
    init_logging();

    match run().await {
        Ok(_) => std::process::ExitCode::SUCCESS,
        Err(err) => {
            error!(error = ?err, "程序执行失败");
            std::process::ExitCode::FAILURE
        }
    }
}

/// 日志写入标准错误，保证标准输出只包含订阅内容。
fn init_logging() {
    let filter = EnvFilter::new("info");

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .with_writer(std::io::stderr)
        .init();
}

/// 主流程编排。
async fn run() -> Result<()> {
    let cfg = RequestConfig::from_env()?;
    info!(server = %cfg.server_host, "配置加载完成");

    let http = HttpClient::new()?;

    // 1~4：登录 -> 拉取列表 -> 并发检测 -> 过滤可用节点
    let checker = SstpChecker::new(&http, &cfg);
    let nodes = checker.run().await?;

    // 渲染输出内容
    let content = nodes
        .iter()
        .map(|n| n.render(&cfg.output_template, &cfg.server_host))
        .collect::<Vec<_>>()
        .join("\n");

    // 结果为空（上游列表为空 / 被限流返回空，或全部节点检测不通）时，
    // 无可用节点时不输出内容，由工作流保留仓库中的上次结果。
    if nodes.is_empty() {
        warn!(
            "结果为空（上游列表为空/被限流，或无可用节点），为避免覆盖上次的有效结果，本次跳过写入"
        );
        return Ok(());
    }

    println!("{content}");

    info!(count = nodes.len(), "全部完成");
    Ok(())
}
