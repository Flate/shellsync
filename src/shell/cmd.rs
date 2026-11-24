//! cmd.exe with clink: generated code, and the hook in clink's profile.
//!
//! cmd has no startup file of its own that could set aliases, so this needs
//! clink (1.6.11 or later, for `os.setalias`), which runs inside cmd and
//! loads every Lua script in its profile folder at the first prompt. The
//! generated file is Lua: `os.setenv` changes cmd's own environment, and
//! `os.setalias` defines doskey macros, which take their arguments as `$*`.
//! Values from the config are cmd's (alias bodies are cmd command lines);
//! `init` blocks are Lua, run by clink. Windows only, so the generated code
//! is the same on every platform.

use std::env;
use std::fmt::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::{Dialect, home_relative};
use crate::Paths;
use crate::config::Condition;
use crate::probe::Probe;
use crate::template::{self, Seg};

pub struct Cmd;

/// Helpers for the `[tool]` checks: clink has no synchronous PATH lookup.
/// A file can run if it has an extension from PATHEXT, in its name or added
/// to it, as for `Probe`. Local to the generated file, so nothing to remove.
const HELPERS: &str = r#"local function __shellsync_executable(path)
  for ext in (os.getenv("PATHEXT") or ""):gmatch("[^;]+") do
    if os.isfile(path .. ext) or (path:sub(-#ext):lower() == ext:lower() and os.isfile(path)) then return true end
  end
  return false
end
local function __shellsync_command(name)
  for dir in (os.getenv("PATH") or ""):gmatch("[^;]+") do
    if __shellsync_executable(dir .. "\\" .. name) then return true end
  end
  return false
end
"#;

/// The clink executables, for the parse check: `clink.bat`, the usual
/// command, is a batch file that picks one of them.
const CLINK_EXES: [&str; 3] = ["clink_x64.exe", "clink_arm64.exe", "clink_x86.exe"];

impl Dialect for Cmd {
    const COMMENT: &'static str = "--";

    fn env(name: &str, value: &str) -> String {
        format!("os.setenv({}, {})\n", string(name), template_expr(value))
    }

    /// Keeps the first occurrence of each directory, compared without case,
    /// so prepended entries move to the front and a reload (Ctrl-X Ctrl-R)
    /// leaves PATH as it is.
    fn path(prepend: &[&str], append: &[&str]) -> String {
        let adds = |l: &[&str]| -> String {
            l.iter()
                .map(|p| format!(" add({});", template_expr(p)))
                .collect()
        };
        format!(
            "do local path, seen = {{}}, {{}}; local function add(dir) local key = dir:lower(); if dir ~= \"\" and not seen[key] then seen[key] = true; path[#path + 1] = dir end end;{} for dir in (os.getenv(\"PATH\") or \"\"):gmatch(\"[^;]+\") do add(dir) end;{} os.setenv(\"PATH\", table.concat(path, \";\")) end\n",
            adds(prepend),
            adds(append),
        )
    }

    /// A doskey macro, which gets the arguments with `$*`. A macro doesn't
    /// expand inside itself, so a body that starts with the alias's own name
    /// (`dir = "dir /w"`) runs the command.
    fn alias(name: &str, body: &str) -> String {
        format!(
            "os.setalias({}, {})\n",
            string(name),
            string(&format!("{} $*", body.trim()))
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
                    format!("__shellsync_command({})", template_expr(name))
                }
                Condition::Executable(path) => {
                    format!("__shellsync_executable({})", template_expr(path))
                }
                Condition::Exists(path) => {
                    let path = template_expr(path);
                    format!("(os.isfile({path}) or os.isdir({path}))")
                }
            })
            .collect();
        Some((format!("if {} then\n", tests.join(" and ")), "end\n".into()))
    }

    fn guard_helpers() -> (&'static str, &'static str) {
        (HELPERS, "")
    }
}

