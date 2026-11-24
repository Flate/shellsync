//! End-to-end tests: hook shellsync into throwaway startup files and start
//! real shells. A test is skipped (with a note) when its shell is not
//! installed, unless SHELLSYNC_REQUIRE_SHELLS is set (as in CI), where a
//! missing shell fails. zsh isn't a Windows shell; there, bash means Git
//! Bash, MSYS2 and Cygwin at their default locations (see
//! `windows_bash_end_to_end`), and plain `zsh` and `bash` are skipped. cmd
//! means cmd with clink, on Windows only; see [`CLINK_START`].
//!
//! Nothing is written outside each test's [`Sandbox`] directory: a test that
//! runs `install` first checks with `shellsync paths` where it would write.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, SystemTime};

const BIN: &str = env!("CARGO_BIN_EXE_shellsync");

/// PATH's separator.
const SEP: &str = if cfg!(windows) { ";" } else { ":" };

/// bash on Windows: Git Bash, MSYS2 and Cygwin, where their installers put
/// them.
#[cfg_attr(not(windows), allow(dead_code))]
const WINDOWS_BASHES: [&str; 3] = [
    r"C:\Program Files\Git\bin\bash.exe",
    r"C:\msys64\usr\bin\bash.exe",
    r"C:\cygwin64\bin\bash.exe",
];

/// What clink does when cmd starts, as far as its Lua interpreter, `clink
/// lua`, which the tests run instead (clink only runs in an interactive
/// console), lacks it: load the scripts (here the hook, `{hook}`), then send
/// `onbeginedit`, and after `clink.reload()` load the scripts again.
const CLINK_START: &str = r#"local reloads = 0
clink.onbeginedit = function(f) f() end
clink.reload = function() reloads = reloads + 1 end
dofile([[{hook}]])
if reloads > 0 then dofile([[{hook}]]) end
if reloads > 1 then error("reloaded again") end
"#;

/// clink's executable, where its installer puts it.
fn clink() -> Option<PathBuf> {
    ["ProgramFiles(x86)", "ProgramFiles"]
        .iter()
        .filter_map(std::env::var_os)
        .map(|dir| Path::new(&dir).join(r"clink\clink_x64.exe"))
        .find(|exe| exe.is_file())
}

/// Code that prints `started`.
fn echo_started(shell: &str) -> &'static str {
    match shell {
        "cmd" => "print('started')",
        _ => "echo started",
    }
}

/// A command that is on PATH on every machine.
const ON_PATH: &str = if cfg!(windows) { "where" } else { "ls" };

fn config() -> String {
    format!(
        r#"
[env]
GOPATH = "~/go"
TRICKY = "it's \"q\" $$x `y` (z) {{w}}"
ONLY_ZSH = {{ zsh = "z" }}
FOR_SH = {{ sh = "s" }}
FOR_PS = {{ ps = "p" }}

[path]
prepend = ["$GOPATH/bin", "/usr/bin"]
append = ["/opt/shellsync-test"]

[aliases]
greet = "echo hello"
ll = {{ zsh = "ls -lh", nu = "ls -l" }}
over = "echo base"
".." = "cd .."
# Calls the program of the same name.
whoami = {{ pwsh = "whoami" }}

# On PATH everywhere, so this applies and overrides the base alias.
[tool.{ON_PATH}.aliases]
over = "echo tool"

[tool.shellsync-no-such-tool.aliases]
missing_tool = "echo nope"

[tool.root]
exists = "/"
env.FROM_TOOL = "yes"

[tool.nowhere]
exists = "/shellsync/does/not/exist"
env.NOT_SET = "no"
"#
    )
}

fn has(shell: &str) -> bool {
    if cfg!(windows) && matches!(shell, "zsh" | "bash") {
        return false;
    }
    // Windows PowerShell 5.1, and cmd with clink.
    if !cfg!(windows) && matches!(shell, "powershell" | "cmd") {
        return false;
    }
    let found = match shell {
        "cmd" => clink().is_some(),
        _ => Command::new(shell)
            .arg("-c")
            .arg("exit 0")
            .output()
            .is_ok_and(|o| o.status.success()),
    };
    if !found && std::env::var_os("SHELLSYNC_REQUIRE_SHELLS").is_some() {
        panic!("{shell} not installed, but SHELLSYNC_REQUIRE_SHELLS is set");
    }
    found
}

/// Whether `shell` (a name or a path) is bash.
fn is_bash(shell: &str) -> bool {
    Path::new(shell).file_stem().is_some_and(|s| s == "bash")
}

fn run(mut c: Command) -> String {
    let out = c.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "failed: {stderr}");
    String::from_utf8(out.stdout).unwrap()
}

/// A throwaway directory for one test, holding shellsync's files and the
/// shells' startup files.
///
/// On Unix it is a fake HOME and hooks are added with `install`, as a user
/// would. On Windows nushell takes the home directory from the system, not
/// the environment, so the real home stays: SHELLSYNC_CONFIG and
/// XDG_CACHE_HOME point shellsync into the directory, and the hook blocks
/// (from `shellsync hook`) go into startup files there, which the shells are
/// started with.
struct Sandbox {
    dir: PathBuf,
    /// What `~` means to the shells.
    home: PathBuf,
}

