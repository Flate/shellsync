//! The supported shells. Everything specific to one shell lives in its
//! module: quoting, the generated code, the startup hooks and where they go.

pub mod bash;
pub mod nu;
pub mod pwsh;
pub mod zsh;

use std::path::{Path, PathBuf};
use std::process::Command;

use crate::config::{Condition, Config, Entries, Tool, Value};
use crate::probe::Probe;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Shell {
    Zsh,
    Bash,
    Nu,
    Pwsh,
}

impl Shell {
    pub const ALL: [Shell; 4] = [Shell::Zsh, Shell::Bash, Shell::Nu, Shell::Pwsh];

    /// The name used in the config file and in generated file names.
    pub fn name(self) -> &'static str {
        match self {
            Shell::Zsh => "zsh",
            Shell::Bash => "bash",
            Shell::Nu => "nu",
            Shell::Pwsh => "pwsh",
        }
    }

    /// Whether the shell shares POSIX sh syntax, so the `sh` config key
    /// applies to it.
    pub fn is_sh(self) -> bool {
        matches!(self, Shell::Zsh | Shell::Bash)
    }

    /// Whether shellsync hooks the shell into this platform: zsh isn't a
    /// Windows shell. (bash is, in Git Bash, MSYS2 and Cygwin.)
    pub fn native(self) -> bool {
        !(cfg!(windows) && self == Shell::Zsh)
    }

    /// The shell with this config name.
    pub fn named(name: &str) -> Option<Shell> {
        Shell::ALL.into_iter().find(|s| s.name() == name)
    }

    /// A shell as given on the command line, where aliases are accepted.
    pub fn parse(s: &str) -> Option<Shell> {
        match s {
            "nushell" => Some(Shell::Nu),
            "powershell" => Some(Shell::Pwsh),
            _ => Shell::named(s),
        }
    }

    /// A command that parses `file` with the shell at `exe` without running
    /// it. It fails, with the shell's error message on stderr, if `file`
    /// doesn't parse.
    pub fn syntax_check(self, exe: &Path, file: &str) -> Command {
        match self {
            Shell::Zsh => zsh::syntax_check(exe, file),
            Shell::Bash => bash::syntax_check(exe, file),
            Shell::Nu => nu::syntax_check(exe, file),
            Shell::Pwsh => pwsh::syntax_check(exe, file),
        }
    }

    /// The files generated for this shell, and what goes into each.
    pub fn files(self) -> &'static [(&'static str, Parts)] {
        match self {
            // `.zshenv` (every zsh) loads the environment, `.zshrc`
            // (interactive) the rest, with PATH again: macOS login shells run
            // path_helper in between, which moves earlier entries to the end.
            Shell::Zsh => &[("env.zsh", Parts::ENV), ("init.zsh", Parts::INTERACTIVE)],
            Shell::Bash => &[("init.bash", Parts::ALL)],
            Shell::Nu => &[("init.nu", Parts::ALL)],
            Shell::Pwsh => &[("init.ps1", Parts::ALL)],
        }
    }
}

/// Which kinds of entries go into a generated file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Parts {
    env: bool,
    path: bool,
    /// Aliases and init code.
    interactive: bool,
}

impl Parts {
    pub const ALL: Parts = Parts {
        env: true,
        path: true,
        interactive: true,
    };
    pub const ENV: Parts = Parts {
        env: true,
        path: true,
        interactive: false,
    };
    pub const INTERACTIVE: Parts = Parts {
        env: false,
        path: true,
        interactive: true,
    };
}

/// How one shell writes each kind of config entry. Each method returns
/// complete lines, including the final newline.
trait Dialect {
    fn env(name: &str, value: &str) -> String;
    fn path(prepend: &[&str], append: &[&str]) -> String;
    fn alias(name: &str, body: &str) -> String;
    /// Code that parses the way an alias's body does, for `check`, and
    /// whether parsing the generated file parses the body too.
    fn alias_snippet(name: &str, body: &str) -> (String, bool);
    /// The lines that open and close a block that applies only if all
    /// `conditions` hold when the shell starts. None if the shell can't check
    /// them at startup; the build then decides, see [`Checks`].
    fn guard(conditions: &[Condition]) -> Option<(String, String)>;
    /// Code the guards need: written before the first guarded block of a
    /// file, and after the last.
    fn guard_helpers() -> (&'static str, &'static str) {
        ("", "")
    }
}

