use anyhow::{bail, Context, Result};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct ServerSettings {
    #[serde(default = "default_listen")]
    pub listen: SocketAddr,

    #[serde(default = "default_health_listen")]
    pub health_listen: SocketAddr,

    #[serde(default = "default_api_listen")]
    pub api_listen: SocketAddr,
}

fn default_listen() -> SocketAddr {
    "0.0.0.0:1935".parse().unwrap()
}

fn default_health_listen() -> SocketAddr {
    "127.0.0.1:8080".parse().unwrap()
}

fn default_api_listen() -> SocketAddr {
    "0.0.0.0:3000".parse().unwrap()
}

impl Default for ServerSettings {
    fn default() -> Self {
        Self {
            listen: default_listen(),
            health_listen: default_health_listen(),
            api_listen: default_api_listen(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct NotificationSettings {
    pub discord_webhook: Option<String>,
    #[serde(default = "default_live_message")]
    pub live_message: String,
    pub webhook_url: Option<String>,
}

fn default_live_message() -> String {
    "Stream is LIVE".to_string()
}

impl Default for NotificationSettings {
    fn default() -> Self {
        Self {
            discord_webhook: None,
            live_message: default_live_message(),
            webhook_url: None,
        }
    }
}

#[derive(Debug, Deserialize, Serialize, Clone)]
pub struct TargetConfig {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub stream_key: String,
    #[serde(default)]
    pub public_url: Option<String>,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Deserialize, Serialize, Clone, Default)]
pub struct AppConfig {
    #[serde(default)]
    pub server: ServerSettings,

    #[serde(default)]
    pub notifications: NotificationSettings,

    #[serde(default)]
    pub targets: Vec<TargetConfig>,
}

impl AppConfig {
    pub fn load_from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let content = fs::read_to_string(path.as_ref())
            .with_context(|| format!("Failed to read configuration file: {:?}", path.as_ref()))?;
        let config: AppConfig = toml::from_str(&content).with_context(|| {
            format!(
                "Failed to parse TOML configuration from {:?}",
                path.as_ref()
            )
        })?;
        Ok(config)
    }

    /// Validate enabled target URLs (allows 0 enabled targets for ingest-only mode)
    pub fn validate(&self) -> Result<()> {
        for target in &self.targets {
            if target.enabled {
                let url = target.url.trim();
                if url.is_empty() {
                    bail!("Target '{}' has an empty RTMP URL.", target.name);
                }
                if !url.starts_with("rtmp://") && !url.starts_with("rtmps://") {
                    bail!(
                        "Target '{}' has an invalid URL. It must start with rtmp:// or rtmps://",
                        target.name
                    );
                }
            }
        }
        Ok(())
    }
}

/// SQLite-backed configuration storage.
///
/// A `.toml` path is treated as a legacy bootstrap file. It is imported only
/// when the sibling `.sqlite3` database has no configuration yet.
#[derive(Debug, Clone)]
pub struct ConfigStore {
    path: PathBuf,
}

impl ConfigStore {
    const STORAGE_VERSION: &'static str = "1";

    pub fn open<P: AsRef<Path>>(config_path: P) -> Result<(Self, AppConfig)> {
        let config_path = config_path.as_ref();
        let is_toml = config_path.extension().is_some_and(|ext| ext == "toml");
        let database_path = if is_toml {
            config_path.with_extension("sqlite3")
        } else {
            config_path.to_path_buf()
        };

        if is_toml && !config_path.exists() && !database_path.exists() {
            bail!(
                "Neither legacy configuration '{}' nor database '{}' exists",
                config_path.display(),
                database_path.display()
            );
        }

        let store = Self {
            path: database_path,
        };
        store.initialize()?;

        // Databases created by the broken form decoder have no storage-version
        // marker. Re-import the legacy TOML once when upgrading those databases,
        // then leave TOML untouched and use SQLite exclusively.
        let needs_legacy_import = is_toml
            && config_path.exists()
            && store.metadata("storage_version")?.as_deref() != Some(Self::STORAGE_VERSION);

        let config = if needs_legacy_import {
            let config = AppConfig::load_from_file(config_path)?;
            config.validate()?;
            store.save(&config)?;
            config
        } else {
            match store.load()? {
                Some(config) => config,
                None if is_toml && config_path.exists() => {
                    let config = AppConfig::load_from_file(config_path)?;
                    config.validate()?;
                    store.save(&config)?;
                    config
                }
                None => {
                    let config = AppConfig::default();
                    store.save(&config)?;
                    config
                }
            }
        };
        store.set_metadata("storage_version", Self::STORAGE_VERSION)?;

        Ok((store, config))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    fn connect(&self) -> Result<Connection> {
        let database_exists = self.path.exists();
        let connection = Connection::open(&self.path)
            .with_context(|| format!("Failed to open config database '{}'", self.path.display()))?;
        #[cfg(unix)]
        if !database_exists {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.path, fs::Permissions::from_mode(0o600)).with_context(
                || format!("Failed to secure config database '{}'", self.path.display()),
            )?;
        }
        Ok(connection)
    }

    fn initialize(&self) -> Result<()> {
        let connection = self.connect()?;
        connection.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE IF NOT EXISTS settings (
                id INTEGER PRIMARY KEY CHECK (id = 1),
                server_listen TEXT NOT NULL,
                health_listen TEXT NOT NULL,
                api_listen TEXT NOT NULL,
                discord_webhook TEXT,
                live_message TEXT NOT NULL,
                webhook_url TEXT
            );
            CREATE TABLE IF NOT EXISTS targets (
                id INTEGER PRIMARY KEY,
                position INTEGER NOT NULL UNIQUE,
                name TEXT NOT NULL,
                url TEXT NOT NULL,
                stream_key TEXT NOT NULL,
                public_url TEXT,
                enabled INTEGER NOT NULL CHECK (enabled IN (0, 1))
            );
            CREATE TABLE IF NOT EXISTS metadata (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );
            ",
        )?;
        Ok(())
    }

    fn metadata(&self, key: &str) -> Result<Option<String>> {
        let connection = self.connect()?;
        Ok(connection
            .query_row("SELECT value FROM metadata WHERE key = ?1", [key], |row| {
                row.get(0)
            })
            .optional()?)
    }

    fn set_metadata(&self, key: &str, value: &str) -> Result<()> {
        let connection = self.connect()?;
        connection.execute(
            "INSERT INTO metadata (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [key, value],
        )?;
        Ok(())
    }

    pub fn load(&self) -> Result<Option<AppConfig>> {
        let connection = self.connect()?;
        let settings = connection
            .query_row(
                "SELECT server_listen, health_listen, api_listen, discord_webhook,
                        live_message, webhook_url
                 FROM settings WHERE id = 1",
                [],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, Option<String>>(3)?,
                        row.get::<_, String>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?;

        let Some((
            server_listen,
            health_listen,
            api_listen,
            discord_webhook,
            live_message,
            webhook_url,
        )) = settings
        else {
            return Ok(None);
        };

        let mut statement = connection.prepare(
            "SELECT name, url, stream_key, public_url, enabled
             FROM targets ORDER BY position",
        )?;
        let targets = statement
            .query_map([], |row| {
                Ok(TargetConfig {
                    name: row.get(0)?,
                    url: row.get(1)?,
                    stream_key: row.get(2)?,
                    public_url: row.get(3)?,
                    enabled: row.get(4)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let config = AppConfig {
            server: ServerSettings {
                listen: server_listen
                    .parse()
                    .context("Invalid server listen address")?,
                health_listen: health_listen
                    .parse()
                    .context("Invalid health listen address")?,
                api_listen: api_listen.parse().context("Invalid API listen address")?,
            },
            notifications: NotificationSettings {
                discord_webhook,
                live_message,
                webhook_url,
            },
            targets,
        };
        Ok(Some(config))
    }

    pub fn save(&self, config: &AppConfig) -> Result<()> {
        config.validate()?;
        let mut connection = self.connect()?;
        let transaction = connection.transaction()?;
        transaction.execute(
            "INSERT INTO settings (
                id, server_listen, health_listen, api_listen, discord_webhook,
                live_message, webhook_url
             ) VALUES (1, ?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
                server_listen = excluded.server_listen,
                health_listen = excluded.health_listen,
                api_listen = excluded.api_listen,
                discord_webhook = excluded.discord_webhook,
                live_message = excluded.live_message,
                webhook_url = excluded.webhook_url",
            params![
                config.server.listen.to_string(),
                config.server.health_listen.to_string(),
                config.server.api_listen.to_string(),
                config.notifications.discord_webhook,
                config.notifications.live_message,
                config.notifications.webhook_url,
            ],
        )?;
        transaction.execute("DELETE FROM targets", [])?;
        for (position, target) in config.targets.iter().enumerate() {
            transaction.execute(
                "INSERT INTO targets (
                    position, name, url, stream_key, public_url, enabled
                 ) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    position as i64,
                    target.name,
                    target.url,
                    target.stream_key,
                    target.public_url,
                    target.enabled,
                ],
            )?;
        }
        transaction.commit()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn imports_legacy_toml_once_and_round_trips_sqlite() {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let directory = std::env::temp_dir().join(format!("rtmp-proxy-config-{unique}"));
        fs::create_dir(&directory).unwrap();
        let toml_path = directory.join("config.toml");
        fs::write(
            &toml_path,
            r#"
                [server]
                api_listen = "127.0.0.1:3001"
                [notifications]
                live_message = "first"
                [[targets]]
                name = "Twitch"
                url = "rtmps://example.test/app"
                enabled = true
            "#,
        )
        .unwrap();

        let (store, mut config) = ConfigStore::open(&toml_path).unwrap();
        assert_eq!(config.server.api_listen.port(), 3001);
        assert_eq!(config.targets[0].stream_key, "");
        config.notifications.live_message = "saved in sqlite".into();
        store.save(&config).unwrap();

        fs::write(
            &toml_path,
            "[notifications]\nlive_message = \"changed toml\"",
        )
        .unwrap();
        let (_, reloaded) = ConfigStore::open(&toml_path).unwrap();
        assert_eq!(reloaded.notifications.live_message, "saved in sqlite");

        fs::remove_dir_all(directory).unwrap();
    }
}
