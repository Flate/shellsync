mod config;
mod hooks;
mod probe;
mod shell;
mod template;

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::SystemTime;

use config::{Config, Entries};
use hooks::HookFile;
use probe::Probe;
use shell::{Checks, Shell};

const USAGE: &str = "\
shellsync - configure zsh, bash, nushell and PowerShell from one file

Usage: shellsync <command> [options]

Commands:
  install          Create the config if missing, build, and hook into
                   ~/.zshenv, ~/.zshrc, ~/.bashrc, nushell's env.nu /
                   config.nu and PowerShell's profile.ps1
  uninstall        Remove the hooks (config and cache are kept)
  edit             Open the config in $VISUAL / $EDITOR (or run the config's
                   [edit] command instead), then rebuild
  build            Regenerate the shell files (shells do this automatically)
  check            Validate the config, and parse the generated code with
                   each shell (where installed)
  print <shell>    Print the generated code for zsh, bash, nu or pwsh
  hook <file>      Print the hook for zshenv, zshrc, bashrc, nu-env, nu-config
                   or pwsh
  paths            Show the files shellsync reads and writes

Options:
  --config <file>   Config file  [default: $SHELLSYNC_CONFIG or
                    ~/.config/shellsync/config.toml]
  --out-dir <dir>   Where generated files go  [default: ~/.cache/shellsync]
  --exe <file>      (install, hook) The shellsync binary the hooks run
                    [default: this binary]
  --if-stale        (build) Only rebuild if the config changed
  --nu-tools [<tool>...]
                    (build) The [tool] tables whose conditions hold in
                    nushell; must come last. nushell's hook passes this.
";

const EXAMPLE_CONFIG: &str = include_str!("../examples/example.toml");

type Result<T = ()> = std::result::Result<T, String>;

struct Paths {
    config: PathBuf,
    out_dir: PathBuf,
}

impl Paths {
    /// A generated file, by name (see `Shell::files`).
    fn output(&self, name: &str) -> PathBuf {
        self.out_dir.join(name)
    }

    /// Every generated file.
    fn outputs(&self) -> Vec<PathBuf> {
        Shell::ALL
            .iter()
            .flat_map(|s| s.files())
            .map(|(name, _)| self.output(name))
            .chain([self.nu_tools()])
            .collect()
    }

    /// nushell's `[tool]` checks, see `shell::nu::tools_check`.
    fn nu_tools(&self) -> PathBuf {
        self.output(shell::nu::TOOLS_FILE)
    }
}

