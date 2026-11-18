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

```sh
cargo install --path .     # or: cargo build --release, then copy target/release/shellsync onto your PATH
shellsync install
```

`install` creates a starter config if you don't have one, builds it, and adds a
small marked block to `~/.zshrc` (or `$ZDOTDIR/.zshrc`) and to nushell's
`env.nu` and `config.nu`. Running it again is safe: the block is replaced, not
duplicated. `shellsync uninstall` removes the blocks and leaves everything else
in those files as it was.

## How it stays fast and invisible

* **Startup does almost nothing.** The hooks compare two file timestamps with
  the shell's own builtins (`[[ -nt ]]` in zsh, `ls` in nushell) and then
  `source` a cached file. No extra process starts unless the config has
  changed. Measured in zsh: about 2.5 ms for a full interactive start.
* **Rebuilds happen on their own.** When the config's timestamp differs from the
  cached files', the first shell to start runs `shellsync build` (a few
  milliseconds), then loads the result.
* **Output is plain shell code.** The generated files are in
  `~/.cache/shellsync/` (`init.zsh`, `init.nu`); `shellsync print zsh` and
  `shellsync print nu` show them.
* **Mistakes don't break your shell.** If the config has an error, the shell
  prints the error with its line number and keeps using the last good build.
* **Writes are atomic,** so a shell that starts during a rebuild never reads a
  half-written file.

nushell parses `config.nu` completely before running it, so the file it
`source`s has to exist beforehand. That's why the rebuild check lives in
`env.nu`, which nushell runs first.

## Config reference

| Section     | Contents                                                                                                       |
| ----------- | -------------------------------------------------------------------------------------------------------------- |
| `[env]`     | `NAME = "value"`. Numbers and booleans are accepted too. Variables are set in the order they're written.       |
| `[path]`    | `prepend = [...]` and `append = [...]`. Duplicate entries are removed and the first occurrence is kept.         |
| `[aliases]` | `name = "command"`. The body is shell code and is used exactly as written.                                     |
| `[zsh]`     | `init = '''...'''`: raw zsh code added at the end of the generated file.                                        |
| `[nu]`      | `init = '''...'''`: raw nushell code added at the end of the generated file.                                    |

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
`ZSH_AUTOSUGGEST_STRATEGY = { zsh = "history" }` is set only in zsh.

Aliases are where the two shells differ most. Commands such as `git status` or
`nvim` work the same in both, but builtins (`ls`, `cd`) and shell syntax (`&&`,
`$(...)`) don't, so give those aliases a per-shell form. Anything more
elaborate, such as functions, prompts, completions, `setopt` or `$env.config`,
goes in the `init` blocks.

## Commands

```
shellsync install        set up hooks (creates the config if missing)
shellsync uninstall      remove hooks
shellsync edit           open the config in $VISUAL/$EDITOR, then rebuild
shellsync check          validate the config
shellsync print zsh|nu   show the generated code
shellsync hook zsh|nu-env|nu-config   print a hook block, for manual setup
shellsync paths          show the files involved
shellsync build          rebuild by hand (normally done automatically)
```

`--config FILE` and `--out-dir DIR` override the defaults. The config path can
also be set with `$SHELLSYNC_CONFIG`, and `$XDG_CONFIG_HOME` and
`$XDG_CACHE_HOME` are respected. `install` writes the resolved paths into the
hooks, so later changes to those variables don't affect existing hooks.

## Notes

* Changes apply to **new** shells. In an already open zsh, run `exec zsh`.
* zsh loads the generated file from `.zshrc`, which only interactive shells read.
  To make variables visible to scripts started by cron or other non-interactive
  zsh sessions, set them in `~/.zshenv` as well.
* After upgrading shellsync, run `shellsync build` once to regenerate the files
  with the new version.

## Development

```sh
cargo test     # unit tests, plus end-to-end tests that start real zsh / nu
               # in a temporary HOME (skipped if a shell isn't installed)
```