/// How the build decides `[tool]` conditions for shells whose `Dialect`
/// can't check them at startup.
pub enum Checks<'a> {
    /// Check them now, in this process's environment.
    Now(&'a Probe),
    /// The tools the shell found when it started: nushell's env.nu hook
    /// checks them and passes the result to `build --nu-tools`.
    Found(&'a [String]),
}

pub struct Rendered {
    /// Each of [`Shell::files`] with its contents.
    pub files: Vec<(&'static str, String)>,
    /// The tools decided at build time that applied, in file order.
    pub applied: Vec<String>,
}

pub fn render(cfg: &Config, shell: Shell, source: &str, checks: &Checks) -> Rendered {
    match shell {
        Shell::Zsh => render_with::<zsh::Zsh>(cfg, shell, source, checks),
        Shell::Bash => render_with::<bash::Bash>(cfg, shell, source, checks),
        Shell::Nu => render_with::<nu::Nu>(cfg, shell, source, checks),
        Shell::Pwsh => render_with::<pwsh::Pwsh>(cfg, shell, source, checks),
    }
}

/// A piece of raw shell code from the config, for `check`.
#[derive(Debug, PartialEq)]
pub struct Snippet {
    /// The config key it comes from, such as `aliases.ll` or `tool.fzf.sh.init`.
    pub key: String,
    pub code: String,
    /// Whether parsing the generated files parses this code too.
    pub in_files: bool,
}

/// The alias bodies and init blocks that apply to `shell`, in file order.
pub fn snippets(cfg: &Config, shell: Shell) -> Vec<Snippet> {
    match shell {
        Shell::Zsh => snippets_with::<zsh::Zsh>(cfg, shell),
        Shell::Bash => snippets_with::<bash::Bash>(cfg, shell),
        Shell::Nu => snippets_with::<nu::Nu>(cfg, shell),
        Shell::Pwsh => snippets_with::<pwsh::Pwsh>(cfg, shell),
    }
}

fn snippets_with<D: Dialect>(cfg: &Config, shell: Shell) -> Vec<Snippet> {
    let tables = std::iter::once((String::new(), &cfg.entries)).chain(
        cfg.tools
            .iter()
            .map(|t| (format!("tool.{}.", t.name), &t.entries)),
    );
    let mut out = Vec::new();
    for (prefix, entries) in tables {
        for (name, v) in &entries.aliases {
            if let Some(body) = v.get(shell) {
                let (code, in_files) = D::alias_snippet(name, body);
                out.push(Snippet {
                    key: format!("{prefix}aliases.{name}"),
                    code,
                    in_files,
                });
            }
        }
        for (target, code) in &entries.init_blocks {
            if target.covers(shell) && !code.trim().is_empty() {
                out.push(Snippet {
                    key: format!("{prefix}{}.init", target.name()),
                    code: code.clone(),
                    in_files: true,
                });
            }
        }
    }
    out
}

/// How a tool's block is written.
enum Decision {
    /// Checked at startup, between these lines.
    Guard(String, String),
    /// Decided at build time.
    Applies,
    Skipped(String),
}

fn render_with<D: Dialect>(cfg: &Config, shell: Shell, source: &str, checks: &Checks) -> Rendered {
    let header = format!(
        "# Generated by shellsync {} from {source}\n# Do not edit: changes are overwritten. Edit the source file instead.\n",
        env!("CARGO_PKG_VERSION")
    );
    // Decided once, in file order, for all files of the shell.
    let mut decider = Decider::new(checks, &cfg.entries, shell);
    let mut applied = Vec::new();
    let mut tools = Vec::new();
    for tool in &cfg.tools {
        if tool_block::<D>(tool, shell, Parts::ALL).is_empty() {
            continue;
        }
        let decision = match D::guard(&tool.conditions) {
            Some((open, close)) => Decision::Guard(open, close),
            None => match decider.decide(tool, shell) {
                Ok(()) => {
                    applied.push(tool.name.clone());
                    Decision::Applies
                }
                Err(why) => Decision::Skipped(why),
            },
        };
        tools.push((tool, decision));
    }
    let files = shell
        .files()
        .iter()
        .map(|&(name, parts)| (name, header.clone() + &body::<D>(cfg, shell, parts, &tools)))
        .collect();
    Rendered { files, applied }
}

/// Order: unconditional entries, then each `[tool]` block in file order,
/// then the unconditional `init` code, so it can rely on everything above.
fn body<D: Dialect>(
    cfg: &Config,
    shell: Shell,
    parts: Parts,
    tools: &[(&Tool, Decision)],
) -> String {
    let mut out = lines::<D>(&cfg.entries, shell, parts);
    let (helpers, helpers_end) = D::guard_helpers();
    let mut guarded = false;
    for (tool, decision) in tools {
        let block = tool_block::<D>(tool, shell, parts);
        if block.is_empty() {
            continue;
        }
        let name = &tool.name;
        if matches!(decision, Decision::Guard(..)) && !guarded {
            guarded = true;
            if !helpers.is_empty() {
                out.push_str(&format!("\n{helpers}"));
            }
        }
        out.push_str(&match decision {
            Decision::Guard(open, close) => format!("\n# tool: {name}\n{open}{block}{close}"),
            Decision::Applies => format!("\n# tool: {name}\n{block}"),
            Decision::Skipped(why) => format!("\n# tool: {name} (skipped: {why})\n"),
        });
    }
    if guarded {
        out.push_str(helpers_end);
    }
    let code = cfg.entries.init(shell);
    if parts.interactive && !code.is_empty() {
        out.push_str("\n# --- init ---\n");
        out.push_str(&code);
        out.push('\n');
    }
    out
}

/// Decides tools in file order. With [`Checks::Now`], a check sees PATH as
/// the shell will have it at that block: the process PATH, plus `[path]`,
/// plus the path entries of earlier tools that applied.
struct Decider<'a> {
    checks: &'a Checks<'a>,
    path: Vec<PathBuf>,
}

impl<'a> Decider<'a> {
    fn new(checks: &'a Checks<'a>, base: &Entries, shell: Shell) -> Decider<'a> {
        let mut path = Vec::new();
        if let Checks::Now(probe) = checks {
            path = probe.path_dirs();
            add_to_path(&mut path, base, shell, probe);
        }
        Decider { checks, path }
    }

    /// Ok if `tool` applies, otherwise why not.
    fn decide(&mut self, tool: &Tool, shell: Shell) -> Result<(), String> {
        match self.checks {
            Checks::Now(probe) => {
                if let Some(failed) = tool.conditions.iter().find(|c| !probe.holds(c, &self.path)) {
                    return Err(format!("{} when this file was built", failed.failure()));
                }
                add_to_path(&mut self.path, &tool.entries, shell, probe);
                Ok(())
            }
            Checks::Found(found) if found.contains(&tool.name) => Ok(()),
            Checks::Found(_) => Err(format!("not found when {} started", shell.name())),
        }
    }
}