impl Sandbox {
    fn new() -> Sandbox {
        static N: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "shellsync-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        let home = if cfg!(windows) {
            PathBuf::from(std::env::var_os("USERPROFILE").unwrap())
        } else {
            dir.clone()
        };
        Sandbox { dir, home }
    }

    /// A sandbox with shellsync hooked in and [`config`] in place.
    fn installed() -> Sandbox {
        let sb = Sandbox::new();
        sb.install(BIN);
        sb.edit_config(&config());
        sb
    }

    fn config(&self) -> PathBuf {
        if cfg!(windows) {
            self.dir.join("shellsync.toml")
        } else {
            self.dir.join(".config/shellsync/config.toml")
        }
    }

    /// Writes the config as an edit would: at least a second after the
    /// version before. bash before 5 compares file times in whole seconds,
    /// so it would miss a quicker edit, and the tests edit faster than people.
    fn edit_config(&self, text: &str) {
        let config = self.config();
        let before = fs::metadata(&config).and_then(|m| m.modified()).ok();
        fs::create_dir_all(config.parent().unwrap()).unwrap();
        fs::write(&config, text).unwrap();
        if let Some(before) = before {
            let at = SystemTime::now().max(before + Duration::from_secs(1));
            let file = fs::File::options().write(true).open(&config).unwrap();
            file.set_modified(at).unwrap();
        }
    }

    fn out_dir(&self) -> PathBuf {
        self.dir.join(".cache/shellsync")
    }

    /// `program` with the sandbox's environment.
    fn cmd(&self, program: &str) -> Command {
        let mut c = Command::new(program);
        if cfg!(windows) {
            // Windows programs need much of the environment to run at all.
            // HOME is bash's home (nushell and PowerShell don't read it).
            c.env("HOME", &self.dir)
                .env("SHELLSYNC_CONFIG", self.config())
                .env("XDG_CACHE_HOME", self.dir.join(".cache"))
                // nushell keeps its own files there.
                .env("XDG_CONFIG_HOME", self.dir.join(".config"))
                // clink's profile folder, which `install` hooks if it exists.
                .env("CLINK_PROFILE", self.dir.join("clink"));
        } else {
            c.env_clear()
                .env("HOME", &self.dir)
                .env("PATH", std::env::var("PATH").unwrap_or_default())
                .env("TERM", "dumb");
        }
        c
    }

    /// Hooks `exe` in; see [`Sandbox`].
    fn install(&self, exe: &str) {
        if cfg!(windows) {
            // As `install` creates a config before it builds, so the next
            // edit is dated later than the build: clink compares whole seconds.
            if !self.config().exists() {
                self.edit_config("");
            }
            run({
                let mut c = self.cmd(BIN);
                c.arg("build");
                c
            });
            for (hook, file) in self.hook_files() {
                let block = run({
                    let mut c = self.cmd(BIN);
                    c.args(["hook", hook, "--exe", exe]);
                    c
                });
                fs::create_dir_all(file.parent().unwrap()).unwrap();
                fs::write(file, block).unwrap();
            }
        } else {
            self.assert_paths_inside();
            run({
                let mut c = self.cmd(exe);
                c.arg("install");
                c
            });
        }
    }

    /// Where the Windows sandbox keeps the hooks.
    fn hook_files(&self) -> [(&'static str, PathBuf); 6] {
        [
            ("nu-env", self.dir.join("nu/env.nu")),
            ("nu-config", self.dir.join("nu/config.nu")),
            ("pwsh", self.dir.join("pwsh/profile.ps1")),
            ("bashrc", self.dir.join(".bashrc")),
            ("clink", self.dir.join("clink/shellsync.lua")),
            ("powershell", self.dir.join("powershell/profile.ps1")),
        ]
    }

    /// Fails unless every file `shellsync paths` names is in the sandbox.
    fn assert_paths_inside(&self) {
        let paths = run({
            let mut c = self.cmd(BIN);
            c.arg("paths");
            c
        });
        self.assert_inside(&paths);
    }

    /// Fails unless every file in `paths` (`shellsync paths` output) is in
    /// the sandbox. Returns them by name.
    fn assert_inside(&self, paths: &str) -> Vec<(String, PathBuf)> {
        let dir = self.dir.canonicalize().unwrap();
        paths
            .lines()
            .map(|line| {
                let (name, path) = line.split_at(13);
                let path = PathBuf::from(path);
                let existing = path.ancestors().find(|p| p.exists()).unwrap();
                assert!(
                    existing.canonicalize().unwrap().starts_with(&dir),
                    "{line} is outside the sandbox"
                );
                (name.trim().to_string(), path)
            })
            .collect()
    }

    /// nushell's env.nu and config.nu. On Unix as nushell itself reports them,
    /// since they differ by platform (~/Library/Application Support on macOS).
    fn nu_files(&self) -> (PathBuf, PathBuf) {
        if cfg!(windows) {
            let [(_, env), (_, config), ..] = self.hook_files();
            return (env, config);
        }
        let mut c = self.cmd("nu");
        c.args([
            "-n",
            "-c",
            "[$nu.env-path $nu.config-path] | str join (char nl)",
        ]);
        let out = run(c);
        let mut lines = out.lines();
        (
            PathBuf::from(lines.next().unwrap()),
            PathBuf::from(lines.next().unwrap()),
        )
    }

    /// The shell, started the way a terminal starts it, running `code`.
    fn shell_cmd(&self, shell: &str, code: &str) -> Command {
        let program = match shell {
            "cmd" => clink().unwrap().display().to_string(),
            _ => shell.to_string(),
        };
        let mut c = self.cmd(&program);
        match shell {
            "cmd" => {
                let [.., (_, hook), _] = self.hook_files();
                let start = self.dir.join("clink-start.lua");
                let script = CLINK_START.replace("{hook}", &hook.display().to_string());
                fs::write(&start, format!("{script}{code}\n")).unwrap();
                c.arg("lua").arg(start);
            }
            "nu" => {
                let (env_nu, config_nu) = self.nu_files();
                c.arg("--env-config").arg(env_nu);
                c.arg("--config").arg(config_nu);
                c.args(["-c", code]);
            }
            "pwsh" if cfg!(windows) => {
                let [_, _, (_, profile), ..] = self.hook_files();
                let code = format!(". '{}'; {code}", profile.display());
                c.args(["-NoProfile", "-NonInteractive", "-Command", &code]);
            }
            "powershell" => {
                let [.., (_, profile)] = self.hook_files();
                let code = format!(". '{}'; {code}", profile.display());
                // Windows PowerShell runs no scripts by default; users with a
                // profile have allowed local ones, usually like this.
                c.args(["-ExecutionPolicy", "RemoteSigned"]);
                c.args(["-NoProfile", "-NonInteractive", "-Command", &code]);
            }
            "pwsh" => {
                c.args(["-NonInteractive", "-Command", code]);
            }
            _ => {
                c.args(["-i", "-c", code]);
            }
        }
        c
    }

    /// Runs `code` in the shell; returns (stdout, stderr).
    fn shell(&self, shell: &str, code: &str) -> (String, String) {
        let out = self.shell_cmd(shell, code).output().unwrap();
        // Interactive bash without a terminal says so.
        let stderr = String::from_utf8_lossy(&out.stderr)
            .lines()
            .filter(|l| {
                !(is_bash(shell)
                    && (l.contains("no job control") || l.contains("terminal process group")))
            })
            .map(|l| format!("{l}\n"))
            .collect();
        (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            stderr,
        )
    }
}

