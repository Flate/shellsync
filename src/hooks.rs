//! The marked blocks shellsync adds to each shell's startup files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::shell::{Shell, bash, cmd, nu, powershell, pwsh, zsh};
use crate::{Paths, Result};

pub const BEGIN: &str = "# >>> shellsync >>>";
pub const END: &str = "# <<< shellsync <<<";
/// The markers in Lua, whose comments start with `--`.
pub const LUA_BEGIN: &str = "-- >>> shellsync >>>";
pub const LUA_END: &str = "-- <<< shellsync <<<";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookFile {
    Zshenv,
    Zshrc,
    Bashrc,
    NuEnv,
    NuConfig,
    PwshProfile,
    PowershellProfile,
    ClinkScript,
}

impl HookFile {
    /// A hook as given on the command line.
    pub fn parse(s: &str) -> Option<HookFile> {
        match s {
            "zshenv" => Some(HookFile::Zshenv),
            "zshrc" => Some(HookFile::Zshrc),
            "bashrc" => Some(HookFile::Bashrc),
            "nu-env" => Some(HookFile::NuEnv),
            "nu-config" => Some(HookFile::NuConfig),
            "pwsh" => Some(HookFile::PwshProfile),
            "powershell" => Some(HookFile::PowershellProfile),
            "clink" => Some(HookFile::ClinkScript),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HookFile::Zshenv => "zshenv",
            HookFile::Zshrc => "zshrc",
            HookFile::Bashrc => "bashrc",
            HookFile::NuEnv => "nu env",
            HookFile::NuConfig => "nu config",
            HookFile::PwshProfile => "pwsh profile",
            HookFile::PowershellProfile => "ps5 profile",
            HookFile::ClinkScript => "clink script",
        }
    }

    /// The lines that start and end the block, as comments of the file's
    /// language.
    pub fn markers(self) -> (&'static str, &'static str) {
        match self {
            HookFile::ClinkScript => (LUA_BEGIN, LUA_END),
            _ => (BEGIN, END),
        }
    }

    /// The marked block for this file.
    ///
    /// The fast path (config unchanged) spawns no process: the hooks compare
    /// mtimes with shell builtins; see `is_stale` for the rule. The binary is
    /// called by its full path so the hook works before PATH is set up, and
    /// paths inside `home` are written relative to it so the hooks survive in
    /// synced dotfiles. Synced dotfiles can also reach a machine without the
    /// binary, so a missing binary or output is skipped silently.
    pub fn block(self, paths: &Paths, exe: &Path, home: &Path) -> String {
        let body = match self {
            HookFile::Zshenv => zsh::env_hook(paths, exe, home),
            HookFile::Zshrc => zsh::rc_hook(paths, home),
            HookFile::Bashrc => bash::rc_hook(paths, exe, home),
            HookFile::NuEnv => nu::env_hook(paths, exe, home),
            HookFile::NuConfig => nu::config_hook(paths, home),
            HookFile::PwshProfile => pwsh::profile_hook(paths, exe, home),
            HookFile::PowershellProfile => powershell::profile_hook(paths, exe, home),
            HookFile::ClinkScript => cmd::script_hook(paths, exe, home),
        };
        let (begin, end) = self.markers();
        format!("{begin}\n{body}{end}\n")
    }
}

/// Where each hook goes on this machine, for the shells native to it. On
/// Windows, `.bashrc` only if it exists: most Windows users have no bash,
/// and Git Bash, MSYS2 and Cygwin each have their own. The same goes for
/// Windows PowerShell's profile: every Windows has 5.1, but by default it
/// runs no scripts, and would complain at every start about a new profile.
/// cmd's hook goes into clink's profile folder, only if clink made it.
pub fn files() -> Vec<(HookFile, PathBuf)> {
    let mut files = Vec::new();
    if Shell::Zsh.native() {
        files.push((HookFile::Zshenv, zsh::env_file()));
        files.push((HookFile::Zshrc, zsh::rc_file()));
    }
    let bashrc = bash::rc_file();
    if !cfg!(windows) || bashrc.exists() {
        files.push((HookFile::Bashrc, bashrc));
    }
    let (nu_env, nu_config) = nu::config_files();
    files.push((HookFile::NuEnv, nu_env));
    files.push((HookFile::NuConfig, nu_config));
    files.push((HookFile::PwshProfile, pwsh::profile_file()));
    if Shell::Powershell.native() {
        let profile = powershell::profile_file();
        if profile.exists() {
            files.push((HookFile::PowershellProfile, profile));
        }
    }
    if Shell::Cmd.native() {
        let script = cmd::script_file();
        if script.parent().is_some_and(Path::is_dir) {
            files.push((HookFile::ClinkScript, script));
        }
    }
    files
}