/// The `parts` a tool sets in `shell`; empty if it sets nothing there.
fn tool_block<D: Dialect>(tool: &Tool, shell: Shell, parts: Parts) -> String {
    let mut block = lines::<D>(&tool.entries, shell, parts);
    let init = tool.entries.init(shell);
    if parts.interactive && !init.is_empty() {
        block.push_str(&init);
        block.push('\n');
    }
    block
}

/// The env, path and alias lines of `entries` in `parts` (not the init code).
fn lines<D: Dialect>(entries: &Entries, shell: Shell, parts: Parts) -> String {
    let mut out = String::new();
    if parts.env {
        for (name, v) in &entries.env {
            if let Some(v) = v.get(shell) {
                out.push_str(&D::env(name, v));
            }
        }
    }
    let (pre, post) = (
        values(&entries.path_prepend, shell),
        values(&entries.path_append, shell),
    );
    if parts.path && (!pre.is_empty() || !post.is_empty()) {
        out.push_str(&D::path(&pre, &post));
    }
    if parts.interactive {
        for (name, v) in &entries.aliases {
            if let Some(body) = v.get(shell) {
                out.push_str(&D::alias(name, body));
            }
        }
    }
    out
}

/// Applies the PATH entries of `entries` to `path`, as the shell would.
fn add_to_path(path: &mut Vec<PathBuf>, entries: &Entries, shell: Shell, probe: &Probe) {
    let expand = |list: &[Value]| -> Vec<PathBuf> {
        values(list, shell)
            .into_iter()
            .map(|p| PathBuf::from(probe.expand(p)))
            .collect()
    };
    let mut new = expand(&entries.path_prepend);
    new.append(path);
    new.extend(expand(&entries.path_append));
    *path = new;
}

