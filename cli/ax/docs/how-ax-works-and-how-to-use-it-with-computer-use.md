# ax - macOS Accessibility CLI for Computer Use

`ax` is a CLI tool for navigating and interacting with the macOS accessibility tree. It's designed for **computer use training** - collecting data to train models that can navigate UI elements via natural language commands.

## How It Works

### The Accessibility Tree

macOS exposes UI elements through the Accessibility API (AXUIElement). Every app window contains a tree of elements:

```
Application
├── Window "Safari"
│   ├── Toolbar
│   │   ├── Button "Back"
│   │   ├── Button "Forward"
│   │   ├── TextField "Search or enter website"
│   │   └── Button "Share"
│   ├── WebArea
│   │   ├── Link "Home"
│   │   ├── Button "Sign In"
│   │   └── ...
```

Each element has:
- **Role**: button, link, textfield, checkbox, menuitem, etc.
- **Label**: The visible text or aria-label
- **Bounding box**: Screen position (x, y) and size (width, height)
- **State**: enabled, focused, value (for inputs)
- **Actions**: What can be done (AXPress, AXFocus, etc.)

### How ax Reads Elements

1. Gets the frontmost application via `NSWorkspace.frontmostApplication`
2. Creates an `AXUIElement` from the app's PID
3. Walks the element tree using `TreeWalker` (depth-limited)
4. Extracts properties: role, title, description, position, size, enabled, focused
5. Outputs as structured data (JSON or formatted text)

### How ax Performs Actions

For clicks:
1. Finds the target element by ID or label
2. Calculates the center of its bounding box
3. Uses `enigo` to move mouse and click at those coordinates

For typing:
1. Optionally clicks an element to focus it
2. Uses `enigo` to send keystrokes

## Setup

### 1. Grant Accessibility Permission

The first time you run `ax`, it will fail with:
```
Error: Accessibility permission not granted.
```

Fix this:
1. Open **System Preferences > Privacy & Security > Accessibility**
2. Click the lock to make changes
3. Add your terminal app (Terminal, iTerm2, Ghostty, etc.)
4. Toggle it ON

### 2. Install

```bash
# From the rust repo root
flow deploy-ax

# Or build manually
cargo build -p ax --release
cp target/release/ax ~/bin/ax
```

## Usage

### List Elements

```bash
# List all elements in focused app
ax list

# Filter by role
ax list --role button

# Filter by label (substring match)
ax list --label "Submit"

# Output as JSON
ax list --json
```

Example output:
```
47 elements in Safari
────────────────────────────────────────────────────────────
  0 │ button          "Back"
  1 │ button          "Forward"
  2 │ textfield       "Search or enter website name" [focused]
  3 │ button          "Share"
  4 │ link            "Apple"
  5 │ button          "Sign In"
```

### Show Tree Hierarchy

```bash
ax tree
ax tree --depth 3
ax tree --all  # Include non-interactive elements
```

### Click Elements

```bash
# By ID (from ax list output)
ax click 5

# By label
ax click --label "Submit"

# By role + label
ax click --label "Sign In" --role button
```

### Type Text

```bash
# Type into focused element
ax type "hello world"

# Click element first, then type
ax type "search query" --element 2
```

### Watch Mode

Stream elements continuously (useful for debugging):

```bash
ax watch
ax watch --interval 1000  # Update every 1 second
ax watch --json --output /tmp/elements.json
```

### Interactive Mode

REPL for exploring and interacting:

```bash
ax interactive
```

Commands:
- `r` - Refresh element list
- `t` - Show tree view
- `c <id>` - Click element
- `s <id>` - Select element
- `i <id>` - Show element details
- `type <text>` - Type text
- `q` - Quit

### Get Element at Coordinates

```bash
ax at 500 300
ax at 500 300 --json
```

### Perform Actions

```bash
ax do click 5
ax do focus 2
ax do value 3 --value "new text"
ax do double 5  # Double-click
ax do right 5   # Right-click
```

## Computer Use Training

The key idea: collect (screen_state, command, target_element) tuples to train a model that predicts which element to click given a natural language command.

### Collecting Training Data

```bash
# Start collection session
ax collect -o training_data.jsonl

# Filter to specific app
ax collect -o safari_data.jsonl --app Safari

# Auto-generate commands (for quick collection)
ax collect -o data.jsonl --auto
```

**Collection workflow:**

1. Run `ax collect -o data.jsonl`
2. Type `r` to refresh and see elements
3. Type `c <id>` to click an element
4. Enter a natural language command like "click the submit button"
5. Sample is saved automatically
6. Repeat until you have enough data
7. Type `q` to quit

### Training Data Format

Each line in the JSONL output:

