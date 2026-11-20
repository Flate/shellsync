//! bash: generated code, and the hook in `.bashrc`. The generated code runs
//! in bash 3.2, which macOS still ships, and on Windows in Git Bash, MSYS2
//! and Cygwin.

use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::zsh::{quote, template_word};
use super::{Dialect, home_relative};
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
pub fn syntax_check(exe: &Path, file: &str) -> Command {
    let mut c = Command::new(exe);
    c.args(["-O", "extglob", "-n", file]);
    c
}

/// A path as a bash word in the hook, relative to the home directory where
/// possible. On Windows that is the user profile, where shellsync's files
/// are, while bash's `HOME` may be elsewhere (MSYS2 and Cygwin have their
/// own); bash and the programs it starts take Windows paths as they are,
/// which Cygwin would not convert from its own form. They are written with
/// `/`: bash looks up a command name without `/` on PATH.
fn path_word(p: &Path, home: &Path) -> String {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    match home_relative(p, home) {
        Some(rel) => template_word(&format!("${var}/{}", rel.replace('$', "$$"))),
        None if cfg!(windows) => quote(&p.display().to_string().replace('\\', "/")),
        None => quote(&p.display().to_string()),
    }
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
///
/// On Windows, bash's home: MSYS2 and Cygwin have their own, which they pass
/// to the programs they start as `HOME`; Git Bash uses the user profile.
pub fn rc_file() -> PathBuf {
    let home = match env::var_os("HOME") {
        Some(home) if cfg!(windows) => PathBuf::from(home),
        _ => crate::home(),
    };
    home.join(".bashrc")
}

/// Whether `exe` is WSL's `bash.exe` (in the Windows folder, or an app alias
/// in WindowsApps), which would parse in Linux, if WSL is set up at all.
pub fn is_wsl_launcher(exe: &Path) -> bool {
    let windir = env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
    cfg!(windows) && wsl_launcher_in(exe, &windir)
}

fn wsl_launcher_in(exe: &Path, windir: &str) -> bool {
    let exe = exe.to_string_lossy().to_lowercase().replace('/', "\\");
    let windir = windir.to_lowercase().replace('/', "\\");
    exe.starts_with(&format!("{}\\", windir.trim_end_matches('\\')))
        || exe.contains("\\windowsapps\\")
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

    /// shellsync's files are in the user profile, which bash's HOME may not
    /// be (MSYS2, Cygwin); Windows paths work in bash and as arguments.
    #[cfg(windows)]
    #[test]
    fn hook_paths_use_the_user_profile() {
        let home = Path::new(r"C:\Users\me");
        let paths = Paths {
            config: home.join(r".config\shellsync\config.toml"),
            out_dir: home.join(r".cache\shellsync"),
        };
        let exe = Path::new(r"C:\Program Files\shellsync\shellsync.exe");
        let hook = rc_hook(&paths, exe, home);
        assert!(
            hook.contains(r#"cfg="${USERPROFILE}/.config/shellsync/config.toml" "#),
            "{hook}"
        );
        // A command name without `/` is looked up on PATH, so not `C:\...`.
        assert!(
            hook.contains("local exe='C:/Program Files/shellsync/shellsync.exe' "),
            "{hook}"
        );
    }

    #[test]
    fn finds_the_wsl_launcher() {
        let windir = r"C:\WINDOWS";
        let is = |exe: &str| wsl_launcher_in(Path::new(exe), windir);
        assert!(is(r"C:\Windows\System32\bash.exe"));
        assert!(is(r"c:/windows/system32/bash.exe"));
        assert!(is(
            r"C:\Users\me\AppData\Local\Microsoft\WindowsApps\bash.exe"
        ));
        assert!(!is(r"C:\Program Files\Git\bin\bash.exe"));
        assert!(!is(r"C:\msys64\usr\bin\bash.exe"));
        assert!(!is(r"C:\Windows2\bash.exe"));
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
