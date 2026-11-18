//! The marked blocks shellsync adds to each shell's startup files.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::shell::{nu, zsh};
use crate::{Paths, Result};

pub const BEGIN: &str = "# >>> shellsync >>>";
pub const END: &str = "# <<< shellsync <<<";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookFile {
    Zshrc,
    NuEnv,
    NuConfig,
}

impl HookFile {
    /// A hook as given on the command line.
    pub fn parse(s: &str) -> Option<HookFile> {
        match s {
            "zsh" | "zshrc" => Some(HookFile::Zshrc),
            "nu-env" => Some(HookFile::NuEnv),
            "nu-config" => Some(HookFile::NuConfig),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            HookFile::Zshrc => "zshrc",
            HookFile::NuEnv => "nu env",
            HookFile::NuConfig => "nu config",
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
            HookFile::Zshrc => zsh::rc_hook(paths, exe, home),
            HookFile::NuEnv => nu::env_hook(paths, exe, home),
            HookFile::NuConfig => nu::config_hook(paths, home),
        };
        format!("{BEGIN}\n{body}{END}\n")
    }
}

/// Where each hook goes on this machine.
pub fn files() -> Vec<(HookFile, PathBuf)> {
    let (nu_env, nu_config) = nu::config_files();
    vec![
        (HookFile::Zshrc, zsh::rc_file()),
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

/// Puts `block` into the file at `path` (or removes the block when `block`
/// is empty). Returns whether the file changed.
pub fn update_file(path: &Path, block: &str) -> Result<bool> {
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

    #[test]
    fn splice_refuses_a_block_without_end_marker() {
        let broken = format!("top\n{BEGIN}\nold\nuser content\n");
        let block = format!("{BEGIN}\nnew\n{END}\n");
        assert!(splice(&broken, &block).is_err());
        assert!(splice(&broken, "").is_err());
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
            HookFile::Zshrc.block(&paths, exe, home),
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
        let nu_env = HookFile::NuEnv.block(&paths, exe, home);
        assert!(nu_env.contains("let exe = ('~/.cargo/bin/shellsync' | path expand)"));
        assert!(nu_env.contains("try { ^$exe build"));
        assert_eq!(
            HookFile::NuConfig.block(&paths, exe, home),
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
        let home = Path::new("/home/me");
        let zsh = HookFile::Zshrc.block(&paths, exe, home);
        assert!(
            zsh.contains("local exe='/opt/homebrew/bin/shellsync' "),
            "{zsh}"
        );
        let nu = HookFile::NuEnv.block(&paths, exe, home);
        assert!(nu.contains("let exe = ('/opt/homebrew/bin/shellsync' | path expand)"));

        // With HOME=/ every path is "under home"; keep them absolute.
        let zsh = HookFile::Zshrc.block(&paths, exe, Path::new("/"));
        assert!(
            zsh.contains("cfg='/home/me/.config/shellsync/config.toml'"),
            "{zsh}"
        );
    }
}