/// A Lua string literal. Control characters are written as three-digit
/// escapes, so a digit after one can't extend it; everything else, UTF-8
/// included, as it is.
pub fn string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii_control() => write!(out, "\\{:03}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// A Lua expression for a template value: variables are read when clink
/// starts, an unset one as empty, as in the other shells. The home directory
/// is the user profile, as in PowerShell.
fn template_expr(s: &str) -> String {
    let parts: Vec<String> = template::parse(s)
        .into_iter()
        .map(|seg| match seg {
            Seg::Var(name) if name == "HOME" => home_expr().into(),
            Seg::Var(name) => format!("(os.getenv({}) or \"\")", string(&name)),
            Seg::Lit(lit) => string(&lit),
        })
        .collect();
    match parts.len() {
        0 => "\"\"".into(),
        _ => parts.join(" .. "),
    }
}

fn home_expr() -> &'static str {
    "(os.getenv(\"USERPROFILE\") or \"\")"
}

/// A path as a Lua expression, relative to the user profile where possible.
fn path_expr(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
        Some(rel) => format!(
            "{} .. {}",
            home_expr(),
            string(&format!("\\{rel}").replace('/', "\\"))
        ),
        None => string(&p.display().to_string()),
    }
}

/// The hook body for `shellsync.lua` in clink's profile folder.
///
/// clink loads scripts at the first prompt of a cmd session, again after a
/// reload, and also in `clink set`, so the script's top level only loads the
/// generated file, which starts no process. The rebuild check runs once per
/// load, when the first prompt is about to show (`onbeginedit`). If the
/// build wrote a new file, `clink.reload()` loads the scripts again before
/// that prompt; a build that writes nothing (a config error, or nothing to
/// do) can't cause another reload. clink reports file times in whole seconds,
/// and a symlink's own time, so for a symlinked config `build --if-stale`
/// decides. `os.execute` runs the command with `cmd /c`, which removes the
/// outer quotes. Without `os.setalias` (clink before 1.6.11) nothing runs.
pub fn script_hook(paths: &Paths, exe: &Path, home: &Path) -> String {
    let q = |p: &Path| path_expr(p, home);
    format!(
        "if os.setalias then\n  \
         local exe, cfg, dir, out = {exe}, {cfg}, {dir}, {out}\n  \
         local function info(file) return os.globfiles(file, 2, {{ system = true }})[1] end\n  \
         local built = info(out)\n  \
         if built then\n    \
         local ok, err = pcall(dofile, out)\n    \
         if not ok then print(\"shellsync: \" .. tostring(err)) end\n  \
         end\n  \
         local checked = false\n  \
         clink.onbeginedit(function()\n    \
         if checked then return end\n    \
         checked = true\n    \
         if not os.isfile(exe) then return end\n    \
         local source = info(cfg)\n    \
         if built and not (source and (source.type:find(\"link\") or source.mtime ~= built.mtime)) then return end\n    \
         os.execute('\"\"' .. exe .. '\" build --if-stale --config \"' .. cfg .. '\" --out-dir \"' .. dir .. '\"\"')\n    \
         local now = info(out)\n    \
         if now and not (built and now.mtime == built.mtime) then clink.reload() end\n  \
         end)\n\
         end\n",
        exe = q(exe),
        cfg = q(&paths.config),
        dir = q(&paths.out_dir),
        out = q(&paths.output("init.lua")),
    )
}

/// The script in clink's profile folder: `%CLINK_PROFILE%` if set, else
/// clink's default, `%LOCALAPPDATA%\clink`.
pub fn script_file() -> PathBuf {
    let set = |var: &str| {
        env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };
    set("CLINK_PROFILE")
        .unwrap_or_else(|| {
            set("LOCALAPPDATA")
                .unwrap_or_else(|| crate::home().join(r"AppData\Local"))
                .join("clink")
        })
        .join("shellsync.lua")
}

/// The clink executable for parsing: the one whose session this is (clink
/// sets `=clink.bin` in cmd, and a rebuild from the hook inherits it), else
/// one on PATH, else the installer's default folders. The installer doesn't
/// put clink on PATH.
pub fn find_clink(probe: &Probe) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = env::var_os("=clink.bin")
        .map(PathBuf::from)
        .into_iter()
        .collect();
    dirs.extend(probe.path_dirs());
    for var in ["ProgramFiles(x86)", "ProgramFiles"] {
        dirs.extend(env::var_os(var).map(|d| PathBuf::from(d).join("clink")));
    }
    dirs.iter()
        .flat_map(|dir| CLINK_EXES.map(|exe| dir.join(exe)))
        .find(|exe| exe.is_file())
}

