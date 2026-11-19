//! End-to-end tests: install into a throwaway HOME and start real shells.
//! A test is skipped (with a note) when its shell is not installed, unless
//! SHELLSYNC_REQUIRE_SHELLS is set (as in CI), where a missing shell fails.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};

const BIN: &str = env!("CARGO_BIN_EXE_shellsync");

const CONFIG: &str = r#"
[env]
GOPATH = "~/go"
TRICKY = "it's \"q\" $$x `y` (z) {w}"
ONLY_ZSH = { zsh = "z" }
FOR_SH = { sh = "s" }

[path]
prepend = ["$GOPATH/bin", "/usr/bin"]
append = ["/opt/shellsync-test"]

[aliases]
greet = "echo hello"
ll = { zsh = "ls -lh", nu = "ls -l" }
over = "echo base"

# `ls` is on PATH everywhere, so this applies and overrides the base alias.
[tool.ls.aliases]
over = "echo tool"

[tool.shellsync-no-such-tool.aliases]
missing_tool = "echo nope"

[tool.root]
exists = "/"
env.FROM_TOOL = "yes"

[tool.nowhere]
exists = "/shellsync/does/not/exist"
env.NOT_SET = "no"
"#;

fn has(shell: &str) -> bool {
    let found = Command::new(shell)
        .arg("-c")
        .arg("exit 0")
        .output()
        .is_ok_and(|o| o.status.success());
    if !found && std::env::var_os("SHELLSYNC_REQUIRE_SHELLS").is_some() {
        panic!("{shell} not installed, but SHELLSYNC_REQUIRE_SHELLS is set");
    }
    found
}

