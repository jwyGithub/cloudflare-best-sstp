//! HTTP 客户端封装。
//!
//! 负责：
//! - 构建带代理 / 超时 / 连接池复用的 `reqwest::Client`（高性能）；
//! - 对瞬时错误（网络错误 / 5xx / 429）提供指数退避重试（高可用）。

use std::time::Duration;

use anyhow::{Context, Result};
use reqwest::{Client, Proxy, Response};
use tracing::{debug, warn};

use crate::config::HttpConfig;

/// 可复用的 HTTP 客户端 + 重试策略。
#[derive(Clone)]
pub struct HttpClient {
    client: Client,
    max_retries: u32,
    backoff: Duration,
}

impl HttpClient {
    /// 根据配置构建客户端。
    pub fn new(cfg: &HttpConfig) -> Result<Self> {
        let mut builder = Client::builder()
            .user_agent(&cfg.user_agent)
            .timeout(cfg.timeout)
            .connect_timeout(cfg.connect_timeout)
            .pool_idle_timeout(Duration::from_secs(90))
            .tcp_keepalive(Duration::from_secs(60));

        // 显式代理优先；未显式配置时，reqwest 默认会读取系统 HTTP(S)_PROXY。
        if let Some(proxy_url) = &cfg.proxy {
            let proxy =
                Proxy::all(proxy_url).with_context(|| format!("非法的代理地址: {proxy_url}"))?;
            builder = builder.proxy(proxy);
            debug!(proxy = %proxy_url, "已启用显式代理");
        }

        let client = builder.build().context("构建 HTTP 客户端失败")?;

        Ok(Self {
            client,
            max_retries: cfg.max_retries,
            backoff: cfg.backoff(),
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
                        warn!(attempt, "响应体为空（上游可能限流/间歇性空响应），准备退避重试");
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

impl HttpConfig {
    fn backoff(&self) -> Duration {
        self.retry_backoff
    }
}
