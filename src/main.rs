mod config;
mod hooks;
mod probe;
mod shell;
mod template;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use config::{Config, Entries};
use hooks::HookFile;
use probe::Probe;
use shell::Shell;

const USAGE: &str = "\
shellsync - configure zsh and nushell from one file

Usage: shellsync <command> [options]

Commands:
  install          Create the config if missing, build, and hook into
                   ~/.zshrc and nushell's env.nu / config.nu
  uninstall        Remove the hooks (config and cache are kept)
  edit             Open the config in $VISUAL / $EDITOR, then rebuild
  build            Regenerate the shell files (shells do this automatically)
  check            Validate the config
  print <shell>    Print the generated code for zsh or nu
  hook <file>      Print the hook for zsh, nu-env or nu-config
  paths            Show the files shellsync reads and writes

Options:
  --config <file>   Config file  [default: $SHELLSYNC_CONFIG or
                    ~/.config/shellsync/config.toml]
  --out-dir <dir>   Where generated files go  [default: ~/.cache/shellsync]
  --if-stale        (build) Only rebuild if the config changed
";

const EXAMPLE_CONFIG: &str = include_str!("../examples/example.toml");

type Result<T = ()> = std::result::Result<T, String>;

struct Paths {
    config: PathBuf,
    out_dir: PathBuf,
}

impl Paths {
    fn output(&self, shell: Shell) -> PathBuf {
        self.out_dir.join(format!("init.{}", shell.name()))
    }
}

fn home() -> PathBuf {
    env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/"))
}

fn xdg(var: &str, fallback: &str) -> PathBuf {
    match env::var_os(var) {
        Some(p) if Path::new(&p).is_absolute() => PathBuf::from(p),
        _ => home().join(fallback),
    }
}

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("shellsync: {e}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result {
    let mut config = None;
    let mut out_dir = None;
    let mut if_stale = false;
    let mut positional = Vec::new();

    let mut it = args.into_iter();
    while let Some(arg) = it.next() {
        match arg.as_str() {
            "--config" => config = Some(PathBuf::from(it.next().ok_or("--config needs a file")?)),
            "--out-dir" => {
                out_dir = Some(PathBuf::from(
                    it.next().ok_or("--out-dir needs a directory")?,
                ))
            }
            "--if-stale" => if_stale = true,
            "-h" | "--help" | "help" => {
                print!("{USAGE}");
                return Ok(());
            }
            "-V" | "--version" => {
                println!("shellsync {}", env!("CARGO_PKG_VERSION"));
                return Ok(());
            }
            s if s.starts_with('-') => return Err(format!("unknown option `{s}` (see --help)")),
            _ => positional.push(arg),
        }
    }

    let absolute = |p: PathBuf| {
        if p.is_absolute() {
            p
        } else {
            env::current_dir().unwrap_or_default().join(p)
        }
    };
    let paths = Paths {
        config: absolute(
            config.unwrap_or_else(|| match env::var_os("SHELLSYNC_CONFIG") {
                Some(p) => PathBuf::from(p),
                None => xdg("XDG_CONFIG_HOME", ".config").join("shellsync/config.toml"),
            }),
        ),
        out_dir: absolute(
            out_dir.unwrap_or_else(|| xdg("XDG_CACHE_HOME", ".cache").join("shellsync")),
        ),
    };

    let shell_arg = |pos: &[String]| -> Result<Shell> {
        let s = pos.get(1).ok_or("expected a shell: zsh or nu")?;
        Shell::parse(s).ok_or_else(|| format!("unknown shell `{s}` (expected zsh or nu)"))
    };

    match positional.first().map(String::as_str) {
        None => {
            print!("{USAGE}");
            Ok(())
        }
        Some("build") => build(&paths, if_stale).map(|_| ()),
        Some("check") => check(&paths),
        Some("print") => {
            let shell = shell_arg(&positional)?;
            let cfg = load(&paths.config)?;
            print!(
                "{}",
                shell::render(
                    &cfg,
                    shell,
                    &paths.config.display().to_string(),
                    &Probe::from_env(),
                )
            );
            Ok(())
        }
        Some("hook") => {
            let which = positional
                .get(1)
                .ok_or("expected zsh, nu-env or nu-config")?;
            let kind = HookFile::parse(which).ok_or_else(|| format!("unknown hook `{which}`"))?;
            print!("{}", kind.block(&paths, &this_exe()?, &home()));
            Ok(())
        }
        Some("install") => install(&paths),
        Some("uninstall") => uninstall(),
        Some("edit") => edit(&paths),
        Some("paths") => {
            println!("config     {}", paths.config.display());
            for shell in Shell::ALL {
                println!(
                    "init.{:<5}  {}",
                    shell.name(),
                    paths.output(shell).display()
                );
            }
            for (kind, file) in hooks::files() {
                println!("{:<10} {}", kind.name(), file.display());
            }
            Ok(())
        }
        Some(cmd) => Err(format!("unknown command `{cmd}` (see --help)")),
    }
}

// ----------------------------------------------------------- building ----

fn load(path: &Path) -> Result<Config> {
    match fs::read_to_string(path) {
        Ok(src) => Config::parse(&src).map_err(|e| format!("{}: {e}", path.display())),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Config::default()),
        Err(e) => Err(format!("{}: {e}", path.display())),
    }
}

