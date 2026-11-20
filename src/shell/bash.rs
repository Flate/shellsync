//! bash: generated code, and the hook in `.bashrc`. The generated code runs
//! in bash 3.2, which macOS still ships.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::Dialect;
use super::zsh::{path_word, quote, template_word};
use crate::Paths;
use crate::config::Condition;

pub struct Bash;

/// Finds a command on PATH as zsh's `whence -p` does: an executable file.
/// bash's own `type -P` also reports files that can't run.
const HELPERS: &str = r#"__shellsync_command() {
  local dir rest
  rest=$PATH:
  while [[ -n $rest ]]; do
    dir=${rest%%:*} rest=${rest#*:}
    [[ -f ${dir:-.}/$1 && -x ${dir:-.}/$1 ]] && return 0
  done
  return 1
}
"#;

const HELPERS_END: &str = "unset -f __shellsync_command\n";

impl Dialect for Bash {
    fn env(name: &str, value: &str) -> String {
        format!("export {name}={}\n", template_word(value))
    }

    /// bash has no `typeset -U`: rebuilds PATH in place, keeping the first
    /// occurrence of each entry, so prepended entries move to the front and a
    /// nested bash leaves PATH as it is.
    fn path(prepend: &[&str], append: &[&str]) -> String {
        let mut words: Vec<String> = prepend.iter().map(|p| template_word(p)).collect();
        words.push("\"$PATH\"".into());
        words.extend(append.iter().map(|p| template_word(p)));
        format!(
            "__shellsync_rest={}: __shellsync_path=\n\
             while [[ -n $__shellsync_rest ]]; do\n  \
             __shellsync_dir=${{__shellsync_rest%%:*}} __shellsync_rest=${{__shellsync_rest#*:}}\n  \
             [[ -z $__shellsync_dir || :$__shellsync_path: == *:\"$__shellsync_dir\":* ]] || __shellsync_path=${{__shellsync_path:+$__shellsync_path:}}$__shellsync_dir\n\
             done\n\
             export PATH=$__shellsync_path\n\
             unset __shellsync_rest __shellsync_path __shellsync_dir\n",
            words.join(":")
        )
    }

    fn alias(name: &str, body: &str) -> String {
        format!("alias -- {name}={}\n", quote(body))
    }

    /// bash parses an alias's body only where the alias is used.
    fn alias_snippet(_: &str, body: &str) -> (String, bool) {
        (body.to_string(), false)
    }

    /// Builtins only, so startup spawns no process.
    fn guard(conditions: &[Condition]) -> Option<(String, String)> {
        let tests: Vec<String> = conditions
            .iter()
            .map(|c| match c {
                Condition::Command(name) => format!("__shellsync_command {}", template_word(name)),
                Condition::Executable(path) => format!("[[ -x {} ]]", template_word(path)),
                Condition::Exists(path) => format!("[[ -e {} ]]", template_word(path)),
            })
            .collect();
        Some((format!("if {}; then\n", tests.join(" && ")), "fi\n".into()))
    }

    fn guard_helpers() -> (&'static str, &'static str) {
        (HELPERS, HELPERS_END)
    }
}

/// `-n` parses without running. `extglob` is on because init code that
/// turns it on with `shopt` parses fine when sourced, line by line, but not
/// in one piece without it.
pub fn syntax_check(file: &str) -> Command {
    let mut c = Command::new("bash");
    c.args(["-O", "extglob", "-n", file]);
    c
}

/// The hook body for `.bashrc`. Only interactive shells rebuild, so `ssh
/// host command` (which bash runs with `.bashrc`) never starts the binary or
/// prints config errors. The rebuild check runs in a function so its locals
/// don't leak; `source` runs at top level so the generated code isn't
/// function-scoped.
pub fn rc_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_word(p, home);
    let out = q(&paths.output("init.bash"));
    format!(
        "__shellsync_build() {{\n  \
         local exe={exe} cfg={cfg} out={out}\n  \
         if [[ $- == *i* && -x $exe && ( ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
         \"$exe\" build --config \"$cfg\" --out-dir {dir}\n  \
         fi\n\
         }}\n\
         __shellsync_build; unset -f __shellsync_build\n\
         [[ -r {out} ]] && source {out}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        dir = q(&paths.out_dir),
    )
}

/// `.bashrc`, which interactive bash reads. Login shells read
/// `.bash_profile` (or `.profile`) instead, which usually sources it.
pub fn rc_file() -> PathBuf {
    crate::home().join(".bashrc")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{Shell, render_body};

    fn render(cfg: &str) -> String {
        let mut files = render_body(cfg, Shell::Bash);
        assert_eq!(files.len(), 1);
        files.remove(0).1
    }

    const PATH: &str = r#"__shellsync_rest="${GOPATH}/bin":"$PATH":"/opt/bin": __shellsync_path=
while [[ -n $__shellsync_rest ]]; do
  __shellsync_dir=${__shellsync_rest%%:*} __shellsync_rest=${__shellsync_rest#*:}
  [[ -z $__shellsync_dir || :$__shellsync_path: == *:"$__shellsync_dir":* ]] || __shellsync_path=${__shellsync_path:+$__shellsync_path:}$__shellsync_dir
done
export PATH=$__shellsync_path
unset __shellsync_rest __shellsync_path __shellsync_dir"#;

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
            ll = { sh = "ls -l", zsh = "ls -lh" }
            [sh]
            init = "shared"
            [bash]
            init = "shopt -s globstar"
        "#;
        assert_eq!(
            render(cfg),
            format!(
                "export GOPATH=\"${{HOME}}/go\"\n\
                 {PATH}\n\
                 alias -- g='git'\n\
                 alias -- q='echo '\\''hi'\\'''\n\
                 alias -- ll='ls -l'\n\
                 \n\
                 # --- init ---\n\
                 shared\n\
                 shopt -s globstar"
            )
        );
    }

    #[test]
    fn renders_tools_with_startup_checks() {
        let cfg = r#"
            [tool.eza.aliases]
            ls = "eza"
            [tool.borg]
            command = "~/bin/borg"
            exists = "/mnt/x"
            aliases.b = "borg"
            [tool.zsh-only]
            zsh.init = "x"
        "#;
        assert_eq!(
            render(cfg),
            format!(
                "\n{HELPERS}\
                 \n# tool: eza\n\
                 if __shellsync_command \"eza\"; then\n\
                 alias -- ls='eza'\n\
                 fi\n\
                 \n# tool: borg\n\
                 if [[ -x \"${{HOME}}/bin/borg\" ]] && [[ -e \"/mnt/x\" ]]; then\n\
                 alias -- b='borg'\n\
                 fi\n\
                 {HELPERS_END}"
            )
            .trim_end()
        );
    }
}
