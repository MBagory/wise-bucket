//! Layered configuration.
//!
//! Precedence (highest first): CLI flags > `WB_*` environment variables >
//! user config > defaults.
//! Every effective value remembers where it came from ([`Origin`]) so that
//! `config show --origin` can explain it.

use std::fmt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{ErrorKind, Result, WbError, err};
use crate::paths::{self, env};

/// Where a configuration value came from.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", content = "detail", rename_all = "snake_case")]
pub enum Origin {
    Default,
    UserConfig(PathBuf),
    Env(String),
    Cli(String),
}

impl fmt::Display for Origin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Origin::Default => write!(f, "default"),
            Origin::UserConfig(p) => write!(f, "user config ({})", p.display()),
            Origin::Env(v) => write!(f, "environment ({v})"),
            Origin::Cli(flag) => write!(f, "command line ({flag})"),
        }
    }
}

/// A value together with its origin.
#[derive(Debug, Clone, Serialize)]
pub struct Sourced<T> {
    pub value: T,
    pub origin: Origin,
}

impl<T> Sourced<T> {
    pub fn new(value: T, origin: Origin) -> Self {
        Self { value, origin }
    }
}

/// `[[roots]]` entry as written in a config file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RootSpec {
    pub name: String,
    pub path: PathBuf,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub robot: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub include: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DatabaseSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keep_running: Option<bool>,
}

/// User configuration file (machine specific).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UserConfigFile {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state_dir: Option<PathBuf>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub database: Option<DatabaseSection>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub roots: Vec<RootSpec>,
}

/// A root declaration before validation, with the directory relative paths resolve against.
#[derive(Debug, Clone, Serialize)]
pub struct RootDecl {
    pub spec: RootSpec,
    pub origin: Origin,
    /// Directory used to resolve relative paths.
    pub base_dir: PathBuf,
}

/// Fully resolved configuration.
#[derive(Debug, Clone, Serialize)]
pub struct EffectiveConfig {
    pub user_config_path: Sourced<PathBuf>,
    pub user_config_exists: bool,
    pub state_dir: Sourced<PathBuf>,
    pub runtime_dir: Sourced<PathBuf>,
    pub keep_running: Sourced<bool>,
    pub roots: Vec<RootDecl>,
}

/// Values given on the command line.
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub config_path: Option<PathBuf>,
    pub state_dir: Option<PathBuf>,
    pub keep_running: Option<bool>,
}

/// Abstraction over the process environment (tests inject their own).
pub trait EnvSource {
    fn get(&self, key: &str) -> Option<String>;
    fn cwd(&self) -> Result<PathBuf>;
}

/// The real process environment.
pub struct ProcessEnv;

impl EnvSource for ProcessEnv {
    fn get(&self, key: &str) -> Option<String> {
        std::env::var(key).ok().filter(|v| !v.is_empty())
    }
    fn cwd(&self) -> Result<PathBuf> {
        Ok(std::env::current_dir()?)
    }
}

fn read_toml<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> Result<(T, bool)> {
    match std::fs::read_to_string(path) {
        Ok(text) => toml::from_str(&text).map(|v| (v, true)).map_err(|e| {
            err(
                ErrorKind::ConfigInvalid,
                format!("{}: {}", path.display(), e.message()),
            )
        }),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok((T::default(), false)),
        Err(e) => Err(err(
            ErrorKind::ConfigInvalid,
            format!("cannot read {}: {e}", path.display()),
        )),
    }
}

fn parse_bool(key: &str, v: &str) -> Result<bool> {
    match v.to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Ok(true),
        "0" | "false" | "no" | "off" => Ok(false),
        _ => Err(err(
            ErrorKind::ConfigInvalid,
            format!("{key} must be true or false, got {v:?}"),
        )),
    }
}

/// Loads the effective configuration from files, environment and overrides.
pub fn load(overrides: &Overrides) -> Result<EffectiveConfig> {
    load_with(overrides, &ProcessEnv)
}

