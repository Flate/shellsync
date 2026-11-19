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
ci/release.sh check                         # Cargo.toml, CHANGELOG.md (and in CI the tag) agree on a release
```

The end-to-end tests in `tests/shells.rs` run `shellsync install` into a throwaway `HOME` under the temp dir, then start real `zsh -i` / `nu` processes and check the resulting env, PATH and aliases. Without `SHELLSYNC_REQUIRE_SHELLS`, a test skips (prints a note) if its shell isn't installed. nushell's config location differs per platform (`~/Library/Application Support/nushell` on macOS without `XDG_CONFIG_HOME`), so tests ask `nu` for it instead of hard-coding a path.

CI (`.gitlab-ci.yml`, remote is a self-hosted GitLab at `git.flatenet.de`): `secrets` (gitleaks) and `lint` first, then `test:linux` (Docker, pinned Rust image, Debian zsh, nushell release tarball pinned by version and SHA-256) and `test:macos` (shell executor on the developer's Mac, tag `macos`, uses whatever Homebrew has installed). Image tags are pinned to explicit versions. On `v*` tags (and by hand, as a dry run, on the default branch) `build:*` jobs build `x86_64-unknown-linux-musl` and `x86_64-pc-windows-gnu` in the Rust image and `aarch64-apple-darwin` on the Mac, and `release` runs `ci/release.sh publish`: uploads the binaries and `SHA256SUMS` to the generic package registry with `CI_JOB_TOKEN` and creates the GitLab release from the `CHANGELOG.md` section. It never replaces an uploaded file, so it is safe to re-run. The release steps for the user are in the README under Development. The Windows runner (tag `windows`) has no tools installed yet; ask the user to install what a job needs (winget).

Project state lives in `STATUS.md`; user-facing changes go into `CHANGELOG.md` under `[Unreleased]`.

## Architecture

The pipeline is: TOML → `Config` → per-shell source text → cached files that shell hooks `source`.

- `src/config.rs`: the config model and parser. It walks a `toml::Table` by hand rather than using serde derive so that errors name the exact key (`env.X.fish`, `path.prepend[0]`) and key order is preserved (an env var may reference one defined above it). `Value` is either `All(String)` or `PerShell(BTreeMap<Target, String>)`, where a `Target` is one shell or `Sh` (every shell with `Shell::is_sh()`); a shell's own key beats `sh`, and a shell not covered skips the entry. `init` blocks don't override each other: `Entries::init(shell)` joins every block covering the shell, in file order. `Config` = top-level `Entries` plus `Vec<Tool>`; a `Tool` (`[tool.X]`) has its own `Entries` and `Condition`s (`Command` on PATH, `Executable` path, `Exists` path). The `section()` parser is shared by both levels.
- `src/probe.rs`: evaluates `Condition`s at build time against the current environment (`Probe::from_env`; tests use `Probe::with_vars` and real temp files). Only used for shells whose `Dialect::guard` returns `None`, and only when the build wasn't given the shell's own answer (`Checks::Now` vs `Checks::Found`).
- `src/template.rs`: parses `~`, `$NAME`, `${NAME}`, `$$` in env/path values into `Seg::Lit`/`Seg::Var`. Variables are emitted as references in each shell's syntax and resolved at shell start, not at build time.
- `src/shell/mod.rs`: the `Shell` enum (`name()` is the config key; `parse()` also accepts CLI aliases like `nushell`; `files()` lists the generated files and which `Parts` (env, path, interactive) go into each: zsh `env.zsh` = env + path, `init.zsh` = path again + aliases + init; nu one `init.nu` with everything) and the shared renderer. `render_with::<D: Dialect>` decides each tool once, then writes every file in this order: top-level entries of its parts; one block per `[tool]` in file order (skipped if empty for those parts); top-level init last (interactive parts only). It asks the shell's `Dialect` for each line. `Dialect::guard` either wraps a tool block in a startup check (zsh: builtins only, no fork) or returns `None` (nushell, which defines aliases at parse time), and then the build decides per `Checks`: `Found` is the list the shell checked itself (`build --nu-tools ...`), `Now` checks with the `Probe` (see `Decider`: PATH as the shell will have it, i.e. process PATH, plus `[path]`, plus the path entries of earlier tools that applied). `render` returns the files' text plus the tools that applied.
- `src/shell/nu.rs` also writes `tools.nu` (`tools_check`): the nu tool conditions as nushell code, with the env/path lines they may depend on, evaluating to `{built: [...], found: [...]}`. The `env.nu` hook sources it in its own scope at every start (about 0.4 ms for 10 tools, measured with hyperfine) and runs `build --nu-tools ...$found` when `found != built`. Since the build then uses nushell's answer as given, it can't loop.
- `src/shell/<shell>.rs`: everything specific to one shell: its `Dialect` impl, quoting/escaping, hook bodies and where its startup files are. Alias bodies are passed through verbatim as shell code; only env/path values go through templating. Adding a shell means a new module, a `Shell` variant, a `render` match arm, and `HookFile` variants.
- `src/hooks.rs`: shell-independent hook handling: the `# >>> shellsync >>>` markers, `HookFile` (one per startup file; `block()` wraps the shell module's body), `files()` (where they go on this machine) and `splice`/`update_file`.
- `src/main.rs`: hand-rolled argument parsing, `Paths` and path resolution (`--config`/`$SHELLSYNC_CONFIG`/XDG), `build` and staleness, `install`/`uninstall`/`edit`.

