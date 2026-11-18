mod config;
mod render;
mod template;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};
use std::time::SystemTime;

use config::{Config, Shell};

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

const EXAMPLE_CONFIG: &str = include_str!("../examples/config.toml");
const BEGIN: &str = "# >>> shellsync >>>";
const END: &str = "# <<< shellsync <<<";

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
                render::render(&cfg, shell, &paths.config.display().to_string())
            );
            Ok(())
        }
        Some("hook") => {
            let which = positional
                .get(1)
                .ok_or("expected zsh, nu-env or nu-config")?;
            let kind = HookFile::parse(which).ok_or_else(|| format!("unknown hook `{which}`"))?;
            print!("{}", hook(kind, &paths)?);
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
            for (kind, file) in hook_files() {
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
/// sources a half-written file.
fn write_atomic(path: &Path, contents: &str, stamp: Option<SystemTime>) -> Result {
    let tmp = path.with_extension(format!("tmp{}", std::process::id()));
    fs::write(&tmp, contents)
        .and_then(|()| match stamp {
            Some(t) => fs::File::options().write(true).open(&tmp)?.set_modified(t),
            None => Ok(()),
        })
        .and_then(|()| fs::rename(&tmp, path))
        .map_err(|e| {
            let _ = fs::remove_file(&tmp);
            format!("writing {}: {e}", path.display())
        })
}

/// Regenerates both outputs. Returns whether anything was written.
///
/// On a config error the previous outputs are left untouched (so shells keep
/// working with the last good config); if there are none yet, placeholders
/// are written so that nushell's `source` line still parses.
fn build(paths: &Paths, if_stale: bool) -> Result<bool> {
    if if_stale && !is_stale(paths) {
        return Ok(false);
    }
    fs::create_dir_all(&paths.out_dir)
        .map_err(|e| format!("creating {}: {e}", paths.out_dir.display()))?;
    let source = paths.config.display().to_string();
    // Taken before reading, so an edit made during the build still differs.
    let stamp = mtime(&paths.config);
    match load(&paths.config) {
        Ok(cfg) => {
            for shell in Shell::ALL {
                write_atomic(
                    &paths.output(shell),
                    &render::render(&cfg, shell, &source),
                    stamp,
                )?;
            }
            Ok(true)
        }
        Err(e) => {
            for shell in Shell::ALL {
                let out = paths.output(shell);
                if !out.exists() {
                    write_atomic(
                        &out,
                        "# shellsync: the config has errors; run `shellsync check`\n",
                        None,
                    )?;
                }
            }
            Err(e)
        }
    }
}

fn check(paths: &Paths) -> Result {
    if !paths.config.exists() {
        return Err(format!(
            "{} does not exist (run `shellsync install`)",
            paths.config.display()
        ));
    }
    let cfg = load(&paths.config)?;
    println!(
        "{}: ok ({} env vars, {} path entries, {} aliases)",
        paths.config.display(),
        cfg.env.len(),
        cfg.path_prepend.len() + cfg.path_append.len(),
        cfg.aliases.len()
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

// -------------------------------------------------------------- hooks ----

#[derive(Clone, Copy)]
enum HookFile {
    Zshrc,
    NuEnv,
    NuConfig,
}

impl HookFile {
    fn parse(s: &str) -> Option<HookFile> {
        match s {
            "zsh" | "zshrc" => Some(HookFile::Zshrc),
            "nu-env" => Some(HookFile::NuEnv),
            "nu-config" => Some(HookFile::NuConfig),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            HookFile::Zshrc => "zshrc",
            HookFile::NuEnv => "nu env",
            HookFile::NuConfig => "nu config",
        }
    }
}

/// The snippet added to each shell's startup file. The fast path (config
/// unchanged) spawns no process: zsh compares mtimes with `-nt`, nushell with
/// its builtin `ls`; see `is_stale` for the rule. The binary is called by its
/// full path so the hook works before PATH is set up, and paths inside $HOME
/// are written relative to it so the hooks survive in synced dotfiles.
fn hook(kind: HookFile, paths: &Paths) -> Result<String> {
    let exe = env::current_exe().map_err(|e| format!("locating the shellsync binary: {e}"))?;
    let body = match kind {
        HookFile::Zshrc => {
            // The check runs in an anonymous function so its locals don't leak;
            // `source` runs at top level so the generated code isn't function-scoped.
            let q = zsh_path;
            let out = q(&paths.output(Shell::Zsh));
            format!(
                "() {{\n  \
                 local exe={exe} cfg={cfg} out={out}\n  \
                 if [[ ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ]]; then\n    \
                 $exe build --config $cfg --out-dir {dir}\n  \
                 fi\n\
                 }}\n\
                 [[ -r {out} ]] && source {out}\n",
                exe = q(&exe),
                cfg = q(&paths.config),
                dir = q(&paths.out_dir),
            )
        }
        HookFile::NuEnv => {
            let q = nu_path;
            format!(
                "do {{\n  \
                 let exe = {exe}\n  \
                 let cfg = {cfg}\n  \
                 let out = {out}\n  \
                 if (not ($out | path exists)) or (($cfg | path exists) and ((ls $cfg | get 0.modified) != (ls $out | get 0.modified))) {{\n    \
                 try {{ ^$exe build --config $cfg --out-dir {dir} }}\n  \
                 }}\n\
                 }}\n",
                exe = q(&exe),
                cfg = q(&paths.config),
                out = q(&paths.output(Shell::Nu)),
                dir = q(&paths.out_dir),
            )
        }
        // `source` needs a path known at parse time; `$nu.home-path` is a constant.
        HookFile::NuConfig => format!("source {}\n", nu_path(&paths.output(Shell::Nu))),
    };
    Ok(format!("{BEGIN}\n{body}{END}\n"))
}

fn home_relative(p: &Path) -> Option<String> {
    let home = home();
    let rel = p.strip_prefix(&home).ok()?.to_str()?;
    (home != Path::new("/") && !rel.is_empty()).then(|| rel.to_string())
}

fn nu_path(p: &Path) -> String {
    match home_relative(p) {
        Some(rel) => format!("($nu.home-path | path join {})", render::nu_quote(&rel)),
        None => render::nu_quote(&p.display().to_string()),
    }
}

fn zsh_path(p: &Path) -> String {
    match home_relative(p) {
        Some(rel) => render::zsh_template(&format!("~/{}", rel.replace('$', "$$"))),
        None => render::zsh_quote(&p.display().to_string()),
    }
}

fn nu_config_dir() -> Option<(PathBuf, PathBuf)> {
    // Ask nushell itself; it knows about XDG_CONFIG_HOME and platform defaults.
    let out = Command::new("nu")
        .args([
            "-n",
            "-c",
            "[$nu.env-path $nu.config-path] | str join (char nl)",
        ])
        .output()
        .ok()
        .filter(|o| o.status.success())?;
    let text = String::from_utf8(out.stdout).ok()?;
    let mut lines = text.lines().map(str::trim).filter(|l| !l.is_empty());
    Some((PathBuf::from(lines.next()?), PathBuf::from(lines.next()?)))
}

fn hook_files() -> Vec<(HookFile, PathBuf)> {
    let zdot = env::var_os("ZDOTDIR")
        .map(PathBuf::from)
        .unwrap_or_else(home);
    let (nu_env, nu_config) = nu_config_dir().unwrap_or_else(|| {
        let dir = if cfg!(target_os = "macos") && env::var_os("XDG_CONFIG_HOME").is_none() {
            home().join("Library/Application Support/nushell")
        } else {
            xdg("XDG_CONFIG_HOME", ".config").join("nushell")
        };
        (dir.join("env.nu"), dir.join("config.nu"))
    });
    vec![
        (HookFile::Zshrc, zdot.join(".zshrc")),
        (HookFile::NuEnv, nu_env),
        (HookFile::NuConfig, nu_config),
    ]
}

/// Returns `text` with the shellsync block replaced by `block` (or removed
/// when `block` is empty). A missing block is appended.
fn splice(text: &str, block: &str) -> String {
    let range = text.find(BEGIN).and_then(|start| {
        let end = start + text[start..].find(END)? + END.len();
        let end = if text[end..].starts_with('\n') {
            end + 1
        } else {
            end
        };
        Some(start..end)
    });
    match range {
        Some(r) => {
            let mut before = &text[..r.start];
            if block.is_empty() && before.ends_with("\n\n") {
                before = &before[..before.len() - 1]; // the separator install added
            }
            format!("{before}{block}{}", &text[r.end..])
        }
        None if block.is_empty() => text.to_string(),
        None => {
            let sep = match text {
                "" => "",
                t if t.ends_with("\n\n") => "",
                t if t.ends_with('\n') => "\n",
                _ => "\n\n",
            };
            format!("{text}{sep}{block}")
        }
    }
}

fn update_file(path: &Path, block: &str) -> Result<bool> {
    let old = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound && block.is_empty() => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let new = splice(&old, block);
    if new == old {
        return Ok(false);
    }
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(|e| format!("creating {}: {e}", dir.display()))?;
    }
    // Write in place (not via rename) so symlinked dotfiles stay symlinks.
    fs::write(path, new).map_err(|e| format!("writing {}: {e}", path.display()))?;
    Ok(true)
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
    for (kind, file) in hook_files() {
        let changed = update_file(&file, &hook(kind, paths)?)?;
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
    for (_, file) in hook_files() {
        if update_file(&file, "")? {
            println!("unhooked {}", file.display());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_appends_replaces_and_removes() {
        let block = format!("{BEGIN}\nnew\n{END}\n");
        assert_eq!(splice("", &block), block);
        assert_eq!(splice("a", &block), format!("a\n\n{block}"));
        assert_eq!(splice("a\n", &block), format!("a\n\n{block}"));

        let installed = format!("top\n{BEGIN}\nold\n{END}\nbottom\n");
        assert_eq!(splice(&installed, &block), format!("top\n{block}bottom\n"));
        assert_eq!(splice(&installed, ""), "top\nbottom\n");
        assert_eq!(splice(&splice("a\n", &block), ""), "a\n");
        assert_eq!(splice("untouched\n", ""), "untouched\n");
    }
}
