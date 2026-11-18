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
            print!("{}", hook(kind, &paths, &this_exe()?, &home()));
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
    for shell in Shell::ALL {
        write_atomic(
            &paths.output(shell),
            &render::render(&cfg, shell, &source),
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
/// full path so the hook works before PATH is set up, and paths inside `home`
/// are written relative to it so the hooks survive in synced dotfiles. Synced
/// dotfiles can also reach a machine without the binary, so a missing binary
/// or output is skipped silently.
fn hook(kind: HookFile, paths: &Paths, exe: &Path, home: &Path) -> String {
    let body = match kind {
        HookFile::Zshrc => {
            // The check runs in an anonymous function so its locals don't leak;
            // `source` runs at top level so the generated code isn't function-scoped.
            let q = |p: &Path| zsh_path(p, home);
            let out = q(&paths.output(Shell::Zsh));
            format!(
                "() {{\n  \
                 local exe={exe} cfg={cfg} out={out}\n  \
                 if [[ -x $exe && ( ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
                 $exe build --config $cfg --out-dir {dir}\n  \
                 fi\n\
                 }}\n\
                 [[ -r {out} ]] && source {out}\n",
                exe = q(exe),
                cfg = q(&paths.config),
                dir = q(&paths.out_dir),
            )
        }
        HookFile::NuEnv => {
            // `try` also covers a missing binary.
            let q = |p: &Path| nu_path(p, home);
            format!(
                "do {{\n  \
                 let exe = ({exe} | path expand)\n  \
                 let cfg = ({cfg} | path expand)\n  \
                 let out = ({out} | path expand)\n  \
                 if (not ($out | path exists)) or (($cfg | path exists) and ((ls $cfg | get 0.modified) != (ls $out | get 0.modified))) {{\n    \
                 try {{ ^$exe build --config $cfg --out-dir ({dir} | path expand) }}\n  \
                 }}\n\
                 }}\n",
                exe = q(exe),
                cfg = q(&paths.config),
                out = q(&paths.output(Shell::Nu)),
                dir = q(&paths.out_dir),
            )
        }
        // `source` resolves its path at parse time, so the existence check has
        // to be a constant expression; `source null` does nothing.
        HookFile::NuConfig => {
            let out = nu_path(&paths.output(Shell::Nu), home);
            format!("source (if ({out} | path exists) {{ {out} }} else {{ null }})\n")
        }
    };
    format!("{BEGIN}\n{body}{END}\n")
}

fn this_exe() -> Result<PathBuf> {
    env::current_exe().map_err(|e| format!("locating the shellsync binary: {e}"))
}

fn home_relative(p: &Path, home: &Path) -> Option<String> {
    let rel = p.strip_prefix(home).ok()?.to_str()?;
    (home != Path::new("/") && !rel.is_empty()).then(|| rel.to_string())
}

fn nu_path(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
        Some(rel) => render::nu_quote(&format!("~/{rel}")),
        None => render::nu_quote(&p.display().to_string()),
    }
}