/// Same as [`load`] with an injectable environment.
pub fn load_with(overrides: &Overrides, envs: &dyn EnvSource) -> Result<EffectiveConfig> {
    let cwd = envs.cwd()?;

    // --- user config file location
    let user_config_path = if let Some(p) = &overrides.config_path {
        Sourced::new(paths::absolutize(p, &cwd), Origin::Cli("--config".into()))
    } else if let Some(p) = envs.get(env::CONFIG) {
        Sourced::new(
            paths::absolutize(Path::new(&p), &cwd),
            Origin::Env(env::CONFIG.into()),
        )
    } else {
        Sourced::new(paths::default_user_config_path()?, Origin::Default)
    };
    let (user, user_exists): (UserConfigFile, bool) = read_toml(&user_config_path.value)?;
    let user_origin = Origin::UserConfig(user_config_path.value.clone());
    let user_base = user_config_path
        .value
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| cwd.clone());

    // --- state dir
    let state_dir = if let Some(p) = &overrides.state_dir {
        Sourced::new(
            paths::absolutize(p, &cwd),
            Origin::Cli("--state-dir".into()),
        )
    } else if let Some(p) = envs.get(env::STATE_DIR) {
        Sourced::new(
            paths::absolutize(Path::new(&p), &cwd),
            Origin::Env(env::STATE_DIR.into()),
        )
    } else if let Some(p) = &user.state_dir {
        Sourced::new(paths::absolutize(p, &user_base), user_origin.clone())
    } else {
        Sourced::new(paths::default_state_dir()?, Origin::Default)
    };

    // --- runtime dir (PostgreSQL binaries); shareable between state dirs
    let runtime_dir = if let Some(p) = envs.get(env::RUNTIME_DIR) {
        Sourced::new(
            paths::absolutize(Path::new(&p), &cwd),
            Origin::Env(env::RUNTIME_DIR.into()),
        )
    } else {
        Sourced::new(state_dir.value.join("runtime"), state_dir.origin.clone())
    };

    // --- database
    let db_section = user.database.clone().unwrap_or_default();
    let keep_running = if let Some(v) = overrides.keep_running {
        Sourced::new(v, Origin::Cli("--keep-running".into()))
    } else if let Some(v) = envs.get(env::KEEP_RUNNING) {
        Sourced::new(
            parse_bool(env::KEEP_RUNNING, &v)?,
            Origin::Env(env::KEEP_RUNNING.into()),
        )
    } else if let Some(v) = db_section.keep_running {
        Sourced::new(v, user_origin.clone())
    } else {
        Sourced::new(false, Origin::Default)
    };

    // --- roots (validated separately, see `roots::resolve`)
    let roots: Vec<RootDecl> = user
        .roots
        .iter()
        .map(|spec| RootDecl {
            spec: spec.clone(),
            origin: user_origin.clone(),
            base_dir: user_base.clone(),
        })
        .collect();

    Ok(EffectiveConfig {
        user_config_path,
        user_config_exists: user_exists,
        state_dir,
        runtime_dir,
        keep_running,
        roots,
    })
}

// ---------------------------------------------------------------------------
// Editing configuration files (comments and formatting are preserved).
// ---------------------------------------------------------------------------

fn load_document(path: &Path) -> Result<toml_edit::DocumentMut> {
    match std::fs::read_to_string(path) {
        Ok(text) => text
            .parse::<toml_edit::DocumentMut>()
            .map_err(|e| err(ErrorKind::ConfigInvalid, format!("{}: {e}", path.display()))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(toml_edit::DocumentMut::new()),
        Err(e) => Err(WbError::from(e)),
    }
}

fn save_document(path: &Path, doc: &toml_edit::DocumentMut, header: &str) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut text = doc.to_string();
    if !text.starts_with('#') && !header.is_empty() {
        text = format!("{header}\n{text}");
    }
    crate::fsutil::write_atomic(path, text.as_bytes())
}

const USER_HEADER: &str = "# Wise Bucket user configuration (machine specific).\n# Docs: https://github.com/MBagory/wise-bucket/blob/main/docs/reference/configuration.md";

fn root_table(spec: &RootSpec) -> toml_edit::Table {
    let mut t = toml_edit::Table::new();
    t["name"] = toml_edit::value(&spec.name);
    t["path"] = toml_edit::value(spec.path.to_string_lossy().as_ref());
    if let Some(r) = &spec.robot {
        t["robot"] = toml_edit::value(r);
    }
    let arr = |v: &Vec<String>| {
        let mut a = toml_edit::Array::new();
        for s in v {
            a.push(s.as_str());
        }
        toml_edit::value(a)
    };
    if !spec.include.is_empty() {
        t["include"] = arr(&spec.include);
    }
    if !spec.exclude.is_empty() {
        t["exclude"] = arr(&spec.exclude);
    }
    t
}

fn roots_array(doc: &mut toml_edit::DocumentMut) -> Result<&mut toml_edit::ArrayOfTables> {
    if !doc.contains_key("roots") {
        doc.insert(
            "roots",
            toml_edit::Item::ArrayOfTables(toml_edit::ArrayOfTables::new()),
        );
    }
    doc["roots"].as_array_of_tables_mut().ok_or_else(|| {
        err(
            ErrorKind::ConfigInvalid,
            "`roots` must be an array of tables ([[roots]])",
        )
    })
}

