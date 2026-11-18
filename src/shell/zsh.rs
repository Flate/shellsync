//! zsh: generated code, and the hook in `.zshrc`.

use std::env;
use std::fmt::Write;
use std::path::{Path, PathBuf};

use super::{Dialect, Shell, home_relative};
use crate::Paths;
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

/// The hook body for `.zshrc`. The rebuild check runs in an anonymous
/// function so its locals don't leak; `source` runs at top level so the
/// generated code isn't function-scoped.
pub fn rc_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_word(p, home);
    let out = q(&paths.output(Shell::Zsh));
    format!(
        "() {{\n  \
         local exe={exe} cfg={cfg} out={out}\n  \
         if [[ -x $exe && ( ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
         $exe build --config $cfg --out-dir {dir}\n  \
         fi\n\
         }}\n\
         [[ -r {out} ]] && source {out}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        dir = q(&paths.out_dir),
    )
}

pub fn rc_file() -> PathBuf {
    env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(crate::home)
        .join(".zshrc")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::render_body;

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
        assert_eq!(
            render_body(cfg, Shell::Zsh),
            [
                r#"export GOPATH="${HOME}/go""#,
                "typeset -gU path",
                r#"path=("${GOPATH}/bin" $path "/opt/bin")"#,
                "alias -- g='git'",
                r"alias -- q='echo '\''hi'\'''",
                "",
                "# --- init ---",
                "setopt autocd",
            ]
            .join("\n")
        );
    }
}