fn zsh_path(p: &Path, home: &Path) -> String {
    match home_relative(p, home) {
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
/// when `block` is empty). A missing block is appended. A start marker
/// without an end marker is an error: where the block ends is a guess, and a
/// wrong guess would delete the user's own lines.
fn splice(text: &str, block: &str) -> Result<String> {
    let Some(start) = text.find(BEGIN) else {
        if block.is_empty() {
            return Ok(text.to_string());
        }
        let sep = match text {
            "" => "",
            t if t.ends_with("\n\n") => "",
            t if t.ends_with('\n') => "\n",
            _ => "\n\n",
        };
        return Ok(format!("{text}{sep}{block}"));
    };
    let end = text[start..]
        .find(END)
        .map(|i| start + i + END.len())
        .ok_or_else(|| {
            format!("found `{BEGIN}` without `{END}`; fix or remove the block by hand")
        })?;
    let end = if text[end..].starts_with('\n') {
        end + 1
    } else {
        end
    };
    let mut before = &text[..start];
    if block.is_empty() && before.ends_with("\n\n") {
        before = &before[..before.len() - 1]; // the separator install added
    }
    Ok(format!("{before}{block}{}", &text[end..]))
}

fn update_file(path: &Path, block: &str) -> Result<bool> {
    let old = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound && block.is_empty() => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let new = splice(&old, block).map_err(|e| format!("{}: {e}", path.display()))?;
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
    let exe = this_exe()?;
    for (kind, file) in hook_files() {
        let changed = update_file(&file, &hook(kind, paths, &exe, &home()))?;
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
        let splice = |text: &str, block: &str| super::splice(text, block).unwrap();
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

    fn paths_under(home: &str) -> Paths {
        Paths {
            config: PathBuf::from(format!("{home}/.config/shellsync/config.toml")),
            out_dir: PathBuf::from(format!("{home}/.cache/shellsync")),
        }
    }

    #[test]
    fn hooks_write_paths_relative_to_home() {
        let home = Path::new("/home/me");
        let paths = paths_under("/home/me");
        let exe = Path::new("/home/me/.cargo/bin/shellsync");
        assert_eq!(
            hook(HookFile::Zshrc, &paths, exe, home),
            format!(
                "{BEGIN}\n\
                 () {{\n  \
                 local exe=\"${{HOME}}/.cargo/bin/shellsync\" cfg=\"${{HOME}}/.config/shellsync/config.toml\" out=\"${{HOME}}/.cache/shellsync/init.zsh\"\n  \
                 if [[ -x $exe && ( ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
                 $exe build --config $cfg --out-dir \"${{HOME}}/.cache/shellsync\"\n  \
                 fi\n\
                 }}\n\
                 [[ -r \"${{HOME}}/.cache/shellsync/init.zsh\" ]] && source \"${{HOME}}/.cache/shellsync/init.zsh\"\n\
                 {END}\n"
            )
        );
        let nu_env = hook(HookFile::NuEnv, &paths, exe, home);
        assert!(nu_env.contains("let exe = ('~/.cargo/bin/shellsync' | path expand)"));
        assert!(nu_env.contains("try { ^$exe build"));
        assert_eq!(
            hook(HookFile::NuConfig, &paths, exe, home),
            format!(
                "{BEGIN}\n\
                 source (if ('~/.cache/shellsync/init.nu' | path exists) {{ '~/.cache/shellsync/init.nu' }} else {{ null }})\n\
                 {END}\n"
            )
        );
    }

    #[test]
    fn hooks_keep_paths_outside_home_absolute() {
        let paths = paths_under("/home/me");
        let exe = Path::new("/opt/homebrew/bin/shellsync");
        let zsh = hook(HookFile::Zshrc, &paths, exe, Path::new("/home/me"));
        assert!(
            zsh.contains("local exe='/opt/homebrew/bin/shellsync' "),
            "{zsh}"
        );
        let nu = hook(HookFile::NuEnv, &paths, exe, Path::new("/home/me"));
        assert!(nu.contains("let exe = ('/opt/homebrew/bin/shellsync' | path expand)"));

        // With HOME=/ every path is "under home"; keep them absolute.
        let zsh = hook(HookFile::Zshrc, &paths, exe, Path::new("/"));
        assert!(
            zsh.contains("cfg='/home/me/.config/shellsync/config.toml'"),
            "{zsh}"
        );
    }

    #[test]
    fn hook_paths_are_quoted_for_each_shell() {
        let home = Path::new("/home/me");
        let odd = Path::new("/home/me/it's $x/bin");
        assert_eq!(zsh_path(odd, home), r#""${HOME}/it's \$x/bin""#);
        assert_eq!(nu_path(odd, home), "r#'~/it's $x/bin'#");
        let outside = Path::new("/opt/it's");
        assert_eq!(zsh_path(outside, home), r"'/opt/it'\''s'");
    }

    #[test]
    fn splice_refuses_a_block_without_end_marker() {
        let broken = format!("top\n{BEGIN}\nold\nuser content\n");
        let block = format!("{BEGIN}\nnew\n{END}\n");
        assert!(splice(&broken, &block).is_err());
        assert!(splice(&broken, "").is_err());
    }

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