impl Drop for Sandbox {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
fn zsh_end_to_end() {
    if !has("zsh") {
        eprintln!("zsh not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    let out = run(sb.shell_cmd(
        "zsh",
        r#"print -r -- $GOPATH; print -r -- $TRICKY; print -r -- $ONLY_ZSH; print -r -- $PATH; alias greet; alias ll; print -r -- $FOR_SH; alias over; print -r -- ${+aliases[missing_tool]}; print -r -- ${FROM_TOOL:-unset} ${NOT_SET:-unset}"#,
    ));
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "z");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin:/usr/bin:")),
        "{}",
        lines[3]
    );
    assert!(lines[3].ends_with(":/opt/shellsync-test"), "{}", lines[3]);
    assert_eq!(
        lines[3].matches(":/usr/bin:").count() + lines[3].matches(":/usr/bin\n").count(),
        1,
        "deduplicated"
    );
    assert_eq!(lines[4], "greet='echo hello'");
    assert_eq!(lines[5], "ll='ls -lh'");
    assert_eq!(lines[6], "s");
    assert_eq!(lines[7], "over='echo tool'");
    assert_eq!(lines[8], "0", "missing tool's alias is not defined");
    assert_eq!(lines[9], "yes unset");
}

/// `.zshenv` gives every zsh the environment, also scripts. Scripts neither
/// rebuild nor print config errors, and don't get aliases.
#[test]
fn zsh_scripts_get_the_environment_only() {
    if !has("zsh") {
        eprintln!("zsh not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    let env_zsh = sb.out_dir().join("env.zsh");
    let script = |code: &str| {
        let mut c = sb.cmd("zsh");
        c.args(["-c", code]);
        let out = c.output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            String::from_utf8_lossy(&out.stderr).to_string(),
        )
    };
    let probe = "print -r -- ${GOPATH:-unset}; alias greet || print no-alias";

    // The config was replaced after install: stale, but a script uses the
    // last build as it is.
    let built = fs::metadata(&env_zsh).unwrap().modified().unwrap();
    assert_eq!(script(probe), ("unset\nno-alias".into(), String::new()));
    assert_eq!(fs::metadata(&env_zsh).unwrap().modified().unwrap(), built);

    // An interactive shell rebuilds; scripts then get the new environment.
    assert!(sb.shell("zsh", "alias greet").0.contains("echo hello"));
    let gopath = format!("{}/go", sb.home.display());
    assert_eq!(
        script(probe),
        (format!("{gopath}\nno-alias"), String::new())
    );

    fs::write(sb.config(), "[env\n").unwrap();
    assert_eq!(
        script(probe),
        (format!("{gopath}\nno-alias"), String::new())
    );
}

/// macOS login shells run path_helper from /etc/zprofile, after `.zshenv`,
/// which moves PATH entries set there to the end. `.zshrc` sets them again.
#[test]
fn zsh_login_shells_keep_path_order() {
    if !has("zsh") {
        eprintln!("zsh not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    fs::write(
        sb.dir.join(".zprofile"),
        "export PATH=\"/usr/bin:/bin:$PATH\"\n",
    )
    .unwrap();
    let mut c = sb.cmd("zsh");
    c.args(["-l", "-i", "-c", "print -r -- $PATH"]);
    let path = run(c);
    assert!(
        path.starts_with(&format!("{}/go/bin:/usr/bin:", sb.home.display())),
        "{path}"
    );
}

#[test]
fn bash_end_to_end() {
    if !has("bash") {
        eprintln!("bash not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    let (out, err) = sb.shell(
        "bash",
        r#"printf '%s\n' "$GOPATH" "$TRICKY" "${ONLY_ZSH-unset}" "$PATH"; alias greet; alias ll 2>/dev/null || echo no-ll; echo "$FOR_SH"; alias over; alias missing_tool 2>/dev/null || echo no-missing-tool; echo "${FROM_TOOL:-unset} ${NOT_SET:-unset}"; [[ $("$BASH" -i -c 'echo "$PATH"' 2>/dev/null) == "$PATH" ]] && echo same-path"#,
    );
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "unset", "zsh only");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin:/usr/bin:")),
        "{}",
        lines[3]
    );
    assert!(lines[3].ends_with(":/opt/shellsync-test"), "{}", lines[3]);
    assert_eq!(
        format!(":{}:", lines[3]).matches(":/usr/bin:").count(),
        1,
        "deduplicated"
    );
    assert_eq!(lines[4], "alias greet='echo hello'");
    assert_eq!(lines[5], "no-ll", "no bash value");
    assert_eq!(lines[6], "s");
    assert_eq!(lines[7], "alias over='echo tool'");
    assert_eq!(lines[8], "no-missing-tool");
    assert_eq!(lines[9], "yes unset");
    assert_eq!(lines[10], "same-path", "a nested bash keeps PATH as it is");
}

/// bash on Windows, in Git Bash, MSYS2 and Cygwin: the environment as for
/// bash elsewhere, `[tool]` commands found without `.exe`, rebuilds from
/// bash, which runs the native binary with paths it must take as they are
/// (Cygwin doesn't convert arguments), and a quiet start without the binary.
/// bash's HOME is the sandbox, as `/c/...` or `/cygdrive/c/...`, so results
/// are compared with what bash says it is.
#[cfg(windows)]
#[test]
fn windows_bash_end_to_end() {
    for bash in WINDOWS_BASHES {
        if !has(bash) {
            eprintln!("{bash} not installed, skipping");
            continue;
        }
        let sb = Sandbox::installed();
        // As in CI, where the binary isn't in the user profile: the hook's
        // paths are all absolute.
        let block = run({
            let mut c = sb.cmd(BIN);
            c.env("USERPROFILE", sb.dir.join("elsewhere"))
                .args(["hook", "bashrc"]);
            c
        });
        assert!(!block.contains("USERPROFILE"), "{block}");
        fs::write(sb.dir.join(".bashrc"), block).unwrap();
        let (out, err) = sb.shell(
            bash,
            r#"printf '%s\n' "$HOME" "$GOPATH" "$TRICKY" "$PATH"; alias greet; alias over; alias missing_tool 2>/dev/null || echo no-missing-tool; echo "${FROM_TOOL:-unset} ${NOT_SET:-unset}"; [[ $("$BASH" -i -c 'echo "$PATH"' 2>/dev/null) == "$PATH" ]] && echo same-path"#,
        );
        assert_eq!(err, "", "{bash}");
        let lines: Vec<&str> = out.lines().collect();
        assert_eq!(lines.len(), 9, "{bash}: {out}");
        let h = lines[0];
        assert_eq!(lines[1], format!("{h}/go"), "{bash}");
        assert_eq!(lines[2], "it's \"q\" $x `y` (z) {w}", "{bash}");
        assert!(
            lines[3].starts_with(&format!("{h}/go/bin:/usr/bin:")),
            "{bash}: {}",
            lines[3]
        );
        assert!(
            lines[3].ends_with(":/opt/shellsync-test"),
            "{bash}: {}",
            lines[3]
        );
        assert_eq!(
            format!(":{}:", lines[3]).matches(":/usr/bin:").count(),
            1,
            "{bash}: deduplicated"
        );
        assert_eq!(lines[4], "alias greet='echo hello'", "{bash}");
        assert_eq!(
            lines[5], "alias over='echo tool'",
            "{bash}: `where` is found as where.exe"
        );
        assert_eq!(lines[6], "no-missing-tool", "{bash}");
        assert_eq!(lines[7], "yes unset", "{bash}");
        assert_eq!(lines[8], "same-path", "{bash}");

        // Edits, and errors that keep the last good build.
        let print = |sb: &Sandbox| sb.shell(bash, r#"echo "${GOPATH#$HOME}""#);
        sb.edit_config("[env]\nGOPATH = '~/elsewhere'\n");
        assert_eq!(print(&sb).0, "/elsewhere", "{bash}");
        sb.edit_config("[env\n");
        let (out, err) = print(&sb);
        assert_eq!(out, "/elsewhere", "{bash}");
        assert!(err.contains("shellsync.toml"), "{bash}: {err}");
        sb.edit_config("[env]\nGOPATH = '~/fixed'\n");
        assert_eq!(print(&sb).0, "/fixed", "{bash}");

        // Without the binary and the generated files, bash starts quietly.
        let sb = Sandbox::new();
        let exe = sb.dir.join("bin/shellsync.exe");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(BIN, &exe).unwrap();
        sb.install(exe.to_str().unwrap());
        fs::remove_file(&exe).unwrap();
        fs::remove_dir_all(sb.out_dir()).unwrap();
        let (out, err) = sb.shell(bash, "echo started");
        assert_eq!(out, "started", "{bash}: {err}");
        assert_eq!(err, "", "{bash}");
    }
}

/// A `[tool]` command must be a file that can run, as at build time: a file
/// of that name on PATH without the executable bit doesn't count.
#[cfg(unix)]
#[test]
fn tools_need_an_executable_file() {
    use std::os::unix::fs::PermissionsExt;

    for sh in ["zsh", "bash"] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        let sb = Sandbox::new();
        sb.install(BIN);
        sb.edit_config("[tool.shellsync-fake-tool.aliases]\nfake = \"shellsync-fake-tool\"\n");
        let bin = sb.dir.join("bin");
        let tool = bin.join("shellsync-fake-tool");
        fs::create_dir_all(&bin).unwrap();
        fs::write(&tool, "#!/bin/sh\n").unwrap();
        let path = format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        );
        let has_alias = || {
            let mut c = sb.shell_cmd(sh, "alias fake >/dev/null 2>&1 && echo yes || echo no");
            c.env("PATH", &path);
            run(c).trim() == "yes"
        };
        assert!(!has_alias(), "{sh}: not executable");
        fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(has_alias(), "{sh}: executable");
    }
}

