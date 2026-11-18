//! The shared configuration model and its TOML parser.
//!
//! The parser walks a `toml::Table` by hand (instead of using serde derive) so
//! that error messages can name the exact key that is wrong, and so that the
//! order of keys in the file is preserved: an env var may refer to one defined
//! above it.

use std::fmt;

use toml::{Table, Value as Toml};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Shell {
    Zsh,
    Nu,
}

impl Shell {
    pub const ALL: [Shell; 2] = [Shell::Zsh, Shell::Nu];

    pub fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Nu => "nu",
        }
    }

    pub fn parse(s: &str) -> Option<Shell> {
        match s {
            "zsh" => Some(Shell::Zsh),
            "nu" | "nushell" => Some(Shell::Nu),
            _ => None,
        }
    }
}

/// A value that is either shared by both shells or given separately per shell.
/// A per-shell value with one side missing is skipped in that shell.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Both(String),
    PerShell {
        zsh: Option<String>,
        nu: Option<String>,
    },
}

impl Value {
    pub fn get(&self, shell: Shell) -> Option<&str> {
        match self {
            Value::Both(s) => Some(s),
            Value::PerShell { zsh, nu } => match shell {
                Shell::Zsh => zsh.as_deref(),
                Shell::Nu => nu.as_deref(),
            },
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub env: Vec<(String, Value)>,
    pub path_prepend: Vec<Value>,
    pub path_append: Vec<Value>,
    pub aliases: Vec<(String, Value)>,
    pub zsh_init: String,
    pub nu_init: String,
}

#[derive(Debug)]
pub struct Error(pub String);

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, Error> {
    Err(Error(msg.into()))
}

impl Config {
    pub fn parse(src: &str) -> Result<Config, Error> {
        let table: Table = src
            .parse()
            .map_err(|e: toml::de::Error| Error(e.to_string()))?;
        let mut cfg = Config::default();

        for (key, val) in &table {
            match key.as_str() {
                "env" => {
                    for (name, v) in as_table(val, "env")? {
                        let ctx = format!("env.{name}");
                        if !is_env_name(name) {
                            return err(format!(
                                "{ctx}: invalid variable name (use letters, digits and _)"
                            ));
                        }
                        cfg.env.push((name.to_string(), value(v, &ctx, true)?));
                    }
                }
                "path" => {
                    for (k, v) in as_table(val, "path")? {
                        let ctx = format!("path.{k}");
                        let list = match k.as_str() {
                            "prepend" => &mut cfg.path_prepend,
                            "append" => &mut cfg.path_append,
                            _ => {
                                return err(format!(
                                    "{ctx}: unknown key (expected `prepend` or `append`)"
                                ));
                            }
                        };
                        let Toml::Array(items) = v else {
                            return err(format!("{ctx}: expected an array of directories"));
                        };
                        for (i, item) in items.iter().enumerate() {
                            list.push(value(item, &format!("{ctx}[{i}]"), false)?);
                        }
                    }
                }
                "aliases" => {
                    for (name, v) in as_table(val, "aliases")? {
                        let ctx = format!("aliases.{name}");
                        if !is_alias_name(name) {
                            return err(format!("{ctx}: invalid alias name"));
                        }
                        let v = value(v, &ctx, false)?;
                        if [Shell::Zsh, Shell::Nu]
                            .iter()
                            .any(|s| v.get(*s).is_some_and(|b| b.trim().is_empty()))
                        {
                            return err(format!("{ctx}: alias body is empty"));
                        }
                        cfg.aliases.push((name.to_string(), v));
                    }
                }
                "zsh" | "nu" => {
                    for (k, v) in as_table(val, key)? {
                        let ctx = format!("{key}.{k}");
                        if k != "init" {
                            return err(format!("{ctx}: unknown key (expected `init`)"));
                        }
                        let Toml::String(s) = v else {
                            return err(format!("{ctx}: expected a string of {key} code"));
                        };
                        if key == "zsh" {
                            cfg.zsh_init = s.clone();
                        } else {
                            cfg.nu_init = s.clone();
                        }
                    }
                }
                _ => {
                    return err(format!(
                        "unknown section `{key}` (expected env, path, aliases, zsh or nu)"
                    ));
                }
            }
        }
        Ok(cfg)
    }
}

fn as_table<'a>(v: &'a Toml, ctx: &str) -> Result<&'a Table, Error> {
    match v {
        Toml::Table(t) => Ok(t),
        _ => err(format!("{ctx}: expected a table, e.g. [{ctx}]")),
    }
}

