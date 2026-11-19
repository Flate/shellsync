//! zsh: generated code, and the hooks in `.zshenv` and `.zshrc`.

use std::env;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Dialect, home_relative};
use crate::Paths;
use crate::config::Condition;
use crate::template::{self, Seg};

pub struct Zsh;

impl Dialect for Zsh {
    fn env(name: &str, value: &str) -> String {
        format!("export {name}={}\n", template_word(value))
    }

    fn path(prepend: &[&str], append: &[&str]) -> String {
        // -U keeps the first occurrence, so prepended entries move to the front.
        let mut words: Vec<String> = prepend.iter().map(|p| template_word(p)).collect();
        words.push("$path".into());
        words.extend(append.iter().map(|p| template_word(p)));
        format!("typeset -gU path\npath=({})\n", words.join(" "))
    }

    fn alias(name: &str, body: &str) -> String {
        format!("alias -- {name}={}\n", quote(body))
    }

    /// zsh parses an alias's body only where the alias is used.
    fn alias_snippet(_: &str, body: &str) -> (String, bool) {
        (body.to_string(), false)
    }

    /// Builtins only, so startup spawns no process. `whence -p` searches PATH
    /// only (not aliases or functions) and skips files that can't run.
    fn guard(conditions: &[Condition]) -> Option<(String, String)> {
        let tests: Vec<String> = conditions
            .iter()
            .map(|c| match c {
                Condition::Command(name) => {
                    format!("whence -p {} >/dev/null", template_word(name))
                }
                Condition::Executable(path) => format!("[[ -x {} ]]", template_word(path)),
                Condition::Exists(path) => format!("[[ -e {} ]]", template_word(path)),
            })
            .collect();
        Some((format!("if {}; then\n", tests.join(" && ")), "fi\n".into()))
    }
}

/// `'...'` with embedded single quotes closed, escaped and reopened.
pub fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// A double-quoted zsh word; template variables become `${NAME}`.
pub fn template_word(s: &str) -> String {
    let mut out = String::from("\"");
    for seg in template::parse(s) {
        match seg {
            Seg::Var(name) => write!(out, "${{{name}}}").unwrap(),
            Seg::Lit(lit) => {
                for c in lit.chars() {
                    if matches!(c, '\\' | '"' | '$' | '`') {
                        out.push('\\');
                    }
                    out.push(c);
                }
            }
        }
    }
    out.push('"');
    out
}

/// A path as a zsh word, relative to `$HOME` where possible.
fn path_word(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
        Some(rel) => template_word(&format!("~/{}", rel.replace('$', "$$"))),
        None => quote(&p.display().to_string()),
    }
}

/// `-n` parses without running; `-f` skips the user's startup files.
pub fn syntax_check(file: &str) -> Command {
    let mut c = Command::new("zsh");
    c.args(["-f", "-n", file]);
    c
}

/// The hook body for `.zshenv`, which every zsh reads, also scripts. Only
/// interactive shells rebuild, so scripts never start the binary or print
/// config errors; they use the last build. The rebuild check runs in an
/// anonymous function so its locals don't leak; `source` runs at top level
/// so the generated code isn't function-scoped.
pub fn env_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_word(p, home);
    let out = q(&paths.output("env.zsh"));
    format!(
        "() {{\n  \
         local exe={exe} cfg={cfg} out={out} rc={rc}\n  \
         if [[ -o interactive && -x $exe && ( ! -e $out || ! -e $rc || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
         $exe build --config $cfg --out-dir {dir}\n  \
         fi\n\
         }}\n\
         [[ -r {out} ]] && source {out}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        rc = q(&paths.output("init.zsh")),
        dir = q(&paths.out_dir),
    )
}

/// The hook body for `.zshrc`; `.zshenv` has already rebuilt if needed.
pub fn rc_hook(paths: &Paths, home: &Path) -> String {
    let rc = path_word(&paths.output("init.zsh"), home);
    format!("[[ -r {rc} ]] && source {rc}\n")
}

/// `.zshenv`, where zsh reads it: in `$ZDOTDIR` if set when zsh starts. If
/// your `~/.zshenv` sets ZDOTDIR, it must source `$ZDOTDIR/.zshenv` itself.
pub fn env_file() -> PathBuf {
    zdotdir().join(".zshenv")
}

pub fn rc_file() -> PathBuf {
    zdotdir().join(".zshrc")
}

