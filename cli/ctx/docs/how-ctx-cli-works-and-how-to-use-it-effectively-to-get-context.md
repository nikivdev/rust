# ctx - Turn Folders into Efficient AI Context

`ctx` is a CLI tool that packs folder contents into a format optimized for AI assistants. It automatically respects `.gitignore`, skips binary files, and fits within context window limits.

## The Problem It Solves

When working with AI assistants, you often need to share code context. Manual copy-paste is tedious, and pasting entire codebases exceeds context limits. `ctx` solves this by:

1. **Smart filtering** - Respects `.gitignore`, skips binaries and lock files
2. **Size limits** - Stays within AI context windows (default 500KB)
3. **Structured output** - Uses XML tags for clear file boundaries
4. **AI-assisted selection** - The `gather` command uses Claude to pick relevant files

## Quick Start

```bash
# Pack current directory, copy to clipboard
ctx

# Pack a specific folder
ctx ./src

# Pack and save to file
ctx pack ./src -o context.txt

# AI-assisted: gather only files relevant to a task
ctx gather . "fix the login bug"

# Fast local selection (no AI)
ctx fast . "fix the login bug"
```

## Commands

### Default: `ctx [path]`

Pack a folder and copy to clipboard.

```bash
ctx              # Current directory
ctx ./src        # Specific folder
ctx ~/projects/myapp
```

Output format:
```xml
<file_map>
/Users/you/project
</file_map>
<file_contents>
File: src/main.rs
```rust
fn main() {
    println!("Hello");
}
```

File: src/lib.rs
```rust
pub fn add(a: i32, b: i32) -> i32 {
    a + b
}
```
</file_contents>
```

### `ctx pack`

Pack a folder with more control.

```bash
# Output to file instead of clipboard
ctx pack ./src -o context.txt

# Increase size limit (default 500KB)
ctx pack ./src --max-size 1000000 -o large-context.txt
```

### `ctx gather` (AI-Assisted)

The most powerful feature. Uses Claude to analyze your codebase and select only the files relevant to a specific task.

```bash
# Basic usage
ctx gather . "add user authentication"

# For a specific bug
ctx gather ./src "fix the login redirect issue"

# Save to file with timestamp
ctx gather . "refactor the API layer" -o ~/contexts/{datetime}.md

# Optimized mode: minimal files, no tree
ctx gather . "fix button styling" --optimized
```

**How it works:**

1. Builds a tree of all files in the directory
2. Sends the tree + your task description to Claude
3. Claude returns a JSON array of relevant file paths
4. Those files are packed into the context
5. Result is copied to clipboard (or saved to file)
6. A copy is always saved to `~/done/{datetime}.md`

**Optimized mode (`--optimized`):**

- Selects only 3-8 essential files
- Skips README, docs, config files
- Omits the file tree from output
- Best for focused tasks in ChatGPT (smaller context)

### `ctx fast` (Local Heuristics)

Fast local selection without any AI calls. It matches files by name and light heuristics, then packs
their contents. Great when you want speed and predictable results.

```bash
ctx fast . "fix login redirect issue"
ctx fast ./src "add caching for users"
```

Behavior:
- Uses file-name matches first
- Falls back to entry points if no matches
- Outputs relative paths + file contents
- Always saves a copy to `~/done/{datetime}.md`

## Options

| Option | Default | Description |
|--------|---------|-------------|
| `--max-size` | 500000 (500KB) | Maximum total output size in bytes |
| `-o, --output` | clipboard | Output file path (supports `{date}`, `{time}`, `{datetime}`) |
| `--optimized` | false | gather: minimal file selection; fast: skip docs/config unless explicitly referenced |

## What Gets Included

### Included
- Source code (`.rs`, `.py`, `.js`, `.ts`, `.go`, etc.)
- Config files (`.json`, `.yaml`, `.toml`)
- Documentation (`.md`)
- Build files (`Makefile`, `Dockerfile`)

### Skipped
- Hidden files and directories (`.git`, `.env`)
- Files in `.gitignore`
- Binary files (images, videos, executables)
- Lock files (`package-lock.json`, `Cargo.lock`)
- Files that would exceed the size limit

## Effective Usage Patterns

### 1. Quick Context Dump

For small projects or when you need everything:

```bash
ctx .
# Paste into ChatGPT/Claude
```

### 2. Focused Module Context

When working on a specific part:

```bash
ctx ./src/auth
ctx ./components/dashboard
```

### 3. AI-Selected Context for Tasks

When you know what you want to accomplish:

```bash
ctx gather . "implement password reset flow"
ctx gather . "the API returns 500 on /users endpoint"
ctx gather . "add dark mode support"
```

### 4. Debugging Sessions

Get relevant files for a specific bug:

```bash
ctx gather . "TypeError: Cannot read property 'map' of undefined in UserList"
```

### 5. Code Review Context

Gather files that changed or are related:

```bash
ctx gather . "review the changes to the payment processing module"
```

### 6. Documentation Context

