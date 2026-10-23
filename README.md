

## Installation on Ubuntu
```
git clone git@github.com:canonical/design-workshop.git exp01
cd exp01
git remote rename origit workshop
git remote add origin git@github.com:hartmutobendorf/testee.git
git branch -M main
git push -u origin main
```

```
workshop launch --verbose
```
Now copy the code shown, and login to github on your browser.

```
code --folder-uri vscode-remote://ssh-remote+workshop@10.4.75.58/projectP
```

## Workshop definition TUI

`workshop-tui` is a Rust terminal editor for creating and editing Workshop
definitions. It searches the SDK Store, selects channels compatible with the
target Ubuntu base and architecture, and previews YAML. The Ratatui interface has
three views: **SDKs**, **Toolchains**, and **YAML preview**. It does not connect to the Workshop
daemon or launch, install, refresh, or remove running workshops.

The TUI opens on a curated SDK list from [data/curated-sdks.txt](./data/curated-sdks.txt), in that file's
order. This list is bundled at build time and appears immediately; rebuild after
changing the list. Highlighting an SDK automatically fetches its description,
compatible channels, and current versions into the details pane. Use `/` to search beyond the curated list, `r` to browse all
Store SDKs, and `f` to return to the curated list. Curated entries are suggestions,
not preinstalled SDKs; unavailable entries report the Store error when selected.

The second tab lists toolchains from
[data/toolchain-sdks.txt](./data/toolchain-sdks.txt), also bundled in file order.
It supports the same descriptions, multi-selection, and optional channel picker.
New definitions start with **uv** checked on `latest/stable`; existing definitions
keep their saved selections without adding uv automatically. All checked SDKs and
toolchains are saved together in the definition's `sdks` list. Each catalog keeps
its own highlighted row and search results when switching tabs.

Use **Space or Enter** to check/uncheck multiple SDKs. The checked combination is
applied to the definition in memory, survives searches and switching views, and
is written only when you save. Existing SDKs are already checked. Deselecting an
SDK referenced by a connection or binding is rejected until you edit those
references.

Channel selection is optional: a newly checked SDK uses `latest/stable` and
shows that channel's latest published version and revision. If `latest/stable`
is unavailable for the target, the Store's default track's stable channel is
preferred, falling back to the compatible channel with the newest revision.
Press **c** to choose another channel; the picker starts on the default channel
for a new SDK or the current channel for an existing selection. Applying a
channel also checks that SDK. Versions are displayed for information, not pinned:
Workshop resolves the channel at launch/refresh. Base-independent (`all`)
platforms are supported.

### Build and run

On Ubuntu, install a current stable Rust toolchain (including Cargo) and the
native build prerequisites:

```sh
sudo apt install build-essential pkg-config
cargo run --release
```

The default file is `workshop.yaml`. To create a separate definition without
changing the existing design workshop:

```sh
cargo run --release -- edit my-workshop.yaml
```

For a definition under `.workshop/`, the workshop name must match the filename;
new definitions there start with that name. The destination directory must
already exist. No file is created or changed until you explicitly save.

To install the executable locally:

```sh
cargo install --path .
workshop-tui edit my-workshop.yaml
```

### Snap package: workshop-config

The snap installs the command `workshop-config` with the same commands and
options as `workshop-tui`. Source and Cargo installations keep the existing
`workshop-tui` command.
Help, version output, and recovery instructions use the invoked command name.

Build the snap using [snap/snapcraft.yaml](./snap/snapcraft.yaml):

```sh
sudo snap install snapcraft --classic
snapcraft pack
sudo snap install ./workshop-config_0.2.0_amd64.snap --dangerous
mkdir -p ~/projects/my-workshop
cd ~/projects/my-workshop
workshop-config edit my-workshop.yaml
workshop-config help
```

Snapcraft builds against `core24` with a stable Rust toolchain and the committed
Cargo lockfile. The snap version comes from `Cargo.toml`. The recipe supports
native amd64 and arm64 builds; the resulting filename includes the version and
build architecture. The recipe is development-grade and does not publish
anything to the Store.

Inside a Workshop container, nested LXD cannot load AppArmor profiles, so the
managed `snapcraft pack` fails with "Failed to enable snapd service". The
Workshop base (`ubuntu@24.04`) matches `core24`, so build on the host instead:

