# shellsync

Configure **zsh** and **nushell** from one file.

You write environment variables, PATH entries and aliases once, in
`~/.config/shellsync/config.toml`. shellsync turns that file into native zsh and
nushell code and hooks it into both shells. After a one-time `install` you
don't run it yourself: edit the file and every new shell picks up the change.

```toml
[env]
EDITOR = "nvim"
GOPATH = "~/go"

[path]
prepend = ["~/.local/bin", "$GOPATH/bin"]

[aliases]
g  = "git"
gs = "git status"
ll = { zsh = "ls -lh", nu = "ls -l" }   # different per shell
```

## Install

Download the binary for your platform and `SHA256SUMS` from the
[releases](https://git.flatenet.de/flate/shellsync/-/releases), check it, and
put it on your PATH as `shellsync`. For example, on a Mac:

```sh
shasum -a 256 --check --ignore-missing SHA256SUMS   # Linux: sha256sum --check --ignore-missing SHA256SUMS
install -m 755 shellsync-aarch64-apple-darwin ~/.local/bin/shellsync
shellsync install
```

Releases have binaries for Linux x86_64 (static, any distribution), macOS on
Apple silicon and Windows x86_64. The Windows binary is built ahead of Windows
support: shellsync doesn't support Windows yet. To build it yourself instead,
run `cargo install --path .`, or `cargo build --release` and copy
`target/release/shellsync` onto your PATH.

`install` creates a starter config if you don't have one, builds it, and adds a
small marked block to `~/.zshenv` and `~/.zshrc` (or the ones in `$ZDOTDIR`)
and to nushell's `env.nu` and `config.nu`. Running it again is safe: the block
is replaced, not duplicated. Paths under your home directory are written as
`$HOME/...` or `~/...`, so the hooks still work if you sync your dotfiles to a
machine with a different home path. The binary's location is written as-is
unless it is under your home directory too, for example `~/.cargo/bin`.
`shellsync uninstall` removes the blocks and leaves everything else in those
files as it was.

**Dotfiles managed by a tool** such as chezmoi: `install` edits the files in
place, which the tool would overwrite or report as changed. Instead, put the
blocks into your templates. `shellsync hook zshenv` (and `zshrc`, `nu-env`,
`nu-config`) prints each one; `--exe` sets where the binary is, for example
`shellsync hook zshenv --exe ~/.local/bin/shellsync`, so the blocks don't
depend on where shellsync ran when you printed them.

## How it stays fast and invisible

* **Startup does almost nothing.** The hooks compare two file timestamps with
  the shell's own builtins (`[[ -nt ]]` in zsh, `ls` in nushell) and then
  `source` a cached file. No extra process starts unless the config has
  changed (or, for nushell, a [tool](#conditional-entries) was installed or
  removed). Measured cost: about 2.5 ms for a whole interactive zsh start, and
  about 3.5 ms added to a nushell start.
* **Rebuilds happen on their own.** When the config's timestamp differs from the
  cached files', the first shell to start runs `shellsync build` (a few
  milliseconds), then loads the result.
* **Output is plain shell code.** The generated files are in
  `~/.cache/shellsync/` (`env.zsh`, `init.zsh`, `init.nu`, and `tools.nu` for
  nushell's [tool checks](#conditional-entries)); `shellsync print zsh` and
  `shellsync print nu` show them. Only your user can read them, since the
  config may contain secrets.
* **Mistakes don't break your shell.** If the config has an error, the shell
  prints the error with its line number and keeps using the last good build.
  If the shellsync binary is missing, for example because your dotfiles were
  synced to a machine without it, shells start normally without the
  generated settings.
* **Writes are atomic,** so a shell that starts during a rebuild never reads a
  half-written file.

**zsh** loads the environment from `.zshenv`, which every zsh reads, also
scripts and cron jobs: environment variables and PATH are in `env.zsh`.
Aliases and init code are in `init.zsh`, which `.zshrc` loads in interactive
shells only. Only interactive shells rebuild, so a script uses the last build
and never prints a config error. `init.zsh` sets the PATH entries again,
because macOS login shells (the default in Terminal and iTerm) run
`path_helper` from `/etc/zprofile` between the two, which moves PATH entries
set earlier to the end.

**nushell** parses `config.nu` completely before running it, so the file it
`source`s has to exist beforehand. That's why the rebuild check lives in
`env.nu`, which nushell runs first, and why `config.nu` only sources the file
if it exists.

## Config reference

| Section     | Contents                                                                                                       |
| ----------- | -------------------------------------------------------------------------------------------------------------- |
| `[env]`     | `NAME = "value"`. Numbers and booleans are accepted too. Variables are set in the order they're written.       |
| `[path]`    | `prepend = [...]` and `append = [...]`. Duplicate entries are removed and the first occurrence is kept.         |
| `[aliases]` | `name = "command"`. The body is shell code and is used exactly as written.                                     |
| `[zsh]`     | `init = '''...'''`: raw zsh code added at the end of the generated file.                                        |
| `[nu]`      | `init = '''...'''`: raw nushell code added at the end of the generated file.                                    |
| `[sh]`      | `init = '''...'''`: raw code for every sh-family shell (currently zsh), combined with `[zsh] init` in file order. |
| `[tool.X]`  | Any of the above that apply only where a tool is installed; see [Conditional entries](#conditional-entries).    |

**Values in `[env]` and `[path]`** can use:

* `~` or `~/...` at the start for your home directory
* `$NAME` or `${NAME}` for another environment variable
* `$$` for a literal `$`

These references are written into each shell's own syntax and resolved when the
shell starts, so `"$GOPATH/bin"` becomes `"${GOPATH}/bin"` in zsh and
`$env.GOPATH`-based code in nushell. Quotes, backticks, parentheses and other
special characters are escaped for each shell.

**Per-shell values.** Any value can be a table: `{ zsh = "...", nu = "..." }`.
If you leave out a shell, that entry is skipped there. For example,
`ZSH_AUTOSUGGEST_STRATEGY = { zsh = "history" }` is set only in zsh. The key
`sh` stands for every shell with POSIX sh syntax (currently zsh); a shell's own
key takes precedence, so `{ sh = "ls -l", zsh = "ls -lh" }` gives zsh
`ls -lh`. TOML 1.1 allows such a table to span several lines:

```toml
ll = {
  sh = "ls -lh",
  nu = "ls -l",
}
```

Aliases are where the two shells differ most. Commands such as `git status` or
`nvim` work the same in both, but builtins (`ls`, `cd`) and shell syntax (`&&`,
`$(...)`) don't, so give those aliases a per-shell form. Anything more
elaborate, such as functions, prompts, completions, `setopt` or `$env.config`,
goes in the `init` blocks.

### Conditional entries

A `[tool.X]` table holds `env`, `path`, `aliases` and `init` entries that apply
only if a condition holds:

```toml
# Only where `eza` is on PATH and can run.
[tool.eza.aliases]
ls = "eza --long --header --group-directories-first"
l  = "ls"

# A command at a fixed location: the file must exist and be executable.
[tool.borg]
command = "/mnt/Private/bin/borg"
env.TMPDIR = "/mnt/Private/borg-tmp"
aliases.borg = "/mnt/Private/bin/borg"

# A file or directory must exist.
[tool.android-sdk]
exists = "~/Library/Android/sdk/platform-tools"
path.append = ["~/Library/Android/sdk/platform-tools"]

[tool.thefuck]
zsh.init = 'eval "$(thefuck --alias)"'
```

* `command = "name"`: a command with this name is on PATH and can run.
* `command = "/a/path"` (or `~/...`): that file exists and is executable.
* `exists = "path"`: a file or directory exists there.
* Without `command` or `exists`, the table's name is the command to look for.
* Both keys can take a list; then every entry must hold. Paths can use `~` and
  `$NAME` as in `[env]`.

The unconditional entries come first in the generated file, then each
`[tool]` table in the order of the config file, then the unconditional `init`
code. A tool's alias therefore overrides an unconditional one of the same name,
and a check sees PATH as set up by `[path]` and by earlier tools that applied,
but not by its own `path` entries. To make a directory's commands depend on the
directory, use `exists`.

**When conditions are checked.** Both shells check them each time they start,
so installing or removing a tool takes effect in the next shell. zsh checks
them in the generated file, using builtins only. nushell defines aliases while
it parses its config, before anything runs, so an alias can't depend on an
`if`. Instead, the hook in `env.nu`, which runs first, checks the conditions
(with nushell's built-in `which`; ten tools add about 0.4 ms) and compares the
result with the last build. Only if they differ does it run `shellsync build`,
and the generated file (`init.nu`, which notes each tool it skipped) is ready
before `config.nu` sources it.

nushell checks with the PATH it has when `env.nu` runs, plus shellsync's own
`[path]` entries. So if a tool's directory is added to PATH only by your zsh
startup files, for example Homebrew's `/opt/homebrew/bin` via `brew shellenv`,
nushell won't find tools there. Add such directories to `[path]`, so both
shells get them.

## Commands

```
shellsync install        set up hooks (creates the config if missing)
shellsync uninstall      remove hooks
shellsync edit           open the config in $VISUAL/$EDITOR, then rebuild
shellsync check          validate the config
shellsync print zsh|nu   show the generated code
shellsync hook zshenv|zshrc|nu-env|nu-config   print a hook block, for manual setup
shellsync paths          show the files involved
shellsync build          rebuild by hand (normally done automatically)
```

`--config FILE` and `--out-dir DIR` override the defaults. The config path can
also be set with `$SHELLSYNC_CONFIG`, and `$XDG_CONFIG_HOME` and
`$XDG_CACHE_HOME` are respected. `install` and `hook` write the resolved paths
into the hooks, so later changes to those variables don't affect existing
hooks. `--exe FILE` sets the binary the hooks run (default: the running one).

## Notes

* Changes apply to **new** shells. In an already open zsh, run `exec zsh`.
* zsh reads `$ZDOTDIR/.zshenv` only if ZDOTDIR is set before zsh starts. If
  your `~/.zshenv` sets ZDOTDIR, have it source `$ZDOTDIR/.zshenv`, or put the
  `zshenv` block into `~/.zshenv` yourself.
* After a config change, scripts get the new environment once an interactive
  zsh (or `shellsync build`) has rebuilt it.
* After upgrading shellsync, run `shellsync install` once. It regenerates the
  files and updates the hooks if the new version changed them.
* If `install` or `uninstall` reports a start marker (`# >>> shellsync >>>`)
  without an end marker, the block was damaged by hand. shellsync doesn't guess
  where it ends; fix or remove it yourself.

## Development

```sh
cargo test     # unit tests, plus end-to-end tests that start real zsh / nu
               # in a temporary HOME (skipped if a shell isn't installed)
SHELLSYNC_REQUIRE_SHELLS=1 cargo test   # fail instead of skipping
```

GitLab CI (`.gitlab-ci.yml`) runs a secret scan, `cargo fmt --check` and
`cargo clippy`, then the tests on Linux (zsh and a pinned nushell) and on a
macOS runner tagged `macos`.

To release a version:

1. Set `version` in `Cargo.toml` and run `cargo build`, so `Cargo.lock` follows.
2. In `CHANGELOG.md`, move the `[Unreleased]` entries into a new section
   `## [X.Y.Z] - YYYY-MM-DD`. `ci/release.sh check` now passes.
3. Commit and push. In the pipeline for the default branch, start the
   `build:*` jobs and then `release`, which only prints what it would publish.
4. Tag the commit `vX.Y.Z` and push the tag. The tag must be protected (protect
   `v*` under Settings > Repository > Protected tags). The tag pipeline runs the
   tests, builds the binaries, uploads them with `SHA256SUMS` to the project's
   package registry (package `shellsync`, version `X.Y.Z`) and creates the
   GitLab release with the changelog section as its notes.

Re-running a release job is safe: files already uploaded are kept, and a
release job that would replace one with different contents fails instead.

## License

shellsync is licensed under the GNU General Public License v3.0 or later.
See [LICENSE](LICENSE).