/// Parses with clink's Lua (5.2) without running the file. `clink lua` is
/// clink's interpreter, from 1.4.13 on.
pub fn syntax_check(exe: &Path, file: &str) -> Command {
    let mut c = Command::new(exe);
    c.args([
        "lua",
        "-e",
        &format!(
            "local f, e = loadfile({}) if not f then io.stderr:write(e, '\\n') os.exit(1) end",
            string(file)
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
        assert_eq!(string("a \"b\" \\c\n1\t"), r#""a \"b\" \\c\0101\009""#);
        assert_eq!(string("Jürgen’s"), "\"Jürgen’s\"");
        assert_eq!(
            template_expr("~/go/$GOPATH/$$x"),
            r#"(os.getenv("USERPROFILE") or "") .. "/go/" .. (os.getenv("GOPATH") or "") .. "/$x""#
        );
        assert_eq!(template_expr(""), "\"\"");
    }

    #[test]
    fn hook_paths() {
        let home = Path::new("/home/me");
        assert_eq!(
            path_expr(Path::new("/home/me/.cache/it's \"x\""), home),
            r#"(os.getenv("USERPROFILE") or "") .. "\\.cache\\it's \"x\"""#
        );
        assert_eq!(path_expr(Path::new("/opt/bin"), home), r#""/opt/bin""#);
    }

    fn render(cfg: &str) -> Vec<String> {
        let files = render_body(cfg, Shell::Cmd);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].0, "init.lua");
        files[0].1.lines().map(String::from).collect()
    }

    #[test]
    fn renders() {
        let cfg = r#"
            [env]
            GOPATH = "~/go"
            ONLY_PWSH = { pwsh = "x" }
            FOR_SH = { sh = "s" }
            [path]
            prepend = ["$GOPATH/bin"]
            append = ['C:\tools']
            [aliases]
            g = "git"
            ll = { cmd = "dir /w", sh = "ls -l" }
            [cmd]
            init = "clink.promptfilter(1)"
            [ps]
            init = "not lua"
        "#;
        assert_eq!(
            render(cfg),
            [
                r#"os.setenv("GOPATH", (os.getenv("USERPROFILE") or "") .. "/go")"#,
                r#"do local path, seen = {}, {}; local function add(dir) local key = dir:lower(); if dir ~= "" and not seen[key] then seen[key] = true; path[#path + 1] = dir end end; add((os.getenv("GOPATH") or "") .. "/bin"); for dir in (os.getenv("PATH") or ""):gmatch("[^;]+") do add(dir) end; add("C:\\tools"); os.setenv("PATH", table.concat(path, ";")) end"#,
                r#"os.setalias("g", "git $*")"#,
                r#"os.setalias("ll", "dir /w $*")"#,
                "",
                "-- --- init ---",
                "clink.promptfilter(1)",
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
            "-- tool: eza",
            r#"if __shellsync_command("eza") then"#,
            r#"os.setalias("ls", "eza $*")"#,
            "end",
            "",
            "-- tool: borg",
            r#"if __shellsync_executable((os.getenv("USERPROFILE") or "") .. "/bin/borg") and (os.isfile("/mnt/x") or os.isdir("/mnt/x")) then"#,
            r#"os.setenv("TMPDIR", "/tmp/b")"#,
            "end",
        ]);
        assert_eq!(render(cfg), expected);
    }

    #[test]
    fn header_is_lua() {
        let cfg = crate::config::Config::parse("").unwrap();
        let out = crate::shell::render(&cfg, Shell::Cmd, "test", &crate::shell::Checks::Found(&[]));
        assert!(
            out.files[0].1.lines().all(|l| l.starts_with("-- ")),
            "{}",
            out.files[0].1
        );
    }
}
