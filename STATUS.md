# Status

## Works

- zsh, nushell and PowerShell 7 generation, hooks, install/uninstall, rebuild on config change
- Windows: nushell and PowerShell (no zsh)
- Tests pass on macOS (zsh 5.9, nu 0.117.0, pwsh 7.7.0) and in the Linux CI image (zsh 5.9, nu 0.117.0, pwsh 7.6.10)
- On the Windows runner (pwsh 7.6.10, nu 0.116.1, run over SSH as `gitlab-ci`) all tests pass except one that needs nu 0.117.0, see Known issues
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner); `test:windows` and `build:windows` not yet run there
- Release pipeline (`v*` tags) and `ci/release.sh`: builds verified locally (Linux musl in the CI image, macOS arm64 on this Mac, Windows MSVC with static CRT on the Windows runner); publishing tested against a mock API only, not yet on GitLab; the dry run on `main` passes up to the missing changelog section
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–12 of 13 done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7, Windows).

## Known issues

- The Windows runner has nu 0.116.1, below the 0.117.0 baseline; `check_parses_the_code_with_each_shell` fails there until it is upgraded (0.116 still allows aliases to pipelines)

## Next steps

1. bash
2. Dotfiles: shellsync config template and hook blocks (via `hook --exe`) in the chezmoi repo

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- None
