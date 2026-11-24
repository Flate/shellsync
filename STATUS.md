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
- cmd with clink (branch `feat/cmd-clink`, not merged): generation, hook, parse check; tests pass on the Windows runner with clink 1.9.37, which runs the hook in `clink lua`

## In progress

Improvement plan, steps 1–13b done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7, Windows, bash, bash on Windows).

cmd with clink on `feat/cmd-clink`: not yet tried in an interactive cmd (the tests can't inject clink: it needs a console), so the first-prompt rebuild and `clink.reload()`, and the hook's startup cost, are unverified there.

## Known issues

- Windows PowerShell 5.1 with a symlinked config: the hook can't resolve the link without a cmdlet, so it runs `build --if-stale` at every start (about 10 ms); untested, since the Windows runner's user can't create symlinks
- bash before 5 and clink compare file times in whole seconds: a config change in the same second as the last build is missed until the next change (documented)
- clink with a symlinked config: clink reports the link's own time, so the hook runs `build --if-stale` at every start; untested and unmeasured, as for Windows PowerShell 5.1

## Next steps

- GitHub releases (`release:github`): untested until the next tag; needs the `GITHUB_TOKEN` CI/CD variable set first (see README, Development). A dry run on the default branch only prints what it would do
- cmd with clink: try it in an interactive cmd on Windows (`shellsync install`, then a new cmd; edit the config, new cmd; Ctrl-X Ctrl-R), measure the hook's startup cost, then merge and update the dotfiles' hook partials (`shellsync hook clink`)

## Constraints

- Compatibility follows semver from 0.1.0 on: incompatible changes (config, hooks, generated files) only with a new minor version and a changelog entry; configs written for an older version stay readable (see `CLAUDE.md`)

## Open decisions

- None
