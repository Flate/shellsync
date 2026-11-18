//! End-to-end tests: install into a throwaway HOME and start real shells.
//! A test is skipped (with a note) when its shell is not installed.

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

[path]
prepend = ["$GOPATH/bin", "/usr/bin"]
append = ["/opt/shellsync-test"]

[aliases]
greet = "echo hello"
ll = { zsh = "ls -lh", nu = "ls -l" }
"#;

fn has(shell: &str) -> bool {
    Command::new(shell)
        .arg("-c")
        .arg("exit 0")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// A fresh fake HOME with shellsync installed and CONFIG in place.
fn setup() -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let home = std::env::temp_dir().join(format!(
        "shellsync-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::SeqCst)
    ));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
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
    c.args(["-i", "-c", r#"print -r -- $GOPATH; print -r -- $TRICKY; print -r -- $ONLY_ZSH; print -r -- $PATH; alias greet; alias ll"#]);
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
}

#[test]
fn nu_end_to_end() {
    if !has("nu") {
        eprintln!("nu not installed, skipping");
        return;
    }
    let home = setup();
    let cfg_dir = home.join(".config/nushell");
    let mut c = cmd("nu", &home);
    c.arg("--env-config")
        .arg(cfg_dir.join("env.nu"))
        .arg("--config")
        .arg(cfg_dir.join("config.nu"))
        .args([
            "-c",
            r#"print $env.GOPATH; print $env.TRICKY; print ($env.ONLY_ZSH? | default 'unset'); print ($env.PATH | str join ':'); print (scope aliases | where name in [greet ll] | sort-by name | get expansion | str join ';')"#,
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
}

/// Runs `code` in the given shell with the installed hooks and returns
/// (stdout, stderr).
fn shell(shell: &str, home: &Path, code: &str) -> (String, String) {
    let mut c = cmd(shell, home);
    if shell == "nu" {
        let dir = home.join(".config/nushell");
        c.arg("--env-config").arg(dir.join("env.nu"));
        c.arg("--config").arg(dir.join("config.nu"));
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
