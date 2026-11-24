# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Release binaries and `SHA256SUMS` are also published as GitHub releases on the mirror, https://github.com/Flate/shellsync

## [0.2.0] - 2026-11-23

### Added

- Windows PowerShell 5.1: `powershell` key in per-shell values, `[powershell] init` section, generated `init-ps5.ps1`, `shellsync print powershell` and `shellsync hook powershell`; on Windows, `install` hooks its profile if it exists
- `ps` key in per-shell values and `[ps] init` section for PowerShell 7 and Windows PowerShell 5.1

### Changed

- On Windows, `build` and `check` also parse the generated code with Windows PowerShell 5.1, so plain-string aliases must parse there too (an alias that uses PowerShell 7 syntax such as `&&` now needs a per-shell value)
- `shellsync print powershell` prints Windows PowerShell 5.1's code instead of PowerShell 7's; use `pwsh` for PowerShell 7

## [0.1.0] - 2026-11-23

### Added

- `shellsync` CLI that generates zsh, bash, nushell and PowerShell 7 configuration from one TOML file: `install`, `uninstall`, `edit`, `build`, `check`, `print`, `hook`, `paths`
- `sh` key in per-shell values and `[sh] init` section for zsh and bash
- `[tool.X]` tables for entries that apply only where a tool is installed (`command`) or a path exists (`exists`); checked at every shell start
- zsh environment variables and PATH load from `.zshenv`, so scripts and other non-interactive zsh sessions get them too; aliases and init code load from `.zshrc`
- `[edit] command` in the config: what `shellsync edit` runs instead of opening the config, e.g. a dotfile manager's edit command
- `--exe` option for `install` and `hook`, to set the binary the hooks run, e.g. in dotfile templates
- `check` parses the generated code with each shell and names the config key of each error; `build` does the same and keeps the last good build if a shell can't parse it
- Generated files are readable only by the user (mode 0600)
- GitLab CI pipeline: lint, end-to-end tests on Linux, macOS and Windows, secret scan
- Windows support for nushell, PowerShell and bash (Git Bash, MSYS2, Cygwin)
- Release binaries for Linux x86_64, macOS arm64 and Windows x86_64 with `SHA256SUMS`, published to the GitLab package registry and releases on `v*` tags
