//! Windows PowerShell 5.1 (`powershell.exe`): generated code, and the hook in
//! its profile.
//!
//! The env, alias and PATH lines are PowerShell 7's (see `pwsh`), which 5.1
//! runs as they are. What differs: 5.1 runs on .NET Framework, which lacks
//! `Path.Join`, `Path.Exists` and `File.ResolveLinkTarget`; it has no `??`;
//! it reads files without a byte order mark in the ANSI code page (see
//! `Shell::bom`); and it runs on Windows only, so the generated code is the
//! same on every platform. As in `pwsh`, no cmdlets: their modules load on
//! first use, which costs about 100 ms per start.

use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::pwsh::{self, Pwsh, quote, template_word};
use super::{Dialect, home_relative};
use crate::Paths;
use crate::config::Condition;

pub struct Powershell;

/// Helpers for the `[tool]` checks, as in `pwsh` (a file can run if it has
/// an extension from PATHEXT, in its name or added to it), without
/// `Path.Join`. Paths are joined as strings rather than with `Path.Combine`
/// or `Path.GetExtension`, which on .NET Framework throw for a PATH entry
/// with a `"` in it.
const HELPERS: &str = "\
$__shellsync_executable = {
  param($path)
  foreach ($ext in $env:PATHEXT -split ';' -ne '') {
    if ([IO.File]::Exists($path + $ext) -or ($path.EndsWith($ext, [StringComparison]::OrdinalIgnoreCase) -and [IO.File]::Exists($path))) { return $true }
  }
  $false
}
$__shellsync_command = {
  param($name)
  foreach ($dir in $env:PATH -split ';') {
    if ($dir -and (& $__shellsync_executable ($dir + '\\' + $name))) { return $true }
  }
  $false
}
";

impl Dialect for Powershell {
    fn env(name: &str, value: &str) -> String {
        Pwsh::env(name, value)
    }

    fn path(prepend: &[&str], append: &[&str]) -> String {
        pwsh::path_line(prepend, append, true)
    }

    fn alias(name: &str, body: &str) -> String {
        Pwsh::alias(name, body)
    }

    fn alias_snippet(name: &str, body: &str) -> (String, bool) {
        Pwsh::alias_snippet(name, body)
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
                Condition::Exists(path) => {
                    let path = template_word(path);
                    format!("([IO.File]::Exists({path}) -or [IO.Directory]::Exists({path}))")
                }
            })
            .collect();
        Some((format!("if ({}) {{\n", tests.join(" -and ")), "}\n".into()))
    }

    fn guard_helpers() -> (&'static str, &'static str) {
        (HELPERS, pwsh::HELPERS_END)
    }
}

/// A path as a double-quoted string, relative to `$HOME` where possible.
/// Only ASCII: other characters are written as `$([char]0x...)`, so the hook
/// means the same in whatever encoding PowerShell reads the profile.
fn path_word(p: &Path, home: &Path) -> String {
    let (mut out, rest) = match home_relative(p, home) {
        Some(rel) => ("\"${HOME}/".to_string(), rel),
        None => ("\"".to_string(), p.display().to_string()),
    };
    for c in rest.chars() {
        if c.is_ascii() {
            if matches!(c, '`' | '$' | '"') {
                out.push('`');
            }
            out.push(c);
        } else {
            for unit in c.encode_utf16(&mut [0; 2]) {
                write!(out, "$([char]0x{unit:04X})").unwrap();
            }
        }
    }
    out.push('"');
    out
}

/// The hook body for the profile; see `pwsh::profile_hook`. .NET Framework
/// can't resolve a symlink, so for a config that is one (or another reparse
/// point) the hook can't compare times itself and leaves that to
/// `build --if-stale`, at the cost of starting the binary.
pub fn profile_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_word(p, home);
    let out = q(&paths.output("init-ps5.ps1"));
    format!(
        "& {{\n  \
         $exe = {exe}; $cfg = {cfg}; $out = {out}\n  \
         if ([IO.File]::Exists($exe) -and (-not [IO.File]::Exists($out) -or ([IO.File]::Exists($cfg) -and ([IO.File]::GetAttributes($cfg).HasFlag([IO.FileAttributes]::ReparsePoint) -or [IO.File]::GetLastWriteTimeUtc($cfg) -ne [IO.File]::GetLastWriteTimeUtc($out))))) {{\n    \
         & $exe build --if-stale --config $cfg --out-dir {dir}\n  \
         }}\n\
         }}\n\
         if ([IO.File]::Exists({out})) {{ . {out} }}\n",
        exe = q(exe),
        cfg = q(&paths.config),
        dir = q(&paths.out_dir),
    )
}

/// The profile for all hosts. It is in Documents, which may have moved (to
/// OneDrive, say), so Windows PowerShell is asked.
pub fn profile_file() -> PathBuf {
    ask_powershell()
        .unwrap_or_else(|| crate::home().join(r"Documents\WindowsPowerShell\profile.ps1"))
}

