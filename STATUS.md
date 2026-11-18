# Status

## Works

- zsh and nushell generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0) and in the Linux CI image (zsh 5.9, nu 0.117.0)
- GitLab CI pipeline passes on GitLab (Linux and the `macos` runner)
- Shells start normally when the binary or the generated files are missing

## In progress

Improvement plan, steps 1–5 of 13 done (housekeeping, test fix, CI, file permissions, hook robustness).

## Known issues

- `install` edits `.zshrc` and the nushell config in place, which conflicts with chezmoi-managed dotfiles
- Existing installs keep their old hooks until `shellsync install` is run again

## Next steps

1. Refactor for more than two shells; per-shell values with an `sh` key (zsh + bash)
2. Conditional entries as `[tool.X]` tables (`command`, `exists`); `hook --exe`; `.zshenv`/`.zshrc` split
3. Release builds on `v*` tags (Linux x86_64, macOS arm64, Windows x86_64)
4. `check` validates the output with each shell's parser
5. PowerShell 7 (not Windows PowerShell), Windows, bash

## Open decisions

- Windows CI runner: planned, not yet registered
