//! VPN Gate 数据源。
//!
//! 直连 VPN Gate 官方 API（返回 CSV），本地解析为 SSTP 节点 URI 列表，
//! 不依赖任何第三方中转。流程：
//! 1. 拉取 CSV；
//! 2. 定位以 `#HostName` 开头的表头，按列名建立下标映射；
//! 3. 解码每行的 Base64 OpenVPN 配置，提取 TCP `remote` 端口；
//! 4. 拼接为 `sstp://user:pass@host:port`。

use anyhow::{Context, Result};
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use tracing::{debug, warn};

use crate::config::VpngateConfig;
use crate::http::HttpClient;

/// VPN Gate 节点数据源。
pub struct VpngateSource<'a> {
    http: &'a HttpClient,
    cfg: &'a VpngateConfig,
}

impl<'a> VpngateSource<'a> {
    pub fn new(http: &'a HttpClient, cfg: &'a VpngateConfig) -> Self {
        Self { http, cfg }
    }

    /// 拉取并解析 VPN Gate 服务器清单，返回 `sstp://...` 节点 URI 列表。
    pub async fn fetch_nodes(&self) -> Result<Vec<String>> {
        // VPN Gate 偶发返回「200 + 空 body」，沿用带空响应退避重试的读取。
        let csv = self
            .http
            .fetch_text_nonempty(|| self.http.inner().get(&self.cfg.api_url))
            .await
            .context("拉取 VPN Gate API 响应失败")?;

        Ok(parse_csv(self.cfg, &csv))
    }
}

/// 解析 VPN Gate CSV 文本为节点 URI 列表（纯函数，便于测试）。
fn parse_csv(cfg: &VpngateConfig, csv: &str) -> Vec<String> {
    let mut header: Option<Vec<String>> = None;
    let mut nodes = Vec::new();

    for raw in csv.lines() {
        let line = raw.trim();
        // 跳过空行与 VPN Gate 的段落标记行（`*vpn_servers` 开头 / 结尾 `*`）。
        if line.is_empty() || line == "*" || line.starts_with("*vpn_servers") {
            continue;
        }

        if line.starts_with('#') {
            // 表头行：去掉前导 `#`，按逗号拆分列名。
            let cols = line
                .trim_start_matches('#')
                .split(',')
                .map(|s| s.trim().to_string())
                .collect::<Vec<_>>();
            header = Some(cols);
            continue;
        }

        let Some(cols) = header.as_ref() else {
            // 还没遇到表头，无法可靠按列解析，跳过。
            continue;
        };

        let fields: Vec<&str> = line.split(',').collect();
        if let Some(node) = row_to_node(cfg, cols, &fields) {
            nodes.push(node);
        }
    }

    if header.is_none() {
        warn!("VPN Gate 响应中未找到以 #HostName 开头的表头，可能格式有变");
    }
    debug!(count = nodes.len(), "VPN Gate 节点解析完成");
    nodes
}

/// 将单行字段转换为一个 `sstp://` URI；无法解析 / 非 TCP 时返回 None。
fn row_to_node(cfg: &VpngateConfig, header: &[String], fields: &[&str]) -> Option<String> {
    let get = |name: &str| -> Option<&str> {
        header
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
            .and_then(|i| fields.get(i).copied())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
    };

    let host_name = get("HostName")?;
    let config_b64 = get("OpenVPN_ConfigData_Base64")?;

    // 解码 OpenVPN 配置，提取 TCP remote 端口。
    let decoded = STANDARD.decode(config_b64.as_bytes()).ok()?;
    let config = String::from_utf8_lossy(&decoded);
    let (port, is_tcp) = parse_remote(&config)?;

    // SSTP 基于 TCP，默认丢弃 UDP-only 节点。
    if cfg.tcp_only && !is_tcp {
        return None;
    }

    // HostName 可能已是 FQDN（含 `.`），此时直接使用，避免重复拼接后缀。
    let host = if host_name.contains('.') {
        host_name.to_string()
    } else {
        format!("{}{}", host_name, cfg.host_suffix)
    };

    Some(format!(
        "{}://{}:{}@{}:{}",
        cfg.scheme, cfg.username, cfg.password, host, port
    ))
}

