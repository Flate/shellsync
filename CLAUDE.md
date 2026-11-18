# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

shellsync is a Rust CLI (edition 2024) that turns one TOML file (`~/.config/shellsync/config.toml`) into native zsh and nushell startup code and hooks it into both shells. The README describes user-facing behaviour and the config format in full; keep it in sync when behaviour changes.

## Commands

```sh
cargo build
cargo test                                  # unit tests + end-to-end tests
cargo test splice                           # one test, by name substring
cargo test --test shells nu_end_to_end      # one end-to-end test
cargo test --bins                           # unit tests only (no real shells)
SHELLSYNC_REQUIRE_SHELLS=1 cargo test       # a missing shell fails instead of skipping (CI sets this)
cargo clippy --locked --all-targets -- -D warnings   # as in CI, together with `cargo fmt --check`
cargo run -- print zsh --config examples/example.toml   # inspect generated output
GITLAB_HOST=git.flatenet.de glab ci lint .gitlab-ci.yml   # validate CI changes before pushing
```

The end-to-end tests in `tests/shells.rs` run `shellsync install` into a throwaway `HOME` under the temp dir, then start real `zsh -i` / `nu` processes and check the resulting env, PATH and aliases. Without `SHELLSYNC_REQUIRE_SHELLS`, a test skips (prints a note) if its shell isn't installed. nushell's config location differs per platform (`~/Library/Application Support/nushell` on macOS without `XDG_CONFIG_HOME`), so tests ask `nu` for it instead of hard-coding a path.

CI (`.gitlab-ci.yml`, remote is a self-hosted GitLab at `git.flatenet.de`): `secrets` (gitleaks) and `lint` first, then `test:linux` (Docker, pinned Rust image, Debian zsh, nushell release tarball pinned by version and SHA-256) and `test:macos` (shell executor on the developer's Mac, tag `macos`, uses whatever Homebrew has installed). Image tags are pinned to explicit versions.

Project state lives in `STATUS.md`; user-facing changes go into `CHANGELOG.md` under `[Unreleased]`.

## Architecture

The pipeline is: TOML → `Config` → per-shell source text → cached files that shell hooks `source`.

- `src/config.rs`: the config model and parser. It walks a `toml::Table` by hand rather than using serde derive so that errors name the exact key (`env.X.fish`, `path.prepend[0]`) and key order is preserved (an env var may reference one defined above it). `Value` is either `All(String)` or `PerShell(BTreeMap<Target, String>)`, where a `Target` is one shell or `Sh` (every shell with `Shell::is_sh()`); a shell's own key beats `sh`, and a shell not covered skips the entry. `init` blocks don't override each other: `Entries::init(shell)` joins every block covering the shell, in file order. `Config` = top-level `Entries` plus `Vec<Tool>`; a `Tool` (`[tool.X]`) has its own `Entries` and `Condition`s (`Command` on PATH, `Executable` path, `Exists` path). The `section()` parser is shared by both levels.
- `src/probe.rs`: evaluates `Condition`s at build time against the current environment (`Probe::from_env`; tests use `Probe::with_vars` and real temp files). Only used for shells whose `Dialect::guard` returns `None`.
- `src/template.rs`: parses `~`, `$NAME`, `${NAME}`, `$$` in env/path values into `Seg::Lit`/`Seg::Var`. Variables are emitted as references in each shell's syntax and resolved at shell start, not at build time.
- `src/shell/mod.rs`: the `Shell` enum (`name()` is the config key and file suffix; `parse()` also accepts CLI aliases like `nushell`) and the shared renderer. `render_with::<D: Dialect>` writes, in this order: top-level env, path, aliases; one block per `[tool]` in file order; top-level init last. It asks the shell's `Dialect` for each line. `Dialect::guard` either wraps a tool block in a startup check (zsh: builtins only, no fork) or returns `None`, and then the renderer checks conditions with the `Probe` while building (nushell, which defines aliases at parse time). Build-time checks see PATH as the shell will: process PATH, plus `[path]`, plus the path entries of earlier tools that applied.
- `src/shell/<shell>.rs`: everything specific to one shell: its `Dialect` impl, quoting/escaping, hook bodies and where its startup files are. Alias bodies are passed through verbatim as shell code; only env/path values go through templating. Adding a shell means a new module, a `Shell` variant, a `render` match arm, and `HookFile` variants.
- `src/hooks.rs`: shell-independent hook handling: the `# >>> shellsync >>>` markers, `HookFile` (one per startup file; `block()` wraps the shell module's body), `files()` (where they go on this machine) and `splice`/`update_file`.
- `src/main.rs`: hand-rolled argument parsing, `Paths` and path resolution (`--config`/`$SHELLSYNC_CONFIG`/XDG), `build` and staleness, `install`/`uninstall`/`edit`.

Key invariants that span the Rust code and the generated shell hooks:

- **Staleness is mtime equality, not "newer".** `build` stamps each output with the config's mtime (taken *before* reading the config). `is_stale` in Rust, the zsh hook (`-nt` both ways) and the nu hook (`ls ... modified !=`) must all apply the same rule. Change one, change all three.
- **The fast path spawns no process.** Hooks compare timestamps with shell builtins and only call the binary when stale. Don't add work to the hooks that forks.
- **Errors keep the last good build.** On a parse error, existing outputs are left untouched and nothing is written.
- **No backward compatibility yet.** shellsync isn't installed anywhere, so the hook format, generated files and config schema can change freely; no migration code or compatibility shims.
- **Hooks must not break a shell when the binary is missing** (dotfiles synced to a machine without shellsync): the zsh hook checks `-x $exe`, the nu env hook wraps the build in `try`, and `config.nu` uses `source (if (… | path exists) {…} else { null })`, which nushell evaluates at parse time. Covered by `shells_start_quietly_without_the_binary`.
- **Outputs are written atomically** (temp file + rename) **with mode 0600**, since the config may hold secrets. Hook edits to dotfiles are written in place instead, so symlinked dotfiles stay symlinks.
- **nushell split:** the rebuild check lives in `env.nu` and `config.nu` only does `source`, because nushell parses `config.nu` completely before running it, so the sourced file must already exist; `env.nu` runs first and can build it.
- **Hooks are portable:** paths under `$HOME` are written as `~/...` (nu) or `"${HOME}/..."` (zsh) via each shell module's path quoting and `shell::home_relative`, so synced dotfiles work on machines with a different home path. `HookFile::block()` takes the binary path and home directory as parameters, so unit tests can pin its exact output.
- **Hook blocks** are delimited by `# >>> shellsync >>>` / `# <<< shellsync <<<`; `splice` replaces, appends or removes the block and must stay idempotent (covered by `install_is_idempotent_and_uninstall_restores`). A start marker without an end marker is an error, never a guess.

`examples/example.toml` is embedded with `include_str!` and written as the starter config by `install`/`edit`, so edits to it change what new users get.

The only dependency is `toml` (with `preserve_order`, which the parser relies on for key order).
