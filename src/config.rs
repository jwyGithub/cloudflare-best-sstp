//! 应用配置。
//!
//! 所有配置项均由环境变量驱动（便于在 docker-compose 中集中管理），
//! 并对敏感项（密码、Token）提供必填校验，其余项提供合理默认值。

use std::str::FromStr;
use std::time::Duration;

use crate::error::ConfigError;

/// 顶层配置聚合。
#[derive(Debug, Clone)]
pub struct Config {
    pub request: RequestConfig,
    /// 节点数据源（VPN Gate）配置。
    pub source: VpngateConfig,
    pub http: HttpConfig,
    pub output: OutputConfig,
    pub github: GithubConfig,
}

/// 业务请求相关配置（地址、密码、模板等）。
#[derive(Debug, Clone)]
pub struct RequestConfig {
    /// 登录地址，用于换取鉴权 Cookie。
    pub login_url: String,
    /// 登录密码（敏感，必填）。
    pub password: String,
    /// 登录表单中密码字段名。
    pub password_field: String,
    /// 鉴权 Cookie 名称。
    pub auth_cookie_name: String,
    /// 单个 SSTP 节点联通性检测地址。
    pub check_url: String,
    /// 请求使用的 Referer。
    pub referer: String,
    /// 输出前缀中展示的服务器主机名。
    pub server_host: String,
    /// 输出行模板，支持占位符：{host} {loc} {proxy} {ip} {response_time}
    pub output_template: String,
}

/// VPN Gate 数据源配置。
///
/// 直接访问 VPN Gate 官方 API（CSV），本地解析为 SSTP 节点列表，
/// 不再依赖任何第三方中转。各项均可通过环境变量覆盖，默认值开箱即用。
#[derive(Debug, Clone)]
pub struct VpngateConfig {
    /// VPN Gate API 地址（返回 CSV 格式的服务器清单）。
    pub api_url: String,
    /// 生成节点使用的 scheme（默认 sstp）。
    pub scheme: String,
    /// 节点用户名（VPN Gate 公共服务器默认 vpn）。
    pub username: String,
    /// 节点密码（VPN Gate 公共服务器默认 vpn）。
    pub password: String,
    /// 由 HostName 拼接出主机名的后缀（默认 .opengw.net）。
    pub host_suffix: String,
    /// 仅保留 OpenVPN 配置为 TCP 的服务器（SSTP 基于 TCP，UDP-only 节点丢弃）。
    pub tcp_only: bool,
}

/// HTTP 客户端相关配置（代理、超时、重试、并发）。
#[derive(Debug, Clone)]
pub struct HttpConfig {
    /// 显式代理地址（http/https/socks5）。为空时回退到系统代理环境变量。
    pub proxy: Option<String>,
    /// 请求超时。
    pub timeout: Duration,
    /// 连接超时。
    pub connect_timeout: Duration,
    /// 瞬时错误最大重试次数。
    pub max_retries: u32,
    /// 重试基准退避时间（指数退避）。
    pub retry_backoff: Duration,
    /// 节点检测并发度。
    pub concurrency: usize,
    /// User-Agent。
    pub user_agent: String,
}

/// 文件输出配置。
#[derive(Debug, Clone)]
pub struct OutputConfig {
    /// 是否写入本地文件。
    pub file_enabled: bool,
    /// 输出文件路径。
    pub file_path: String,
}

/// GitHub 同步配置（通过 Contents API，无需容器内安装 git）。
#[derive(Debug, Clone)]
pub struct GithubConfig {
    pub enabled: bool,
    pub token: Option<String>,
    /// owner/repo
    pub repo: Option<String>,
    pub branch: String,
    /// 目标文件在仓库中的路径。
    pub file_path: String,
    pub commit_message: String,
    pub author_name: Option<String>,
    pub author_email: Option<String>,
    /// GitHub API Base，默认 https://api.github.com（可指向 GHE）。
    pub api_base: String,
}

