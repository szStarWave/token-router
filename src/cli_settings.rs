use crate::config::ConfigFile;
use crate::config::paths;

pub struct CliSettings {
    pub file: ConfigFile,
    pub app_home: std::path::PathBuf,
    pub config_path: std::path::PathBuf,
    pub port: Option<u16>,
}

impl CliSettings {
    pub fn from_parts(
        file: ConfigFile,
        app_home: std::path::PathBuf,
        port: Option<u16>,
    ) -> Self {
        let config_path = app_home.join("config.toml");
        Self {
            file,
            app_home,
            config_path,
            port,
        }
    }

    pub fn gateway_url(&self) -> String {
        self.file.gateway_http_url()
    }

    pub fn api_key(&self) -> Option<String> {
        self.file.gateway.api_key.clone()
    }

    pub fn admin_token(&self) -> Option<String> {
        self.file.gateway.admin_token.clone()
    }
}

pub fn resolve_app_home(home: Option<&std::path::Path>) -> anyhow::Result<std::path::PathBuf> {
    paths::resolve_app_dir(home)
}
