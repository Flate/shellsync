//! Checks `[tool]` conditions while the files are built, for shells that
//! can't check them in the generated code: nushell defines aliases while
//! parsing, so an alias can't depend on a runtime `if`. nushell's env.nu hook
//! checks again at startup and rebuilds with its own answer if it differs.

use std::env;
use std::path::{Path, PathBuf};

use crate::config::Condition;
use crate::template::{self, Seg};

/// Looks up an environment variable by name.
type VarLookup = Box<dyn Fn(&str) -> Option<String>>;

pub struct Probe {
    var: VarLookup,
}

impl Probe {
    /// Uses this process's environment, with HOME as shellsync resolves it.
    pub fn from_env() -> Probe {
        Probe {
            var: Box::new(|name| match name {
                "HOME" => Some(crate::home().display().to_string()),
                _ => env::var(name).ok(),
            }),
        }
    }

    /// Uses only the given variables.
    #[cfg(test)]
    pub fn with_vars(vars: &[(&str, &str)]) -> Probe {
        let vars: Vec<(String, String)> = vars
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        Probe {
            var: Box::new(move |name| vars.iter().find(|(k, _)| k == name).map(|(_, v)| v.clone())),
        }
    }

    /// `s` with template variables replaced by their values now. Unset
    /// variables become empty, as they do in the shells.
    pub fn expand(&self, s: &str) -> String {
        template::parse(s)
            .into_iter()
            .map(|seg| match seg {
                Seg::Lit(lit) => lit,
                Seg::Var(name) => (self.var)(&name).unwrap_or_default(),
            })
            .collect()
    }

    /// The directories in PATH now.
    pub fn path_dirs(&self) -> Vec<PathBuf> {
        (self.var)("PATH")
            .map(|p| env::split_paths(&p).collect())
            .unwrap_or_default()
    }

    /// Whether `condition` holds now, looking up commands in `path`.
    pub fn holds(&self, condition: &Condition, path: &[PathBuf]) -> bool {
        match condition {
            Condition::Command(name) => {
                let name = self.expand(name);
                path.iter().any(|dir| self.is_executable(&dir.join(&name)))
            }
            Condition::Executable(p) => self.is_executable(Path::new(&self.expand(p))),
            Condition::Exists(p) => Path::new(&self.expand(p)).exists(),
        }
    }

    /// Whether a file at `p` can run. On Windows that's a file whose extension
    /// is in PATHEXT, named with or without it, as the shells find commands.
    #[cfg(unix)]
    fn is_executable(&self, p: &Path) -> bool {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    }

    #[cfg(windows)]
    fn is_executable(&self, p: &Path) -> bool {
        let pathext = (self.var)("PATHEXT").unwrap_or_else(|| ".COM;.EXE;.BAT;.CMD".into());
        let exts: Vec<&str> = pathext.split(';').filter(|e| !e.is_empty()).collect();
        let has_ext = p.extension().is_some_and(|e| {
            let e = format!(".{}", e.to_string_lossy());
            exts.iter().any(|x| x.eq_ignore_ascii_case(&e))
        });
        (has_ext && p.is_file())
            || exts.iter().any(|ext| {
                let mut with_ext = p.as_os_str().to_owned();
                with_ext.push(ext);
                Path::new(&with_ext).is_file()
            })
    }
}

/// For tests: what an executable's file name ends with here.
#[cfg(test)]
pub const EXE: &str = if cfg!(windows) { ".cmd" } else { "" };

/// For tests: a temp dir with an executable `tool` (`tool.cmd` on Windows)
/// and a plain file `data`.
#[cfg(test)]
pub fn fixture(name: &str) -> PathBuf {
    let dir = env::temp_dir().join(format!("shellsync-probe-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("tool{EXE}")), "#!/bin/sh\n").unwrap();
    std::fs::write(dir.join("data"), "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir.join("tool"), std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(dir.join("data"), std::fs::Permissions::from_mode(0o644)).unwrap();
    }
    dir
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn expands_templates() {
        let probe = Probe::with_vars(&[("HOME", "/home/me"), ("X", "x")]);
        assert_eq!(probe.expand("~/a/$X/${UNSET}b$$"), "/home/me/a/x/b$");
    }

    #[test]
    fn checks_conditions() {
        let dir = fixture("conditions");
        let d = dir.display().to_string();
        let probe = Probe::with_vars(&[("D", &d)]);
        let path = vec![dir.clone()];
        let holds = |c: Condition| probe.holds(&c, &path);

        assert!(holds(Condition::Command("tool".into())));
        assert!(!holds(Condition::Command("data".into())), "not executable");
        assert!(!holds(Condition::Command("missing".into())));
        assert!(holds(Condition::Executable("$D/tool".into())));
        assert!(!holds(Condition::Executable("$D/data".into())));
        assert!(holds(Condition::Exists("$D/data".into())));
        assert!(holds(Condition::Exists("$D".into())), "directories count");
        assert!(!holds(Condition::Exists("$D/missing".into())));
        assert!(!probe.holds(&Condition::Command("tool".into()), &[]));
        fs::remove_dir_all(&dir).unwrap();
    }
}
