# Rust

## Setup

Install [task](https://taskfile.dev/docs/installation). Then run `task setup` & follow instructions until it says `✔️ you are setup`.

## Commands

Run `task` to see all possible commands.

## `flow` CLI

`flow` is a CLI as a dump of various commands written in Go that are useful to me. See [cli/flow](cli/flow) for its code/docs.

Run `task deploy` to install `flow-rs` into PATH. It also puts `fw` command in path (my own personal shorthand, but you can change it).

## `index` CLI

`index` scans folders for Git repositories and writes a JSON index that captures metadata (branch, HEAD commit, dirty files, remote, etc.). It's handy for quickly understanding the state of many repos.

- Run `task index -- --root ~/code --output ~/Desktop/repos.json` to index everything under `~/code`.
- The resulting JSON includes stats for each repo plus a list of failures so you can retry problematic ones.
- Pass `--jobs` to control concurrency or `--quiet` to silence progress logs.

## `stream` CLI

`stream` is a lightweight daemon/CLI combo that launches an efficient macOS screen capture pipeline (ffmpeg with VideoToolbox) and a remote receiver (tmux + ffmpeg or headless OBS) over SSH/SRT. The goal is to replace GUI-only OBS on the host machine and keep frame drops off the main desktop.

- Run `task stream -- --help` to see the commands.
- Start with `stream config init` to write `~/Library/Application Support/stream/config.toml`, then customize the example profiles (remote host, ffmpeg path, ingest port, etc.).
- `stream start` launches the remote tmux session plus a detached local ffmpeg process and writes logs to `~/Library/Application Support/dev.nikiv.stream/logs`.
- `stream stop` tears down both sides, `stream status` reports health, and `stream check` verifies binaries/SSH connectivity.

The default config ships with two profiles:

1. **main** – ffmpeg listening on the Linux receiver and remuxing to MPEG-TS.
2. **obs** – headless OBS on the remote machine (ideal when you already have a Media Source pointed at the matching SRT port).

Modify encoder, bitrate, scale filters, tmux session names, or headless OBS flags per profile. The CLI keeps a JSON session file so you can script it or hook it into launchd/cron.

## `code` CLI

`code` prints the surrounding block for a given file/line and follows referenced functions within the same file to build a quick context dump.

- Run `task code -- path/to/file.ts 42` to print the block that contains line 42 plus any locally defined callees it references.
- Pass `--depth 2` (or more) to recursively expand referenced functions a few levels deep.

## Libraries

All library code is in `lib/` (currently git ignored as there is only one library there in separate repo).

- [log_macro](https://github.com/nikivdev/log_macro) - Macro to print variable(s) with values nicely

## Contributing

Any PR to improve is welcome. [codex](https://github.com/openai/codex) & [cursor](https://cursor.com) are nice for dev. Great **working** & **useful** patches are most appreciated (ideally). Issues with bugs or ideas are welcome too.

### 🖤

[![Discord](https://go.nikiv.dev/badge-discord)](https://go.nikiv.dev/discord) [![X](https://go.nikiv.dev/badge-x)](https://x.com/nikivdev) [![nikiv.dev](https://go.nikiv.dev/badge-nikiv)](https://nikiv.dev)