#[cfg(unix)]
#[test]
fn hook_uses_the_given_binary() {
    let sb = Sandbox::new();
    let mut c = sb.cmd(BIN);
    c.args(["hook", "zshenv", "--exe", "/opt/it's/shellsync"]);
    let block = run(c);
    assert!(
        block.contains(r"local exe='/opt/it'\''s/shellsync' "),
        "{block}"
    );

    // The current directory is reported resolved; on macOS the temp dir is
    // behind a symlink.
    let home = sb.dir.canonicalize().unwrap();
    let mut c = sb.cmd(BIN);
    c.env("HOME", &home)
        .args(["hook", "nu-env", "--exe", "bin/shellsync"])
        .current_dir(&home);
    let block = run(c);
    assert!(
        block.contains("let exe = ('~/bin/shellsync' | path expand)"),
        "relative to the current directory: {block}"
    );
}

#[test]
fn nu_end_to_end() {
    if !has("nu") {
        eprintln!("nu not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    let out = run(sb.shell_cmd(
        "nu",
        r#"print $env.GOPATH; print $env.TRICKY; print ($env.ONLY_ZSH? | default 'unset'); print ($env.PATH | str join (char esep)); print (scope aliases | where name in [greet ll] | sort-by name | get expansion | str join ';'); print ($env.FOR_SH? | default 'unset'); print (scope aliases | where name in [over missing_tool] | get expansion | str join ';'); print $"($env.FROM_TOOL? | default 'unset') ($env.NOT_SET? | default 'unset')""#,
    ));
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "unset");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin{SEP}/usr/bin{SEP}")),
        "{}",
        lines[3]
    );
    assert!(
        lines[3].ends_with(&format!("{SEP}/opt/shellsync-test")),
        "{}",
        lines[3]
    );
    assert_eq!(lines[4], "echo hello;ls -l");
    assert_eq!(lines[5], "unset");
    assert_eq!(lines[6], "echo tool", "only the applied tool's alias");
    assert_eq!(lines[7], "yes unset");
}

