use std::{env, fs, path::PathBuf, process};

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();

    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        eprintln!("trash - safe rm replacement");
        eprintln!();
        eprintln!("Usage: trash <files...>");
        eprintln!("       trash -rf <files...>  (flags ignored, just moves)");
        eprintln!();
        eprintln!("Moves files to ~/trash instead of deleting.");
        eprintln!("Alias: rr");
        return;
    }

    let home = env::var("HOME").expect("HOME not set");
    let trash_dir = PathBuf::from(&home).join("trash");

    // Ensure ~/trash exists
    if let Err(e) = fs::create_dir_all(&trash_dir) {
        eprintln!("Failed to create ~/trash: {}", e);
        process::exit(1);
    }

    let mut errors = 0;

    for arg in args {
        // Skip flags (compatibility with rm -rf)
        if arg.starts_with('-') {
            continue;
        }

        let path = PathBuf::from(&arg);
        if !path.exists() {
            eprintln!("trash: {}: No such file or directory", arg);
            errors += 1;
            continue;
        }

        let name = path.file_name().unwrap_or_default();
        let mut dest = trash_dir.join(name);

        // Handle conflicts by adding numeric suffix
        if dest.exists() {
            let name_str = name.to_string_lossy();
            let mut i = 2;
            loop {
                dest = trash_dir.join(format!("{}.{}", name_str, i));
                if !dest.exists() {
                    break;
                }
                i += 1;
            }
        }

        // Move (instant on same filesystem)
        if let Err(e) = fs::rename(&path, &dest) {
            // If rename fails (cross-device), fall back to copy+delete
            if e.raw_os_error() == Some(18) {
                // EXDEV - cross-device link
                if let Err(e) = move_cross_device(&path, &dest) {
                    eprintln!("trash: {}: {}", arg, e);
                    errors += 1;
                }
            } else {
                eprintln!("trash: {}: {}", arg, e);
                errors += 1;
            }
        }
    }

    if errors > 0 {
        process::exit(1);
    }
}

fn move_cross_device(src: &PathBuf, dest: &PathBuf) -> std::io::Result<()> {
    if src.is_dir() {
        // Use system cp -R for directories
        let status = std::process::Command::new("cp")
            .arg("-R")
            .arg(src)
            .arg(dest)
            .status()?;
        if !status.success() {
            return Err(std::io::Error::new(std::io::ErrorKind::Other, "cp failed"));
        }
        fs::remove_dir_all(src)?;
    } else {
        fs::copy(src, dest)?;
        fs::remove_file(src)?;
    }
    Ok(())
}
