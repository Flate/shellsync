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

    /// The key in the config file.
    pub fn name(self) -> &'static str {
        match self {
            Target::Sh => "sh",
            Target::Shell(s) => s.name(),
        }
    }

    pub fn covers(self, shell: Shell) -> bool {
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

/// Entries that apply together: the top level of the config, or one
/// `[tool.X]` table.
#[derive(Debug, Default, PartialEq)]
pub struct Entries {
    pub env: Vec<(String, Value)>,
    pub path_prepend: Vec<Value>,
    pub path_append: Vec<Value>,
    pub aliases: Vec<(String, Value)>,
    /// Raw code from the `init` of each `[sh]`/`[<shell>]` section, in file order.
    pub init_blocks: Vec<(Target, String)>,
}

impl Entries {
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

/// Something that must hold for a `[tool.X]` table to apply. Names and paths
/// are templates (`~`, `$NAME`).
#[derive(Clone, Debug, PartialEq)]
pub enum Condition {
    /// A command with this name is on PATH and can run.
    Command(String),
    /// The file at this path can run.
    Executable(String),
    /// A file or directory exists at this path.
    Exists(String),
}

impl Condition {
    /// Why the condition failed, for comments in generated files.
    pub fn failure(&self) -> String {
        match self {
            Condition::Command(name) => format!("`{name}` not on PATH"),
            Condition::Executable(path) => format!("`{path}` not executable"),
            Condition::Exists(path) => format!("`{path}` does not exist"),
        }
    }
}

/// A `[tool.X]` table: entries that apply only if all conditions hold.
#[derive(Debug, PartialEq)]
pub struct Tool {
    pub name: String,
    pub conditions: Vec<Condition>,
    pub entries: Entries,
}

#[derive(Debug, Default, PartialEq)]
pub struct Config {
    /// Entries outside `[tool]` tables; they always apply.
    pub entries: Entries,
    pub tools: Vec<Tool>,
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
            if key == "tool" {
                for (name, t) in as_table(val, "tool")? {
                    let ctx = format!("tool.{name}");
                    cfg.tools.push(tool(name, as_table(t, &ctx)?, &ctx)?);
                }
            } else if !section(&mut cfg.entries, key, val, "")? {
                let mut sections = vec!["env", "path", "aliases", "tool"];
                sections.extend(Target::names());
                return err(format!(
                    "unknown section `{key}` (expected {})",
                    one_of(&sections)
                ));
            }
        }
        Ok(cfg)
    }
}

/// Parses one `[tool.X]` table. Without `command` or `exists`, the tool's
/// name is the command to look for.
fn tool(name: &str, table: &Table, ctx: &str) -> Result<Tool, Error> {
    if name.trim().is_empty() {
        return err(format!("{ctx}: the tool name is empty"));
    }
    let prefix = format!("{ctx}.");
    let mut conditions = Vec::new();
    let mut entries = Entries::default();
    for (key, val) in table {
        let key_ctx = format!("{prefix}{key}");
        match key.as_str() {
            "command" => {
                for s in strings(val, &key_ctx)? {
                    conditions.push(if s.contains('/') || s.starts_with('~') {
                        Condition::Executable(s)
                    } else {
                        Condition::Command(s)
                    });
                }
            }
            "exists" => {
                for s in strings(val, &key_ctx)? {
                    conditions.push(Condition::Exists(s));
                }
            }
            _ => {
                if !section(&mut entries, key, val, &prefix)? {
                    let mut keys = vec!["command", "exists", "env", "path", "aliases"];
                    keys.extend(Target::names());
                    return err(format!(
                        "{key_ctx}: unknown key (expected {})",
                        one_of(&keys)
                    ));
                }
            }
        }
    }
    if conditions.is_empty() {
        conditions.push(Condition::Command(name.to_string()));
    }
    Ok(Tool {
        name: name.to_string(),
        conditions,
        entries,
    })
}

/// A string or a non-empty array of strings, none of them empty.
fn strings(v: &Toml, ctx: &str) -> Result<Vec<String>, Error> {
    let items = match v {
        Toml::String(s) => vec![s.clone()],
        Toml::Array(items) if !items.is_empty() => items
            .iter()
            .map(|item| match item {
                Toml::String(s) => Ok(s.clone()),
                _ => err(format!("{ctx}: expected strings")),
            })
            .collect::<Result<_, _>>()?,
        _ => return err(format!("{ctx}: expected a string or an array of strings")),
    };
    if items.iter().any(|s| s.trim().is_empty()) {
        return err(format!("{ctx}: empty string"));
    }
    Ok(items)
}

