//! 统一的错误类型定义。
//!
//! 业务边界使用 `anyhow` 便于携带上下文；对于配置与外部服务等
//! 需要调用方区分处理的错误，使用 `thiserror` 定义强类型错误。

use thiserror::Error;

/// 配置加载 / 校验相关错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("缺少必填环境变量: {0}")]
    MissingRequired(&'static str),

    #[error("环境变量 {key} 的值非法: {value} ({reason})")]
    InvalidValue {
        key: &'static str,
        value: String,
        reason: String,
    },
}

/// GitHub 同步相关错误。
#[derive(Debug, Error)]
pub enum GithubError {
    #[error("GitHub 配置不完整: {0}")]
    Config(String),

    #[error("GitHub API 请求失败: status={status}, body={body}")]
    Api { status: u16, body: String },

    #[error(transparent)]
    Http(#[from] reqwest::Error),
}
