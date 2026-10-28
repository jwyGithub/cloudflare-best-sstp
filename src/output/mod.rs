//! 输出模块。
//!
//! 通过 `OutputSink` trait 抽象不同的输出目标，便于扩展（新增目标只需
//! 实现该 trait 并在 `build_sinks` 中注册）。目前内置：
//! - [`file::FileSink`]：写入本地文件（挂载到容器卷或 CI 产物）。
//! - [`github::GithubSink`]：通过 GitHub Contents API 同步到仓库。

pub mod file;
pub mod github;

use anyhow::Result;
use async_trait::async_trait;
use tracing::{error, info};

use crate::config::Config;
use crate::http::HttpClient;

/// 统一的输出目标抽象。
#[async_trait]
pub trait OutputSink: Send + Sync {
    /// 目标名称（用于日志）。
    fn name(&self) -> &'static str;

    /// 写入内容。
    async fn write(&self, content: &str) -> Result<()>;
}

/// 根据配置构建所有启用的输出目标。
pub fn build_sinks(cfg: &Config, http: &HttpClient) -> Vec<Box<dyn OutputSink>> {
    let mut sinks: Vec<Box<dyn OutputSink>> = Vec::new();

    if cfg.output.file_enabled {
        sinks.push(Box::new(file::FileSink::new(cfg.output.file_path.clone())));
    }

    if cfg.github.enabled {
        sinks.push(Box::new(github::GithubSink::new(
            cfg.github.clone(),
            http.inner().clone(),
        )));
    }

    sinks
}

/// 将内容写入所有目标。单个目标失败不影响其他目标（高可用），
/// 但只要有任一目标失败，最终返回错误以便上层感知。
pub async fn write_all(sinks: &[Box<dyn OutputSink>], content: &str) -> Result<()> {
    if sinks.is_empty() {
        info!("未启用任何输出目标");
        return Ok(());
    }

    let mut failed = 0usize;
    for sink in sinks {
        match sink.write(content).await {
            Ok(_) => info!(target = sink.name(), "输出成功"),
            Err(err) => {
                failed += 1;
                error!(target = sink.name(), error = %err, "输出失败");
            }
        }
    }

    if failed > 0 {
        anyhow::bail!("{} 个输出目标写入失败", failed);
    }
    Ok(())
}
