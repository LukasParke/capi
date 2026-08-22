//! On-disk configuration: load at boot, mutate under lock, atomically persist.
//!
//! Fix vs Go: a corrupt `config.json` no longer silently resets to defaults
//! (which then overwrote the user's file on next save). Parse failures are
//! fatal at boot with a clear message pointing at the backup path.

use crate::types::Config;
use std::path::{Path, PathBuf};
use std::sync::RwLock;
use tracing::{error, info, warn};

pub struct Settings {
    path: PathBuf,
    current: RwLock<Config>,
}

impl Settings {
    /// Load from `path`. Missing file -> defaults. Corrupt file -> error:
    /// the caller decides whether to abort (server mode) or continue with
    /// defaults (never silently rewriting the user's file).
    pub fn load(path: &Path) -> Result<(Self, Option<String>), String> {
        let raw = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                return Ok((
                    Self {
                        path: path.to_path_buf(),
                        current: RwLock::new(Config::default()),
                    },
                    None,
                ));
            }
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        if raw.trim().is_empty() {
            return Ok((
                Self {
                    path: path.to_path_buf(),
                    current: RwLock::new(Config::default()),
                },
                None,
            ));
        }
        match serde_json::from_str::<Config>(&raw) {
            Ok(cfg) => Ok((Self { path: path.to_path_buf(), current: RwLock::new(cfg) }, None)),
            Err(e) => Err(format!(
                "config {} failed to parse: {e}. Fix or remove the file (a copy is kept as {}.corrupt); refusing to start with defaults",
                path.display(),
                path.display()
            )),
        }
    }

    pub fn get(&self) -> Config {
        self.current.read().expect("settings lock").clone()
    }

    /// Apply `f` to the config and persist. The in-memory value is only
    /// replaced when persistence succeeds, so memory and disk cannot diverge.
    pub fn update<T>(&self, f: impl FnOnce(&mut Config) -> T) -> Result<T, String> {
        let mut next = self.get();
        let out = f(&mut next);
        self.persist(&next)?;
        *self.current.write().expect("settings lock") = next;
        Ok(out)
    }

    /// Atomic write: temp file in the same directory + rename.
    pub fn persist(&self, cfg: &Config) -> Result<(), String> {
        let data = serde_json::to_string_pretty(cfg).map_err(|e| format!("marshal config: {e}"))?;
        let tmp = self.path.with_extension("json.tmp");
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        }
        std::fs::write(&tmp, data).map_err(|e| format!("write {}: {e}", tmp.display()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600));
        }
        std::fs::rename(&tmp, &self.path).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            format!("rename {}: {e}", self.path.display())
        })?;
        Ok(())
    }

    /// Quarantine an unparseable config so a human can recover it.
    pub fn quarantine_corrupt(path: &Path) {
        let bak = path.with_extension("json.corrupt");
        match std::fs::rename(path, &bak) {
            Ok(()) => warn!("moved unreadable config to {}", bak.display()),
            Err(e) => error!(
                "could not preserve unreadable config {}: {e}",
                path.display()
            ),
        }
    }
}

/// Merge CLI flags over the persisted config. Empty CLI values keep the file
/// value; `mqtt_prefix` only overrides when explicitly passed (parity with Go).
pub struct CliOverrides {
    pub mqtt_broker: Option<String>,
    pub mqtt_user: Option<String>,
    pub mqtt_pass: Option<String>,
    pub mqtt_prefix_explicit: bool,
    pub mqtt_prefix: String,
    pub token: Option<String>,
}

impl Settings {
    /// Boot-time CLI overlay. Mutates in place; not persisted.
    pub fn apply_overrides(&self, o: &CliOverrides) {
        let mut g = self.current.write().expect("settings lock");
        if let Some(b) = &o.mqtt_broker {
            g.mqtt.broker = b.clone();
        }
        if let Some(u) = &o.mqtt_user {
            g.mqtt.user = u.clone();
        }
        if let Some(p) = &o.mqtt_pass {
            g.mqtt.pass = p.clone();
        }
        if o.mqtt_prefix_explicit || g.mqtt.prefix.is_empty() {
            g.mqtt.prefix = o.mqtt_prefix.clone();
        }
        if let Some(t) = &o.token {
            g.auth_token = t.clone();
        }
    }
}

/// Parse `-flag value` / `-flag=value` style args (parity with the Go flag set).
#[derive(Default)]
pub struct Flags {
    pub bind: String,
    pub config_dir: Option<String>,
    pub name: String,
    pub adapter: String,
    pub mqtt_broker: String,
    pub mqtt_user: String,
    pub mqtt_pass: String,
    pub mqtt_prefix: String,
    pub cec_monitor: bool,
    /// Test hook: auto-shutdown N ms after the listener binds.
    pub shutdown_after_ms: Option<u64>,
    pub token: String,
    pub show_version: bool,
    pub do_update: bool,
}

