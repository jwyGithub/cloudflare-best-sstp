//! SSTP 检测相关数据模型。

use serde::Deserialize;

/// 检测接口返回结构。
///
/// 接口对成功 / 失败返回的字段并不完全一致，这里统一用可选字段兼容：
/// - 成功: `{ success, proxy, ip, loc, responseTime }`
/// - 失败: `{ success, proxy, responseTime, error }`
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CheckResponse {
    pub success: bool,
    #[serde(default)]
    pub proxy: Option<String>,
    #[serde(default)]
    pub ip: Option<String>,
    #[serde(default)]
    pub loc: Option<String>,
    #[serde(default)]
    pub response_time: Option<f64>,
    #[serde(default)]
    pub error: Option<String>,
}

/// 校验通过的可用节点（字段已规整，便于渲染输出）。
#[derive(Debug, Clone)]
pub struct SuccessNode {
    pub proxy: String,
    pub ip: String,
    pub loc: String,
    pub response_time: f64,
}

impl SuccessNode {
    /// 从检测响应构造，若关键字段缺失则返回 None。
    pub fn from_response(resp: &CheckResponse) -> Option<Self> {
        if !resp.success {
            return None;
        }
        Some(Self {
            proxy: resp.proxy.clone().unwrap_or_default(),
            ip: resp.ip.clone().unwrap_or_default(),
            loc: resp.loc.clone().unwrap_or_default(),
            response_time: resp.response_time.unwrap_or_default(),
        })
    }

    /// 按模板渲染输出行。
    ///
    /// 支持占位符：`{host}` `{loc}` `{proxy}` `{ip}` `{response_time}`
    pub fn render(&self, template: &str, host: &str) -> String {
        template
            .replace("{host}", host)
            .replace("{loc}", &self.loc)
            .replace("{proxy}", &self.proxy)
            .replace("{ip}", &self.ip)
            .replace("{response_time}", &self.response_time.to_string())
    }
}