/// Converts a TOML value into a `Value`. Numbers and booleans are accepted only
/// where `allow_scalars` is set (env values), and become their text form.
fn value(v: &Toml, ctx: &str, allow_scalars: bool) -> Result<Value, Error> {
    let scalar = |v: &Toml, ctx: &str| -> Result<String, Error> {
        match v {
            Toml::String(s) => Ok(s.clone()),
            Toml::Integer(i) if allow_scalars => Ok(i.to_string()),
            Toml::Float(f) if allow_scalars => Ok(f.to_string()),
            Toml::Boolean(b) if allow_scalars => Ok(b.to_string()),
            _ if allow_scalars => err(format!("{ctx}: expected a string, number or boolean")),
            _ => err(format!("{ctx}: expected a string")),
        }
    };
    match v {
        Toml::Table(t) => {
            let (mut zsh, mut nu) = (None, None);
            for (k, v) in t {
                let sub = format!("{ctx}.{k}");
                match k.as_str() {
                    "zsh" => zsh = Some(scalar(v, &sub)?),
                    "nu" => nu = Some(scalar(v, &sub)?),
                    _ => return err(format!("{sub}: unknown key (expected `zsh` or `nu`)")),
                }
            }
            Ok(Value::PerShell { zsh, nu })
        }
        other => Ok(Value::Both(scalar(other, ctx)?)),
    }
}

pub fn is_env_name(s: &str) -> bool {
    let mut chars = s.chars();
    matches!(chars.next(), Some(c) if c == '_' || c.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
}

fn is_alias_name(s: &str) -> bool {
    !s.is_empty()
        && !s.starts_with('-')
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '+'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_sections_in_order() {
        let cfg = Config::parse(
            r#"
            [env]
            ZED = "z"
            ALPHA = 1
            PAGER = { zsh = "less", nu = "bat" }

            [path]
            prepend = ["~/bin", { zsh = "/zsh/only" }]
            append = ["/opt/x"]

            [aliases]
            gs = "git status"
            ll = { zsh = "ls -lh", nu = "ls -l" }

            [zsh]
            init = "setopt autocd"

            [nu]
            init = "$env.config.show_banner = false"
            "#,
        )
        .unwrap();
        let names: Vec<_> = cfg.env.iter().map(|(k, _)| k.as_str()).collect();
        assert_eq!(names, ["ZED", "ALPHA", "PAGER"]);
        assert_eq!(cfg.env[1].1, Value::Both("1".into()));
        assert_eq!(cfg.env[2].1.get(Shell::Nu), Some("bat"));
        assert_eq!(cfg.path_prepend[1].get(Shell::Nu), None);
        assert_eq!(cfg.path_append.len(), 1);
        assert_eq!(cfg.aliases[1].1.get(Shell::Zsh), Some("ls -lh"));
        assert_eq!(cfg.zsh_init, "setopt autocd");
    }

    #[test]
    fn reports_the_offending_key() {
        let e = |s: &str| Config::parse(s).unwrap_err().0;
        assert!(e("[env]\n\"BAD-NAME\" = 'x'").starts_with("env.BAD-NAME"));
        assert!(e("[env]\nX = { fish = 'x' }").starts_with("env.X.fish"));
        assert!(e("[path]\nprepend = [1]").starts_with("path.prepend[0]"));
        assert!(e("[aliases]\nx = ''").contains("empty"));
        assert!(e("[bogus]").contains("unknown section"));
        assert!(e("[env\n").contains("line 1"));
    }
}