fn zdotdir() -> PathBuf {
    env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::home)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{Shell, render_body};

    #[test]
    fn quoting() {
        assert_eq!(quote("it's"), r"'it'\''s'");
        assert_eq!(
            template_word(r#"~/a "b" $$c `d` \e"#),
            r#""${HOME}/a \"b\" \$c \`d\` \\e""#
        );
    }

    #[test]
    fn hook_paths() {
        let home = Path::new("/home/me");
        assert_eq!(
            path_word(Path::new("/home/me/it's $x/bin"), home),
            r#""${HOME}/it's \$x/bin""#
        );
        assert_eq!(path_word(Path::new("/opt/it's"), home), r"'/opt/it'\''s'");
    }

    /// The two generated files, each as lines without the header.
    fn render(cfg: &str) -> Vec<(&'static str, Vec<String>)> {
        render_body(cfg, Shell::Zsh)
            .into_iter()
            .map(|(name, body)| (name, body.lines().map(String::from).collect()))
            .collect()
    }

    fn expect(files: [(&'static str, &[&str]); 2]) -> Vec<(&'static str, Vec<String>)> {
        files
            .into_iter()
            .map(|(name, lines)| (name, lines.iter().map(|l| l.to_string()).collect()))
            .collect()
    }

    #[test]
    fn renders() {
        let cfg = r#"
            [env]
            GOPATH = "~/go"
            ONLY_NU = { nu = "x" }
            [path]
            prepend = ["$GOPATH/bin"]
            append = ["/opt/bin"]
            [aliases]
            g = "git"
            q = "echo 'hi'"
            [zsh]
            init = "\nsetopt autocd\n"
        "#;
        let path = r#"path=("${GOPATH}/bin" $path "/opt/bin")"#;
        assert_eq!(
            render(cfg),
            expect([
                (
                    "env.zsh",
                    &[r#"export GOPATH="${HOME}/go""#, "typeset -gU path", path]
                ),
                (
                    "init.zsh",
                    &[
                        "typeset -gU path",
                        path,
                        "alias -- g='git'",
                        r"alias -- q='echo '\''hi'\'''",
                        "",
                        "# --- init ---",
                        "setopt autocd",
                    ]
                ),
            ])
        );
    }

    #[test]
    fn renders_tools_with_startup_checks() {
        let cfg = r#"
            [aliases]
            g = "git"
            [zsh]
            init = "base init"
            [tool.eza.aliases]
            ls = "eza"
            [tool.borg]
            command = "~/bin/borg"
            exists = "/mnt/x"
            env.TMPDIR = "/tmp/b"
            aliases.b = "borg"
            [tool.nu-only.aliases]
            x = { nu = "y" }
            [tool.fzf]
            sh.init = "source fzf.zsh"
        "#;
        let borg = r#"if [[ -x "${HOME}/bin/borg" ]] && [[ -e "/mnt/x" ]]; then"#;
        assert_eq!(
            render(cfg),
            expect([
                (
                    "env.zsh",
                    &["", "# tool: borg", borg, r#"export TMPDIR="/tmp/b""#, "fi"]
                ),
                (
                    "init.zsh",
                    &[
                        "alias -- g='git'",
                        "",
                        "# tool: eza",
                        r#"if whence -p "eza" >/dev/null; then"#,
                        "alias -- ls='eza'",
                        "fi",
                        "",
                        "# tool: borg",
                        borg,
                        "alias -- b='borg'",
                        "fi",
                        "",
                        "# tool: fzf",
                        r#"if whence -p "fzf" >/dev/null; then"#,
                        "source fzf.zsh",
                        "fi",
                        "",
                        "# --- init ---",
                        "base init",
                    ]
                ),
            ])
        );
    }

    #[test]
    fn renders_sh_entries() {
        let cfg = r#"
            [env]
            PAGER = { sh = "less", nu = "bat" }
            [aliases]
            ll = { sh = "ls -l", zsh = "ls -lh" }
            [sh]
            init = "shared"
            [zsh]
            init = "zsh only"
        "#;
        assert_eq!(
            render(cfg),
            expect([
                ("env.zsh", &[r#"export PAGER="less""#]),
                (
                    "init.zsh",
                    &[
                        "alias -- ll='ls -lh'",
                        "",
                        "# --- init ---",
                        "shared",
                        "zsh only",
                    ]
                ),
            ])
        );
    }
}
