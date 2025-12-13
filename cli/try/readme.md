# Rust try clone

This crate rewrites the minimal "teleport me to a fresh directory" workflow from `try.rb` in Rust.

```
cd rust_cli
cargo run -- --print-script
```

- Defaults to `~/tries` but honors `TRY_PATH` or `--path`.
- Launches your login shell inside the new directory when stdout is a TTY.
- Use `-p/--print-script` when you prefer to emit a `cd` script for shell wrappers (e.g. `eval "$(try --print-script)"`).
- Pass words after the options to form part of the directory slug (`try api spike`).

Set `TRY_PRINT_ONLY=1` to force script emission even when running interactively.