impl Config {
    /// 从环境变量加载配置，并对必填项做校验。
    pub fn from_env() -> Result<Self, ConfigError> {
        let request = RequestConfig {
            login_url: env_or("SSTP_LOGIN_URL", "https://vps-vless.nicoo.dpdns.org/login"),
            password: env_required("SSTP_PASSWORD")?,
            password_field: env_or("SSTP_PASSWORD_FIELD", "password"),
            auth_cookie_name: env_or("SSTP_AUTH_COOKIE_NAME", "auth"),
            check_url: env_or(
                "SSTP_CHECK_URL",
                "https://vps-vless.nicoo.dpdns.org/admin/check",
            ),
            referer: env_or("SSTP_REFERER", "https://vps-vless.nicoo.dpdns.org"),
            server_host: env_or("SSTP_SERVER_HOST", "vps-vless.nicoo.dpdns.org"),
            output_template: env_or("SSTP_OUTPUT_TEMPLATE", "{host}#{loc}链式SSTP代理{proxy}"),
        };

        // 节点数据源：直连 VPN Gate 官方 API，本地解析 CSV，不再经任何第三方中转。
        let source = VpngateConfig {
            api_url: env_or("SSTP_VPNGATE_API_URL", "https://www.vpngate.net/api/iphone/"),
            scheme: env_or("SSTP_NODE_SCHEME", "sstp"),
            username: env_or("SSTP_NODE_USERNAME", "vpn"),
            password: env_or("SSTP_NODE_PASSWORD", "vpn"),
            host_suffix: env_or("SSTP_NODE_HOST_SUFFIX", ".opengw.net"),
            tcp_only: env_bool("SSTP_VPNGATE_TCP_ONLY", true)?,
        };

        let http = HttpConfig {
            proxy: env_opt("SSTP_PROXY").or_else(|| env_opt("ALL_PROXY")),
            timeout: Duration::from_secs(env_parse("SSTP_TIMEOUT_SECS", 30)?),
            connect_timeout: Duration::from_secs(env_parse("SSTP_CONNECT_TIMEOUT_SECS", 15)?),
            max_retries: env_parse("SSTP_MAX_RETRIES", 3)?,
            retry_backoff: Duration::from_millis(env_parse("SSTP_RETRY_BACKOFF_MS", 500)?),
            concurrency: env_parse("SSTP_CONCURRENCY", 16usize)?.max(1),
            user_agent: env_or(
                "SSTP_USER_AGENT",
                "cloudflare-best-sstp/0.1 (+https://github.com/jwyGithub/cloudflare-best-sstp)",
            ),
        };

        let output = OutputConfig {
            file_enabled: env_bool("SSTP_OUTPUT_FILE_ENABLED", true)?,
            file_path: env_or("OUTPUT_PATH", "sstp.txt"),
        };

        let github_enabled = env_bool("GITHUB_SYNC_ENABLED", false)?;
        let github = GithubConfig {
            enabled: github_enabled,
            token: env_opt("GITHUB_TOKEN"),
            repo: env_opt("GITHUB_REPO"),
            branch: env_or("GITHUB_BRANCH", "main"),
            file_path: env_or("GITHUB_FILE_PATH", "sstp.txt"),
            commit_message: env_or("GITHUB_COMMIT_MESSAGE", "chore: update sstp nodes"),
            author_name: env_opt("GITHUB_AUTHOR_NAME"),
            author_email: env_opt("GITHUB_AUTHOR_EMAIL"),
            api_base: env_or("GITHUB_API_BASE", "https://api.github.com"),
        };

        // 开启 GitHub 同步时校验必填项。
        if github.enabled {
            if github.token.is_none() {
                return Err(ConfigError::MissingRequired("GITHUB_TOKEN"));
            }
            if github.repo.is_none() {
                return Err(ConfigError::MissingRequired("GITHUB_REPO"));
            }
        }

        Ok(Config {
            request,
            source,
            http,
            output,
            github,
        })
    }
}

// ---------- 环境变量读取辅助函数 ----------

/// 读取字符串，缺省返回默认值。
fn env_or(key: &str, default: &str) -> String {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => v,
        _ => default.to_string(),
    }
}

/// 读取可选字符串，缺省或空返回 None。
fn env_opt(key: &str) -> Option<String> {
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => Some(v),
        _ => None,
    }
}

/// 读取必填字符串，缺省报错。
fn env_required(key: &'static str) -> Result<String, ConfigError> {
    env_opt(key).ok_or(ConfigError::MissingRequired(key))
}

/// 读取并解析为指定类型，缺省返回默认值。
fn env_parse<T>(key: &'static str, default: T) -> Result<T, ConfigError>
where
    T: FromStr,
    T::Err: std::fmt::Display,
{
    match std::env::var(key) {
        Ok(v) if !v.trim().is_empty() => {
            v.trim()
                .parse::<T>()
                .map_err(|e| ConfigError::InvalidValue {
                    key,
                    value: v,
                    reason: e.to_string(),
                })
        }
        _ => Ok(default),
    }
}

/// 读取布尔值，支持 true/false/1/0/yes/no/on/off。
fn env_bool(key: &'static str, default: bool) -> Result<bool, ConfigError> {
    match env_opt(key) {
        None => Ok(default),
        Some(v) => match v.trim().to_ascii_lowercase().as_str() {
            "1" | "true" | "yes" | "on" => Ok(true),
            "0" | "false" | "no" | "off" => Ok(false),
            other => Err(ConfigError::InvalidValue {
                key,
                value: other.to_string(),
                reason: "期望布尔值 (true/false/1/0/yes/no/on/off)".to_string(),
            }),
        },
    }
}