#[test]
fn pwsh_end_to_end() {
    if !has("pwsh") {
        eprintln!("pwsh not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    let (out, err) = sb.shell(
        "pwsh",
        "$env:GOPATH; $env:TRICKY; [bool]$env:ONLY_ZSH; $env:PATH; greet; over; \
         [bool](Get-Command missing_tool -ErrorAction Ignore); $env:FROM_TOOL; \
         [bool]$env:NOT_SET; [bool]$env:FOR_SH; [bool](Get-Command ll -ErrorAction Ignore); \
         [bool](Get-Command .. -ErrorAction Ignore); whoami; $env:FOR_PS",
    );
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "False", "zsh only");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin{SEP}/usr/bin{SEP}")),
        "{}",
        lines[3]
    );
    assert!(
        lines[3].ends_with(&format!("{SEP}/opt/shellsync-test")),
        "{}",
        lines[3]
    );
    assert_eq!(
        lines[3].matches(&format!("{SEP}/usr/bin{SEP}")).count(),
        1,
        "deduplicated"
    );
    assert_eq!(lines[4], "hello");
    assert_eq!(
        lines[5], "tool",
        "the [tool] alias overrides the base alias"
    );
    assert_eq!(lines[6], "False", "tool not installed");
    assert_eq!(lines[7], "yes");
    assert_eq!(lines[8], "False", "path doesn't exist");
    assert_eq!(lines[9], "False", "`sh` doesn't cover pwsh");
    assert_eq!(lines[10], "False", "no pwsh value");
    assert_eq!(lines[11], "True", "an alias named ..");
    assert!(
        !lines[12].is_empty(),
        "an alias that runs the program of its name"
    );
    assert_eq!(lines[13], "p", "`ps` covers pwsh");
}

/// Windows PowerShell 5.1, on Windows only.
#[test]
fn powershell_end_to_end() {
    if !has("powershell") {
        eprintln!("Windows PowerShell not installed, skipping");
        return;
    }
    let sb = Sandbox::new();
    sb.install(BIN);
    // Not ASCII: Windows PowerShell reads the generated file as UTF-8 only
    // because of its byte order mark.
    sb.edit_config(&format!(
        "{}\n[tool.unicode]\nexists = \"/\"\nenv.UNICODE = \"\\u00e4 \\u2192 \\U0001F980\"\n",
        config()
    ));
    let (out, err) = sb.shell(
        "powershell",
        "$PSVersionTable.PSEdition; $env:GOPATH; $env:TRICKY; [bool]$env:ONLY_ZSH; $env:PATH; \
         greet; over; [bool](Get-Command missing_tool -ErrorAction Ignore); $env:FROM_TOOL; \
         [bool]$env:NOT_SET; [bool]$env:FOR_SH; [bool](Get-Command ll -ErrorAction Ignore); \
         [bool](Get-Command .. -ErrorAction Ignore); $env:FOR_PS; \
         [int[]][char[]]$env:UNICODE -join ','",
    );
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], "Desktop", "Windows PowerShell, not pwsh");
    assert_eq!(lines[1], format!("{h}/go"));
    assert_eq!(lines[2], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[3], "False", "zsh only");
    assert!(
        lines[4].starts_with(&format!("{h}/go/bin;/usr/bin;")),
        "{}",
        lines[4]
    );
    assert!(lines[4].ends_with(";/opt/shellsync-test"), "{}", lines[4]);
    assert_eq!(lines[4].matches(";/usr/bin;").count(), 1, "deduplicated");
    assert_eq!(lines[5], "hello");
    assert_eq!(
        lines[6], "tool",
        "the [tool] alias overrides the base alias"
    );
    assert_eq!(lines[7], "False", "tool not installed");
    assert_eq!(lines[8], "yes");
    assert_eq!(lines[9], "False", "path doesn't exist");
    assert_eq!(lines[10], "False", "`sh` doesn't cover powershell");
    assert_eq!(lines[11], "False", "no powershell value");
    assert_eq!(lines[12], "True", "an alias named ..");
    assert_eq!(lines[13], "p", "`ps` covers powershell");
    assert_eq!(lines[14], "228,32,8594,32,55358,56704");
}

/// cmd with clink, on Windows only. The hook runs in `clink lua`, see
/// [`CLINK_START`]: without a previous build it builds, has clink reload, and
/// the reloaded hook loads the result.
#[test]
fn cmd_end_to_end() {
    if !has("cmd") {
        eprintln!("clink not installed, skipping");
        return;
    }
    let sb = Sandbox::installed();
    fs::remove_dir_all(sb.out_dir()).unwrap();
    let (out, err) = sb.shell(
        "cmd",
        r#"for _, name in ipairs({ "GOPATH", "TRICKY", "ONLY_ZSH", "PATH", "FROM_TOOL", "NOT_SET", "FOR_SH", "FOR_PS" }) do print(os.getenv(name) or "nil") end
for _, name in ipairs({ "greet", "over", "missing_tool", "ll", ".." }) do print(os.getalias(name) or "nil") end"#,
    );
    assert_eq!(err, "");
    let lines: Vec<&str> = out.lines().collect();
    let h = sb.home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "nil", "zsh only");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin;/usr/bin;")),
        "{}",
        lines[3]
    );
    assert!(lines[3].ends_with(";/opt/shellsync-test"), "{}", lines[3]);
    assert_eq!(lines[3].matches(";/usr/bin;").count(), 1, "deduplicated");
    assert_eq!(lines[4], "yes");
    assert_eq!(lines[5], "nil", "path doesn't exist");
    assert_eq!(lines[6], "nil", "`sh` doesn't cover cmd");
    assert_eq!(lines[7], "nil", "`ps` doesn't cover cmd");
    assert_eq!(lines[8], "echo hello $*");
    assert_eq!(
        lines[9], "echo tool $*",
        "the [tool] alias overrides the base alias"
    );
    assert_eq!(lines[10], "nil", "tool not installed");
    assert_eq!(lines[11], "nil", "no cmd value");
    assert_eq!(lines[12], "cd .. $*", "an alias named ..");
}