```json
{
  "screen_state": {
    "elements": [
      {
        "id": 0,
        "role": "button",
        "label": "Submit",
        "bbox": {"x": 100, "y": 200, "width": 80, "height": 32},
        "enabled": true,
        "focused": false,
        "depth": 2
      }
    ],
    "screen_width": 2560,
    "screen_height": 1440,
    "focused_app": "Safari",
    "timestamp": 1702000000.0
  },
  "command": "click the submit button",
  "target_element_id": 0
}
```

### Training a Model

The collected data can train a model to:

1. **Encode the screen state**: Element roles, labels, positions
2. **Encode the command**: Natural language instruction
3. **Predict the target**: Which element ID to click

Simple approach (embedding similarity):
```python
# Embed command
command_emb = embed(command)

# Embed each element (role + label)
for elem in elements:
    elem_emb = embed(f"{elem.role} {elem.label}")
    score = cosine_similarity(command_emb, elem_emb)

# Pick highest scoring element
target = argmax(scores)
```

More sophisticated:
- Use a transformer to attend over all elements given the command
- Include positional features (normalized bbox coordinates)
- Include context (what's focused, app name)

### End-to-End Flow

1. **Collect data**: `ax collect -o data.jsonl`
2. **Train model**: Use the JSONL to train element prediction
3. **Deploy model**: Load trained weights
4. **Run inference**:
   ```python
   # Get current screen state
   screen = json.loads(subprocess.run(["ax", "list", "--json"], capture_output=True).stdout)

   # Predict target element
   target_id = model.predict(screen["elements"], command="click sign in")

   # Execute click
   subprocess.run(["ax", "click", str(target_id)])
   ```

## Integration with Claude Computer Use

For integrating with Claude's computer use capabilities:

### 1. Screen State as Context

```python
import subprocess
import json

def get_screen_context():
    result = subprocess.run(["ax", "list", "--json"], capture_output=True, text=True)
    state = json.loads(result.stdout)

    # Format for Claude
    context = f"Current app: {state['focused_app']}\n\nUI Elements:\n"
    for elem in state['elements']:
        context += f"- [{elem['id']}] {elem['role']}: \"{elem['label']}\"\n"

    return context
```

### 2. Tool Definition

```python
tools = [
    {
        "name": "click_element",
        "description": "Click a UI element by its ID",
        "input_schema": {
            "type": "object",
            "properties": {
                "element_id": {"type": "integer", "description": "The element ID from the screen state"}
            },
            "required": ["element_id"]
        }
    },
    {
        "name": "type_text",
        "description": "Type text into the focused element",
        "input_schema": {
            "type": "object",
            "properties": {
                "text": {"type": "string", "description": "Text to type"}
            },
            "required": ["text"]
        }
    }
]

def execute_tool(name, params):
    if name == "click_element":
        subprocess.run(["ax", "click", str(params["element_id"])])
    elif name == "type_text":
        subprocess.run(["ax", "type", params["text"]])
```

### 3. Agent Loop

```python
while not done:
    # Get current screen state
    screen_context = get_screen_context()

    # Ask Claude what to do
    response = claude.messages.create(
        model="claude-sonnet-4-20250514",
        system="You are controlling a Mac. Use the tools to interact with UI elements.",
        messages=[
            {"role": "user", "content": f"Task: {task}\n\n{screen_context}"}
        ],
        tools=tools
    )

    # Execute tool calls
    for tool_use in response.content:
        if tool_use.type == "tool_use":
            execute_tool(tool_use.name, tool_use.input)

    # Check if done
    done = check_task_complete()
```

## Tips

### Accuracy

- Element positions can shift - always refresh before clicking
- Some apps have poor accessibility support
- Web content in browsers is usually well-labeled
- Native macOS apps generally work well

### Performance

- Limit tree depth (`--depth 5`) for faster reads
- Use `--role` filter to reduce output
- Watch mode with long intervals for monitoring

### Debugging

- `ax tree --all` shows the full hierarchy
- `ax at <x> <y>` finds what's at a point
- `ax focus` shows the currently focused element

## Limitations

- **macOS only**: Uses Apple's Accessibility API
- **Permission required**: Must be granted in System Preferences
- **App support varies**: Some apps expose more elements than others
- **Dynamic content**: Elements change as the UI updates
- **No screenshot**: Only structured element data, not visual appearance

## Architecture

```
ax CLI
├── element.rs     # Element types, tree walking, screen state
├── tree.rs        # Tree visualization, interactive mode
├── actions.rs     # Click, type, and other interactions
├── collector.rs   # Training data collection
└── main.rs        # CLI argument parsing
```

Dependencies:
- `accessibility` - macOS Accessibility API bindings
- `core-foundation` / `core-graphics` - Apple framework bindings
- `cocoa` / `objc` - Objective-C runtime for NSWorkspace
- `enigo` - Cross-platform mouse/keyboard control
- `clap` - CLI argument parsing
- `serde` / `serde_json` - JSON serialization
