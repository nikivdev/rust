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
            template => {
                // Check if it's a template in ~/new/
                let home = dirs::home_dir().expect("Could not find home directory");
                let template_dir = home.join("new").join(template);
                if template_dir.exists() && template_dir.is_dir() {
                    create_from_template(template);
                    return;
                }
                // Unknown argument, show help
                eprintln!("Unknown command or template: {}", template);
                eprintln!("Usage: t [template]");
                eprintln!("       t archive");
                eprintln!("       t init <shell>");
                list_templates();
                return;
            }
        }
    }

    // Default: create new temp directory
    create_new(None);
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

fn create_new(template: Option<&str>) -> PathBuf {
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

    // Output path immediately so shell can cd
    println!("{}", path.display());

    // Copy template in background if provided
    if let Some(tmpl) = template {
        let template_dir = home.join("new").join(tmpl);
        let dst = path.clone();

        // Spawn background process to copy
        std::process::Command::new("cp")
            .arg("-R")
            .arg(format!("{}/.", template_dir.display()))
            .arg(&dst)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok();
    }

    path
}

fn create_from_template(template: &str) {
    create_new(Some(template));
}

fn list_templates() {
    let home = dirs::home_dir().expect("Could not find home directory");
    let new_dir = home.join("new");

    if !new_dir.exists() {
        return;
    }

    let templates: Vec<_> = fs::read_dir(&new_dir)
        .ok()
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();

    if !templates.is_empty() {
        eprintln!("Available templates: {}", templates.join(", "));
    }
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

