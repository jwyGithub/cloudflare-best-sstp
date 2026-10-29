//! GitHub Actions 环境变量配置。

use crate::error::ConfigError;

#[derive(Debug, Clone)]
pub struct RequestConfig {
    pub password: String,
    pub server_host: String,
    pub list_url: String,
    pub output_template: String,
    pub login_url: String,
    pub password_field: String,
    pub auth_cookie_name: String,
    pub check_url: String,
    pub referer: String,
}

impl RequestConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let server_host = required("SSTP_SERVER_HOST")?
            .trim_end_matches('/')
            .to_string();
        let (login_url, check_url, referer) = service_urls(&server_host);

        Ok(Self {
            password: required("SSTP_PASSWORD")?,
            server_host,
            list_url: required("SSTP_LIST_URL")?,
            output_template: required("SSTP_OUTPUT_TEMPLATE")?,
            login_url,
            password_field: "password".to_string(),
            auth_cookie_name: "auth".to_string(),
            check_url,
            referer,
        })
    }
}

fn service_urls(host: &str) -> (String, String, String) {
    let base_url = format!("https://{}", host.trim_end_matches('/'));
    (
        format!("{base_url}/login"),
        format!("{base_url}/admin/check"),
        base_url,
    )
}

fn required(key: &'static str) -> Result<String, ConfigError> {
    std::env::var(key)
        .ok()
        .filter(|value| !value.trim().is_empty())
        .ok_or(ConfigError::MissingRequired(key))
}

#[cfg(test)]
mod tests {
    use super::service_urls;

    #[test]
    fn derives_service_urls_from_host() {
        assert_eq!(
            service_urls("example.com/"),
            (
                "https://example.com/login".to_string(),
                "https://example.com/admin/check".to_string(),
                "https://example.com".to_string(),
            )
        );
    }
}
