//! nushell: generated code, and the hooks in `env.nu` and `config.nu`.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Dialect, Shell, home_relative};
use crate::Paths;
use crate::config::Condition;
use crate::template::{self, Seg};

pub struct Nu;

impl Dialect for Nu {
    fn env(name: &str, value: &str) -> String {
        format!("$env.{name} = {}\n", template_expr(value))
    }

    fn path(prepend: &[&str], append: &[&str]) -> String {
        let list = |l: &[&str]| {
            l.iter()
                .map(|p| template_expr(p))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let mut expr = String::from("$env.PATH | split row (char esep)");
        if !prepend.is_empty() {
            expr.push_str(&format!(" | prepend [{}]", list(prepend)));
        }
        if !append.is_empty() {
            expr.push_str(&format!(" | append [{}]", list(append)));
        }
        format!("$env.PATH = ({expr} | uniq)\n")
    }

    fn alias(name: &str, body: &str) -> String {
        format!("alias {name} = {}\n", body.trim())
    }

    /// nushell defines aliases while parsing, so an alias inside a runtime
    /// `if` would only exist in that block.
    fn guard(_: &[Condition]) -> Option<(String, String)> {
        None
    }
}

/// A nushell string literal that needs no escaping: `'...'` when possible,
/// otherwise a raw string `r#'...'#` with enough `#`s.
pub fn quote(s: &str) -> String {
    if !s.contains('\'') {
        return format!("'{s}'");
    }
    let mut hashes = String::from("#");
    while s.contains(&format!("'{hashes}")) {
        hashes.push('#');
    }
    format!("r{hashes}'{s}'{hashes}")
}

fn var(name: &str) -> String {
    if name == "PATH" {
        // $env.PATH is a list in nushell; flatten it the way other shells see it.
        "($env.PATH | split row (char esep) | str join (char esep))".into()
    } else {
        format!("($env.{name}? | default '')")
    }
}

/// A nushell expression for a template string.
fn template_expr(s: &str) -> String {
    let parts: Vec<String> = template::parse(s)
        .into_iter()
        .map(|seg| match seg {
            Seg::Lit(lit) => quote(&lit),
            Seg::Var(name) => var(&name),
        })
        .collect();
    match parts.len() {
        0 => "''".into(),
        1 => parts.into_iter().next().unwrap(),
        _ => format!("([{}] | str join)", parts.join(" ")),
    }
}

/// A path as a nushell string, `~`-relative where possible.
fn path_str(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
        Some(rel) => quote(&format!("~/{rel}")),
        None => quote(&p.display().to_string()),
    }
}

/// The hook body for `env.nu`, which nushell runs before parsing
/// `config.nu`, so a rebuild here is visible to `config.nu`'s `source`.
/// `try` also covers a missing binary.
pub fn env_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_str(p, home);
    format!(
        "do {{\n  \
         let exe = ({exe} | path expand)\n  \
         let cfg = ({cfg} | path expand)\n  \
         let out = ({out} | path expand)\n  \
         if (not ($out | path exists)) or (($cfg | path exists) and ((ls $cfg | get 0.modified) != (ls $out | get 0.modified))) {{\n    \
         try {{ ^$exe build --config $cfg --out-dir ({dir} | path expand) }}\n  \
         }}\n\
         }}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        out = q(&paths.output(Shell::Nu)),
        dir = q(&paths.out_dir),
    )
}

/// The hook body for `config.nu`. `source` resolves its path at parse time,
/// so the existence check has to be a constant expression; `source null`
/// does nothing.
pub fn config_hook(paths: &Paths, home: &Path) -> String {
    let out = path_str(&paths.output(Shell::Nu), home);
    format!("source (if ({out} | path exists) {{ {out} }} else {{ null }})\n")
}

/// `env.nu` and `config.nu`.
pub fn config_files() -> (PathBuf, PathBuf) {
    ask_nu().unwrap_or_else(|| {
        let dir = if cfg!(target_os = "macos") && env::var_os("XDG_CONFIG_HOME").is_none() {
            crate::home().join("Library/Application Support/nushell")
        } else {
            crate::xdg("XDG_CONFIG_HOME", ".config").join("nushell")
        };
        (dir.join("env.nu"), dir.join("config.nu"))
    })
}

