# Status

## Works

- zsh, bash, nushell and PowerShell 7 generation, hooks, install/uninstall, rebuild on config change
- Windows: nushell, PowerShell and bash in Git Bash, MSYS2 and Cygwin (no zsh)
- Tests pass on macOS (zsh 5.9, bash 3.2, nu 0.117.0, pwsh 7.7.0) and in the Linux CI image (zsh 5.9, bash 5.2, nu 0.117.0, pwsh 7.6.10)
- Tests pass on the Windows runner (pwsh 7.6.10, nu 0.117.0, Git Bash 5.3, MSYS2 bash 5.3, Cygwin bash 5.2; run over SSH as `gitlab-ci`)
- GitLab CI pipeline passes on GitLab, including `test:windows` and the release jobs
- Released 0.1.0 (`v0.1.0`): binaries and `SHA256SUMS` in the package registry, which anyone can download from
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–13b done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7, Windows, bash, bash on Windows).

## Known issues

- bash before 5 compares file times in whole seconds: a config change in the same second as the last build is missed until the next change (documented)

## Next steps

1. Dotfiles (`flate/dotfiles-new`, branch `feature/shellsync`): shared config, hooks for all four shells, binary via chezmoi external, pinned to 0.1.0; merge once its pipeline passes. `truenas_admin` needs the binary in `/mnt/Private/bin/.local/bin` by hand

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- None
