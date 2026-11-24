# Status

## Works

- zsh, bash, nushell and PowerShell 7 generation, hooks, install/uninstall, rebuild on config change
- Windows: nushell, PowerShell 7, Windows PowerShell 5.1 and bash in Git Bash, MSYS2 and Cygwin (no zsh)
- Tests pass on macOS (zsh 5.9, bash 3.2, nu 0.117.0, pwsh 7.7.0) and in the Linux CI image (zsh 5.9, bash 5.2, nu 0.117.0, pwsh 7.6.10)
- Tests pass on the Windows runner (pwsh 7.6.10, Windows PowerShell 5.1.26100, nu 0.117.0, Git Bash 5.3, MSYS2 bash 5.3, Cygwin bash 5.2)
- GitLab CI pipeline passes on GitLab, including `test:windows` and the release jobs
- Tests also pass on the Windows runner built with the GNU toolchain (`test:windows-gnu`, Rust 1.100.0, no Visual Studio Build Tools)
- Released 0.2.0 (`v0.2.0`, Windows PowerShell 5.1 support): binaries and `SHA256SUMS` in the package registry, which anyone can download from through the API URLs (the release page's `/downloads/` links ask for a login)
- 0.1.0 and 0.2.0 also on GitHub releases (`Flate/shellsync`), added by hand with `gh` from the GitLab package files (checksums verified); later releases come from `release:github`
- In daily use through the developer's dotfiles (version pinned there) on macOS, Linux and Windows
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–13b done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7, Windows, bash, bash on Windows).

## Known issues

- Windows PowerShell 5.1 with a symlinked config: the hook can't resolve the link without a cmdlet, so it runs `build --if-stale` at every start (about 10 ms); untested, since the Windows runner's user can't create symlinks
- bash before 5 compares file times in whole seconds: a config change in the same second as the last build is missed until the next change (documented)

## Next steps


- GitHub releases (`release:github`): untested until the next tag; needs the `GITHUB_TOKEN` CI/CD variable set first (see README, Development). A dry run on the default branch only prints what it would do
- Idea: cmd with clink. A generated Lua file sets env and PATH, aliases become `doskey` macros (`$*` for the arguments), a Lua hook in clink's scripts folder rebuilds when stale, `[tool]` checks via PATH/PATHEXT in Lua. To find out: whether clink's Lua API can read a file's mtime without a process, how `build` can parse-check the Lua and macros, config key `cmd` or `clink`

## Constraints

- Compatibility follows semver from 0.1.0 on: incompatible changes (config, hooks, generated files) only with a new minor version and a changelog entry; configs written for an older version stay readable (see `CLAUDE.md`)

## Open decisions

- None