/// Asks nushell itself; it knows about XDG_CONFIG_HOME and platform defaults.
fn ask_nu() -> Option<(PathBuf, PathBuf)> {
    let out = Command::new("nu")
        .args([
            "-n",
            "-c",
            "[$nu.env-path $nu.config-path] | str join (char nl)",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    Some((PathBuf::from(lines.next()?), PathBuf::from(lines.next()?)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::render_body;

    #[test]
    fn quoting() {
        assert_eq!(quote("plain"), "'plain'");
        assert_eq!(quote("it's"), "r#'it's'#");
        assert_eq!(quote("a'#b"), "r##'a'#b'##");
        assert_eq!(template_expr(""), "''");
        assert_eq!(template_expr("$HOME"), "($env.HOME? | default '')");
        assert_eq!(
            template_expr("~/x"),
            "([($env.HOME? | default '') '/x'] | str join)"
        );
    }

    #[test]
    fn hook_paths() {
        let home = Path::new("/home/me");
        assert_eq!(
            path_str(Path::new("/home/me/it's $x/bin"), home),
            "r#'~/it's $x/bin'#"
        );
        assert_eq!(path_str(Path::new("/opt/x"), home), "'/opt/x'");
    }

    #[test]
    fn renders() {
        let cfg = r#"
            [env]
            EDITOR = "nvim"
            ONLY_ZSH = { zsh = "x" }
            [path]
            prepend = ["/a", "/b"]
            [aliases]
            ll = { zsh = "ls -lh", nu = "ls -l" }
        "#;
        assert_eq!(
            render_body(cfg, Shell::Nu),
            [
                "$env.EDITOR = 'nvim'",
                "$env.PATH = ($env.PATH | split row (char esep) | prepend ['/a' '/b'] | uniq)",
                "alias ll = ls -l",
            ]
            .join("\n")
        );
    }

    /// Conditions are checked at build time, against PATH as built up by
    /// the config so far: the base [path], then each applied tool's entries.
    #[test]
    fn checks_tools_when_built() {
        use crate::probe::{Probe, fixture};
        use crate::shell::render_body_with;
        use std::fs;

        let dir = fixture("nu-tools");
        fs::create_dir(dir.join("sub")).unwrap();
        fs::copy(dir.join("tool"), dir.join("sub/inner")).unwrap();
        let d = dir.display().to_string();
        let probe = Probe::with_vars(&[("D", &d)]);
        let cfg = r#"
            [path]
            prepend = ["$D"]
            [tool.tool.aliases]
            t = "tool"
            [tool.missing.aliases]
            m = "missing"
            [tool.inner-too-early.aliases]
            i = "inner"
            [tool.sub]
            exists = "$D/sub"
            path.prepend = ["$D/sub"]
            [tool.inner.aliases]
            i = "inner"
            [tool.data]
            command = "$D/data"
            aliases.d = "data"
        "#;
        let d_expr = "($env.D? | default '')";
        assert_eq!(
            render_body_with(cfg, Shell::Nu, &probe),
            [
                format!("$env.PATH = ($env.PATH | split row (char esep) | prepend [{d_expr}] | uniq)"),
                String::new(),
                "# tool: tool".into(),
                "alias t = tool".into(),
                String::new(),
                "# tool: missing (skipped: `missing` not on PATH when this file was built)".into(),
                String::new(),
                "# tool: inner-too-early (skipped: `inner-too-early` not on PATH when this file was built)".into(),
                String::new(),
                "# tool: sub".into(),
                format!("$env.PATH = ($env.PATH | split row (char esep) | prepend [([{d_expr} '/sub'] | str join)] | uniq)"),
                String::new(),
                "# tool: inner".into(),
                "alias i = inner".into(),
                String::new(),
                "# tool: data (skipped: `$D/data` not executable when this file was built)".into(),
            ]
            .join("\n")
        );
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn ignores_sh_entries() {
        let cfg = r#"
            [env]
            PAGER = { sh = "less" }
            [aliases]
            ll = { sh = "ls -lh", nu = "ls -l" }
            [sh]
            init = "setopt autocd"
        "#;
        assert_eq!(render_body(cfg, Shell::Nu), "alias ll = ls -l");
    }
}