/// A symlinked config (dotfiles linked into place) must not look changed at
/// every start: the build stamps the outputs with the time of the file the
/// link points to, so the hooks must compare that time, not the link's own.
#[cfg(unix)]
#[test]
fn symlinked_config_does_not_rebuild_every_start() {
    use std::os::unix::fs::MetadataExt;

    for sh in ["zsh", "bash", "nu", "pwsh"] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        let sb = Sandbox::installed();
        let target = sb.dir.join("dotfiles/shellsync.toml");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::rename(sb.config(), &target).unwrap();
        std::os::unix::fs::symlink(&target, sb.config()).unwrap();

        let out = sb.out_dir().join("init.nu");
        let inode = || fs::metadata(&out).unwrap().ino();
        let (_, err) = sb.shell(sh, "echo started");
        assert_eq!(err, "", "{sh}");
        let built = inode();
        let (_, err) = sb.shell(sh, "echo started");
        assert_eq!(err, "", "{sh}");
        assert_eq!(inode(), built, "{sh}: rebuilt although nothing changed");
    }
}

/// nushell checks `[tool]` conditions at every start, with its own PATH, and
/// rebuilds only when the answer differs from the last build: here the build
/// ran without the tool's directory on PATH, nushell has it.
#[cfg(unix)]
#[test]
fn nu_rechecks_tools_at_startup() {
    use std::os::unix::fs::{MetadataExt, PermissionsExt};

    if !has("nu") {
        eprintln!("nu not installed, skipping");
        return;
    }
    let sb = Sandbox::new();
    sb.install(BIN);
    fs::write(
        sb.config(),
        "[tool.shellsync-fake-tool.aliases]\nfake = \"shellsync-fake-tool\"\n",
    )
    .unwrap();
    let bin = sb.dir.join("bin");
    let tool = bin.join("shellsync-fake-tool");
    fs::create_dir_all(&bin).unwrap();
    fs::write(&tool, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    run({
        let mut c = sb.cmd(BIN);
        c.arg("build");
        c
    });

    let nu_path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let has_alias = || {
        let mut c = sb.shell_cmd("nu", "print (scope aliases | where name == fake | length)");
        c.env("PATH", &nu_path);
        run(c).trim() == "1"
    };
    let init = sb.out_dir().join("init.nu");
    let inode = || fs::metadata(&init).unwrap().ino();

    let built = inode();
    assert!(has_alias(), "rebuilt in env.nu, before config.nu is parsed");
    let rebuilt = inode();
    assert_ne!(rebuilt, built);
    assert!(has_alias());
    assert_eq!(inode(), rebuilt, "no rebuild while nothing changed");

    fs::remove_file(&tool).unwrap();
    assert!(!has_alias(), "a removed tool is noticed");
}

/// Runs `shellsync check` on `config` with `path` as PATH; returns
/// (success, stdout, stderr).
fn check(sb: &Sandbox, config: &str, path: &str) -> (bool, String, String) {
    let file = sb.dir.join("check.toml");
    fs::write(&file, config).unwrap();
    let out = sb
        .cmd(BIN)
        .env("PATH", path)
        .args(["check", "--config"])
        .arg(&file)
        .output()
        .unwrap();
    (
        out.status.success(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

#[test]
fn check_parses_the_code_with_each_shell() {
    let zsh = !cfg!(windows);
    // On Windows Git Bash, which isn't on PATH by itself.
    let bash = if cfg!(windows) {
        WINDOWS_BASHES[0]
    } else {
        "bash"
    };
    // Windows PowerShell 5.1 and cmd with clink.
    let windows = cfg!(windows);
    if (zsh && !has("zsh"))
        || !has(bash)
        || !has("nu")
        || !has("pwsh")
        || (windows && (!has("powershell") || !has("cmd")))
    {
        eprintln!("a shell is not installed, skipping");
        return;
    }
    let sb = Sandbox::new();
    let mut path = std::env::var("PATH").unwrap_or_default();
    if cfg!(windows) {
        let dir = Path::new(bash).parent().unwrap();
        path = format!("{};{path}", dir.display());
    }

    let broken = r#"
[aliases]
ok = "echo ok"
quote = { sh = "echo 'open", nu = "echo ok" }
piped = "ls | sort"

# Not installed anywhere, but checked all the same.
[tool.c]
exists = "/shellsync/does/not/exist"
sh.init = """
if true; then
  echo x
"""
pwsh.init = "if ($true) {"
powershell.init = "if ($true) {"
cmd.init = "if true then"
"#;
    let (ok, out, err) = check(&sb, broken, &path);
    assert!(!ok);
    if zsh {
        assert_eq!(err, "shellsync: 6 parse errors\n");
        assert!(
            out.contains("\nzsh: 2 errors\n  aliases.quote:1: "),
            "{out}"
        );
        assert!(out.contains("\n  tool.c.sh.init:3: parse error"), "{out}");
    } else {
        assert_eq!(err, "shellsync: 6 parse errors\n");
        assert!(
            out.contains("\npowershell: 1 error\n  tool.c.powershell.init:1:"),
            "{out}"
        );
        assert!(
            out.contains("\ncmd: 1 error\n  tool.c.cmd.init:1: 'end' expected near <eof>\n"),
            "{out}"
        );
    }
    assert!(
        out.contains("\nbash: 2 errors\n  aliases.quote: line 1: "),
        "{out}"
    );
    assert!(
        out.contains("\n  tool.c.sh.init: line 3: syntax error"),
        "{out}"
    );
    assert!(out.contains("\nnu: 1 error\n  aliases.piped:\n"), "{out}");
    assert!(out.contains("Can't create alias to expression"), "{out}");
    assert!(
        out.contains("\npwsh: 1 error\n  tool.c.pwsh.init:1:"),
        "{out}"
    );

    // Parses only after the earlier block, as it does in the generated file.
    let in_context = r#"
[tool.a]
exists = "/"
nu.init = "let shared = 1"
[tool.b]
exists = "/"
nu.init = "print $shared"
"#;
    let sh_lines = if zsh {
        "zsh: ok\nbash: ok\n"
    } else {
        "bash: ok\n"
    };
    let windows_lines = if windows {
        "powershell: ok\ncmd: ok\n"
    } else {
        ""
    };
    for config in [in_context, &config()] {
        let (ok, out, err) = check(&sb, config, &path);
        assert!(ok, "{out}{err}");
        assert!(
            out.ends_with(&format!("\n{sh_lines}nu: ok\npwsh: ok\n{windows_lines}")),
            "{out}"
        );
    }
}

#[cfg(unix)]
#[test]
fn check_skips_shells_that_are_not_installed() {
    if !has("zsh") {
        eprintln!("zsh not installed, skipping");
        return;
    }
    let sb = Sandbox::new();
    let zsh = run({
        let mut c = sb.cmd("sh");
        c.args(["-c", "command -v zsh"]);
        c
    });
    let bin = sb.dir.join("bin");
    fs::create_dir(&bin).unwrap();
    std::os::unix::fs::symlink(zsh.trim(), bin.join("zsh")).unwrap();

    let (ok, out, err) = check(&sb, &config(), bin.to_str().unwrap());
    assert!(ok, "{out}{err}");
    assert!(
        out.ends_with(
            "\nzsh: ok\nbash: not installed, not checked\nnu: not installed, not checked\npwsh: not installed, not checked\n"
        ),
        "{out}"
    );
}

#[test]
fn edits_are_picked_up_and_errors_keep_last_good_config() {
    for (sh, print) in [
        ("zsh", "print -r -- $GOPATH"),
        ("bash", r#"echo "$GOPATH""#),
        ("nu", "print $env.GOPATH"),
        ("pwsh", "$env:GOPATH"),
        ("powershell", "$env:GOPATH"),
        ("cmd", "print(os.getenv('GOPATH'))"),
    ] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        // Each shell rebuilds on its own: no other shell runs in this sandbox.
        let sb = Sandbox::installed();
        let config = sb.config();
        assert_eq!(
            sb.shell(sh, print).0,
            format!("{}/go", sb.home.display()),
            "{sh}"
        );

        sb.edit_config("[env]\nGOPATH = '/elsewhere'\n");
        assert_eq!(sb.shell(sh, print).0, "/elsewhere", "{sh}");

        // A broken config reports the error; the shell keeps the last good build.
        sb.edit_config("[env\n");
        let (out, err) = sb.shell(sh, print);
        assert_eq!(out, "/elsewhere", "{sh}");
        let name = config.file_name().unwrap().to_str().unwrap();
        assert!(err.contains(name), "{sh}: {err}");

        // So does valid TOML with code the shells can't parse; in nushell it
        // would stop the whole config from loading.
        sb.edit_config(
            "[env]\nGOPATH = '/broken'\n[aliases]\nbad = { sh = \"echo 'open\", nu = \"ls | sort\", pwsh = \"echo (\", powershell = \"echo (\" }\n[cmd]\ninit = 'if'\n",
        );
        let (out, err) = sb.shell(sh, print);
        assert_eq!(out, "/elsewhere", "{sh}: {err}");
        assert!(err.contains("keep the last good build"), "{sh}: {err}");
        let key = if sh == "cmd" {
            "cmd.init"
        } else {
            "aliases.bad"
        };
        assert!(err.contains(key), "{sh}: {err}");

        sb.edit_config("[env]\nGOPATH = '/fixed'\n");
        assert_eq!(sb.shell(sh, print).0, "/fixed", "{sh}");

        // Even with no previous build, the shell still starts.
        fs::remove_dir_all(sb.out_dir()).unwrap();
        let (out, err) = sb.shell(sh, echo_started(sh));
        assert_eq!(out, "started", "{sh}: {err}");
    }
}

/// `build` parses the generated code, which may hold secrets from the
/// config, in shellsync's own cache directory, not in the temp directory:
/// that may not be writable, or, as `/tmp` in MSYS2 and Cygwin, readable by
/// other users.
#[test]
fn build_does_not_use_the_temp_dir() {
    let sb = Sandbox::new();
    sb.edit_config(&config());
    let missing = sb.dir.join("no-such-temp-dir");
    let mut c = sb.cmd(BIN);
    c.env("TMPDIR", &missing)
        .env("TMP", &missing)
        .env("TEMP", &missing)
        .args(["build", "--config"])
        .arg(sb.config());
    run(c);
    assert!(sb.out_dir().join("init.bash").exists());
    let left: Vec<_> = fs::read_dir(sb.out_dir())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .filter(|name| {
            !name.to_string_lossy().starts_with("init.")
                && !name.to_string_lossy().starts_with("env.")
                && name != "init-ps5.ps1"
                && name != "tools.nu"
        })
        .collect();
    assert!(left.is_empty(), "left behind: {left:?}");
}

/// `edit` opens the config in $VISUAL, or runs `[edit] command` instead, such
/// as `chezmoi edit --apply` for a config that a dotfile manager writes from
/// a template; either way it rebuilds afterwards. A config with errors is
/// opened in the editor, so that it can be fixed.
#[test]
fn edit_runs_the_editor_or_the_configured_command() {
    let sb = Sandbox::new();
    let (copy, fails) = if cfg!(windows) {
        ("copy /Y", "exit /b 1")
    } else {
        ("cp", "false")
    };
    let edit = |visual: &str| {
        let mut c = sb.cmd(BIN);
        c.env("VISUAL", visual).env_remove("EDITOR").arg("edit");
        run(c)
    };
    let built = || fs::read_to_string(sb.out_dir().join("init.ps1")).unwrap();
    let by_editor = sb.dir.join("by-editor.toml");
    fs::write(&by_editor, "[env]\nEDITED = 'by editor'\n").unwrap();
    let editor = format!("{copy} \"{}\"", by_editor.display());

    sb.edit_config("[env]\nEDITED = 'before'\n");
    edit(&editor);
    assert!(built().contains("by editor"), "{}", built());

    // Here the command stands for `chezmoi edit --apply`: it writes the config.
    let by_command = sb.dir.join("by-command.toml");
    fs::write(&by_command, "[env]\nEDITED = 'by command'\n").unwrap();
    let command = format!(
        "{copy} \"{}\" \"{}\"",
        by_command.display(),
        sb.config().display()
    );
    sb.edit_config(&format!(
        "[env]\nEDITED = 'before'\n[edit]\ncommand = '{command}'\n"
    ));
    edit(fails);
    assert!(built().contains("by command"), "{}", built());

    sb.edit_config(&format!("[env\n[edit]\ncommand = '{command}'\n"));
    edit(&editor);
    assert!(built().contains("by editor"), "{}", built());
}

/// Synced dotfiles can carry the hooks to a machine where the binary is
/// missing or has moved. Shells must still start, without error output.
#[test]
fn shells_start_quietly_without_the_binary() {
    for sh in ["zsh", "bash", "nu", "pwsh", "powershell", "cmd"] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        let sb = Sandbox::new();
        let exe = sb.dir.join("bin").join(Path::new(BIN).file_name().unwrap());
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(BIN, &exe).unwrap();
        sb.install(exe.to_str().unwrap());
        fs::remove_file(&exe).unwrap();
        fs::remove_dir_all(sb.out_dir()).unwrap();

        let (out, err) = sb.shell(sh, echo_started(sh));
        assert_eq!(out, "started", "{sh}: {err}");
        assert_eq!(err, "", "{sh}");
    }
}

/// `install` writes where the shells look for their startup files. On
/// Windows that is found through USERPROFILE (PowerShell) and
/// XDG_CONFIG_HOME (nushell), both pointed into the sandbox here; the shells
/// aren't started.
#[test]
fn install_is_idempotent_and_uninstall_restores() {
    let sb = Sandbox::new();
    let shellsync = |arg: &str| {
        let mut c = sb.cmd(BIN);
        if cfg!(windows) {
            c.env_remove("SHELLSYNC_CONFIG")
                .env_remove("XDG_CACHE_HOME")
                .env("USERPROFILE", &sb.dir);
        }
        c.arg(arg);
        run(c)
    };
    // On Windows, `.bashrc` and Windows PowerShell's profile are hooked only
    // if they exist, clink's script only if its folder does (CLINK_PROFILE,
    // see `Sandbox::cmd`). (Windows PowerShell finds no Documents folder in
    // the sandbox, so shellsync falls back to the one in USERPROFILE.)
    let bashrc = sb.dir.join(".bashrc");
    let ps5_profile = sb.dir.join(r"Documents\WindowsPowerShell\profile.ps1");
    let clink_script = sb.dir.join(r"clink\shellsync.lua");
    if cfg!(windows) {
        fs::write(&bashrc, "# mine\n").unwrap();
        fs::create_dir_all(ps5_profile.parent().unwrap()).unwrap();
        fs::write(&ps5_profile, "# mine too\n").unwrap();
        fs::create_dir_all(clink_script.parent().unwrap()).unwrap();
        fs::write(&clink_script, "-- mine as well\n").unwrap();
    }
    let hooks: Vec<(String, PathBuf)> = sb
        .assert_inside(&shellsync("paths"))
        .into_iter()
        .filter(|(name, _)| {
            ["zsh", "bash", "nu ", "pwsh", "ps5", "clink"]
                .iter()
                .any(|s| name.starts_with(s))
        })
        .collect();
    // On Windows no zsh, but Windows PowerShell and clink.
    assert_eq!(hooks.len(), 6, "{hooks:?}");
    if cfg!(windows) {
        assert!(
            hooks.contains(&("ps5 profile".into(), ps5_profile.clone())),
            "{hooks:?}"
        );
        assert!(
            hooks.contains(&("clink script".into(), clink_script.clone())),
            "{hooks:?}"
        );
    }
    let profile = &hooks
        .iter()
        .find(|(name, _)| name == "pwsh profile")
        .unwrap()
        .1;
    fs::create_dir_all(profile.parent().unwrap()).unwrap();
    fs::write(profile, "$env:KEEP = 1\n").unwrap();
    let hooks: Vec<PathBuf> = hooks.iter().map(|(_, path)| path.clone()).collect();

    shellsync("install");
    let read = || -> Vec<String> {
        hooks
            .iter()
            .map(|f| fs::read_to_string(f).unwrap())
            .collect()
    };
    let once = read();
    shellsync("install");
    assert_eq!(read(), once);
    for text in &once {
        assert_eq!(text.matches(">>> shellsync >>>").count(), 1);
    }

    shellsync("uninstall");
    assert_eq!(fs::read_to_string(profile).unwrap(), "$env:KEEP = 1\n");
    if cfg!(windows) {
        assert_eq!(fs::read_to_string(&bashrc).unwrap(), "# mine\n");
        assert_eq!(fs::read_to_string(&ps5_profile).unwrap(), "# mine too\n");
        assert_eq!(
            fs::read_to_string(&clink_script).unwrap(),
            "-- mine as well\n"
        );
    }
    for text in read() {
        assert!(!text.contains("shellsync"));
    }
}
