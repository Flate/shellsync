# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `shellsync` CLI that generates zsh and nushell configuration from one TOML file: `install`, `uninstall`, `edit`, `build`, `check`, `print`, `hook`, `paths`
- `sh` key in per-shell values and `[sh] init` section for all sh-family shells (currently zsh)
- `[tool.X]` tables for entries that apply only where a tool is installed (`command`) or a path exists (`exists`); checked at every shell start
- zsh environment variables and PATH load from `.zshenv`, so scripts and other non-interactive zsh sessions get them too; aliases and init code load from `.zshrc`
- `--exe` option for `install` and `hook`, to set the binary the hooks run, e.g. in dotfile templates
- Generated files are readable only by the user (mode 0600)
- GitLab CI pipeline: lint, end-to-end tests on Linux and macOS, secret scan
