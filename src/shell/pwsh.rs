//! PowerShell 7 (pwsh): generated code, and the hook in the profile.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Dialect, home_relative};
use crate::Paths;
use crate::config::Condition;
use crate::template::{self, Seg};

pub struct Pwsh;

// The generated code and the hook use no cmdlets: the first use of a cmdlet
// such as `Test-Path` loads its module, which adds about 100 ms to every
// start. .NET calls and the language itself don't.

/// Helpers for the `[tool]` checks, defined before the first one and removed
/// after the last. (`Get-Command` would also take about 9 ms per lookup, and
/// doesn't check that a file can run.) The files are generated on the machine
/// that runs them, so the helpers can differ per platform: a file can run if
/// it has an execute bit, or on Windows an extension from PATHEXT (with or
/// without it in the name), as for `Probe`.
const HELPERS: &str = if cfg!(windows) {
    HELPERS_WINDOWS
} else {
    HELPERS_UNIX
};

const HELPERS_UNIX: &str = "\
$__shellsync_executable = {
  param($path)
  [IO.File]::Exists($path) -and (([int][IO.File]::GetUnixFileMode($path) -band 73) -ne 0)
}
$__shellsync_command = {
  param($name)
  foreach ($dir in $env:PATH -split [IO.Path]::PathSeparator) {
    if ($dir -and (& $__shellsync_executable ([IO.Path]::Join($dir, $name)))) { return $true }
  }
  $false
}
";

const HELPERS_WINDOWS: &str = "\
$__shellsync_executable = {
  param($path)
  $exts = $env:PATHEXT -split ';' -ne ''
  if ([IO.Path]::GetExtension($path) -in $exts -and [IO.File]::Exists($path)) { return $true }
  foreach ($ext in $exts) { if ([IO.File]::Exists($path + $ext)) { return $true } }
  $false
}
$__shellsync_command = {
  param($name)
  foreach ($dir in $env:PATH -split [IO.Path]::PathSeparator) {
    if ($dir -and (& $__shellsync_executable ([IO.Path]::Join($dir, $name)))) { return $true }
  }
  $false
}
";

const HELPERS_END: &str = "\
$ExecutionContext.SessionState.PSVariable.Remove('__shellsync_executable')
$ExecutionContext.SessionState.PSVariable.Remove('__shellsync_command')
";

impl Dialect for Pwsh {
    fn env(name: &str, value: &str) -> String {
        format!("$env:{name} = {}\n", template_word(value))
    }

    /// The first occurrence of a directory is kept, so prepended entries
    /// move to the front. Paths are case-insensitive on Windows.
    fn path(prepend: &[&str], append: &[&str]) -> String {
        let list = |l: &[&str]| {
            let words: Vec<String> = l.iter().map(|p| template_word(p)).collect();
            format!("@({})", words.join(", "))
        };
        let mut parts = Vec::new();
        if !prepend.is_empty() {
            parts.push(list(prepend));
        }
        parts.push("($env:PATH -split [IO.Path]::PathSeparator)".into());
        if !append.is_empty() {
            parts.push(list(append));
        }
        format!(
            "$env:PATH = & {{ $path = @(); foreach ($dir in {}) {{ if ($dir -and $path -{}notcontains $dir) {{ $path += $dir }} }}; $path -join [IO.Path]::PathSeparator }}\n",
            parts.join(" + "),
            if cfg!(windows) { "i" } else { "c" }
        )
    }

    /// PowerShell aliases can't take arguments, so an alias becomes a function
    /// that appends its arguments, as a zsh alias does. Aliases take
    /// precedence over functions, so a built-in alias of the same name (such
    /// as `gc`) is removed first.
    fn alias(name: &str, body: &str) -> String {
        format!(
            "if (${{alias:{name}}}) {{ $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\\{name}', $false, $true, $true) }}\n\
             function {name} {{ {} @args }}\n",
            body.trim()
        )
    }