/// The home directory, as the shells see it: on Windows the user profile
/// (PowerShell's `$HOME`; nushell asks Windows for the same folder).
fn home() -> PathBuf {
    let var = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
    env::var_os(var)
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
    let mut nu_tools = None;
    let mut exe = None;
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
            "--exe" => exe = Some(PathBuf::from(it.next().ok_or("--exe needs a file")?)),
            "--if-stale" => if_stale = true,
            "--nu-tools" => nu_tools = Some(it.by_ref().collect::<Vec<_>>()),
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

    let absolute = |p: PathBuf| std::path::absolute(&p).unwrap_or(p);
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

    let exe = || match &exe {
        Some(p) => Ok(absolute(p.clone())),
        None => this_exe(),
    };

    let shell_arg = |pos: &[String]| -> Result<Shell> {
        let s = pos
            .get(1)
            .ok_or("expected a shell: zsh, bash, nu or pwsh")?;
        Shell::parse(s)
            .ok_or_else(|| format!("unknown shell `{s}` (expected zsh, bash, nu or pwsh)"))
    };

    match positional.first().map(String::as_str) {
        None => {
            print!("{USAGE}");
            Ok(())
        }
        Some("build") => build(&paths, if_stale, nu_tools.as_deref()).map(|_| ()),
        Some("check") => check(&paths),
        Some("print") => {
            let shell = shell_arg(&positional)?;
            let cfg = load(&paths.config)?;
            let out = shell::render(
                &cfg,
                shell,
                &paths.config.display().to_string(),
                &Checks::Now(&Probe::from_env()),
            );
            for (i, (name, text)) in out.files.iter().enumerate() {
                if out.files.len() > 1 {
                    println!("{}# ===== {name}", if i > 0 { "\n" } else { "" });
                }
                print!("{text}");
            }
            Ok(())
        }
        Some("hook") => {
            let which = positional
                .get(1)
                .ok_or("expected zshenv, zshrc, bashrc, nu-env, nu-config or pwsh")?;
            let kind = HookFile::parse(which).ok_or_else(|| format!("unknown hook `{which}`"))?;
            print!("{}", kind.block(&paths, &exe()?, &home()));
            Ok(())
        }
        Some("install") => install(&paths, &exe()?),
        Some("uninstall") => uninstall(),
        Some("edit") => edit(&paths),
        Some("paths") => {
            println!("{:<12} {}", "config", paths.config.display());
            for file in paths.outputs() {
                let name = file.file_name().unwrap_or_default().to_string_lossy();
                println!("{name:<12} {}", file.display());
            }
            for (kind, file) in hooks::files() {
                println!("{:<12} {}", kind.name(), file.display());
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
    paths.outputs().iter().any(|out| match (mtime(out), cfg) {
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

/// Every generated file: each shell's files, then nushell's tool checks.
fn generate(cfg: &Config, source: &str, checks: &Checks) -> Vec<(Shell, &'static str, String)> {
    let mut files = Vec::new();
    let mut nu_applied = Vec::new();
    for shell in Shell::ALL {
        let out = shell::render(cfg, shell, source, checks);
        files.extend(
            out.files
                .into_iter()
                .map(|(name, text)| (shell, name, text)),
        );
        if shell == Shell::Nu {
            nu_applied = out.applied;
        }
    }
    let tools = shell::nu::tools_check(cfg, &nu_applied);
    files.push((Shell::Nu, shell::nu::TOOLS_FILE, tools));
    files
}

/// Regenerates the outputs. Returns whether anything was written.
/// `nu_tools` are the `[tool]` tables nushell found when it started; without
/// it, conditions are checked in this process's environment.
///
/// On a config error, or if a shell can't parse the code generated for it,
/// the previous outputs are left untouched, so shells keep working with the
/// last good config.
fn build(paths: &Paths, if_stale: bool, nu_tools: Option<&[String]>) -> Result<bool> {
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
    let checks = match nu_tools {
        Some(found) => Checks::Found(found),
        None => Checks::Now(&probe),
    };
    let files = generate(&cfg, &source, &checks);
    let failed: String = parse_all(&cfg, &files, &paths.out_dir)?
        .into_iter()
        .filter_map(|(shell, errors)| match errors {
            Some(errors) if !errors.is_empty() => Some(report(shell, &errors)),
            _ => None,
        })
        .collect();
    if !failed.is_empty() {
        return Err(format!(
            "the generated code doesn't parse, so the shells keep the last good build\n{}",
            failed.trim_end()
        ));
    }
    for (_, name, text) in files {
        write_atomic(&paths.output(name), &text, stamp)?;
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
        "{}: {} env vars, {} path entries, {} aliases, {} tools",
        paths.config.display(),
        count(|e| e.env.len()),
        count(|e| e.path_prepend.len() + e.path_append.len()),
        count(|e| e.aliases.len()),
        cfg.tools.len()
    );

    // As if every tool applied, so all of them are checked, installed here or not.
    let all: Vec<String> = cfg.tools.iter().map(|t| t.name.clone()).collect();
    let files = generate(&cfg, "check", &Checks::Found(&all));
    let mut count = 0;
    for (shell, errors) in parse_all(&cfg, &files, &paths.out_dir)? {
        match errors {
            Some(errors) => {
                count += errors.len();
                print!("{}", report(shell, &errors));
            }
            None => println!("{}: not installed, not checked", shell.name()),
        }
    }
    match count {
        0 => Ok(()),
        1 => Err("1 parse error".into()),
        n => Err(format!("{n} parse errors")),
    }
}

/// A parse error: what it is in (a config key or a generated file) and the
/// shell's message.
type ParseError = (String, String);

/// Parses the generated `files` with the shell each is for, for the shells
/// native to this platform, in a directory made in `out_dir`. Returns each
/// shell's errors, or None if the shell isn't installed.
fn parse_all(
    cfg: &Config,
    files: &[(Shell, &str, String)],
    out_dir: &Path,
) -> Result<Vec<(Shell, Option<Vec<ParseError>>)>> {
    let dir = TempDir::new(out_dir)?;
    Shell::ALL
        .into_iter()
        .filter(|shell| shell.native())
        .map(|shell| {
            let files: Vec<_> = files
                .iter()
                .filter(|(s, ..)| *s == shell)
                .map(|(_, name, text)| (*name, text.as_str()))
                .collect();
            Ok((shell, parse_shell(cfg, shell, &files, &dir.0)?))
        })
        .collect()
}

/// Parses `code` with `shell` (at `exe`), from a file named `name` in `dir`.
/// Returns the shell's error message if it doesn't parse.
fn parse(shell: Shell, exe: &Path, dir: &Path, name: &str, code: &str) -> Result<Option<String>> {
    write_atomic(&dir.join(name), code, None)?;
    let out = shell
        .syntax_check(exe, name)
        .current_dir(dir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("running {}: {e}", shell.name()))?;
    if out.status.success() {
        return Ok(None);
    }
    let msg = String::from_utf8_lossy(&out.stderr).trim_end().to_string();
    Ok(Some(if msg.is_empty() {
        out.status.to_string()
    } else {
        msg
    }))
}

/// Parses the files generated for `shell` and the config's raw code for it
/// with the shell itself. None if the shell isn't installed.
///
/// Errors are reported by config key where possible: if a generated file
/// doesn't parse, each alias and init block is parsed on its own to find the
/// culprit. Code that only parses in context (a nushell variable from an
/// earlier block) is then reported too.
fn parse_shell(
    cfg: &Config,
    shell: Shell,
    files: &[(&str, &str)],
    dir: &Path,
) -> Result<Option<Vec<ParseError>>> {
    let sh = shell.name();
    // Looked up here: on Linux, running a missing program can fail like a
    // program that exits with 127, not with "not found". And on Windows,
    // `Command` would look in the system folders before PATH.
    let probe = Probe::from_env();
    let Some(exe) = probe.find(sh, &probe.path_dirs()) else {
        return Ok(None);
    };
    if shell == Shell::Bash && shell::bash::is_wsl_launcher(&exe) {
        return Ok(None);
    }
    let mut file_errors = Vec::new();
    for (name, text) in files {
        if let Some(msg) = parse(shell, &exe, dir, name, text)? {
            file_errors.push((
                format!("{name} (generated, see `shellsync print {sh}`)"),
                msg,
            ));
        }
    }
    let mut errors = Vec::new();
    let mut explained = false;
    for (i, snippet) in shell::snippets(cfg, shell).into_iter().enumerate() {
        if snippet.in_files && file_errors.is_empty() {
            continue;
        }
        let name = format!("snippet-{i}.{sh}");
        if let Some(msg) = parse(shell, &exe, dir, &name, &snippet.code)? {
            explained |= snippet.in_files;
            errors.push((snippet.key.clone(), msg.replace(&name, &snippet.key)));
        }
    }
    if !explained {
        errors.extend(file_errors);
    }
    Ok(Some(errors))
}

/// `shell`'s line in `check`'s output, followed by the errors.
fn report(shell: Shell, errors: &[ParseError]) -> String {
    let sh = shell.name();
    let mut out = match errors.len() {
        0 => format!("{sh}: ok\n"),
        1 => format!("{sh}: 1 error\n"),
        n => format!("{sh}: {n} errors\n"),
    };
    for (what, msg) in errors {
        // zsh's and PowerShell's messages start with the file name, which is
        // the key already.
        if msg.starts_with(what.as_str()) {
            for line in msg.lines() {
                out += &format!("  {line}\n");
            }
            continue;
        }
        out += &format!("  {what}:\n");
        for line in msg.lines() {
            out += &format!("    {line}\n");
        }
    }
    out
}

/// A directory only the user can read, removed when dropped. It is made in
/// `parent`, the output directory: the files in it may hold secrets from the
/// config, and the temp directory may be readable by others (`/tmp` in MSYS2
/// and Cygwin, without the mode set here on Unix) or not writable at all.
struct TempDir(PathBuf);

impl TempDir {
    fn new(parent: &Path) -> Result<TempDir> {
        fs::create_dir_all(parent).map_err(|e| format!("creating {}: {e}", parent.display()))?;
        let dir = parent.join(format!(".parse-{}", std::process::id()));
        // A leftover from an earlier process with the same id.
        let _ = fs::remove_dir_all(&dir);
        #[cfg_attr(not(unix), allow(unused_mut))]
        let mut builder = fs::DirBuilder::new();
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
        builder
            .create(&dir)
            .map_err(|e| format!("creating {}: {e}", dir.display()))?;
        Ok(TempDir(dir))
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Opens the config in the user's editor, or runs the config's `[edit]
/// command` instead (for a config that a dotfile manager writes from a
/// template, its own edit command), then rebuilds. A config with errors is
/// opened in the editor, so that it can be fixed.
fn edit(paths: &Paths) -> Result {
    ensure_config(paths)?;
    let command = match load(&paths.config) {
        Ok(cfg) => cfg.edit_command,
        Err(e) => {
            eprintln!("shellsync: {e}\nshellsync: opening the file in your editor");
            None
        }
    };
    let (what, mut cmd) = match command {
        Some(command) => (command.clone(), shell_command(&command)),
        None => {
            let editor = ["VISUAL", "EDITOR"]
                .iter()
                .find_map(|v| env::var(v).ok().filter(|s| !s.trim().is_empty()))
                .unwrap_or_else(|| if cfg!(windows) { "notepad" } else { "vi" }.into());
            let cmd = editor_command(&editor, &paths.config);
            (editor, cmd)
        }
    };
    let status = cmd.status().map_err(|e| format!("running {what}: {e}"))?;
    if !status.success() {
        return Err(format!("{what} exited with {status}"));
    }
    build(paths, false, None)?;
    let zsh_hint = if cfg!(windows) {
        ""
    } else {
        "; in zsh you can also run `exec zsh`"
    };
    println!("Rebuilt. New shells pick up the changes{zsh_hint}.");
    Ok(())
}

/// Runs `editor` through the system shell, so values with arguments
/// (`code --wait`) work.
#[cfg(unix)]
fn editor_command(editor: &str, file: &Path) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c")
        .arg(format!("{editor} \"$1\""))
        .arg("sh")
        .arg(file);
    c
}

/// Runs `command` through the system shell.
#[cfg(unix)]
fn shell_command(command: &str) -> Command {
    let mut c = Command::new("sh");
    c.arg("-c").arg(command);
    c
}

/// `cmd /S /C "<command>"`, passed raw; see `editor_command`.
#[cfg(windows)]
fn shell_command(command: &str) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    c.args(["/S", "/C"]).raw_arg(format!("\"{command}\""));
    c
}

/// `cmd /S /C "<editor> "<file>""`: with /S, cmd drops the outer quotes and
/// runs the rest as typed. Passed raw, since cmd doesn't parse arguments the
/// way Rust quotes them. Windows paths can't contain `"`.
#[cfg(windows)]
fn editor_command(editor: &str, file: &Path) -> Command {
    use std::os::windows::process::CommandExt;
    let mut c = Command::new("cmd");
    c.args(["/S", "/C"])
        .raw_arg(format!("\"{editor} \"{}\"\"", file.display()));
    c
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

fn install(paths: &Paths, exe: &Path) -> Result {
    let mut stdout = io::stdout();
    if ensure_config(paths)? {
        writeln!(
            stdout,
            "created  {} (an example to edit)",
            paths.config.display()
        )
        .ok();
    }
    build(paths, false, None)?;
    writeln!(stdout, "built    {}", paths.out_dir.display()).ok();
    for (kind, file) in hooks::files() {
        let changed = hooks::update_file(&file, &kind.block(paths, exe, &home()))?;
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

#[cfg(all(test, unix))]
mod tests {
    use super::*;

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
        let old = paths.output("init.zsh");
        fs::write(&old, "").unwrap();
        fs::set_permissions(&old, fs::Permissions::from_mode(0o644)).unwrap();

        build(&paths, false, None).unwrap();
        for file in paths.outputs() {
            let mode = fs::metadata(&file).unwrap().permissions().mode();
            assert_eq!(format!("{:o}", mode & 0o777), "600", "{}", file.display());
        }
        fs::remove_dir_all(&dir).unwrap();
    }
}
