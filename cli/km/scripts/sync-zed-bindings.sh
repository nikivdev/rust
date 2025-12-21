#!/bin/bash
# Sync karabiner.edn "zed:" bindings to Keyboard Maestro macros
# Usage: ./sync-zed-bindings.sh [--dry-run]

set -e

DRY_RUN=false
if [[ "$1" == "--dry-run" ]]; then
    DRY_RUN=true
fi

KARABINER_EDN="$HOME/config/i/karabiner/karabiner.edn"
KM_BIN="$HOME/bin/km"

# Extract all unique "zed:" macro names from karabiner.edn
# Matches patterns like [:km "zed: name"]
extract_bindings() {
    grep -oE 'zed: [^"]+' "$KARABINER_EDN" | sort -u | sed 's/zed: //'
}

# Map macro name to path
# Add new mappings here as needed
get_path() {
    local name="$1"
    case "$name" in
        # Config files
        "~/config/i/karabiner/karabiner.edn") echo "$HOME/config/i/karabiner/karabiner.edn" ;;
        "~/.config/lin/config.ts") echo "$HOME/.config/lin/config.ts" ;;
        "~/config/wip.md") echo "$HOME/config/wip.md" ;;
        "~/config/fish/fn.fish") echo "$HOME/config/fish/fn.fish" ;;
        "~/config") echo "$HOME/config" ;;

        # Main project dirs
        "flow") echo "$HOME/flow" ;;
        "train") echo "$HOME/train" ;;
        "research") echo "$HOME/research" ;;
        "nikiv") echo "$HOME/nikiv" ;;

        # org/1f projects
        "glide") echo "$HOME/org/1f/glide" ;;
        "gitedit") echo "$HOME/org/1f/gitedit" ;;
        "focus") echo "$HOME/org/1f/focus" ;;
        "base") echo "$HOME/org/1f/base" ;;

        # lang dirs
        "rust") echo "$HOME/lang/rust" ;;
        "ts") echo "$HOME/lang/ts" ;;
        "py") echo "$HOME/lang/py" ;;
        "mojo") echo "$HOME/lang/mojo" ;;

        # org/linsa
        "lin") echo "$HOME/org/linsa/lin" ;;
        "lin-ios") echo "$HOME/org/linsa/lin-ios" ;;
        "la") echo "$HOME/org/linsa/la" ;;

        # ai/ml projects
        "ai") echo "$HOME/ai" ;;
        "gen") echo "$HOME/gen" ;;
        "genx") echo "$HOME/genx" ;;
        "infra") echo "$HOME/infra" ;;

        # Fork/external projects (fork/i/)
        "reatom") echo "$HOME/fork/i/artalar/reatom" ;;
        "verifiers") echo "$HOME/fork/i/openai/verifiers" ;;
        "codex") echo "$HOME/fork/i/openai/codex" ;;
        "rig") echo "$HOME/fork/i/0xPlaygrounds/rig" ;;
        "llama.cpp") echo "$HOME/fork/i/ggerganov/llama.cpp" ;;
        "opencode") echo "$HOME/fork/i/opencode-ai/opencode" ;;
        "elysia") echo "$HOME/fork/i/elysiajs/elysia" ;;
        "raycast-fork") echo "$HOME/fork/i/raycast/extensions" ;;
        "jax") echo "$HOME/fork/i/jax-ml/jax" ;;
        "ell") echo "$HOME/fork/i/MadcowD/ell" ;;
        "agno") echo "$HOME/fork/i/agno-agi/agno" ;;
        "jax-js") echo "$HOME/fork/i/aspect-build/aspect-cli" ;;
        "jazz") echo "$HOME/fork/i/garden-co/jazz" ;;
        "flox") echo "$HOME/fork/i/flox/flox" ;;
        "electric") echo "$HOME/fork/i/electric-sql/electric" ;;
        "encore") echo "$HOME/fork/i/encoredev/encore" ;;
        "obs") echo "$HOME/fork/i/obsproject/obs-studio" ;;
        "vllm") echo "$HOME/fork/i/vllm-project/vllm" ;;
        "zed") echo "$HOME/fork/i/zed-industries/zed" ;;
        "flowglad") echo "$HOME/fork/i/flowglad/flowglad" ;;
        "LMCache") echo "$HOME/fork/i/LMCache/LMCache" ;;
        "karabiner") echo "$HOME/fork/i/pqrs-org/Karabiner-Elements" ;;
        "mini-sglang") echo "$HOME/fork-i/sgl-project/mini-sglang" ;;
        "mlx") echo "$HOME/fork-i/ml-explore/mlx" ;;

        # Algos
        "algos") echo "$HOME/algos" ;;

        # Default: try common locations
        *)
            # Search fork/i subdirectories for matching project name
            local found=$(find "$HOME/fork/i" -maxdepth 2 -type d -name "$name" 2>/dev/null | head -1)
            if [[ -n "$found" ]]; then
                echo "$found"
            # Try org/1f
            elif [[ -d "$HOME/org/1f/$name" ]]; then
                echo "$HOME/org/1f/$name"
            # Try org/linsa
            elif [[ -d "$HOME/org/linsa/$name" ]]; then
                echo "$HOME/org/linsa/$name"
            # Try lang
            elif [[ -d "$HOME/lang/$name" ]]; then
                echo "$HOME/lang/$name"
            # Try direct home
            elif [[ -d "$HOME/$name" ]]; then
                echo "$HOME/$name"
            else
                echo ""
            fi
            ;;
    esac
}

# Get existing KM macros
get_existing_macros() {
    "$KM_BIN" list 2>/dev/null | grep "^zed:" | cut -f1 | sed 's/zed: //'
}

echo "Extracting zed bindings from karabiner.edn..."
bindings=$(extract_bindings)

echo "Checking existing KM macros..."
existing=$(get_existing_macros)

echo ""
echo "=== Sync Report ==="
echo ""

missing_count=0
created_count=0

while IFS= read -r name; do
    [[ -z "$name" ]] && continue

    # Check if macro exists
    if echo "$existing" | grep -qx "$name"; then
        continue
    fi

    path=$(get_path "$name")

    if [[ -z "$path" ]]; then
        echo "SKIP: zed: $name (no path mapping, add to script)"
        ((missing_count++))
        continue
    fi

    if [[ ! -e "$path" ]]; then
        echo "SKIP: zed: $name -> $path (path doesn't exist)"
        ((missing_count++))
        continue
    fi

    if $DRY_RUN; then
        echo "WOULD CREATE: zed: $name -> $path"
    else
        echo "CREATE: zed: $name -> $path"
        "$KM_BIN" create-open "zed: $name" Zed "$path"
    fi
    ((created_count++))

done <<< "$bindings"

echo ""
echo "=== Summary ==="
if $DRY_RUN; then
    echo "Would create: $created_count macros"
else
    echo "Created: $created_count macros"
fi
echo "Skipped (no mapping or path): $missing_count"
