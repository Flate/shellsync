# shellsync

Configure **zsh**, **bash**, **nushell** and **PowerShell** from one file.

You write environment variables, PATH entries and aliases once, in
`~/.config/shellsync/config.toml`. shellsync turns that file into native zsh,
bash, nushell and PowerShell code and hooks it into each shell. After a one-time `install` you
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
ll = { sh = "ls -lh", nu = "ls -l" }   # different per shell
```

## Install

Download the binary for your platform and `SHA256SUMS` from the
[releases](https://git.flatenet.de/flate/shellsync/-/releases) (or the same
files from the [GitHub releases](https://github.com/Flate/shellsync/releases)),
check it, and put it on your PATH as `shellsync`. For example, on a Mac:

```sh
shasum -a 256 --check --ignore-missing SHA256SUMS   # Linux: sha256sum --check --ignore-missing SHA256SUMS
install -m 755 shellsync-aarch64-apple-darwin ~/.local/bin/shellsync
shellsync install
```

On Windows, in PowerShell:

```powershell
(Get-FileHash shellsync-x86_64-pc-windows-msvc.exe).Hash   # compare with SHA256SUMS
Copy-Item shellsync-x86_64-pc-windows-msvc.exe <a directory on your PATH>\shellsync.exe
shellsync install
```

Releases have binaries for Linux x86_64 (static, any distribution), macOS on
Apple silicon and Windows x86_64 (no Visual C++ runtime needed). To build it
yourself instead, run `cargo install --path .`, or `cargo build --release` and
copy `target/release/shellsync` onto your PATH.

`install` creates a starter config if you don't have one, builds it, and adds a
small marked block to `~/.zshenv` and `~/.zshrc` (or the ones in `$ZDOTDIR`)
(not on Windows) and `~/.bashrc` (on Windows only if it exists, see below), to
nushell's `env.nu` and `config.nu`, to PowerShell's `profile.ps1`, and on
Windows to Windows PowerShell's `profile.ps1` if it exists (see below).
Running it again is safe: the block
is replaced, not duplicated. Paths under your home directory are written as
`$HOME/...` or `~/...`, so the hooks still work if you sync your dotfiles to a
machine with a different home path. The binary's location is written as-is
unless it is under your home directory too, for example `~/.cargo/bin`.
`shellsync uninstall` removes the blocks and leaves everything else in those
files as it was.

**Dotfiles managed by a tool** such as chezmoi: `install` edits the files in
place, which the tool would overwrite or report as changed. Instead, put the
blocks into your templates. `shellsync hook zshenv` (and `zshrc`, `bashrc`,
`nu-env`, `nu-config`, `pwsh`, `powershell`) prints each one; `--exe` sets where the binary is, for example
`shellsync hook zshenv --exe ~/.local/bin/shellsync`, so the blocks don't
depend on where shellsync ran when you printed them. If the tool also writes
the config from a template, edits to `config.toml` itself are overwritten on
its next run. Have `shellsync edit` run the tool's own edit command instead:

```toml
[edit]
command = "chezmoi edit --apply ~/.config/shellsync/config.toml"
```

## How it stays fast and invisible

* **Startup does almost nothing.** The hooks compare two file timestamps with
  the shell's own builtins (`[[ -nt ]]` in zsh and bash, `ls` in nushell, .NET
  calls in PowerShell) and then
  `source` a cached file. No extra process starts unless the config has
  changed (or, for nushell, a [tool](#conditional-entries) was installed or
  removed). Measured cost: about 2.5 ms for a whole interactive zsh start,
  about 0.4 ms added to a bash start, and about 3.5 ms added to a nushell
  start. PowerShell is slower to start on its
  own (about 170 ms); the hook and the generated code add about 45 ms with ten
  aliases and ten tools, most of it PowerShell's one-time cost for the first
  code it compiles, which any profile code pays. Windows PowerShell 5.1 starts
  in about 205 ms; there the hook adds about 15 ms, ten aliases about 25 ms,
  and ten tools that aren't installed about 80 ms, since each one is looked
  for in every PATH directory with every PATHEXT extension.
* **Rebuilds happen on their own.** When the config's timestamp differs from the
  cached files', the first shell to start runs `shellsync build` (about 35 ms,
  or about 235 ms with PowerShell installed, most of it the shells parsing the
  result, and on Windows about 200 ms more for Windows PowerShell's), then
  loads the result.
* **Output is plain shell code.** The generated files are in
  `~/.cache/shellsync/` (`env.zsh`, `init.zsh`, `init.bash`, `init.nu`,
  `init.ps1`, `init-ps5.ps1` for Windows PowerShell, and `tools.nu` for
  nushell's [tool checks](#conditional-entries)); `shellsync print zsh` (or
  `bash`, `nu`, `pwsh`, `powershell`) shows them. Only your user can read them, since the
  config may contain secrets.
* **Mistakes don't break your shell.** If the config has an error, the shell
  prints the error with its line number and keeps using the last good build.
  The same goes for code that a shell can't parse, such as a typo in an
  alias: before writing, `build` has each installed shell parse what it
  generated (see [`check`](#commands)), and writes nothing if one fails.
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

**bash** (3.2 or later, so also the one macOS ships) loads everything from
`init.bash`, which `.bashrc` sources. Interactive shells read `.bashrc`; login
shells read `.bash_profile` or `.profile` instead, which on most Linux
distributions source `.bashrc`. On macOS, where Terminal starts bash as a
login shell, add `source ~/.bashrc` to your `.bash_profile` if it doesn't
have it. bash also reads `.bashrc` for `ssh host command`; such a shell isn't
interactive, so it uses the last build without rebuilding. PATH entries are
deduplicated as in the other shells, so a bash started from bash keeps PATH
as it is. bash before 5.x compares file times in whole seconds: a config
change in the same second as the previous build is picked up only with the
next change (or `shellsync build`).

**nushell** parses `config.nu` completely before running it, so the file it
`source`s has to exist beforehand. That's why the rebuild check lives in
`env.nu`, which nushell runs first, and why `config.nu` only sources the file
if it exists.

**PowerShell** 7.4 or later (`pwsh`, not Windows PowerShell 5.1) loads
`init.ps1` from the profile for all hosts, `~/.config/powershell/profile.ps1`.
PowerShell aliases can't take arguments, so each alias becomes a function that
passes its arguments on: `gs = "git status"` becomes
`function gs { git status @args }`. A built-in alias of the same name, such as
`gc` or `gp`, is removed so that the function takes effect. A function also
comes before the program of its name, so an alias whose body starts with its
own name, such as `rsync = "rsync -a"`, runs the program, as in the other shells. The hook and the
generated code use .NET calls instead of cmdlets such as `Test-Path`, because
the first use of a cmdlet loads its module, which would add about 100 ms to
every start.

**Windows PowerShell 5.1** (`powershell.exe`, part of every Windows) has its
own key, `powershell`, its own file, `init-ps5.ps1`, and its own profile:
`Documents\WindowsPowerShell\profile.ps1`, wherever Documents is. It gets the
same code as PowerShell 7, with aliases as functions and no cmdlets, except
for what 5.1 lacks. `pwsh` values and `[pwsh] init` don't apply to it, since
they may use syntax that 5.1 doesn't have, such as `&&`, `??` or `? :`; use
`ps` for values both PowerShells share (see [per-shell
values](#config-reference)). A plain string applies to every shell, so on
Windows a plain alias must parse in 5.1 too.

By default Windows PowerShell runs no scripts, not even a profile, and reports
an error at every start if one exists. So `install` hooks its profile only if
the profile is already there. To use shellsync with it, run this in Windows
PowerShell once, then `shellsync install`:

```powershell
Set-ExecutionPolicy RemoteSigned -Scope CurrentUser
New-Item -ItemType File -Force $PROFILE.CurrentUserAllHosts
```

shellsync writes `init-ps5.ps1` with a byte order mark, since Windows
PowerShell reads a file without one in the ANSI code page, not as UTF-8.
If your config is a symlink, the hook can't compare its time on its own:
.NET Framework can't resolve a link. It then runs `shellsync build
--if-stale` at every start, which adds about 10 ms.

**Windows**: nushell, PowerShell, Windows PowerShell and bash in Git Bash,
MSYS2 or Cygwin are supported; zsh isn't. shellsync's own files are where they are on
other systems, under your user profile: `.config\shellsync\config.toml` and
`.cache\shellsync\`. They get the profile folder's permissions (you,
administrators and the system) instead of mode 0600. `install` asks
PowerShell where its profile is, since Documents may have moved, to OneDrive
for example. Commands in `[tool]` checks are found the way the shells find
them, through PATHEXT: `command = "eza"` matches `eza.exe`. `edit` starts the
editor through `cmd`, Notepad if `VISUAL` and `EDITOR` aren't set.

**bash on Windows**: Git Bash, MSYS2 and Cygwin each have their own home
directory and `.bashrc`. Git Bash uses your user profile; MSYS2 and Cygwin
use `/home/<you>` in their install folder, unless a Windows `HOME` variable is
set. `install` hooks the `.bashrc` of the bash it runs in, or Git Bash's when
started from PowerShell, and only if the file exists, since most Windows users
have no bash. So, in each bash you use: create `~/.bashrc` if it isn't there
(`touch ~/.bashrc`), then run `shellsync install`. All of them share the one
config in your user profile. Some things to know:

* `~` and `$HOME` in the config are bash's home, so with MSYS2 or Cygwin not
  the same folder as in PowerShell.
* bash's PATH uses its own paths, such as `/c/tools` (Git Bash, MSYS2) or
  `/cygdrive/c/tools` (Cygwin). `[path]` entries for Windows directories need
  a `bash = "..."` value in that form.
* Login shells, which the Git Bash, MSYS2 and Cygwin terminals start, read
  `.bash_profile`, which should source `.bashrc`. The starter files of MSYS2
  and Cygwin do that, and Git Bash creates such a `.bash_profile` when it
  finds only a `.bashrc`.
* `build` parses the bash code with the bash on PATH, so it is checked when
  the rebuild runs from bash. A `bash.exe` in the Windows folder or in
  WindowsApps is WSL's and is never used for that.

## Config reference

| Section     | Contents                                                                                                       |
| ----------- | -------------------------------------------------------------------------------------------------------------- |
| `[env]`     | `NAME = "value"`. Numbers and booleans are accepted too. Variables are set in the order they're written.       |
| `[path]`    | `prepend = [...]` and `append = [...]`. Duplicate entries are removed and the first occurrence is kept.         |
| `[aliases]` | `name = "command"`. The body is shell code and is used exactly as written.                                     |
| `[zsh]`     | `init = '''...'''`: raw zsh code added at the end of the generated file.                                        |
| `[bash]`    | `init = '''...'''`: raw bash code added at the end of the generated file.                                       |
| `[nu]`      | `init = '''...'''`: raw nushell code added at the end of the generated file.                                    |
| `[pwsh]`    | `init = '''...'''`: raw PowerShell 7 code added at the end of the generated file.                               |
| `[powershell]` | `init = '''...'''`: raw Windows PowerShell 5.1 code added at the end of the generated file.                  |
| `[sh]`      | `init = '''...'''`: raw code for zsh and bash, combined with `[zsh]` and `[bash] init` in file order.            |
| `[ps]`      | `init = '''...'''`: raw code for both PowerShells, combined with `[pwsh]` and `[powershell] init` in file order. |
| `[tool.X]`  | Any of the above that apply only where a tool is installed; see [Conditional entries](#conditional-entries).    |
| `[edit]`    | `command = "..."`: what `shellsync edit` runs instead of opening the config, through `sh -c` (Windows: `cmd /C`). |

**Values in `[env]` and `[path]`** can use:

* `~` or `~/...` at the start for your home directory
* `$NAME` or `${NAME}` for another environment variable
* `$$` for a literal `$`

These references are written into each shell's own syntax and resolved when the
shell starts, so `"$GOPATH/bin"` becomes `"${GOPATH}/bin"` in zsh and bash,
`$env.GOPATH`-based code in nushell and `"${env:GOPATH}/bin"` in PowerShell. Quotes, backticks, parentheses and other
special characters are escaped for each shell.

**Per-shell values.** Any value can be a table:
`{ zsh = "...", bash = "...", nu = "...", pwsh = "...", powershell = "..." }`.
If you leave out a shell, that entry is skipped there. For example,
`ZSH_AUTOSUGGEST_STRATEGY = { zsh = "history" }` is set only in zsh. The key
`sh` stands for every shell with POSIX sh syntax, zsh and bash; a shell's own
key takes precedence, so `{ sh = "ls -l", zsh = "ls -lh" }` gives zsh
`ls -lh` and bash `ls -l`. In the same way `ps` stands for PowerShell 7
(`pwsh`) and Windows PowerShell 5.1 (`powershell`). TOML 1.1 allows such a table to span several lines:

```toml
ll = {
  sh = "ls -lh",
  nu = "ls -l",
}
```

Aliases are where the shells differ most. Commands such as `git status` or
`nvim` work the same everywhere, but builtins (`ls`, `cd`) and shell syntax
(`&&`, `$(...)`) don't, so give those aliases a per-shell form. Anything more
elaborate, such as functions, prompts, completions, `setopt`, `$env.config` or
`Set-PSReadLineOption`, goes in the `init` blocks.

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

**When conditions are checked.** Every shell checks them each time it starts,
so installing or removing a tool takes effect in the next shell. zsh, bash
and PowerShell check them in the generated file, without starting a process
(zsh and bash with builtins, PowerShell with .NET calls; ten tools add about
12 ms there).
nushell defines aliases while
it parses its config, before anything runs, so an alias can't depend on an
`if`. Instead, the hook in `env.nu`, which runs first, checks the conditions
(with nushell's built-in `which`; ten tools add about 0.4 ms) and compares the
result with the last build. Only if they differ does it run `shellsync build`,
and the generated file (`init.nu`, which notes each tool it skipped) is ready
before `config.nu` sources it.

nushell checks with the PATH it has when `env.nu` runs, plus shellsync's own
`[path]` entries. So if a tool's directory is added to PATH only by your zsh
startup files, for example Homebrew's `/opt/homebrew/bin` via `brew shellenv`,
nushell won't find tools there. Add such directories to `[path]`, so every
shell gets them.

## Commands

```
shellsync install        set up hooks (creates the config if missing)
shellsync uninstall      remove hooks
shellsync edit           open the config in $VISUAL/$EDITOR (or run [edit] command), then rebuild
shellsync check          validate the config and parse the generated code
shellsync print zsh|bash|nu|pwsh|powershell   show the generated code
shellsync hook zshenv|zshrc|bashrc|nu-env|nu-config|pwsh|powershell   print a hook block, for manual setup
shellsync paths          show the files involved
shellsync build          rebuild by hand (normally done automatically)
```

`--config FILE` and `--out-dir DIR` override the defaults. The config path can
also be set with `$SHELLSYNC_CONFIG`, and `$XDG_CONFIG_HOME` and
`$XDG_CACHE_HOME` are respected. `install` and `hook` write the resolved paths
into the hooks, so later changes to those variables don't affect existing
hooks. `--exe FILE` sets the binary the hooks run (default: the running one).

**`check`** also hands the generated code to zsh (`zsh -n`), bash
(`bash -n`), nushell (`nu-check`), PowerShell and, on Windows, Windows
PowerShell (their parsers) to parse, without running it, so a typo in an alias or init
block shows up before a shell trips over it. In nushell, a line that doesn't
parse stops the whole config from loading, and some aliases that work in zsh
don't parse there: nushell can't alias a pipeline (`ls -l | sort`). Errors
name the config key they come from, such as `aliases.ll` or
`tool.fzf.zsh.init`. Every `[tool]` table is checked, whether the tool is
installed or not. A shell that isn't installed is skipped. `build` (and so
`edit` and the hooks) runs the same check on the code it is about to write,
for the tools that apply on this machine, and keeps the last good build if
it fails. The code is parsed from a folder inside the cache directory, not
from the temp directory, since it may contain values from your config.

The code is parsed on its own, without the rest of your `.zshrc`, `.bashrc`,
`config.nu` or PowerShell profiles. A nushell variable defined in your `config.nu` isn't known
there, so code that uses it doesn't parse; define it in `[nu] init` instead.

## Notes

* Changes apply to **new** shells. In an already open zsh, run `exec zsh`
  (in bash, `exec bash`).
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
cargo test     # unit tests, plus end-to-end tests that start real zsh / bash / nu / pwsh
               # (and Windows PowerShell on Windows) in a temporary HOME
               # (skipped if a shell isn't installed)
SHELLSYNC_REQUIRE_SHELLS=1 cargo test   # fail instead of skipping
```

GitLab CI (`.gitlab-ci.yml`) runs a secret scan, `cargo fmt --check` and
`cargo clippy` (also for the Windows target), then the tests on Linux (zsh,
bash 5, and pinned nushell and PowerShell releases), on a Windows runner tagged
`windows` (with Git Bash, MSYS2, Cygwin and Windows PowerShell), and on a
macOS runner tagged `macos` (with the system's bash 3.2).

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
   GitLab release with the changelog section as its notes. `release:github`
   then waits for the push mirror to bring the tag to GitHub and creates the
   same release there.

`release:github` needs a CI/CD variable `GITHUB_TOKEN` (protected, masked): a
fine-grained GitHub token for the mirror repository only, with "Contents: Read
and write". The repository is set in `GITHUB_REPOSITORY` in `.gitlab-ci.yml`.

Re-running a release job is safe: files already uploaded are kept, and a
release job that would replace one with different contents fails instead.
A GitHub release is created as a draft and published once all files are
uploaded.

## License

shellsync is licensed under the GNU General Public License v3.0 or later.
See [LICENSE](LICENSE).