pub fn parse_flags(args: &[String]) -> Result<Flags, String> {
    let mut f = Flags {
        bind: ":8080".into(),
        name: "CEC HTTP Bridge".into(),
        config_dir: None,
        mqtt_prefix: "capi".into(),
        ..Default::default()
    };
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        let (name, inline_val) = match a.split_once('=') {
            Some((n, v)) => (n.to_string(), Some(v.to_string())),
            None => (a.clone(), None),
        };
        let mut take = |name: &str, slot: &mut String| -> Result<(), String> {
            let v = match inline_val.clone() {
                Some(v) => v,
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| format!("flag {name} requires a value"))?
                }
            };
            *slot = v;
            Ok(())
        };
        match name.as_str() {
            "-bind" => take("-bind", &mut f.bind)?,
            "-name" => take("-name", &mut f.name)?,
            "-adapter" => take("-adapter", &mut f.adapter)?,
            "-mqtt-broker" => take("-mqtt-broker", &mut f.mqtt_broker)?,
            "-mqtt-user" => take("-mqtt-user", &mut f.mqtt_user)?,
            "-mqtt-pass" => take("-mqtt-pass", &mut f.mqtt_pass)?,
            "-mqtt-prefix" => take("-mqtt-prefix", &mut f.mqtt_prefix)?,
            "-token" => take("-token", &mut f.token)?,
            "-cec-monitor" => f.cec_monitor = true,
            "-version" => f.show_version = true,
            "-update" => f.do_update = true,
            other => return Err(format!("unknown flag {other}")),
        }
        i += 1;
    }
    Ok(f)
}

pub fn init_tracing() {
    use tracing_subscriber::EnvFilter;
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();
    info!("");
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn tmp_path(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("capi-cfg-{}-{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d.join("config.json")
    }

    #[test]
    fn missing_file_yields_defaults() {
        let (s, note) = Settings::load(&tmp_path("missing")).unwrap();
        assert!(note.is_none());
        assert_eq!(s.get().mqtt.prefix, "capi");
    }

    #[test]
    fn corrupt_config_is_an_error_not_silent_defaults() {
        // Regression: the Go loader swallowed parse errors and reset to
        // defaults, then overwrote the user's file on next save.
        let p = tmp_path("corrupt");
        std::fs::write(&p, "{ not json").unwrap();
        assert!(Settings::load(&p).is_err());
    }

    #[test]
    fn quarantine_moves_file_aside() {
        let p = tmp_path("quar");
        std::fs::write(&p, "garbage").unwrap();
        Settings::quarantine_corrupt(&p);
        assert!(!p.exists());
        assert!(p.with_extension("json.corrupt").exists());
    }

    #[test]
    fn update_persists_and_is_atomic() {
        let p = tmp_path("atomic");
        let (s, _) = Settings::load(&p).unwrap();
        s.update(|c| c.auth_token = "tok".into()).unwrap();
        let on_disk = std::fs::read_to_string(&p).unwrap();
        assert!(on_disk.contains("\"tok\""));
        // No temp file left behind.
        assert!(!p.with_extension("json.tmp").exists());
        let (s2, _) = Settings::load(&p).unwrap();
        assert_eq!(s2.get().auth_token, "tok");
    }

    #[test]
    fn update_failure_leaves_memory_untouched() {
        let p = tmp_path("ro");
        let (s, _) = Settings::load(&p).unwrap();
        // Point persistence at an impossible directory.
        let bad = Settings {
            path: PathBuf::from("/proc/nonexistent/config.json"),
            current: std::sync::RwLock::new(s.get()),
        };
        assert!(bad.update(|c| c.auth_token = "x".into()).is_err());
        assert_eq!(bad.get().auth_token, ""); // memory never diverged
    }

    #[test]
    fn apply_overrides_semantics() {
        let p = tmp_path("ovr");
        let (s, _) = Settings::load(&p).unwrap();
        s.apply_overrides(&CliOverrides {
            mqtt_broker: Some("tcp://b:1".into()),
            mqtt_user: None,
            mqtt_pass: None,
            mqtt_prefix_explicit: false,
            mqtt_prefix: "capi".into(),
            token: Some("t".into()),
        });
        let cfg = s.get();
        assert_eq!(cfg.mqtt.broker, "tcp://b:1");
        assert_eq!(cfg.mqtt.prefix, "capi"); // default kept when not explicit
        assert_eq!(cfg.auth_token, "t");
        // Empty CLI values keep file values.
        s.apply_overrides(&CliOverrides {
            mqtt_broker: None,
            mqtt_user: None,
            mqtt_pass: None,
            mqtt_prefix_explicit: false,
            mqtt_prefix: String::new(),
            token: None,
        });
        assert_eq!(s.get().mqtt.broker, "tcp://b:1");
    }

    #[test]
    fn flag_parsing_styles() {
        let f = parse_flags(&[
            "-bind".into(),
            ":9".into(),
            "-token=xyz".into(),
            "-cec-monitor".into(),
        ])
        .unwrap();
        assert_eq!(f.bind, ":9");
        assert_eq!(f.token, "xyz");
        assert!(f.cec_monitor);
        assert!(parse_flags(&["-nope".into()]).is_err());
        assert!(parse_flags(&["-bind".into()]).is_err()); // missing value
    }
}
