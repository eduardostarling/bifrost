use std::collections::HashMap;
use std::fs::{self, File};
use std::sync::Arc;

use axum_server::tls_rustls::RustlsConfig;
use camino::Utf8Path;
use chrono::Utc;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use crate::error::{ApiError, ApiResult};
use crate::hue::legacy_api::{ApiConfig, ApiShortConfig, Whitelist};
use crate::model::state::{State, StateVersion};
use crate::resource::Resources;
use crate::server::{self, certificate};

#[derive(Clone)]
pub struct AppState {
    conf: Arc<AppConfig>,
    pub res: Arc<Mutex<Resources>>,
}

impl AppState {
    pub fn from_config(config: AppConfig) -> ApiResult<Self> {
        let certfile = &config.bifrost.cert_file;

        let certpath = Utf8Path::new(certfile);
        if certpath.is_file() {
            certificate::check_certificate(certpath, config.bridge.mac)?;
        } else {
            log::warn!("Missing certificate file [{certfile}], generating..");
            certificate::generate_and_save(certpath, config.bridge.mac)?;
        }

        let mut res;

        if let Ok(fd) = File::open(&config.bifrost.state_file) {
            log::debug!("Existing state file found, loading..");
            let yaml = serde_yml::from_reader(fd)?;
            let state = match State::version(&yaml)? {
                StateVersion::V0 => {
                    log::info!("Detected state file version 0. Upgrading to new version..");
                    let backup_path = &config.bifrost.state_file.with_extension("v0.bak");
                    fs::rename(&config.bifrost.state_file, backup_path)?;
                    log::info!("  ..saved old state file as {backup_path}");
                    State::from_v0(yaml)?
                }
                StateVersion::V1 => {
                    log::info!("Detected state file version 1. Loading..");
                    State::from_v1(yaml)?
                }
            };
            res = Resources::new(state);
        } else {
            log::debug!("No state file found, initializing..");
            res = Resources::new(State::new());
            res.init(&server::certificate::hue_bridge_id(config.bridge.mac))?;
        }

        let conf = Arc::new(config);
        let res = Arc::new(Mutex::new(res));

        Ok(Self { conf, res })
    }

    pub async fn tls_config(&self) -> ApiResult<RustlsConfig> {
        let certfile = &self.conf.bifrost.cert_file;

        log::debug!("Loading certificate from [{certfile}]");
        RustlsConfig::from_pem_file(&certfile, &certfile)
            .await
            .map_err(|e| ApiError::Certificate(certfile.to_owned(), e))
    }

    #[must_use]
    pub fn config(&self) -> Arc<AppConfig> {
        self.conf.clone()
    }

    #[must_use]
    pub fn api_short_config(&self) -> ApiShortConfig {
        let mac = self.conf.bridge.mac;
        ApiShortConfig {
            bridgeid: certificate::hue_bridge_id(mac),
            mac,
            ..Default::default()
        }
    }

    #[must_use]
    pub fn api_config(&self, username: &String) -> ApiConfig {
        ApiConfig {
            short_config: self.api_short_config(),
            ipaddress: self.conf.bridge.ipaddress,
            netmask: self.conf.bridge.netmask,
            gateway: self.conf.bridge.gateway,
            timezone: self.conf.bridge.timezone.clone(),
            whitelist: HashMap::from([(
                username.clone().to_string(),
                Whitelist {
                    create_date: Utc::now(),
                    last_use_date: Utc::now(),
                    name: "User#foo".to_string(),
                },
            )]),
            ..ApiConfig::default()
        }
    }
}