/// Parses a section that can appear at the top level and in `[tool.X]`
/// tables into `entries`. Returns false if `key` is not such a section.
/// `prefix` is prepended to keys in error messages.
fn section(entries: &mut Entries, key: &str, val: &Toml, prefix: &str) -> Result<bool, Error> {
    let ctx = format!("{prefix}{key}");
    match key {
        "env" => {
            for (name, v) in as_table(val, &ctx)? {
                let ctx = format!("{ctx}.{name}");
                if !is_env_name(name) {
                    return err(format!(
                        "{ctx}: invalid variable name (use letters, digits and _)"
                    ));
                }
                entries.env.push((name.to_string(), value(v, &ctx, true)?));
            }
        }
        "path" => {
            for (k, v) in as_table(val, &ctx)? {
                let ctx = format!("{ctx}.{k}");
                let list = match k.as_str() {
                    "prepend" => &mut entries.path_prepend,
                    "append" => &mut entries.path_append,
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
            for (name, v) in as_table(val, &ctx)? {
                let ctx = format!("{ctx}.{name}");
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
                entries.aliases.push((name.to_string(), v));
            }
        }
        _ => {
            let Some(target) = Target::named(key) else {
                return Ok(false);
            };
            for (k, v) in as_table(val, &ctx)? {
                let ctx = format!("{ctx}.{k}");
                if k != "init" {
                    return err(format!("{ctx}: unknown key (expected `init`)"));
                }
                let Toml::String(s) = v else {
                    return err(format!("{ctx}: expected a string of {key} code"));
                };
                entries.init_blocks.push((target, s.clone()));
            }
        }
    }
    Ok(true)
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
        assert!(cfg.tools.is_empty());
        let cfg = cfg.entries;
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
        let get = |i: usize, shell| cfg.entries.aliases[i].1.get(shell);
        assert_eq!(get(0, Shell::Zsh), Some("for sh"));
        assert_eq!(get(0, Shell::Bash), Some("for sh"));
        assert_eq!(get(0, Shell::Nu), Some("for nu"));
        assert_eq!(get(1, Shell::Zsh), Some("for zsh"));
        assert_eq!(get(1, Shell::Bash), Some("for sh"));
        assert_eq!(get(2, Shell::Pwsh), None);
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
        assert_eq!(cfg.entries.init(Shell::Zsh), "zsh first\nshared");
        assert_eq!(cfg.entries.init(Shell::Nu), "nu only");
    }

    #[test]
    fn reports_the_offending_key() {
        let e = |s: &str| Config::parse(s).unwrap_err().0;
        assert!(e("[env]\n\"BAD-NAME\" = 'x'").starts_with("env.BAD-NAME"));
        assert_eq!(
            e("[env]\nX = { fish = 'x' }"),
            "env.X.fish: unknown key (expected `sh`, `zsh`, `bash`, `nu` or `pwsh`)"
        );
        assert!(e("[aliases]\nx = { sh = ' ' }").contains("empty"));
        assert!(e("[path]\nprepend = [1]").starts_with("path.prepend[0]"));
        assert!(e("[aliases]\nx = ''").contains("empty"));
        assert_eq!(
            e("[bogus]"),
            "unknown section `bogus` (expected env, path, aliases, tool, sh, zsh, bash, nu or pwsh)"
        );
        assert!(e("[env\n").contains("line 1"));
        assert_eq!(
            e("[tool.eza]\nfoo = 1"),
            "tool.eza.foo: unknown key (expected command, exists, env, path, aliases, sh, zsh, bash, nu or pwsh)"
        );
        assert!(e("[tool.eza.env]\n\"BAD-NAME\" = 'x'").starts_with("tool.eza.env.BAD-NAME"));
        assert!(e("[tool.eza.aliases]\nx = { fish = 'x' }").starts_with("tool.eza.aliases.x.fish"));
        assert!(e("[tool.eza]\ncommand = []").contains("expected a string or an array"));
        assert!(e("[tool.eza]\nexists = ['']").contains("empty string"));
        assert!(e("[tool.eza]\ncommand = 1").starts_with("tool.eza.command"));
        assert!(e("[tool.x.tool.y]").contains("unknown key"));
        assert!(e("[tool.' '.aliases]\nx = 'y'").contains("tool name is empty"));
    }

    #[test]
    fn parses_tools() {
        let cfg = Config::parse(
            r#"
            [aliases]
            g = "git"

            [tool.eza.aliases]
            ls = "eza"

            [tool.borg]
            command = "/mnt/Private/bin/borg"
            env.TMPDIR = "/mnt/Private/borg-tmp"

            [tool.android]
            exists = "~/Library/Android/sdk"
            path.append = ["~/Library/Android/sdk/platform-tools"]

            [tool.py]
            command = ["python3", "~/bin/uv"]
            exists = ["/a", "/b"]
            zsh.init = "x"
            "#,
        )
        .unwrap();
        assert_eq!(cfg.entries.aliases.len(), 1);
        let names: Vec<_> = cfg.tools.iter().map(|t| t.name.as_str()).collect();
        assert_eq!(names, ["eza", "borg", "android", "py"]);
        use Condition::*;
        assert_eq!(cfg.tools[0].conditions, [Command("eza".into())]);
        assert_eq!(cfg.tools[0].entries.aliases[0].0, "ls");
        assert_eq!(
            cfg.tools[1].conditions,
            [Executable("/mnt/Private/bin/borg".into())]
        );
        assert_eq!(cfg.tools[1].entries.env[0].0, "TMPDIR");
        assert_eq!(
            cfg.tools[2].conditions,
            [Exists("~/Library/Android/sdk".into())]
        );
        assert_eq!(cfg.tools[2].entries.path_append.len(), 1);
        assert_eq!(
            cfg.tools[3].conditions,
            [
                Command("python3".into()),
                Executable("~/bin/uv".into()),
                Exists("/a".into()),
                Exists("/b".into()),
            ]
        );
        assert_eq!(cfg.tools[3].entries.init(Shell::Zsh), "x");
    }
}