```sh
sudo snapcraft pack --destructive-mode
sudo chown -R "$USER": parts stage prime ./*.snap
```

**Strict confinement** uses `home`, `network`, and a read-only `personal-files`
plug named `workshop-try`. The `home`
interface permits ordinary user-owned project paths beneath your real home
directory, such as `~/projects/my-workshop`. It does not grant arbitrary access
to `/project`, `/etc`, `/var`, another user's files, or top-level hidden home
directories. Nested `.workshop/` definitions can be kept inside an accessible
project directory. The `workshop-try` plug permits reading only
`~/.local/share/workshop/try`. No `system-files`, removable-media, or Workshop
socket access is requested. Custom `XDG_DATA_HOME` paths require access permitted
by confinement; the plug covers only the standard try directory.

The snap bundles nano and terminal descriptions. Pressing `e` always opens that
nano inside confinement, ignoring host `$VISUAL`/`$EDITOR` and nano rc files;
arbitrary host editors cannot be launched. Source/Cargo installations retain
host-editor discovery. `--dangerous` is needed only for a locally built unsigned
snap; **do not use `--classic`**.

If file access is denied, move the project to an ordinary home-directory path
and check `snap connections workshop-config`. For a locally installed snap,
connect the permitted interfaces if needed:

```sh
sudo snap connect workshop-config:home
sudo snap connect workshop-config:network
sudo snap connect workshop-config:workshop-try
```

Version 0.2.0 intentionally removes the experimental daemon-status feature.
The old `status` invocation is hidden and exits with code 2 and a removal
message; it never connects to a daemon. Its socket/URL options and environment
variables are no longer used. The install command above also replaces the
earlier local classic build without removing existing snap-managed data.
Confirm the change with `snap list workshop-config` (no `classic` note) and
`snap connections workshop-config` (`home`, `network`, and `workshop-try`).

### Commands and appearance

The command grammar and help follow [cli-standard.md](./specs/cli-standard.md),
[cli-help.md](./specs/cli-help.md), and [cli-help-examples.md](./specs/cli-help-examples.md).

| Command | Purpose |
| --- | --- |
| `edit [FILE]` | Open the SDK/toolchain pickers and YAML preview; default file is `workshop.yaml` |
| `validate [FILE]` | Check an existing definition without a terminal, network, or writes |
| `version` | Print the release version |
| `help [COMMAND]` | Show general or command-specific help |
| `help --all` | Show help for all commands |

No command opens the editor; `workshop-tui FILE` remains shorthand for
`workshop-tui edit FILE`. The previous `--check FILE` form remains an alias for
`validate FILE`. Commands also accept `--help`/`-h`; successful help and data go
to stdout, errors and usage guidance go to stderr with a nonzero exit code.
`--quiet` suppresses validation success messages, not data or errors.

The light palette is the default, using [DESIGN.md](./DESIGN.md)'s Canonical
tokens. `--theme light|dark|paper` selects the interface theme. The TUI uses
square borders, neutral surfaces, blue focus, and a single orange Save action.
The terminal controls fonts; no graphical fonts or web components are required.
`--color auto|always|never` controls color; `NO_COLOR` always disables it.
Selection markers, bold labels, and explicit messages remain readable without
color. Non-interactive commands do not produce terminal control sequences.

```sh
workshop-tui edit my-workshop.yaml --theme dark
NO_COLOR=1 workshop-tui edit
workshop-tui help edit
```

### Controls

New definitions use the current directory name, normalized to a valid workshop
name. Named `.workshop/<name>.yaml` definitions retain filename-based naming;
existing definitions are unchanged.

When local SDKs exist in `$XDG_DATA_HOME/workshop/try` (default
`~/.local/share/workshop/try`), a third **Try SDKs** tab appears. Selection adds
`try-<name>` without a channel and requires an artifact matching the selected
base and architecture, plus its metadata and checksum sidecars. YAML preview
becomes the fourth tab. Local SDK discovery does not require the Store or daemon.
If the try directory is inaccessible, the Try SDKs tab stays hidden. On exit,
the editor prints a notice suggesting `sudo snap connect workshop-config:workshop-try`
for snap installations. A missing or empty try directory needs no connection notice.