fn values(list: &[Value], shell: Shell) -> Vec<&str> {
    list.iter().filter_map(|v| v.get(shell)).collect()
}

/// `p` relative to `home`, if it is inside it. With home `/` every path
/// would be inside, so none is.
/// Separated by `/`, which the shells also take on Windows.
pub fn home_relative(p: &Path, home: &Path) -> Option<String> {
    let rel = p.strip_prefix(home).ok()?.to_str()?;
    (home != Path::new("/") && !rel.is_empty()).then(|| rel.replace(std::path::MAIN_SEPARATOR, "/"))
}

/// For tests: each generated file without its header, with no environment
/// variables set for build-time checks.
#[cfg(test)]
pub fn render_body(cfg: &str, shell: Shell) -> Vec<(&'static str, String)> {
    render_body_with(cfg, shell, &Checks::Now(&Probe::with_vars(&[])))
}

#[cfg(test)]
pub fn render_body_with(cfg: &str, shell: Shell, checks: &Checks) -> Vec<(&'static str, String)> {
    render(&Config::parse(cfg).unwrap(), shell, "test", checks)
        .files
        .into_iter()
        .map(|(name, text)| (name, text.lines().skip(2).collect::<Vec<_>>().join("\n")))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_config_is_just_a_header() {
        for shell in Shell::ALL {
            for (name, body) in render_body("", shell) {
                assert_eq!(body, "", "{name}");
            }
        }
    }

    #[test]
    fn snippets_name_their_config_keys() {
        let cfg = Config::parse(
            r#"
            [aliases]
            g = "git"
            ll = { sh = "ls -l", nu = "ls" }
            [sh]
            init = "shared"
            [zsh]
            init = "zsh only"
            [nu]
            init = "  "
            [tool.fzf]
            aliases.f = { nu = "fzf" }
            nu.init = "nu init"
            "#,
        )
        .unwrap();
        let keys = |shell| -> Vec<(String, String, bool)> {
            snippets(&cfg, shell)
                .into_iter()
                .map(|s| (s.key, s.code, s.in_files))
                .collect()
        };
        let s = |k: &str, c: &str, f| (k.to_string(), c.to_string(), f);
        assert_eq!(
            keys(Shell::Zsh),
            [
                s("aliases.g", "git", false),
                s("aliases.ll", "ls -l", false),
                s("sh.init", "shared", true),
                s("zsh.init", "zsh only", true),
            ]
        );
        assert_eq!(
            keys(Shell::Nu),
            [
                s("aliases.g", "alias g = git\n", true),
                s("aliases.ll", "alias ll = ls\n", true),
                s("tool.fzf.aliases.f", "alias f = fzf\n", true),
                s("tool.fzf.nu.init", "nu init", true),
            ]
        );
    }

    #[test]
    fn parses_shell_names() {
        assert_eq!(Shell::named("nu"), Some(Shell::Nu));
        assert_eq!(Shell::named("nushell"), None);
        assert_eq!(Shell::parse("nushell"), Some(Shell::Nu));
        assert_eq!(Shell::parse("fish"), None);
    }
}