Key invariants that span the Rust code and the generated shell hooks:

- **Staleness is mtime equality, not "newer".** `build` stamps each output with the config's mtime (taken *before* reading the config). `is_stale` in Rust (all outputs), the zsh `.zshenv` hook (`-nt` both ways on `env.zsh`) and the nu hook (`ls ... modified !=` on `init.nu`) must all apply the same rule. Change one, change all three.
- **The fast path spawns no process.** Hooks compare timestamps with shell builtins and only call the binary when stale. Don't add work to the hooks that forks.
- **Errors keep the last good build.** On a parse error, existing outputs are left untouched and nothing is written.
- **No backward compatibility yet.** shellsync isn't installed anywhere, so the hook format, generated files and config schema can change freely; no migration code or compatibility shims.
- **Hooks must not break a shell when the binary is missing** (dotfiles synced to a machine without shellsync): the zsh hook checks `-x $exe`, the nu env hook wraps the build in `try`, and `config.nu` uses `source (if (… | path exists) {…} else { null })`, which nushell evaluates at parse time. Covered by `shells_start_quietly_without_the_binary`.
- **Outputs are written atomically** (temp file + rename) **with mode 0600**, since the config may hold secrets. Hook edits to dotfiles are written in place instead, so symlinked dotfiles stay symlinks.
- **nushell split:** the rebuild check lives in `env.nu` and `config.nu` only does `source`, because nushell parses `config.nu` completely before running it, so the sourced file must already exist; `env.nu` runs first and can build it. The same applies to the `[tool]` re-check: a rebuild in `env.nu` takes effect in the same start. `source` paths must be constant, hence `source (if (… | path exists) {…} else { null })` for both `init.nu` and `tools.nu`.
- **zsh split:** `.zshenv` (every zsh, also scripts) sources `env.zsh` and rebuilds only if `[[ -o interactive ]]`, so scripts never fork the binary or print config errors; `.zshrc` only sources `init.zsh`. `init.zsh` repeats the path lines because macOS login shells run `path_helper` (`/etc/zprofile`) between the two and move earlier entries to the end; `typeset -U` makes the repeat idempotent. Covered by `zsh_scripts_get_the_environment_only` and `zsh_login_shells_keep_path_order`.
- **Hooks are portable:** paths under `$HOME` are written as `~/...` (nu) or `"${HOME}/..."` (zsh) via each shell module's path quoting and `shell::home_relative`, so synced dotfiles work on machines with a different home path. `HookFile::block()` takes the binary path and home directory as parameters, so unit tests can pin its exact output; `--exe` sets the binary for `install`/`hook` (for dotfile templates).
- **Hook blocks** are delimited by `# >>> shellsync >>>` / `# <<< shellsync <<<`; `splice` replaces, appends or removes the block and must stay idempotent (covered by `install_is_idempotent_and_uninstall_restores`). A start marker without an end marker is an error, never a guess.

`examples/example.toml` is embedded with `include_str!` and written as the starter config by `install`/`edit`, so edits to it change what new users get.

The only dependency is `toml` (with `preserve_order`, which the parser relies on for key order).
