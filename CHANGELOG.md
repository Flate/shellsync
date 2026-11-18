# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `shellsync` CLI that generates zsh and nushell configuration from one TOML file: `install`, `uninstall`, `edit`, `build`, `check`, `print`, `hook`, `paths`
- `sh` key in per-shell values and `[sh] init` section for all sh-family shells (currently zsh)
- Generated files are readable only by the user (mode 0600)
- GitLab CI pipeline: lint, end-to-end tests on Linux and macOS, secret scan
