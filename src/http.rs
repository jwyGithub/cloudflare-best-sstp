//! HTTP 客户端封装。
//!
//! 负责：
//! - 构建直连 / 超时 / 连接池复用的 `reqwest::Client`；
//! - 对瞬时错误（网络错误 / 5xx / 429）提供指数退避重试（高可用）。

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, Response};
use tracing::warn;

const MAX_RETRIES: u32 = 3;
const RETRY_BACKOFF: Duration = Duration::from_millis(500);

/// 可复用的 HTTP 客户端 + 重试策略。
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    max_retries: u32,
    backoff: Duration,
}

impl HttpClient {
    /// 根据配置构建客户端。
    pub fn new() -> Result<Self> {
        let builder = Client::builder()
            .user_agent("cloudflare-best-sstp/0.1")
            .timeout(Duration::from_secs(30))
            .connect_timeout(Duration::from_secs(15))
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60))
            .no_proxy();

        let client = builder.build().context("构建 HTTP 客户端失败")?;

        Ok(Self {
            client,
            max_retries: MAX_RETRIES,
            backoff: RETRY_BACKOFF,
        })
    }

    /// 暴露底层 client，便于使用 RequestBuilder 组装复杂请求。
    pub fn inner(&self) -> &Client {
        &self.client
    }

    /// 以给定的请求构造闭包执行请求，并对瞬时错误重试。
    ///
    /// 传入闭包而非 `RequestBuilder`，因为重试需要重新构造请求。
    pub async fn execute_with_retry<F>(&self, make_request: F) -> Result<Response>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0u32;
        loop {
            let result = make_request().send().await;

            match result {
                Ok(resp) => {
                    let status = resp.status();
                    // 对 5xx / 429 视为可重试。
                    if (status.is_server_error() || status.as_u16() == 429)
                        && attempt < self.max_retries
                    {
                        warn!(
                            status = status.as_u16(),
                            attempt, "收到可重试状态码，准备退避重试"
                        );
                        self.sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Ok(resp);
                }
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect() || err.is_request();
                    if retryable && attempt < self.max_retries {
                        warn!(error = %err, attempt, "请求失败，准备退避重试");
                        self.sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(err).context("HTTP 请求失败且已达最大重试次数");
                }
            }
        }
    }

    /// 发送请求并读取响应体文本。
    ///
    /// 相比 [`execute_with_retry`]，除了对网络错误 / 5xx / 429 重试外，
    /// 还会在 **2xx 但响应体为空** 时按退避重试——上游（Cloudflare Worker
    /// 代理 VPN Gate）在被限流时会返回 `200 + 空 body`，若不重试会被误判为
    /// 「列表为空」。重试耗尽后仍为空则返回该空字符串，交由上层决定。
    pub async fn fetch_text_nonempty<F>(&self, make_request: F) -> Result<String>
    where
        F: Fn() -> reqwest::RequestBuilder,
    {
        let mut attempt = 0u32;
        loop {
            match make_request().send().await {
                Ok(resp) => {
                    let status = resp.status();
                    if (status.is_server_error() || status.as_u16() == 429)
                        && attempt < self.max_retries
                    {
                        warn!(
                            status = status.as_u16(),
                            attempt, "收到可重试状态码，准备退避重试"
                        );
                        self.sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }

                    let text = resp.text().await.context("读取响应体失败")?;
                    if text.trim().is_empty() && attempt < self.max_retries {
                        warn!(
                            attempt,
                            "响应体为空（上游可能限流/间歇性空响应），准备退避重试"
                        );
                        self.sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Ok(text);
                }
                Err(err) => {
                    let retryable = err.is_timeout() || err.is_connect() || err.is_request();
                    if retryable && attempt < self.max_retries {
                        warn!(error = %err, attempt, "请求失败，准备退避重试");
                        self.sleep_backoff(attempt).await;
                        attempt += 1;
                        continue;
                    }
                    return Err(err).context("HTTP 请求失败且已达最大重试次数");
                }
            }
        }
    }

    /// 指数退避：base * 2^attempt。
    async fn sleep_backoff(&self, attempt: u32) {
        let factor = 2u32.saturating_pow(attempt);
        let delay = self.backoff.saturating_mul(factor);
        tokio::time::sleep(delay).await;
    }
}