    fn alias_snippet(name: &str, body: &str) -> (String, bool) {
        (Self::alias(name, body), true)
    }

    /// Checked at startup without starting a process, see [`HELPERS`].
    fn guard(conditions: &[Condition]) -> Option<(String, String)> {
        let tests: Vec<String> = conditions
            .iter()
            .map(|c| match c {
                Condition::Command(name) => {
                    format!("(& $__shellsync_command {})", template_word(name))
                }
                Condition::Executable(path) => {
                    format!("(& $__shellsync_executable {})", template_word(path))
                }
                Condition::Exists(path) => format!("[IO.Path]::Exists({})", template_word(path)),
            })
            .collect();
        Some((format!("if ({}) {{\n", tests.join(" -and ")), "}\n".into()))
    }

    fn guard_helpers() -> (&'static str, &'static str) {
        (HELPERS, HELPERS_END)
    }
}

/// `'...'` with embedded single quotes doubled. PowerShell also takes the
/// typographic single quotes as quotes.
pub fn quote(s: &str) -> String {
    let mut out = String::from("'");
    for c in s.chars() {
        if matches!(c, '\'' | '\u{2018}' | '\u{2019}' | '\u{201A}' | '\u{201B}') {
            out.push(c);
        }
        out.push(c);
    }
    out.push('\'');
    out
}

/// A double-quoted PowerShell string; template variables become
/// `${env:NAME}`, and the home directory PowerShell's own `${HOME}`.
pub fn template_word(s: &str) -> String {
    let mut out = String::from("\"");
    for seg in template::parse(s) {
        match seg {
            Seg::Var(name) if name == "HOME" => out.push_str("${HOME}"),
            Seg::Var(name) => write!(out, "${{env:{name}}}").unwrap(),
            Seg::Lit(lit) => {
                for c in lit.chars() {
                    // The typographic double quotes count as quotes too.
                    if matches!(c, '`' | '$' | '"' | '\u{201C}' | '\u{201D}' | '\u{201E}') {
                        out.push('`');
                    }
                    out.push(c);
                }
            }
        }
    }
    out.push('"');
    out
}

/// A path as a PowerShell string, relative to `$HOME` where possible.
fn path_word(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
        Some(rel) => template_word(&format!("~/{}", rel.replace('$', "$$"))),
        None => quote(&p.display().to_string()),
    }
}

/// The hook body for the profile. The rebuild check runs in its own scope so
/// its variables don't leak; the generated file is dot-sourced at the
/// profile's top level, so its functions and variables stay defined.
/// .NET reports a symlink's own time, so the config's link is resolved
/// first: the build stamps the outputs with the time of the file it read.
pub fn profile_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_word(p, home);
    let out = q(&paths.output("init.ps1"));
    format!(
        "& {{\n  \
         $exe = {exe}; $cfg = {cfg}; $out = {out}\n  \
         if ([IO.File]::Exists($exe) -and (-not [IO.File]::Exists($out) -or ([IO.File]::Exists($cfg) -and ([IO.File]::ResolveLinkTarget($cfg, $true) ?? [IO.FileInfo]::new($cfg)).LastWriteTimeUtc -ne [IO.File]::GetLastWriteTimeUtc($out)))) {{\n    \
         & $exe build --config $cfg --out-dir {dir}\n  \
         }}\n\
         }}\n\
         if ([IO.File]::Exists({out})) {{ . {out} }}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        dir = q(&paths.out_dir),
    )
}

/// The profile for all hosts (the terminal, editors). On Windows it is in
/// Documents, which may have moved (to OneDrive, say), so PowerShell is asked.
pub fn profile_file() -> PathBuf {
    if cfg!(windows) {
        if let Some(file) = ask_pwsh() {
            return file;
        }
        return crate::home().join(r"Documents\PowerShell\profile.ps1");
    }
    crate::xdg("XDG_CONFIG_HOME", ".config").join("powershell/profile.ps1")
}