/// Appends a root to the user config file.
pub fn add_root_to_file(path: &Path, spec: &RootSpec) -> Result<()> {
    let mut doc = load_document(path)?;
    let roots = roots_array(&mut doc)?;
    if roots
        .iter()
        .any(|t| t.get("name").and_then(|v| v.as_str()) == Some(spec.name.as_str()))
    {
        return Err(err(
            ErrorKind::RootDuplicateName,
            format!(
                "a root named {:?} already exists in {}",
                spec.name,
                path.display()
            ),
        ));
    }
    roots.push(root_table(spec));
    save_document(path, &doc, USER_HEADER)
}

/// Removes a root by name. Returns `false` if it was not present.
pub fn remove_root_from_file(path: &Path, name: &str) -> Result<bool> {
    let mut doc = load_document(path)?;
    if !doc.contains_key("roots") {
        return Ok(false);
    }
    let roots = roots_array(&mut doc)?;
    let before = roots.len();
    roots.retain(|t| t.get("name").and_then(|v| v.as_str()) != Some(name));
    if roots.len() == before {
        return Ok(false);
    }
    save_document(path, &doc, "")?;
    Ok(true)
}

/// Sets a top-level or dotted scalar key (e.g. `database.keep_running`).
pub fn set_key_in_file(path: &Path, key: &str, value: toml_edit::Value) -> Result<()> {
    let mut doc = load_document(path)?;
    let mut parts = key.split('.').peekable();
    let mut table = doc.as_table_mut();
    while let Some(part) = parts.next() {
        if parts.peek().is_none() {
            table[part] = toml_edit::Item::Value(value);
            break;
        }
        if !table.contains_key(part) {
            table.insert(part, toml_edit::Item::Table(toml_edit::Table::new()));
        }
        table = table[part].as_table_mut().ok_or_else(|| {
            err(
                ErrorKind::ConfigInvalid,
                format!("`{part}` must be a table"),
            )
        })?;
    }
    save_document(path, &doc, USER_HEADER)
}

// ---------------------------------------------------------------------------
// Reference documentation for configuration keys.
// ---------------------------------------------------------------------------

/// One documented configuration key.
pub struct KeyDoc {
    pub key: &'static str,
    pub layer: &'static str,
    pub env: &'static str,
    pub cli: &'static str,
    pub default: &'static str,
    pub description: &'static str,
}

/// All configuration keys, used to generate `reference/configuration.md`.
pub const KEYS: &[KeyDoc] = &[
    KeyDoc {
        key: "state_dir",
        layer: "user",
        env: env::STATE_DIR,
        cli: "--state-dir",
        default: "Linux `~/.local/share/wisebucket`, macOS `~/Library/Application Support/wisebucket`",
        description: "Where Wise Bucket keeps its database, caches and logs.",
    },
    KeyDoc {
        key: "database.keep_running",
        layer: "user",
        env: env::KEEP_RUNNING,
        cli: "--keep-running",
        default: "`false`",
        description: "Keep PostgreSQL running after the last session ends.",
    },
    KeyDoc {
        key: "[[roots]] name",
        layer: "user",
        env: "",
        cli: "`roots add <name>`",
        default: "",
        description: "Short unique name used in log references (`name:relative/path`). Lowercase letters, digits, `-`, `_`; max 32.",
    },
    KeyDoc {
        key: "[[roots]] path",
        layer: "user",
        env: "",
        cli: "`roots add <name> <path>`",
        default: "",
        description: "Folder Wise Bucket may read: absolute or `~/…`.",
    },
    KeyDoc {
        key: "[[roots]] robot",
        layer: "user",
        env: "",
        cli: "`--robot`",
        default: "",
        description: "Default robot for recordings found in this root (used from milestone M3).",
    },
    KeyDoc {
        key: "[[roots]] include / exclude",
        layer: "user",
        env: "",
        cli: "`--include` / `--exclude`",
        default: "all supported formats",
        description: "Glob patterns restricting which files are considered recordings.",
    },
    KeyDoc {
        key: "(file) user config",
        layer: "-",
        env: env::CONFIG,
        cli: "--config",
        default: "Linux `~/.config/wisebucket/config.toml`, macOS `~/Library/Application Support/wisebucket/config.toml`",
        description: "Location of the user configuration file.",
    },
    KeyDoc {
        key: "(dir) runtime",
        layer: "-",
        env: env::RUNTIME_DIR,
        cli: "",
        default: "`<state_dir>/runtime`",
        description: "Where managed PostgreSQL binaries are installed. Can be shared between state directories.",
    },
];

