//! GitHub 仓库同步输出目标（基于 GitHub Contents API）。
//!
//! 采用 REST API 而非在容器内调用 git，优点：
//! - 运行镜像无需安装 git / 配置 SSH；
//! - 天然支持任意分支、作者信息、提交信息；
//! - 幂等：内容未变化时自动跳过提交。

use anyhow::{Context, Result};
use async_trait::async_trait;
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use reqwest::{Client, StatusCode};
use serde::Deserialize;
use serde_json::json;
use tracing::{debug, info};

use super::OutputSink;
use crate::config::GithubConfig;
use crate::error::GithubError;

pub struct GithubSink {
    cfg: GithubConfig,
    client: Client,
}

#[derive(Debug, Deserialize)]
struct ContentMeta {
    sha: String,
    #[serde(default)]
    content: Option<String>,
}

impl GithubSink {
    pub fn new(cfg: GithubConfig, client: Client) -> Self {
        Self { cfg, client }
    }

    fn token(&self) -> Result<&str, GithubError> {
        self.cfg
            .token
            .as_deref()
            .ok_or_else(|| GithubError::Config("缺少 GITHUB_TOKEN".into()))
    }

    fn repo(&self) -> Result<&str, GithubError> {
        self.cfg
            .repo
            .as_deref()
            .ok_or_else(|| GithubError::Config("缺少 GITHUB_REPO (owner/repo)".into()))
    }

    fn contents_url(&self) -> Result<String, GithubError> {
        Ok(format!(
            "{}/repos/{}/contents/{}",
            self.cfg.api_base.trim_end_matches('/'),
            self.repo()?,
            self.cfg.file_path.trim_start_matches('/')
        ))
    }

    fn auth_request(
        &self,
        builder: reqwest::RequestBuilder,
    ) -> Result<reqwest::RequestBuilder, GithubError> {
        Ok(builder
            .header("Authorization", format!("Bearer {}", self.token()?))
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28"))
    }

    /// 查询远端文件的 sha 与现有内容；文件不存在返回 Ok(None)。
    async fn fetch_remote(&self) -> Result<Option<ContentMeta>, GithubError> {
        let url = self.contents_url()?;
        let resp = self
            .auth_request(self.client.get(&url).query(&[("ref", &self.cfg.branch)]))?
            .send()
            .await?;

        match resp.status() {
            StatusCode::OK => {
                let meta = resp.json::<ContentMeta>().await?;
                Ok(Some(meta))
            }
            StatusCode::NOT_FOUND => Ok(None),
            status => Err(GithubError::Api {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            }),
        }
    }

    /// 创建或更新文件。
    async fn put_content(&self, content: &str, sha: Option<&str>) -> Result<(), GithubError> {
        let url = self.contents_url()?;
        let encoded = STANDARD.encode(content.as_bytes());

        let mut body = json!({
            "message": self.cfg.commit_message,
            "content": encoded,
            "branch": self.cfg.branch,
        });
        if let Some(sha) = sha {
            body["sha"] = json!(sha);
        }
        if let (Some(name), Some(email)) = (&self.cfg.author_name, &self.cfg.author_email) {
            body["committer"] = json!({ "name": name, "email": email });
            body["author"] = json!({ "name": name, "email": email });
        }

        let resp = self
            .auth_request(self.client.put(&url))?
            .json(&body)
            .send()
            .await?;

        let status = resp.status();
        if status.is_success() {
            Ok(())
        } else {
            Err(GithubError::Api {
                status: status.as_u16(),
                body: resp.text().await.unwrap_or_default(),
            })
        }
    }
}

/// 解码 GitHub 返回的 base64 内容（含换行符需先剔除）。
fn decode_remote_content(raw: &str) -> Option<String> {
    let cleaned: String = raw.chars().filter(|c| !c.is_whitespace()).collect();
    STANDARD
        .decode(cleaned.as_bytes())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
}

#[async_trait]
impl OutputSink for GithubSink {
    fn name(&self) -> &'static str {
        "github"
    }

    async fn write(&self, content: &str) -> Result<()> {
        let remote = self.fetch_remote().await.context("查询远端文件失败")?;

        let sha = match &remote {
            Some(meta) => {
                // 内容未变化则跳过提交（幂等）。
                if let Some(existing) = meta.content.as_deref().and_then(decode_remote_content) {
                    if existing == content {
                        info!("远端内容未变化，跳过提交");
                        return Ok(());
                    }
                }
                debug!(sha = %meta.sha, "远端文件已存在，执行更新");
                Some(meta.sha.as_str())
            }
            None => {
                debug!("远端文件不存在，执行创建");
                None
            }
        };

        self.put_content(content, sha)
            .await
            .context("提交到 GitHub 失败")?;
        info!(
            repo = %self.repo().unwrap_or("?"),
            path = %self.cfg.file_path,
            branch = %self.cfg.branch,
            "已同步到 GitHub"
        );
        Ok(())
    }
}