/// nushell's env.nu and config.nu for `home`, as nushell itself reports them.
/// They differ by platform (e.g. ~/Library/Application Support on macOS).
fn nu_config_files(home: &Path) -> (PathBuf, PathBuf) {
    let mut c = cmd("nu", home);
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

/// A fresh, empty fake HOME.
fn temp_home() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let home = std::env::temp_dir().join(format!(
        "shellsync-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    home
}

/// A fresh fake HOME with shellsync installed and CONFIG in place.
fn setup() -> PathBuf {
    let home = temp_home();
    let out = cmd(BIN, &home).arg("install").output().unwrap();
    assert!(
        out.status.success(),
        "install failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    fs::write(home.join(".config/shellsync/config.toml"), CONFIG).unwrap();
    home
}

fn cmd(program: &str, home: &Path) -> Command {
    let mut c = Command::new(program);
    c.env_clear()
        .env("HOME", home)
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("TERM", "dumb");
    c
}

fn run(mut c: Command) -> String {
    let out = c.output().unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "failed: {stderr}");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn zsh_end_to_end() {
    if !has("zsh") {
        eprintln!("zsh not installed, skipping");
        return;
    }
    let home = setup();
    let mut c = cmd("zsh", &home);
    c.args(["-i", "-c", r#"print -r -- $GOPATH; print -r -- $TRICKY; print -r -- $ONLY_ZSH; print -r -- $PATH; alias greet; alias ll; print -r -- $FOR_SH; alias over; print -r -- ${+aliases[missing_tool]}; print -r -- ${FROM_TOOL:-unset} ${NOT_SET:-unset}"#]);
    let out = run(c);
    let lines: Vec<&str> = out.lines().collect();
    let h = home.display();
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

#[test]
fn nu_end_to_end() {
    if !has("nu") {
        eprintln!("nu not installed, skipping");
        return;
    }
    let home = setup();
    let (env_nu, config_nu) = nu_config_files(&home);
    let mut c = cmd("nu", &home);
    c.arg("--env-config")
        .arg(env_nu)
        .arg("--config")
        .arg(config_nu)
        .args([
            "-c",
            r#"print $env.GOPATH; print $env.TRICKY; print ($env.ONLY_ZSH? | default 'unset'); print ($env.PATH | str join ':'); print (scope aliases | where name in [greet ll] | sort-by name | get expansion | str join ';'); print ($env.FOR_SH? | default 'unset'); print (scope aliases | where name in [over missing_tool] | get expansion | str join ';'); print $"($env.FROM_TOOL? | default 'unset') ($env.NOT_SET? | default 'unset')""#,
        ]);
    let out = run(c);
    let lines: Vec<&str> = out.lines().collect();
    let h = home.display();
    assert_eq!(lines[0], format!("{h}/go"));
    assert_eq!(lines[1], "it's \"q\" $x `y` (z) {w}");
    assert_eq!(lines[2], "unset");
    assert!(
        lines[3].starts_with(&format!("{h}/go/bin:/usr/bin:")),
        "{}",
        lines[3]
    );
    assert!(lines[3].ends_with(":/opt/shellsync-test"), "{}", lines[3]);
    assert_eq!(lines[4], "echo hello;ls -l");
    assert_eq!(lines[5], "unset");
    assert_eq!(lines[6], "echo tool", "only the applied tool's alias");
    assert_eq!(lines[7], "yes unset");
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
    let home = temp_home();
    run({
        let mut c = cmd(BIN, &home);
        c.arg("install");
        c
    });
    fs::write(
        home.join(".config/shellsync/config.toml"),
        "[tool.shellsync-fake-tool.aliases]\nfake = \"shellsync-fake-tool\"\n",
    )
    .unwrap();
    let bin = home.join("bin");
    let tool = bin.join("shellsync-fake-tool");
    fs::create_dir_all(&bin).unwrap();
    fs::write(&tool, "#!/bin/sh\n").unwrap();
    fs::set_permissions(&tool, fs::Permissions::from_mode(0o755)).unwrap();
    run({
        let mut c = cmd(BIN, &home);
        c.arg("build");
        c
    });

    let nu_path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let (env_nu, config_nu) = nu_config_files(&home);
    let has_alias = || {
        let mut c = cmd("nu", &home);
        c.env("PATH", &nu_path)
            .arg("--env-config")
            .arg(&env_nu)
            .arg("--config")
            .arg(&config_nu)
            .args(["-c", "print (scope aliases | where name == fake | length)"]);
        run(c).trim() == "1"
    };
    let init = home.join(".cache/shellsync/init.nu");
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

/// Runs `code` in the given shell with the installed hooks and returns
/// (stdout, stderr).
fn shell(shell: &str, home: &Path, code: &str) -> (String, String) {
    let mut c = cmd(shell, home);
    if shell == "nu" {
        let (env_nu, config_nu) = nu_config_files(home);
        c.arg("--env-config").arg(env_nu);
        c.arg("--config").arg(config_nu);
        c.args(["-c", code]);
    } else {
        c.args(["-i", "-c", code]);
    }
    let out = c.output().unwrap();
    (
        String::from_utf8_lossy(&out.stdout).trim().to_string(),
        String::from_utf8_lossy(&out.stderr).to_string(),
    )
}

#[test]
fn edits_are_picked_up_and_errors_keep_last_good_config() {
    for (sh, print) in [("zsh", "print -r -- $GOPATH"), ("nu", "print $env.GOPATH")] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        // Each shell rebuilds on its own: no other shell runs in this HOME.
        let home = setup();
        let config = home.join(".config/shellsync/config.toml");
        assert_eq!(
            shell(sh, &home, print).0,
            format!("{}/go", home.display()),
            "{sh}"
        );

        fs::write(&config, "[env]\nGOPATH = '/elsewhere'\n").unwrap();
        assert_eq!(shell(sh, &home, print).0, "/elsewhere", "{sh}");

        // A broken config reports the error; the shell keeps the last good build.
        fs::write(&config, "[env\n").unwrap();
        let (out, err) = shell(sh, &home, print);
        assert_eq!(out, "/elsewhere", "{sh}");
        assert!(err.contains("config.toml"), "{sh}: {err}");

        // Even with no previous build, the shell still starts.
        fs::remove_dir_all(home.join(".cache/shellsync")).unwrap();
        let (out, err) = shell(sh, &home, "print started");
        assert_eq!(out, "started", "{sh}: {err}");
    }
}

/// Synced dotfiles can carry the hooks to a machine where the binary is
/// missing or has moved. Shells must still start, without error output.
#[test]
fn shells_start_quietly_without_the_binary() {
    for sh in ["zsh", "nu"] {
        if !has(sh) {
            eprintln!("{sh} not installed, skipping");
            continue;
        }
        let home = temp_home();
        let exe = home.join("bin/shellsync");
        fs::create_dir_all(exe.parent().unwrap()).unwrap();
        fs::copy(BIN, &exe).unwrap();
        run({
            let mut c = cmd(exe.to_str().unwrap(), &home);
            c.arg("install");
            c
        });
        fs::remove_file(&exe).unwrap();
        fs::remove_dir_all(home.join(".cache/shellsync")).unwrap();

        let (out, err) = shell(sh, &home, "print started");
        assert_eq!(out, "started", "{sh}: {err}");
        assert_eq!(err, "", "{sh}");
    }
}

#[test]
fn install_is_idempotent_and_uninstall_restores() {
    let home = std::env::temp_dir().join(format!("shellsync-test-idem-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join(".zshrc"), "export KEEP=1\n").unwrap();

    let install = || {
        run({
            let mut c = cmd(BIN, &home);
            c.arg("install");
            c
        })
    };
    install();
    let once = fs::read_to_string(home.join(".zshrc")).unwrap();
    install();
    assert_eq!(fs::read_to_string(home.join(".zshrc")).unwrap(), once);
    assert_eq!(once.matches(">>> shellsync >>>").count(), 1);

    run({
        let mut c = cmd(BIN, &home);
        c.arg("uninstall");
        c
    });
    assert_eq!(
        fs::read_to_string(home.join(".zshrc")).unwrap(),
        "export KEEP=1\n"
    );
}
