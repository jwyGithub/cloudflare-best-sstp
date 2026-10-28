//! SSTP 联通性检测核心逻辑。
//!
//! 对应原 index.ts 的流程：
//! 1. 登录换取鉴权 Cookie；
//! 2. 拉取 SSTP 节点列表；
//! 3. 并发检测每个节点联通性；
//! 4. 过滤出可用节点并按模板渲染。

use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{anyhow, Context, Result};
use futures::stream::{self, StreamExt};
use tracing::{debug, info, warn};

use crate::config::{RequestConfig, VpngateConfig};
use crate::http::HttpClient;
use crate::sstp::model::{CheckResponse, SuccessNode};
use crate::sstp::source::VpngateSource;

/// SSTP 检测器。
pub struct SstpChecker<'a> {
    http: &'a HttpClient,
    cfg: &'a RequestConfig,
    source: &'a VpngateConfig,
    concurrency: usize,
}

impl<'a> SstpChecker<'a> {
    pub fn new(
        http: &'a HttpClient,
        cfg: &'a RequestConfig,
        source: &'a VpngateConfig,
        concurrency: usize,
    ) -> Self {
        Self {
            http,
            cfg,
            source,
            concurrency,
        }
    }

    /// 执行完整检测流程，返回可用节点列表（保持列表原始顺序）。
    pub async fn run(&self) -> Result<Vec<SuccessNode>> {
        let auth = self.get_auth().await.context("获取鉴权 Cookie 失败")?;
        info!("已获取鉴权 Cookie");

        let list = self.get_list().await.context("获取 SSTP 列表失败")?;
        info!(count = list.len(), "已获取 SSTP 列表");

        if list.is_empty() {
            warn!("SSTP 列表为空，直接返回");
            return Ok(Vec::new());
        }

        let nodes = self.check_all(&auth, list).await;
        info!(available = nodes.len(), "联通性检测完成");
        Ok(nodes)
    }

    /// 登录并解析出鉴权 Cookie 值。
    async fn get_auth(&self) -> Result<String> {
        let body = format!(
            "{}={}",
            self.cfg.password_field,
            urlencode(&self.cfg.password)
        );

        let resp = self
            .http
            .execute_with_retry(|| {
                self.http
                    .inner()
                    .post(&self.cfg.login_url)
                    .header("accept", "*/*")
                    .header(
                        "accept-language",
                        "zh-CN,zh;q=0.9,en;q=0.8,en-GB;q=0.7,en-US;q=0.6,zh-TW;q=0.5",
                    )
                    .header("content-type", "application/x-www-form-urlencoded")
                    .header("priority", "u=1, i")
                    .header("Referer", &self.cfg.login_url)
                    .body(body.clone())
            })
            .await?;

        let auth = extract_cookie(&resp, &self.cfg.auth_cookie_name).ok_or_else(|| {
            anyhow!(
                "未能从响应中解析出名为 `{}` 的 Cookie",
                self.cfg.auth_cookie_name
            )
        })?;

        Ok(auth)
    }

    /// 从 VPN Gate 官方 API 直接获取并解析出 SSTP 节点列表。
    async fn get_list(&self) -> Result<Vec<String>> {
        VpngateSource::new(self.http, self.source)
            .fetch_nodes()
            .await
            .context("从 VPN Gate 获取 SSTP 列表失败")
    }

    /// 并发检测所有节点，保持原始顺序。
    async fn check_all(&self, auth: &str, list: Vec<String>) -> Vec<SuccessNode> {
        let cookie = format!("{}={}", self.cfg.auth_cookie_name, auth);

        // 携带索引以便最终按原始顺序排序。
        let mut indexed: Vec<(usize, SuccessNode)> = stream::iter(list.into_iter().enumerate())
            .map(|(idx, item)| {
                let cookie = cookie.clone();
                async move {
                    match self.check_one(&item, &cookie).await {
                        Ok(Some(node)) => Some((idx, node)),
                        Ok(None) => None,
                        Err(err) => {
                            warn!(sstp = %item, error = %err, "节点检测异常，已跳过");
                            None
                        }
                    }
                }
            })
            .buffer_unordered(self.concurrency)
            .filter_map(|r| async move { r })
            .collect()
            .await;

        indexed.sort_by_key(|(idx, _)| *idx);
        indexed.into_iter().map(|(_, node)| node).collect()
    }

    /// 检测单个节点。返回 Ok(None) 表示检测成功但节点不可用。
    async fn check_one(&self, item: &str, cookie: &str) -> Result<Option<SuccessNode>> {
        debug!(sstp = %item, "开始检测节点");
        let resp = self
            .http
            .execute_with_retry(|| {
                self.http
                    .inner()
                    .get(&self.cfg.check_url)
                    .query(&[("sstp", item), ("_t", &timestamp_ms().to_string())])
                    .header("Referer", format!("{}/admin", self.cfg.referer))
                    .header("Cookie", cookie)
            })
            .await?;

        let parsed: CheckResponse = resp.json().await.context("解析检测响应 JSON 失败")?;
        debug!(sstp = %item, success = parsed.success, "节点检测结果");
        if !parsed.success {
            if let Some(err) = parsed.error.as_deref() {
                debug!(sstp = %item, reason = %err, "节点不可用");
            }
        }
        Ok(SuccessNode::from_response(&parsed))
    }
}

/// 从响应的 Set-Cookie 头中提取指定名称的 Cookie 值。
fn extract_cookie(resp: &reqwest::Response, name: &str) -> Option<String> {
    for value in resp.headers().get_all(reqwest::header::SET_COOKIE).iter() {
        let Ok(s) = value.to_str() else { continue };
        // 形如: `auth=xxxx; Path=/; HttpOnly`
        if let Some(first) = s.split(';').next() {
            let mut kv = first.splitn(2, '=');
            if let (Some(k), Some(v)) = (kv.next(), kv.next()) {
                if k.trim() == name {
                    return Some(v.trim().to_string());
                }
            }
        }
    }
    None
}

/// 当前毫秒时间戳（用于防缓存 `_t` 参数）。
fn timestamp_ms() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default()
}

/// 对 x-www-form-urlencoded 的 value 做最小化编码。
fn urlencode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}
