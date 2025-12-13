# How km CLI Works

`km` is a CLI for managing Keyboard Maestro macros programmatically.

## Commands

### create-open

Creates a macro that focuses an existing app window or opens a path if no window exists.

```bash
km create-open "zed: focus" Zed "~/org/1f/focus"
km create-open "zed: focus" Zed "~/org/1f/focus" --goku v.o
```

**What it does:**

1. Checks if a KM macro with that name already exists (errors if so)
2. If `--goku` provided, checks if that key binding exists (errors if so)
3. Generates a plist with an If/Then/Else action:
   - **If** app has a window ending with the folder name → focus that window
   - **Else** → run `open -a /Applications/App.app path`
4. Imports the macro into Keyboard Maestro via `.kmmacros` file
5. If `--goku` provided, adds the binding to karabiner.edn

**Generated macro structure:**

```
If window title ends with "focus" in Zed
  → Select that window
Else
  → Execute: open -a /Applications/Zed.app ~/org/1f/focus
```

### list

Lists all macros from Keyboard Maestro.

```bash
km list
```

### run

Runs a macro by name.

```bash
km run "zed: focus"
```

### inspect

Shows a macro's actions as JSON.

```bash
km inspect "zed: focus"
```

## Integration with karabiner CLI

The `--goku` flag integrates with the `karabiner` CLI to add bindings:

```bash
# This:
km create-open "zed: focus" Zed "~/org/1f/focus" --goku v.o

# Is equivalent to:
km create-open "zed: focus" Zed "~/org/1f/focus"
karabiner add v o "zed: focus"
```

The binding format is `layer.key`:
- `v.o` → v-mode + o key
- `semicolon.spacebar` → semicolon-mode + spacebar

## Conflict Detection

Before creating anything, km checks:

1. **KM macro exists?** → `Error: macro 'name' already exists in Keyboard Maestro`
2. **Goku key bound?** → `Error: key 'o' already bound in layer 'v'. Use 'karabiner comment v o' first.`

## File Locations

- Karabiner config: `/Users/nikiv/config/i/karabiner/karabiner.edn`
- Temp macro file: `/tmp/km_macro_import.kmmacros` (deleted after import)