fn mtime(p: &Path) -> Option<SystemTime> {
    fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// Outputs are stamped with the config's exact mtime when built, so "stale"
/// means "the mtimes differ". Comparing for equality instead of "newer" also
/// catches edits within the filesystem's timestamp granularity and configs
/// restored with an older timestamp. The shell hooks use the same rule.
fn is_stale(paths: &Paths) -> bool {
    let cfg = mtime(&paths.config);
    Shell::ALL
        .iter()
        .any(|&s| match (mtime(&paths.output(s)), cfg) {
            (None, _) => true,
            (Some(out), Some(cfg)) => cfg != out,
            (Some(_), None) => false,
        })
}

/// Writes via a temp file + rename so a shell starting concurrently never
/// sources a half-written file. The result is readable only by the user,
/// because the config may contain secrets.
fn write_atomic(path: &Path, contents: &str, stamp: Option<SystemTime>) -> Result {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    let write = || -> io::Result<()> {
        // A leftover temp file would keep its old mode, so start fresh.
        let _ = fs::remove_file(&tmp);
        let mut file = private_file(&tmp)?;
        file.write_all(contents.as_bytes())?;
        if let Some(t) = stamp {
            file.set_modified(t)?;
        }
        fs::rename(&tmp, path)
    };
    write().map_err(|e| {
        let _ = fs::remove_file(&tmp);
        format!("writing {}: {e}", path.display())
    })
}

fn private_file(path: &Path) -> io::Result<fs::File> {
    let mut options = fs::File::options();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    options.open(path)
}

/// Regenerates both outputs. Returns whether anything was written.
///
/// On a config error the previous outputs are left untouched, so shells keep
/// working with the last good config.
fn build(paths: &Paths, if_stale: bool) -> Result<bool> {
    if if_stale && !is_stale(paths) {
        return Ok(false);
    }
    // Taken before reading, so an edit made during the build still differs.
    let stamp = mtime(&paths.config);
    let cfg = load(&paths.config)?;
    fs::create_dir_all(&paths.out_dir)
        .map_err(|e| format!("creating {}: {e}", paths.out_dir.display()))?;
    let source = paths.config.display().to_string();
    let probe = Probe::from_env();
    for shell in Shell::ALL {
        write_atomic(
            &paths.output(shell),
            &shell::render(&cfg, shell, &source, &probe),
            stamp,
        )?;
    }
    Ok(true)
}

fn check(paths: &Paths) -> Result {
    if !paths.config.exists() {
        return Err(format!(
            "{} does not exist (run `shellsync install`)",
            paths.config.display()
        ));
    }
    let cfg = load(&paths.config)?;
    let all: Vec<&Entries> = std::iter::once(&cfg.entries)
        .chain(cfg.tools.iter().map(|t| &t.entries))
        .collect();
    let count = |f: fn(&Entries) -> usize| all.iter().map(|e| f(e)).sum::<usize>();
    println!(
        "{}: ok ({} env vars, {} path entries, {} aliases, {} tools)",
        paths.config.display(),
        count(|e| e.env.len()),
        count(|e| e.path_prepend.len() + e.path_append.len()),
        count(|e| e.aliases.len()),
        cfg.tools.len()
    );
    Ok(())
}

fn edit(paths: &Paths) -> Result {
    ensure_config(paths)?;
    let editor = ["VISUAL", "EDITOR"]
        .iter()
        .find_map(|v| env::var(v).ok().filter(|s| !s.trim().is_empty()))
        .unwrap_or_else(|| "vi".into());
    // Run through sh so EDITOR values with arguments ("code --wait") work.
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(&paths.config)
        .status()
        .map_err(|e| format!("running {editor}: {e}"))?;
    if !status.success() {
        return Err(format!("{editor} exited with {status}"));
    }
    build(paths, false)?;
    println!("Rebuilt. New shells pick up the changes; in zsh you can also run `exec zsh`.");
    Ok(())
}

fn ensure_config(paths: &Paths) -> Result<bool> {
    if paths.config.exists() {
        return Ok(false);
    }
    if let Some(dir) = paths.config.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    fs::write(&paths.config, EXAMPLE_CONFIG)
        .map_err(|e| format!("writing {}: {e}", paths.config.display()))?;
    Ok(true)
}

// ------------------------------------------------------------ install ----

fn this_exe() -> Result<PathBuf> {
    env::current_exe().map_err(|e| format!("locating the shellsync binary: {e}"))
}

fn install(paths: &Paths) -> Result {
    let mut stdout = io::stdout();
    if ensure_config(paths)? {
        writeln!(
            stdout,
            "created  {} (an example to edit)",
            paths.config.display()
        )
        .ok();
    }
    build(paths, false)?;
    writeln!(stdout, "built    {}", paths.out_dir.display()).ok();
    let exe = this_exe()?;
    for (kind, file) in hooks::files() {
        let changed = hooks::update_file(&file, &kind.block(paths, &exe, &home()))?;
        let verb = if changed { "hooked" } else { "ok" };
        writeln!(stdout, "{verb:<8} {}", file.display()).ok();
    }
    writeln!(
        stdout,
        "\nDone. Edit your settings with `shellsync edit`; new shells pick them up automatically."
    )
    .ok();
    Ok(())
}

fn uninstall() -> Result {
    for (_, file) in hooks::files() {
        if hooks::update_file(&file, "")? {
            println!("unhooked {}", file.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
    fn outputs_are_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;

        let dir = env::temp_dir().join(format!("shellsync-unit-perms-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        let paths = Paths {
            config: dir.join("config.toml"),
            out_dir: dir.join("out"),
        };
        fs::create_dir_all(&paths.out_dir).unwrap();
        fs::write(&paths.config, "[env]\nTOKEN = 'secret'\n").unwrap();
        // An existing output with looser permissions, e.g. created by hand.
        let old = paths.output(Shell::Zsh);
        fs::write(&old, "").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();

        build(&paths, false).unwrap();
        for shell in Shell::ALL {
            let mode = fs::metadata(paths.output(shell))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(format!("{:o}", mode & 0o777), "600", "{}", shell.name());
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
