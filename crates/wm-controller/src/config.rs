//! Controller configuration loading (WM-0104).
//!
//! Layered via figment: defaults < config file (TOML) < `WM_CONTROLLER_*`
//! environment variables.

use figment::{
    providers::{Env, Format, Toml},
    Figment,
};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ControllerConfig {
    /// HTTP listen address for the REST/status API.
    #[serde(default = "default_http_addr")]
    pub http_addr: String,
    /// gRPC listen address for the control-plane service.
    #[serde(default = "default_grpc_addr")]
    pub grpc_addr: String,
    /// Expected worker heartbeat interval in milliseconds (lease basis).
    #[serde(default = "default_heartbeat_interval_ms")]
    pub heartbeat_interval_ms: u32,
}

fn default_http_addr() -> String {
    "127.0.0.1:7600".into()
}

fn default_grpc_addr() -> String {
    "127.0.0.1:7601".into()
}

fn default_heartbeat_interval_ms() -> u32 {
    5000
}

impl Default for ControllerConfig {
    fn default() -> Self {
        Self {
            http_addr: default_http_addr(),
            grpc_addr: default_grpc_addr(),
            heartbeat_interval_ms: default_heartbeat_interval_ms(),
        }
    }
}

impl ControllerConfig {
    /// Load config from an optional TOML file plus `WM_CONTROLLER_*` env
    /// vars (e.g. `WM_CONTROLLER_HTTP_ADDR`).
    pub fn load(path: Option<&Path>) -> Result<Self, figment::Error> {
        let mut figment = Figment::from(figment::providers::Serialized::defaults(
            ControllerConfig::default(),
        ));
        if let Some(p) = path {
            figment = figment.merge(Toml::file(p));
        }
        figment
            .merge(Env::prefixed("WM_CONTROLLER_").lowercase(true))
            .extract()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn defaults_apply_without_file() {
        let cfg = ControllerConfig::load(None).unwrap();
        assert_eq!(cfg.http_addr, "127.0.0.1:7600");
        assert_eq!(cfg.grpc_addr, "127.0.0.1:7601");
        assert_eq!(cfg.heartbeat_interval_ms, 5000);
    }

    #[test]
    fn toml_file_overrides_defaults() {
        let mut f = tempfile::Builder::new().suffix(".toml").tempfile().unwrap();
        writeln!(
            f,
            "http_addr = '0.0.0.0:8000'\nheartbeat_interval_ms = 1000"
        )
        .unwrap();
        let cfg = ControllerConfig::load(Some(f.path())).unwrap();
        assert_eq!(cfg.http_addr, "0.0.0.0:8000");
        assert_eq!(cfg.heartbeat_interval_ms, 1000);
        // unset field keeps default
        assert_eq!(cfg.grpc_addr, "127.0.0.1:7601");
    }
}
