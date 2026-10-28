//! 本地文件输出目标。

use anyhow::{Context, Result};
use async_trait::async_trait;
use tracing::debug;

use super::OutputSink;

/// 将结果写入指定文件路径（自动创建父目录）。
pub struct FileSink {
    path: String,
}

impl FileSink {
    pub fn new(path: String) -> Self {
        Self { path }
    }
}

#[async_trait]
impl OutputSink for FileSink {
    fn name(&self) -> &'static str {
        "file"
    }

    async fn write(&self, content: &str) -> Result<()> {
        if let Some(parent) = std::path::Path::new(&self.path).parent() {
            if !parent.as_os_str().is_empty() {
                tokio::fs::create_dir_all(parent)
                    .await
                    .with_context(|| format!("创建目录失败: {}", parent.display()))?;
            }
        }
        tokio::fs::write(&self.path, content)
            .await
            .with_context(|| format!("写入文件失败: {}", self.path))?;
        debug!(path = %self.path, bytes = content.len(), "文件写入完成");
        Ok(())
    }
}
