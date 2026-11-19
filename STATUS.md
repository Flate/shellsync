# Status

## Works

- zsh and nushell generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0) and in the Linux CI image (zsh 5.9, nu 0.117.0)
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner)
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–8 of 13 done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup, `.zshenv`/`.zshrc` split and `--exe`).

## Known issues

- None known

## Next steps

1. Release builds on `v*` tags (Linux x86_64, macOS arm64, Windows x86_64)
2. `check` validates the output with each shell's parser
3. PowerShell 7 (not Windows PowerShell), Windows, bash
4. Dotfiles: shellsync config template and hook blocks (via `hook --exe`) in the chezmoi repo

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- Windows CI runner: planned, not yet registered