/// Returns `text` with the shellsync block, delimited by `markers`, replaced
/// by `block` (or removed when `block` is empty). A missing block is
/// appended. A start marker without an end marker is an error: where the
/// block ends is a guess, and a wrong guess would delete the user's own lines.
fn splice(text: &str, block: &str, (begin, end): (&str, &str)) -> Result<String> {
    let Some(start) = text.find(begin) else {
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
        .find(end)
        .map(|i| start + i + end.len())
        .ok_or_else(|| {
            format!("found `{begin}` without `{end}`; fix or remove the block by hand")
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

/// Puts `block` into the file at `path`, a `kind` file (or removes the block
/// when `block` is empty). Returns whether the file changed.
pub fn update_file(path: &Path, kind: HookFile, block: &str) -> Result<bool> {
    let old = match fs::read_to_string(path) {
        Ok(s) => s,
        Err(e) if e.kind() == io::ErrorKind::NotFound && block.is_empty() => return Ok(false),
        Err(e) if e.kind() == io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("reading {}: {e}", path.display())),
    };
    let new =
        splice(&old, block, kind.markers()).map_err(|e| format!("{}: {e}", path.display()))?;
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splice_appends_replaces_and_removes() {
        let splice = |text: &str, block: &str| super::splice(text, block, (BEGIN, END)).unwrap();
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

    #[test]
    fn splice_refuses_a_block_without_end_marker() {
        let broken = format!("top\n{BEGIN}\nold\nuser content\n");
        let block = format!("{BEGIN}\nnew\n{END}\n");
        assert!(splice(&broken, &block, (BEGIN, END)).is_err());
        assert!(splice(&broken, "", (BEGIN, END)).is_err());
    }

    #[test]
    fn splice_uses_the_markers_of_the_language() {
        let lua = (LUA_BEGIN, LUA_END);
        let block = format!("{LUA_BEGIN}\nnew\n{LUA_END}\n");
        let installed = splice("-- mine\n", &block, lua).unwrap();
        assert_eq!(installed, format!("-- mine\n\n{block}"));
        let again = format!("{LUA_BEGIN}\nnewer\n{LUA_END}\n");
        assert_eq!(
            splice(&installed, &again, lua).unwrap(),
            format!("-- mine\n\n{again}")
        );
        assert_eq!(splice(&installed, "", lua).unwrap(), "-- mine\n");
        // A `#` block is someone else's text here, and the other way round.
        assert_eq!(splice(&installed, "", (BEGIN, END)).unwrap(), installed);
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
            HookFile::Zshenv.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 () {{\n  \
                 local exe=\"${{HOME}}/.cargo/bin/shellsync\" cfg=\"${{HOME}}/.config/shellsync/config.toml\" out=\"${{HOME}}/.cache/shellsync/env.zsh\" rc=\"${{HOME}}/.cache/shellsync/init.zsh\"\n  \
                 if [[ -o interactive && -x $exe && ( ! -e $out || ! -e $rc || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
                 $exe build --config $cfg --out-dir \"${{HOME}}/.cache/shellsync\"\n  \
                 fi\n\
                 }}\n\
                 [[ -r \"${{HOME}}/.cache/shellsync/env.zsh\" ]] && source \"${{HOME}}/.cache/shellsync/env.zsh\"\n\
                 {END}\n"
            )
        );
        assert_eq!(
            HookFile::Zshrc.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 [[ -r \"${{HOME}}/.cache/shellsync/init.zsh\" ]] && source \"${{HOME}}/.cache/shellsync/init.zsh\"\n\
                 {END}\n"
            )
        );
        // bash on Windows: the user profile, see `bash::path_word`.
        let h = if cfg!(windows) { "USERPROFILE" } else { "HOME" };
        assert_eq!(
            HookFile::Bashrc.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 __shellsync_build() {{\n  \
                 local exe=\"${{{h}}}/.cargo/bin/shellsync\" cfg=\"${{{h}}}/.config/shellsync/config.toml\" out=\"${{{h}}}/.cache/shellsync/init.bash\"\n  \
                 if [[ $- == *i* && -x $exe && ( ! -e $out || ( -e $cfg && ( $cfg -nt $out || $out -nt $cfg ) ) ) ]]; then\n    \
                 \"$exe\" build --config \"$cfg\" --out-dir \"${{{h}}}/.cache/shellsync\"\n  \
                 fi\n\
                 }}\n\
                 __shellsync_build; unset -f __shellsync_build\n\
                 [[ -r \"${{{h}}}/.cache/shellsync/init.bash\" ]] && source \"${{{h}}}/.cache/shellsync/init.bash\"\n\
                 {END}\n"
            )
        );
        let nu_env = HookFile::NuEnv.block(&paths, exe, home);
        assert!(nu_env.contains("let exe = ('~/.cargo/bin/shellsync' | path expand)"));
        assert!(nu_env.contains("try { ^$exe build --config $cfg --out-dir $dir }"));
        assert!(nu_env.contains(
            "source (if ('~/.cache/shellsync/tools.nu' | path exists) { '~/.cache/shellsync/tools.nu' } else { null })"
        ));
        assert!(nu_env.contains("--nu-tools ...$tools.found"));
        assert_eq!(
            HookFile::NuConfig.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 source (if ('~/.cache/shellsync/init.nu' | path exists) {{ '~/.cache/shellsync/init.nu' }} else {{ null }})\n\
                 {END}\n"
            )
        );
        assert_eq!(
            HookFile::PwshProfile.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 & {{\n  \
                 $exe = \"${{HOME}}/.cargo/bin/shellsync\"; $cfg = \"${{HOME}}/.config/shellsync/config.toml\"; $out = \"${{HOME}}/.cache/shellsync/init.ps1\"\n  \
                 if ([IO.File]::Exists($exe) -and (-not [IO.File]::Exists($out) -or ([IO.File]::Exists($cfg) -and ([IO.File]::ResolveLinkTarget($cfg, $true) ?? [IO.FileInfo]::new($cfg)).LastWriteTimeUtc -ne [IO.File]::GetLastWriteTimeUtc($out)))) {{\n    \
                 & $exe build --config $cfg --out-dir \"${{HOME}}/.cache/shellsync\"\n  \
                 }}\n\
                 }}\n\
                 if ([IO.File]::Exists(\"${{HOME}}/.cache/shellsync/init.ps1\")) {{ . \"${{HOME}}/.cache/shellsync/init.ps1\" }}\n\
                 {END}\n"
            )
        );
        assert_eq!(
            HookFile::PowershellProfile.block(&paths, exe, home),
            format!(
                "{BEGIN}\n\
                 & {{\n  \
                 $exe = \"${{HOME}}/.cargo/bin/shellsync\"; $cfg = \"${{HOME}}/.config/shellsync/config.toml\"; $out = \"${{HOME}}/.cache/shellsync/init-ps5.ps1\"\n  \
                 if ([IO.File]::Exists($exe) -and (-not [IO.File]::Exists($out) -or ([IO.File]::Exists($cfg) -and ([IO.File]::GetAttributes($cfg).HasFlag([IO.FileAttributes]::ReparsePoint) -or [IO.File]::GetLastWriteTimeUtc($cfg) -ne [IO.File]::GetLastWriteTimeUtc($out))))) {{\n    \
                 & $exe build --if-stale --config $cfg --out-dir \"${{HOME}}/.cache/shellsync\"\n  \
                 }}\n\
                 }}\n\
                 if ([IO.File]::Exists(\"${{HOME}}/.cache/shellsync/init-ps5.ps1\")) {{ . \"${{HOME}}/.cache/shellsync/init-ps5.ps1\" }}\n\
                 {END}\n"
            )
        );
    }

    #[test]
    fn clink_hook_is_lua() {
        let paths = paths_under("/home/me");
        let exe = Path::new("/home/me/.cargo/bin/shellsync");
        let home = r#"(os.getenv("USERPROFILE") or "")"#;
        assert_eq!(
            HookFile::ClinkScript.block(&paths, exe, Path::new("/home/me")),
            format!(
                "{LUA_BEGIN}\n\
                 if os.setalias then\n  \
                 local exe, cfg, dir, out = {home} .. \"\\\\.cargo\\\\bin\\\\shellsync\", {home} .. \"\\\\.config\\\\shellsync\\\\config.toml\", {home} .. \"\\\\.cache\\\\shellsync\", {home} .. \"\\\\.cache\\\\shellsync\\\\init.lua\"\n  \
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
                 end\n\
                 {LUA_END}\n"
            )
        );
    }

    #[test]
    fn hooks_keep_paths_outside_home_absolute() {
        let paths = paths_under("/home/me");
        let exe = Path::new("/opt/homebrew/bin/shellsync");
        let home = Path::new("/home/me");
        let zsh = HookFile::Zshenv.block(&paths, exe, home);
        assert!(
            zsh.contains("local exe='/opt/homebrew/bin/shellsync' "),
            "{zsh}"
        );
        let bash = HookFile::Bashrc.block(&paths, exe, home);
        assert!(
            bash.contains("local exe='/opt/homebrew/bin/shellsync' "),
            "{bash}"
        );
        let nu = HookFile::NuEnv.block(&paths, exe, home);
        assert!(nu.contains("let exe = ('/opt/homebrew/bin/shellsync' | path expand)"));
        let pwsh = HookFile::PwshProfile.block(&paths, exe, home);
        assert!(
            pwsh.contains("$exe = '/opt/homebrew/bin/shellsync';"),
            "{pwsh}"
        );
        let ps5 = HookFile::PowershellProfile.block(&paths, exe, home);
        assert!(
            ps5.contains("$exe = \"/opt/homebrew/bin/shellsync\";"),
            "{ps5}"
        );
        let clink = HookFile::ClinkScript.block(&paths, exe, home);
        assert!(
            clink.contains("local exe, cfg, dir, out = \"/opt/homebrew/bin/shellsync\", "),
            "{clink}"
        );

        // With HOME=/ every path is "under home"; keep them absolute.
        let zsh = HookFile::Zshenv.block(&paths, exe, Path::new("/"));
        assert!(
            zsh.contains("cfg='/home/me/.config/shellsync/config.toml'"),
            "{zsh}"
        );
    }
}