fn ask_pwsh() -> Option<PathBuf> {
    let out = Command::new("pwsh")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "$PROFILE.CurrentUserAllHosts",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let file = String::from_utf8(out.stdout).ok()?;
    Some(PathBuf::from(file.trim())).filter(|f| !f.as_os_str().is_empty())
}

/// Parses with PowerShell's own parser and prints each error as
/// `file:line:column: message`. Most of the time is PowerShell's start.
/// (No cmdlets, see above.)
pub fn syntax_check(file: &str) -> Command {
    let mut c = Command::new("pwsh");
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "$errors = $null; \
             $null = [System.Management.Automation.Language.Parser]::ParseFile([IO.Path]::Join($PWD.Path, {f}), [ref]$null, [ref]$errors); \
             foreach ($e in $errors) {{ [Console]::Error.WriteLine(('{{0}}:{{1}}:{{2}}: {{3}}' -f {f}, $e.Extent.StartLineNumber, $e.Extent.StartColumnNumber, $e.Message)) }}; \
             exit [int]($errors.Count -gt 0)",
            f = quote(file)
        ),
    ]);
    c
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shell::{Shell, render_body};

    #[test]
    fn quoting() {
        assert_eq!(quote("it's ‘x’"), "'it''s ‘‘x’’'");
        assert_eq!(
            template_word("~/a \"b\" $$c `d` “e” $X"),
            "\"${HOME}/a `\"b`\" `$c ``d`` `“e`” ${env:X}\""
        );
    }

    #[test]
    fn hook_paths() {
        let home = Path::new("/home/me");
        assert_eq!(
            path_word(Path::new("/home/me/it's $x/bin"), home),
            "\"${HOME}/it's `$x/bin\""
        );
        assert_eq!(path_word(Path::new("/opt/it's"), home), "'/opt/it''s'");
    }

    fn render(cfg: &str) -> Vec<String> {
        let files = render_body(cfg, Shell::Pwsh);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "init.ps1");
        files[0].1.lines().map(String::from).collect()
    }

    #[test]
    fn renders() {
        let cfg = r#"
            [env]
            GOPATH = "~/go"
            ONLY_NU = { nu = "x" }
            FOR_SH = { sh = "s" }
            [path]
            prepend = ["$GOPATH/bin"]
            append = ["/opt/bin"]
            [aliases]
            g = "git"
            [pwsh]
            init = "Set-PSReadLineOption -EditMode Emacs"
        "#;
        let op = if cfg!(windows) {
            "-inotcontains"
        } else {
            "-cnotcontains"
        };
        let path = format!(
            r#"$env:PATH = & {{ $path = @(); foreach ($dir in @("${{env:GOPATH}}/bin") + ($env:PATH -split [IO.Path]::PathSeparator) + @("/opt/bin")) {{ if ($dir -and $path {op} $dir) {{ $path += $dir }} }}; $path -join [IO.Path]::PathSeparator }}"#
        );
        assert_eq!(
            render(cfg),
            [
                r#"$env:GOPATH = "${HOME}/go""#,
                &path,
                r"if (${alias:g}) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\g', $false, $true, $true) }",
                "function g { git @args }",
                "",
                "# --- init ---",
                "Set-PSReadLineOption -EditMode Emacs",
            ]
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
            env.TMPDIR = "/tmp/b"
        "#;
        let mut expected = vec![""];
        expected.extend(HELPERS.lines());
        expected.extend([
            "",
            "# tool: eza",
            r#"if ((& $__shellsync_command "eza")) {"#,
            r"if (${alias:ls}) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\ls', $false, $true, $true) }",
            "function ls { eza @args }",
            "}",
            "",
            "# tool: borg",
            r#"if ((& $__shellsync_executable "${HOME}/bin/borg") -and [IO.Path]::Exists("/mnt/x")) {"#,
            r#"$env:TMPDIR = "/tmp/b""#,
            "}",
        ]);
        expected.extend(HELPERS_END.lines());
        assert_eq!(render(cfg), expected);
    }
}
