//! The shared configuration model and its TOML parser.
//!
//! The parser walks a `toml::Table` by hand (instead of using serde derive) so
//! that error messages can name the exact key that is wrong, and so that the
//! order of keys in the file is preserved: an env var may refer to one defined
//! above it.

use std::collections::BTreeMap;
use std::fmt;

use toml::{Table, Value as Toml};

use crate::shell::Shell;

/// What a per-shell key selects: one shell, or with `sh` every shell of the
/// sh family.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Target {
    Sh,
    Shell(Shell),
}

impl Target {
    fn named(key: &str) -> Option<Target> {
        match key {
            "sh" => Some(Target::Sh),
            _ => Shell::named(key).map(Target::Shell),
        }
    }

    fn names() -> Vec<&'static str> {
        let mut names = vec!["sh"];
        names.extend(Shell::ALL.map(Shell::name));
        names
    }

    fn covers(self, shell: Shell) -> bool {
        match self {
            Target::Sh => shell.is_sh(),
            Target::Shell(s) => s == shell,
        }
    }
}

/// A value that is either shared by all shells or given separately per shell.
/// A per-shell value is skipped in the shells it doesn't cover; a shell's own
/// key takes precedence over `sh`.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    All(String),
    PerShell(BTreeMap<Target, String>),
}

impl Value {
    pub fn get(&self, shell: Shell) -> Option<&str> {
        match self {
            Value::All(s) => Some(s),
            Value::PerShell(by_target) => by_target
                .get(&Target::Shell(shell))
                .or_else(|| by_target.get(&Target::Sh).filter(|_| shell.is_sh()))
                .map(String::as_str),
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    pub env: Vec<(String, Value)>,
    pub path_prepend: Vec<Value>,
    pub path_append: Vec<Value>,
    pub aliases: Vec<(String, Value)>,
    /// Raw code from the `init` of each `[sh]`/`[<shell>]` section, in file order.
    pub init_blocks: Vec<(Target, String)>,
}

impl Config {
    /// The init code for `shell`: every block that covers it, in file order.
    pub fn init(&self, shell: Shell) -> String {
        self.init_blocks
            .iter()
            .filter(|(target, code)| target.covers(shell) && !code.trim().is_empty())
            .map(|(_, code)| code.trim_matches('\n'))
            .collect::<Vec<_>>()
            .join("\n")
    }
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
                        if Shell::ALL
                            .iter()
                            .any(|s| v.get(*s).is_some_and(|b| b.trim().is_empty()))
                        {
                            return err(format!("{ctx}: alias body is empty"));
                        }
                        cfg.aliases.push((name.to_string(), v));
                    }
                }
                _ => {
                    let Some(target) = Target::named(key) else {
                        let mut sections = vec!["env", "path", "aliases"];
                        sections.extend(Target::names());
                        return err(format!(
                            "unknown section `{key}` (expected {})",
                            one_of(&sections)
                        ));
                    };
                    for (k, v) in as_table(val, key)? {
                        let ctx = format!("{key}.{k}");
                        if k != "init" {
                            return err(format!("{ctx}: unknown key (expected `init`)"));
                        }
                        let Toml::String(s) = v else {
                            return err(format!("{ctx}: expected a string of {key} code"));
                        };
                        cfg.init_blocks.push((target, s.clone()));
                    }
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
            let mut by_target = BTreeMap::new();
            for (k, v) in t {
                let sub = format!("{ctx}.{k}");
                let Some(target) = Target::named(k) else {
                    let names: Vec<String> =
                        Target::names().iter().map(|n| format!("`{n}`")).collect();
                    return err(format!("{sub}: unknown key (expected {})", one_of(&names)));
                };
                by_target.insert(target, scalar(v, &sub)?);
            }
            Ok(Value::PerShell(by_target))
        }
        other => Ok(Value::All(scalar(other, ctx)?)),
    }
}

/// "a, b or c"
fn one_of<S: AsRef<str>>(items: &[S]) -> String {
    match items {
        [] => String::new(),
        [only] => only.as_ref().to_string(),
        [init @ .., last] => {
            let init: Vec<&str> = init.iter().map(AsRef::as_ref).collect();
            format!("{} or {}", init.join(", "), last.as_ref())
        }
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
        assert_eq!(cfg.env[1].1, Value::All("1".into()));
        assert_eq!(cfg.env[2].1.get(Shell::Nu), Some("bat"));
        assert_eq!(cfg.path_prepend[1].get(Shell::Nu), None);
        assert_eq!(cfg.path_append.len(), 1);
        assert_eq!(cfg.aliases[1].1.get(Shell::Zsh), Some("ls -lh"));
        assert_eq!(cfg.init(Shell::Zsh), "setopt autocd");
        assert_eq!(cfg.init(Shell::Nu), "$env.config.show_banner = false");
    }

    #[test]
    fn sh_covers_the_sh_family_and_yields_to_a_shell_key() {
        let cfg = Config::parse(
            r#"
            [aliases]
            a = { sh = "for sh", nu = "for nu" }
            b = { sh = "for sh", zsh = "for zsh" }
            c = { sh = "for sh" }
            "#,
        )
        .unwrap();
        let get = |i: usize, shell| cfg.aliases[i].1.get(shell);
        assert_eq!(get(0, Shell::Zsh), Some("for sh"));
        assert_eq!(get(0, Shell::Nu), Some("for nu"));
        assert_eq!(get(1, Shell::Zsh), Some("for zsh"));
        assert_eq!(get(2, Shell::Nu), None);
    }

    #[test]
    fn init_blocks_add_up_in_file_order() {
        let cfg = Config::parse(
            r#"
            [zsh]
            init = "zsh first"
            [nu]
            init = "nu only"
            [sh]
            init = "\nshared\n"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.init(Shell::Zsh), "zsh first\nshared");
        assert_eq!(cfg.init(Shell::Nu), "nu only");
    }

    #[test]
    fn reports_the_offending_key() {
        let e = |s: &str| Config::parse(s).unwrap_err().0;
        assert!(e("[env]\n\"BAD-NAME\" = 'x'").starts_with("env.BAD-NAME"));
        assert_eq!(
            e("[env]\nX = { fish = 'x' }"),
            "env.X.fish: unknown key (expected `sh`, `zsh` or `nu`)"
        );
        assert!(e("[aliases]\nx = { sh = ' ' }").contains("empty"));
        assert!(e("[path]\nprepend = [1]").starts_with("path.prepend[0]"));
        assert!(e("[aliases]\nx = ''").contains("empty"));
        assert_eq!(
            e("[bogus]"),
            "unknown section `bogus` (expected env, path, aliases, sh, zsh or nu)"
        );
        assert!(e("[env\n").contains("line 1"));
    }
}
