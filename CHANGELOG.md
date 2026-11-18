# Changelog

All notable changes to this project are documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- `shellsync` CLI that generates zsh and nushell configuration from one TOML file: `install`, `uninstall`, `edit`, `build`, `check`, `print`, `hook`, `paths`
- GitLab CI pipeline: lint, end-to-end tests on Linux and macOS, secret scan

### Fixed

- nushell no longer fails to start when the generated `init.nu` is missing; rerun `shellsync install` to update the hook
- The zsh hook no longer prints an error on every start when the shellsync binary is missing

### Security

- Generated files in the cache directory are created with mode 0600