/// Renders `reference/configuration.md`.
pub fn render_configuration_reference() -> String {
    let mut out = String::from(
        "<!-- GENERATED by `wisebucket docs gen`. Do not edit by hand. -->\n\n# Configuration reference\n\n\
         Precedence, highest first: **command line** > **`WB_*` environment variables** > **user config** > **defaults**. \
         Run `wisebucket config show --origin` to see the effective value of each key and where it comes from.\n\n\
         | Key | Layer | Environment | Command line | Default | Description |\n| --- | --- | --- | --- | --- | --- |\n",
    );
    let code = |s: &str| {
        if s.is_empty() || s.starts_with('`') {
            s.to_string()
        } else {
            format!("`{s}`")
        }
    };
    for k in KEYS {
        out.push_str(&format!(
            "| `{}` | {} | {} | {} | {} | {} |\n",
            k.key,
            k.layer,
            code(k.env),
            code(k.cli),
            k.default,
            k.description
        ));
    }
    out.push_str("\n## Examples\n\n### User configuration\n\n```toml\n");
    out.push_str(include_str!("../../../docs/examples/user-config.toml"));
    out.push_str("```\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct FakeEnv {
        vars: HashMap<String, String>,
        cwd: PathBuf,
    }

    impl EnvSource for FakeEnv {
        fn get(&self, key: &str) -> Option<String> {
            self.vars.get(key).cloned()
        }
        fn cwd(&self) -> Result<PathBuf> {
            Ok(self.cwd.clone())
        }
    }

    fn fake(dir: &Path, vars: &[(&str, &str)]) -> FakeEnv {
        FakeEnv {
            vars: vars
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            cwd: dir.to_path_buf(),
        }
    }

    #[test]
    fn precedence_cli_env_file_default() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config.toml");
        std::fs::write(
            &cfg,
            "state_dir = \"from-file\"\n[database]\nkeep_running = true\n",
        )
        .unwrap();
        let path_s = cfg.to_string_lossy().to_string();

        let env = fake(d.path(), &[(env::CONFIG, &path_s)]);
        let c = load_with(&Overrides::default(), &env).unwrap();
        assert_eq!(c.state_dir.value, d.path().join("from-file"));
        assert!(matches!(c.state_dir.origin, Origin::UserConfig(_)));
        assert!(c.keep_running.value);

        let env = fake(
            d.path(),
            &[(env::CONFIG, &path_s), (env::STATE_DIR, "/tmp/from-env")],
        );
        let c = load_with(&Overrides::default(), &env).unwrap();
        assert_eq!(c.state_dir.value, PathBuf::from("/tmp/from-env"));
        assert_eq!(c.state_dir.origin, Origin::Env(env::STATE_DIR.into()));

        let o = Overrides {
            state_dir: Some("/tmp/from-cli".into()),
            ..Default::default()
        };
        let c = load_with(&o, &env).unwrap();
        assert_eq!(c.state_dir.value, PathBuf::from("/tmp/from-cli"));
        assert!(matches!(c.state_dir.origin, Origin::Cli(_)));
        assert_eq!(c.runtime_dir.value, PathBuf::from("/tmp/from-cli/runtime"));
    }

    #[test]
    fn unknown_keys_are_rejected() {
        let d = tempfile::tempdir().unwrap();
        let cfg = d.path().join("config.toml");
        std::fs::write(&cfg, "stat_dir = \"typo\"\n").unwrap();
        let p = cfg.to_string_lossy().to_string();
        let e =
            load_with(&Overrides::default(), &fake(d.path(), &[(env::CONFIG, &p)])).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::ConfigInvalid);
    }

    #[test]
    fn add_and_remove_root_preserves_comments() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("c.toml");
        std::fs::write(&p, "# my comment\nstate_dir = \"x\"\n").unwrap();
        let spec = RootSpec {
            name: "bags".into(),
            path: "/data/bags".into(),
            robot: Some("r".into()),
            include: vec![],
            exclude: vec![],
        };
        add_root_to_file(&p, &spec).unwrap();
        let e = add_root_to_file(&p, &spec).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::RootDuplicateName);
        let text = std::fs::read_to_string(&p).unwrap();
        assert!(text.contains("# my comment"));
        assert!(text.contains("[[roots]]"));
        let parsed: UserConfigFile = toml::from_str(&text).unwrap();
        assert_eq!(parsed.roots, vec![spec]);
        assert!(remove_root_from_file(&p, "bags").unwrap());
        assert!(!remove_root_from_file(&p, "bags").unwrap());
    }
}
