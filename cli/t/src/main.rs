use chrono::Local;
use std::fs;
use std::path::PathBuf;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() > 1 {
        match args[1].as_str() {
            "archive" => {
                archive();
                return;
            }
            "init" => {
                if args.len() > 2 {
                    init_shell(&args[2]);
                } else {
                    eprintln!("Usage: t init <shell>");
                    eprintln!("Supported shells: fish, zsh, bash");
                }
                return;
            }
            _ => {}
        }
    }

    // Default: create new temp directory
    create_new();
}

fn init_shell(shell: &str) {
    match shell {
        "fish" => {
            print!(
                r#"if not functions -q t
    function t
        if test (count $argv) -gt 0 -a "$argv[1]" = "init"
            ~/bin/t $argv
            return
        end
        set -l path (~/bin/t $argv)
        if test -n "$path" -a -d "$path"
            cd $path
        end
    end
end
"#
            );
        }
        "zsh" | "bash" => {
            print!(
                r#"if ! type t &>/dev/null; then
    t() {{
        if [[ "$1" == "init" ]]; then
            ~/bin/t "$@"
            return
        fi
        local path
        path=$(~/bin/t "$@")
        if [[ -n "$path" && -d "$path" ]]; then
            cd "$path"
        fi
    }}
fi
"#
            );
        }
        _ => {
            eprintln!("Unsupported shell: {}", shell);
            eprintln!("Supported shells: fish, zsh, bash");
        }
    }
}

fn create_new() {
    let home = dirs::home_dir().expect("Could not find home directory");
    let base_dir = home.join("t");

    // Ensure ~/t exists
    fs::create_dir_all(&base_dir).expect("Could not create ~/t directory");

    // Get current date as "jan-13" format
    let now = Local::now();
    let date_str = now.format("%b-%d").to_string().to_lowercase();

    // Find available path
    let path = find_available_path(&base_dir, &date_str);

    // Create the directory
    fs::create_dir_all(&path).expect("Could not create directory");

    // Output path for shell to cd into
    println!("{}", path.display());
}

fn archive() {
    let home = dirs::home_dir().expect("Could not find home directory");
    let source_dir = home.join("t");
    let archive_base = home.join("past").join("t");

    if !source_dir.exists() {
        eprintln!("~/t does not exist");
        return;
    }

    // Check if there's anything to archive
    let entries: Vec<_> = fs::read_dir(&source_dir)
        .expect("Could not read ~/t")
        .flatten()
        .filter(|e| !e.file_name().to_string_lossy().starts_with('.'))
        .collect();

    if entries.is_empty() {
        eprintln!("~/t is empty, nothing to archive");
        return;
    }

    // Ensure ~/past/t exists
    fs::create_dir_all(&archive_base).expect("Could not create ~/past/t directory");

    // Create dated archive folder: 2026-jan-13 or 2026-jan-13-2 if conflict
    let now = Local::now();
    let date_str = now.format("%Y-%b-%d").to_string().to_lowercase();
    let archive_dir = find_available_path(&archive_base, &date_str);

    fs::create_dir_all(&archive_dir).expect("Could not create archive directory");

    let mut moved = 0;
    for entry in entries {
        let path = entry.path();
        let name = entry.file_name();
        let dest = archive_dir.join(&name);

        if let Err(e) = fs::rename(&path, &dest) {
            eprintln!("Failed to move {}: {}", name.to_string_lossy(), e);
            continue;
        }
        moved += 1;
    }

    eprintln!(
        "Archived {} items to {}",
        moved,
        archive_dir.display()
    );
}

fn find_available_path(base: &PathBuf, date: &str) -> PathBuf {
    let first = base.join(date);
    if !first.exists() {
        return first;
    }

    let mut i = 2;
    loop {
        let path = base.join(format!("{}-{}", date, i));
        if !path.exists() {
            return path;
        }
        i += 1;
    }
}

