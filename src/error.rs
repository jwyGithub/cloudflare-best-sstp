//! 统一的错误类型定义。
//!
//! 配置错误使用强类型，运行期错误由 `anyhow` 携带上下文。

use thiserror::Error;

/// 配置加载 / 校验相关错误。
#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("缺少必填环境变量: {0}")]
    MissingRequired(&'static str),
}