/// The answer comes as UTF-16 code units: Windows PowerShell writes to a
/// pipe in the console's code page, and user names needn't be ASCII.
fn ask_powershell() -> Option<PathBuf> {
    let out = Command::new("powershell")
        .args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "[int[]][char[]]$PROFILE.CurrentUserAllHosts -join ','",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let units: Vec<u16> = String::from_utf8(out.stdout)
        .ok()?
        .trim()
        .split(',')
        .filter(|s| !s.is_empty())
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    let file = String::from_utf16(&units).ok()?;
    Some(PathBuf::from(file)).filter(|f| !f.as_os_str().is_empty())
}

/// Parses with Windows PowerShell's own parser, as `pwsh::syntax_check`
/// does, without `Path.Join`.
pub fn syntax_check(exe: &Path, file: &str) -> Command {
    let mut c = Command::new(exe);
    c.args([
        "-NoProfile",
        "-NonInteractive",
        "-Command",
        &format!(
            "$errors = $null; \
             $null = [System.Management.Automation.Language.Parser]::ParseFile([IO.Path]::Combine($PWD.Path, {f}), [ref]$null, [ref]$errors); \
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
    fn hook_paths_are_ascii() {
        let home = Path::new("/home/me");
        assert_eq!(
            path_word(Path::new("/home/me/it's $x `y` \"z\"/bin"), home),
            "\"${HOME}/it's `$x ``y`` `\"z`\"/bin\""
        );
        assert_eq!(
            path_word(Path::new("/opt/Jürgen’s 🦀"), home),
            "\"/opt/J$([char]0x00FC)rgen$([char]0x2019)s $([char]0xD83E)$([char]0xDD80)\""
        );
    }

    fn render(cfg: &str) -> Vec<String> {
        let files = render_body(cfg, Shell::Powershell);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "init-ps5.ps1");
        files[0].1.lines().map(String::from).collect()
    }

    #[test]
    fn renders() {
        let cfg = r#"
            [env]
            GOPATH = "~/go"
            ONLY_PWSH = { pwsh = "x" }
            FOR_PS = { ps = "p" }
            [path]
            prepend = ["$GOPATH/bin"]
            append = ["/opt/bin"]
            [aliases]
            g = "git"
            rsync = "rsync -a"
            [ps]
            init = "both"
            [pwsh]
            init = "$x = $y ?? 1"
            [powershell]
            init = "5.1 only"
        "#;
        assert_eq!(
            render(cfg),
            [
                r#"$env:GOPATH = "${HOME}/go""#,
                r#"$env:FOR_PS = "p""#,
                // Case-insensitive, also when printed on another platform.
                r#"$env:PATH = & { $path = @(); foreach ($dir in @("${env:GOPATH}/bin") + ($env:PATH -split [IO.Path]::PathSeparator) + @("/opt/bin")) { if ($dir -and $path -inotcontains $dir) { $path += $dir } }; $path -join [IO.Path]::PathSeparator }"#,
                r"if ($ExecutionContext.SessionState.InvokeProvider.Item.Exists('Alias:\g', $false, $true)) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\g', $false, $true, $true) }",
                "function g { git @args }",
                r"if ($ExecutionContext.SessionState.InvokeProvider.Item.Exists('Alias:\rsync', $false, $true)) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\rsync', $false, $true, $true) }",
                "function rsync { & $ExecutionContext.InvokeCommand.GetCommand('rsync', 'Application') -a @args }",
                "",
                "# --- init ---",
                "both",
                "5.1 only",
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
            r"if ($ExecutionContext.SessionState.InvokeProvider.Item.Exists('Alias:\ls', $false, $true)) { $ExecutionContext.SessionState.InvokeProvider.Item.Remove([string[]]'Alias:\ls', $false, $true, $true) }",
            "function ls { eza @args }",
            "}",
            "",
            "# tool: borg",
            r#"if ((& $__shellsync_executable "${HOME}/bin/borg") -and ([IO.File]::Exists("/mnt/x") -or [IO.Directory]::Exists("/mnt/x"))) {"#,
            r#"$env:TMPDIR = "/tmp/b""#,
            "}",
        ]);
        expected.extend(pwsh::HELPERS_END.lines());
        assert_eq!(render(cfg), expected);
    }

    /// Nothing that needs PowerShell 7 or .NET Core.
    #[test]
    fn uses_nothing_newer_than_5_1() {
        let cfg = r#"
            [env]
            X = "~/x"
            [path]
            prepend = ["/a"]
            [aliases]
            g = "git"
            [tool.a]
            command = ["a", "~/bin/a"]
            exists = "/a"
        "#;
        let paths = Paths {
            config: PathBuf::from("/home/me/.config/shellsync/config.toml"),
            out_dir: PathBuf::from("/home/me/.cache/shellsync"),
        };
        let mut code = render(cfg).join("\n");
        code += &profile_hook(&paths, Path::new("/bin/shellsync"), Path::new("/home/me"));
        for newer in [
            "??",
            "Path]::Join",
            "Path]::Exists",
            "ResolveLinkTarget",
            "GetUnixFileMode",
        ] {
            assert!(!code.contains(newer), "{newer} in\n{code}");
        }
    }
}
