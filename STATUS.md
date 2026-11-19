# Status

## Works

- zsh and nushell generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0) and in the Linux CI image (zsh 5.9, nu 0.117.0)
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner)
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–7b of 13 done (housekeeping, test fix, CI, file permissions, hook robustness, one module per shell with the `sh` key, `[tool.X]` conditions, nushell re-checks tools at startup).

## Known issues

- `install` edits `.zshrc` and the nushell config in place, which conflicts with chezmoi-managed dotfiles

## Next steps

1. `hook --exe`; `.zshenv`/`.zshrc` split
2. Release builds on `v*` tags (Linux x86_64, macOS arm64, Windows x86_64)
3. `check` validates the output with each shell's parser
4. PowerShell 7 (not Windows PowerShell), Windows, bash

## Constraints

- Not installed on any machine yet: no backward compatibility needed for hooks, outputs or config format

## Open decisions

- Windows CI runner: planned, not yet registered
