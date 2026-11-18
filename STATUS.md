# Status

## Works

- zsh and nushell generation, hooks, install/uninstall, rebuild on config change
- Tests pass on macOS (zsh 5.9, nu 0.117.0)

## In progress

Improvement plan, steps 1–2 of 13 done (housekeeping, test fix).

## Known issues

- nushell fails to start if the generated `init.nu` is missing (e.g. hooks deployed via dotfiles, binary not installed)
- zsh hook prints an error on every start if the binary has moved
- A hook block with a missing end marker is duplicated by `install` and ignored by `uninstall`
- `install` edits `.zshrc` and the nushell config in place, which conflicts with chezmoi-managed dotfiles

## Next steps

1. GitLab CI pipeline; generated files readable only by the user
2. Fix the known issues above, with unit tests for hook generation
2. Refactor for more than two shells; per-shell values with an `sh` key (zsh + bash)
3. Conditional entries as `[tool.X]` tables (`command`, `exists`); `hook --exe`; `.zshenv`/`.zshrc` split
4. Release builds on `v*` tags (Linux x86_64, macOS arm64, Windows x86_64)
5. `check` validates the output with each shell's parser
6. PowerShell 7 (not Windows PowerShell), Windows, bash

## Open decisions

- Windows CI runner: planned, not yet registered