/// 从 OpenVPN 配置文本中解析出连接端口及其是否为 TCP。
///
/// 兼容两种写法：
/// - `remote <host> <port> <proto>`（proto 作为第 3 个 token）；
/// - `remote <host> <port>` 搭配独立的 `proto tcp` / `proto udp` 指令行。
fn parse_remote(config: &str) -> Option<(u16, bool)> {
    let mut global_proto: Option<bool> = None; // Some(true)=tcp, Some(false)=udp
    let mut remote: Option<(u16, Option<bool>)> = None;

    for line in config.lines() {
        let mut it = line.trim().split_whitespace();
        match it.next() {
            Some("proto") => {
                if let Some(p) = it.next() {
                    global_proto = Some(p.to_ascii_lowercase().starts_with("tcp"));
                }
            }
            Some("remote") => {
                // remote <host> <port> [proto]
                let _host = it.next();
                let port = it.next().and_then(|p| p.parse::<u16>().ok());
                let proto = it.next().map(|p| p.to_ascii_lowercase().starts_with("tcp"));
                // 仅取首个 remote：VPN Gate 每份配置通常只有一个 remote。
                if let (Some(port), true) = (port, remote.is_none()) {
                    remote = Some((port, proto));
                }
            }
            _ => {}
        }
    }

    let (port, remote_proto) = remote?;
    // 判定 TCP：优先 remote 行自带 proto，其次全局 proto 指令，均缺失则视为非 TCP。
    let is_tcp = remote_proto.or(global_proto).unwrap_or(false);
    Some((port, is_tcp))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg() -> VpngateConfig {
        VpngateConfig {
            api_url: "https://www.vpngate.net/api/iphone/".to_string(),
            scheme: "sstp".to_string(),
            username: "vpn".to_string(),
            password: "vpn".to_string(),
            host_suffix: ".opengw.net".to_string(),
            tcp_only: true,
        }
    }

    #[test]
    fn parses_proto_line_and_remote() {
        let ovpn = "dev tun\nproto tcp\nremote 1.2.3.4 1621\ncipher AES-128-CBC\n";
        assert_eq!(parse_remote(ovpn), Some((1621, true)));
    }

    #[test]
    fn parses_proto_on_remote_line() {
        let ovpn = "dev tun\nremote 1.2.3.4 995 tcp\n";
        assert_eq!(parse_remote(ovpn), Some((995, true)));
    }

    #[test]
    fn detects_udp() {
        let ovpn = "dev tun\nproto udp\nremote 1.2.3.4 1195\n";
        assert_eq!(parse_remote(ovpn), Some((1195, false)));
    }

    #[test]
    fn parses_csv_builds_tcp_uri_and_skips_udp() {
        let tcp_cfg = STANDARD.encode("proto tcp\nremote 9.9.9.9 1741\n");
        let udp_cfg = STANDARD.encode("proto udp\nremote 8.8.8.8 1195\n");
        let csv = format!(
            "*vpn_servers\n\
             #HostName,IP,Score,CountryShort,OpenVPN_ConfigData_Base64\n\
             vpn727209238,9.9.9.9,100,JP,{tcp_cfg}\n\
             vpnudp,8.8.8.8,50,US,{udp_cfg}\n\
             *\n"
        );

        let nodes = parse_csv(&cfg(), &csv);
        // UDP 节点被 tcp_only 过滤，仅保留 TCP 节点。
        assert_eq!(nodes, vec!["sstp://vpn:vpn@vpn727209238.opengw.net:1741"]);
    }

    #[test]
    fn keeps_udp_when_tcp_only_disabled() {
        let udp_cfg = STANDARD.encode("proto udp\nremote 8.8.8.8 1195\n");
        let csv = format!(
            "#HostName,OpenVPN_ConfigData_Base64\nvpnudp,{udp_cfg}\n"
        );
        let mut c = cfg();
        c.tcp_only = false;
        let nodes = parse_csv(&c, &csv);
        assert_eq!(nodes, vec!["sstp://vpn:vpn@vpnudp.opengw.net:1195"]);
    }

    #[test]
    fn fqdn_hostname_not_double_suffixed() {
        let tcp_cfg = STANDARD.encode("proto tcp\nremote 1.1.1.1 443\n");
        let csv = format!(
            "#HostName,OpenVPN_ConfigData_Base64\npublic-vpn-65.opengw.net,{tcp_cfg}\n"
        );
        let nodes = parse_csv(&cfg(), &csv);
        assert_eq!(nodes, vec!["sstp://vpn:vpn@public-vpn-65.opengw.net:443"]);
    }
}