When asking for help with docs:

```bash
ctx gather . "write API documentation for the auth endpoints"
```

## Size Management

AI context windows have limits:
- **GPT-4**: ~128K tokens (~400KB text)
- **Claude**: ~200K tokens (~600KB text)
- **ChatGPT**: varies, often 32K-128K

`ctx` defaults to 500KB which fits most models. Adjust as needed:

```bash
# For GPT-3.5 or smaller contexts
ctx --max-size 100000

# For Claude with full context
ctx --max-size 800000

# For gather (defaults to 200KB for ChatGPT compatibility)
ctx gather . "task" --max-size 400000
```

When the limit is reached, `ctx` skips remaining files and reports how many were skipped.

## Output Placeholders

The `-o` flag supports date/time placeholders:

```bash
ctx pack . -o ~/contexts/{date}/context.txt
# Creates: ~/contexts/2024-01-15/context.txt

ctx gather . "task" -o ~/contexts/{datetime}.md
# Creates: ~/contexts/2024-01-15-14-30-45.md

ctx pack . -o ~/backup/{date}-{time}.txt
# Creates: ~/backup/2024-01-15-14-30-45.txt
```

Placeholders:
- `{date}` → `2024-01-15`
- `{time}` → `14-30-45`
- `{datetime}` → `2024-01-15-14-30-45`

## Integration with AI Workflows

### With ChatGPT

```bash
# 1. Gather context for your task
ctx gather . "implement user notifications"

# 2. Paste into ChatGPT
# 3. Ask your question
```

### With Claude (Anthropic Console)

```bash
# Pack with larger limit
ctx --max-size 600000

# Or use gather for focused context
ctx gather . "your task"
```

### With Claude Code (CLI)

For seamless integration, `ctx gather` already uses the `claude` CLI internally. You can also pipe context:

```bash
# Pack and send directly to claude
ctx | claude -p "explain this codebase"
```

### With Other Tools

```bash
# Save to file for later use
ctx pack . -o project-context.txt

# Use with any tool that reads stdin
ctx | my-ai-tool

# Combine with git for changed files context
git diff --name-only | xargs -I {} cat {} > changes.txt
ctx pack . -o full-context.txt
```

## Tips for Effective Context

### 1. Be Specific in Gather Tasks

Instead of:
```bash
ctx gather . "fix bug"
```

Do:
```bash
ctx gather . "fix the 'undefined is not a function' error when clicking Submit on the login form"
```

### 2. Use Optimized for Chat

When pasting into ChatGPT where context is limited:

```bash
ctx gather . "your task" --optimized
```

### 3. Start Narrow, Expand if Needed

```bash
# Start with just the module
ctx ./src/auth

# If that's not enough, gather more
ctx gather . "auth issue - need related files too"
```

### 4. Combine with Manual Selection

```bash
# Get AI selection
ctx gather . "task" -o base.txt

# Manually add specific files
cat base.txt specific-file.ts > full-context.txt
```

### 5. Save Contexts for Reference

The `gather` command auto-saves to `~/done/`. This creates a history of contexts you've gathered, useful for:
- Continuing conversations later
- Comparing what context was used for different tasks
- Building a knowledge base of your codebase explorations

## Architecture

```
ctx CLI
└── main.rs
    ├── pack_context()    # Walk directory, format files
    ├── gather_context()  # Use Claude to select files
    ├── build_file_tree() # Create directory tree string
    └── Utilities
        ├── is_binary_file()     # Detect binary files
        ├── get_language_hint()  # Syntax highlighting hints
        └── copy_to_clipboard()  # macOS pbcopy
```

Dependencies:
- `clap` - CLI argument parsing
- `ignore` - Respects `.gitignore` when walking directories
- `chrono` - Date/time for output placeholders
- `serde_json` - Parse Claude's JSON response
- `anyhow` - Error handling

## Comparison with Alternatives

| Tool | Approach | Best For |
|------|----------|----------|
| `ctx` | Smart packing + AI selection | Focused context for AI chats |
| `tree` | Directory listing only | Quick structure overview |
| `find + cat` | Manual concatenation | Custom selection |
| `repomix` | Full repo packing | Complete codebase context |

`ctx` excels when you need **right-sized context** - not too little, not too much - and especially when you want **AI-assisted file selection** for a specific task.

## Troubleshooting

### "claude CLI failed"

The `gather` command requires the `claude` CLI to be installed and authenticated:

```bash
# Install claude CLI
npm install -g @anthropic-ai/claude-code
# or
brew install claude

# Authenticate
claude login
```

### "copied 0 files"

Check if:
- The path exists and has files
- Files aren't all in `.gitignore`
- Files aren't all binary

### Context too large

Reduce with:
```bash
ctx --max-size 200000
# or
ctx gather . "task" --optimized
```

### Files missing from gather

Claude makes selections based on file names and your task description. If important files are missed:
1. Make your task description more specific
2. Mention the file or module name explicitly
3. Fall back to manual `ctx pack` of the specific directory
