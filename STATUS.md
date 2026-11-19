# Status

## Works

- zsh, nushell and PowerShell 7 generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0, pwsh 7.7.0) and in the Linux CI image (zsh 5.9, nu 0.117.0, pwsh 7.6.10)
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner)
- Release pipeline (`v*` tags) and `ci/release.sh`: builds verified locally (Linux musl in the CI image, macOS arm64 on this Mac); publishing tested against a mock API only, not yet on GitLab; the dry run on `main` passes up to the missing changelog section
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–11 of 13 done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` and `build` parse with each shell, PowerShell 7).

## Known issues

- None known

## Next steps

1. Windows (including a Windows release binary), then bash
2. Dotfiles: shellsync config template and hook blocks (via `hook --exe`) in the chezmoi repo

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- Windows runner (tag `windows`) is registered but has no tools yet; needed from step 11/12 on
