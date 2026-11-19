# Status

## Works

- zsh and nushell generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0) and in the Linux CI image (zsh 5.9, nu 0.117.0)
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner)
- Release pipeline (`v*` tags) and `ci/release.sh`: builds verified locally (Linux musl in the CI image, macOS arm64 on this Mac); publishing tested against a mock API only, not yet on GitLab; the dry run on `main` passes up to the missing changelog section
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–10 of 13 done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`, release builds, `check` parses with each shell).

## Known issues

- Only `check` parses the shell code: `build`/`edit` write an alias or init block that doesn't parse, and in nushell that stops the whole config from loading

## Next steps

1. PowerShell 7 (not Windows PowerShell), Windows (including a Windows release binary), bash
2. Dotfiles: shellsync config template and hook blocks (via `hook --exe`) in the chezmoi repo

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- Windows runner (tag `windows`) is registered but has no tools yet; needed from step 11/12 on