| Key | Action |
| --- | --- |
| Tab / Shift-Tab, or 1-4 | Switch between SDKs, Toolchains, optional Try SDKs, and YAML preview |
| n / b / t | Edit name, Ubuntu base, or runtime from either view |
| a | Add an SDK by name, including `system`, `try-*`, or `project-*` |
| Up/Down or j/k | Highlight an SDK/channel, or scroll YAML preview |
| c | Optionally choose a published channel for the highlighted SDK |
| o | Show checked SDKs, including local SDKs, in installation order |
| J / K | Move the selected SDK down/up in installation order |
| / | Enter an SDK Store search on the Store tab |
| r | Browse all SDKs |
| f | Return to the curated SDK list on the Store tab |
| Space / Enter | Check/uncheck the highlighted Store SDK, using its default channel |
| Enter (channel picker) | Apply the chosen channel and check the SDK |
| PageUp / PageDown | Scroll the highlighted SDK's description |
| i | Retry loading the highlighted SDK's details |
| e | Edit advanced YAML using `$VISUAL`, `$EDITOR`, or an automatically discovered editor |
| s | Validate and save |
| q / Ctrl-C | Quit; confirm before discarding unsaved changes |
| Esc | Cancel input, close SDK details, or cancel quitting |
| Ctrl-U | Clear the current input |

In source/Cargo installations, the external editor uses `$VISUAL`, then `$EDITOR`. If both are unset or empty,
it searches `PATH` for `nano`, `pico`, `micro`, `nvim`, `vim`, `vi`, `emacs`, then
`editor`, in that order. An explicitly configured editor takes precedence over
discovery and may include quoted arguments, for example
`VISUAL='code --wait' cargo run --release`. Invalid configuration or an editor
that cannot start is reported rather than silently replaced. If none is
available, install `nano` or set one of the editor variables.

Advanced YAML editing covers actions, plugs, slots, and connections. The editor receives a temporary
copy, not the destination file. Its changes are validated and applied in memory;
you still need to save. If edited YAML is invalid, the status message reports a
recovery file containing the rejected draft.

Existing actions, endpoint attributes, connections, SDK order, and local SDK
references are retained. **Saving normalizes YAML formatting and removes
comments**, including commented-out SDK entries. Save to a separate file if those
comments are important. A concurrent change to the destination is reported
instead of overwritten. Saves use an atomic replacement, preserve existing file
permissions, and reject symlink destinations.

### API configuration

The implementation uses the HTTP contracts in
[`canonical/workshop`](https://github.com/canonical/workshop), checked against
upstream commit `f8fdf0d59d223ded69d076db357509ead1d795f7`:

- SDK Store: `GET /v2/sdks/find` and `GET /v2/sdks/info/{name}`, the Store
  transport used by Workshop's SDK tooling. Requests go directly to
  `https://api.pkg.store` by default. Channel selection filters the Store's
  platform metadata by base and architecture.

Definition editing and Store access do not use the Workshop daemon.
Network requests run off the terminal event loop and have a 15-second timeout;
connection, HTTP, and protocol errors are shown in the status area.
SDK details are cached for the session; navigation remains available while
metadata loads. Press `i` to refresh metadata or retry a failed lookup.

```sh
# Target a different machine's architecture
workshop-tui --architecture arm64 my-workshop.yaml

# Override the Store, as with the upstream SDK tooling
SDK_STORE_URL=https://store.example.org workshop-tui

```

`--store-url` also provides a command-line override.

The editor checks definition structure, names, supported bases/runtimes, channel
syntax, duplicate SDKs, connection SDK references, and plug binding constraints.
It is not a replacement for daemon validation: SDK existence, local SDK contents,
interface compatibility, and interface-specific attributes are resolved by
Workshop. Manually entered channels are not checked for Store availability.
Changing the base does not automatically migrate existing SDK channels.

### Validation and development

```sh
# Validate an existing file without a terminal or network
cargo run -- validate workshop.yaml

cargo fmt --all -- --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets

# Optional live SDK Store smoke test
cargo test live_store_search_and_compatible_channels -- --ignored
```

The automated suite covers definition round-trips, safe saves, validation,
terminal input/rendering, SDK Store responses, bundled-editor selection, and
the removed-command failure message. Supported runtime target: Linux.
