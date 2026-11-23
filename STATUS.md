# Status

## Works

- zsh, bash, nushell and PowerShell 7 generation, hooks, install/uninstall, rebuild on config change
- Windows: nushell, PowerShell 7, Windows PowerShell 5.1 and bash in Git Bash, MSYS2 and Cygwin (no zsh)
- Tests pass on macOS (zsh 5.9, bash 3.2, nu 0.117.0, pwsh 7.7.0) and in the Linux CI image (zsh 5.9, bash 5.2, nu 0.117.0, pwsh 7.6.10)
- Tests pass on the Windows runner (pwsh 7.6.10, Windows PowerShell 5.1.26100, nu 0.117.0, Git Bash 5.3, MSYS2 bash 5.3, Cygwin bash 5.2; run over SSH as `gitlab-ci`)
- GitLab CI pipeline passes on GitLab, including `test:windows` and the release jobs
- Released 0.1.0 (`v0.1.0`): binaries and `SHA256SUMS` in the package registry, which anyone can download from
- Installed through the dotfiles (`flate/dotfiles-new`, merged in !16) on the Mac, the Linux users and TrueNAS' `truenas_admin` (binary placed by hand there)
- Shells start normally when the binary or the generated files are missing

## In progress

Release 0.2.0 (Windows PowerShell 5.1 support, merged in !2): version and changelog set on branch `chore/release-0.2.0`; not tagged yet.

Improvement plan, steps 1–13b done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7, Windows, bash, bash on Windows).

## Known issues

- Windows PowerShell 5.1 with a symlinked config: the hook can't resolve the link without a cmdlet, so it runs `build --if-stale` at every start (about 10 ms); untested, since `gitlab-ci` can't create symlinks
- bash before 5 compares file times in whole seconds: a config change in the same second as the last build is missed until the next change (documented)

## Next steps

- Tag `v0.2.0` and check the release; then pin 0.2.0 in the dotfiles and add a `shellsync hook powershell` partial to their `.chezmoitemplates/shellsync/`

## Constraints

- Compatibility follows semver from 0.1.0 on: incompatible changes (config, hooks, generated files) only with a new minor version and a changelog entry; configs written for an older version stay readable (see `CLAUDE.md`)

## Open decisions

- None
