use dialoguer::{theme::ColorfulTheme, FuzzySelect};
use rand::distributions::Alphanumeric;
use rand::Rng;
use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::process::{Command as ProcessCommand, Stdio};

const DEFAULT_ROOT: &str = "~/t";
const TEMPLATE_ROOT: &str = "~/.config/try/templates";

fn main() {
    if let Err(err) = run() {
        eprintln!("try (rust) error: {err}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    match parse_args()? {
        CliCommand::Help => {
            print_help();
            return Ok(());
        }
        CliCommand::Version => {
            println!("{}", env!("CARGO_PKG_VERSION"));
            return Ok(());
        }
        CliCommand::Run(cfg) => execute(cfg),
    }
}

fn execute(cfg: ParsedArgs) -> Result<(), Box<dyn Error>> {
    let root = match cfg.root_override {
        Some(value) => expand_path(&value)?,
        None => {
            let raw = env::var("TRY_PATH").unwrap_or_else(|_| DEFAULT_ROOT.to_string());
            expand_path(&raw)?
        }
    };

    fs::create_dir_all(&root)?;
    let template_root = expand_path(TEMPLATE_ROOT)?;
    let template = select_template(&template_root)?;

    let custom = if let Some(ref raw) = cfg.custom_name {
        Some(sanitize_name(raw).ok_or_else(|| {
            format!("Provided name \"{raw}\" does not contain any valid characters")
        })?)
    } else {
        None
    };

    let (dir_name, target_path) = select_directory(&root, custom)?;
    fs::create_dir(&target_path)?;
    copy_template_contents(&template.path, &target_path)?;

    let absolute = target_path.canonicalize().unwrap_or(target_path.clone());
    eprintln!(
        "Created {dir_name} at {} using template {}",
        absolute.display(),
        template.name
    );

    if cfg.print_only || env::var("TRY_PRINT_ONLY").is_ok() || !io::stdout().is_terminal() {
        emit_cd_script(&absolute);
    } else {
        spawn_shell(&absolute)?;
    }

    Ok(())
}

fn emit_cd_script(path: &Path) {
    println!("cd {}", shell_quote(path));
}

fn spawn_shell(path: &Path) -> Result<(), Box<dyn Error>> {
    let shell = env::var("SHELL").unwrap_or_else(|_| "/bin/sh".to_string());
    eprintln!("Opening shell ({shell}) in {}", path.display());
    let mut command = ProcessCommand::new(&shell);
    command.current_dir(path);

    // For most shells, passing -i ensures an interactive session with prompts.
    command.arg("-i");
    command.stdin(Stdio::inherit());
    command.stdout(Stdio::inherit());
    command.stderr(Stdio::inherit());

    let status = command.status()?;
    if !status.success() {
        return Err(format!("Shell exited with status {status}").into());
    }

    Ok(())
}

fn select_directory(root: &Path, custom_name: Option<String>) -> Result<(String, PathBuf), Box<dyn Error>> {
    if let Some(name) = custom_name {
        let initial = name;
        let mut candidate = root.join(&initial);
        if !candidate.exists() {
            return Ok((initial, candidate));
        }

        let mut counter = 2;
        loop {
            let fallback = format!("{initial}-{counter}");
            candidate = root.join(&fallback);
            if !candidate.exists() {
                return Ok((fallback, candidate));
            }
            counter += 1;
        }
    }

    loop {
        let slug = random_slug(4);
        let candidate = root.join(&slug);
        if !candidate.exists() {
            return Ok((slug, candidate));
        }
    }
}

#[derive(Clone)]
struct Template {
    name: String,
    path: PathBuf,
}

fn select_template(template_root: &Path) -> Result<Template, Box<dyn Error>> {
    if !template_root.exists() {
        return Err(format!(
            "Template directory {} not found. Create templates inside this folder.",
            template_root.display()
        )
        .into());
    }

    let mut templates: Vec<Template> = Vec::new();
    for entry in fs::read_dir(template_root)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            templates.push(Template { name, path });
        }
    }

    if templates.is_empty() {
        return Err(
            format!(
                "No templates found in {}. Create a folder per template.",
                template_root.display()
            )
            .into(),
        );
    }

    templates.sort_by(|a, b| a.name.cmp(&b.name));

    let should_prompt =
        io::stdin().is_terminal() && io::stdout().is_terminal() && templates.len() > 1;
    if should_prompt {
        let names: Vec<String> = templates.iter().map(|t| t.name.clone()).collect();
        let selection = FuzzySelect::with_theme(&ColorfulTheme::default())
            .with_prompt("Select template")
            .default(0)
            .items(&names)
            .interact()?;
        Ok(templates[selection].clone())
    } else {
        Ok(templates.into_iter().next().unwrap())
    }
}

fn copy_template_contents(src: &Path, dest: &Path) -> Result<(), Box<dyn Error>> {
    if !src.is_dir() {
        return Err(format!("Template {} is not a directory", src.display()).into());
    }

    for entry in fs::read_dir(src)? {
        let entry = entry?;
        let entry_path = entry.path();
        let target_path = dest.join(entry.file_name());
        if entry_path.is_dir() {
            fs::create_dir(&target_path)?;
            copy_template_contents(&entry_path, &target_path)?;
        } else {
            fs::copy(&entry_path, &target_path)?;
        }
    }

    Ok(())
}

fn expand_path(input: &str) -> Result<PathBuf, Box<dyn Error>> {
    if input == "~" {
        return Ok(home_dir()?);
    }

    if let Some(stripped) = input.strip_prefix("~/") {
        let mut home = home_dir()?;
        if !stripped.is_empty() {
            home.push(stripped);
        }
        return Ok(home);
    }

    Ok(PathBuf::from(input))
}

fn home_dir() -> Result<PathBuf, Box<dyn Error>> {
    match env::var("HOME") {
        Ok(val) => Ok(PathBuf::from(val)),
        Err(_) => Err("HOME environment variable is not set".into()),
    }
}

fn random_slug(len: usize) -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .map(char::from)
        .map(|c| c.to_ascii_lowercase())
        .filter(|c| c.is_ascii_alphanumeric())
        .take(len)
        .collect()
}

fn sanitize_name(input: &str) -> Option<String> {
    let mut normalized = String::new();
    let mut last_was_dash = false;

    for ch in input.chars() {
        if ch.is_ascii_alphanumeric() {
            normalized.push(ch.to_ascii_lowercase());
            last_was_dash = false;
        } else if ch.is_ascii_whitespace() || matches!(ch, '-' | '_' | '.') {
            if !normalized.is_empty() && !last_was_dash {
                normalized.push('-');
                last_was_dash = true;
            }
        }
    }

    while normalized.starts_with('-') {
        normalized.remove(0);
    }

    while normalized.ends_with('-') {
        normalized.pop();
    }

    if normalized.is_empty() {
        None
    } else {
        Some(normalized)
    }
}

fn shell_quote(path: &Path) -> String {
    let raw = path.to_string_lossy();
    if raw.is_empty() {
        return "''".to_string();
    }

    let escaped = raw.replace('\'', "'\"'\"'");
    format!("'{}'", escaped)
}

fn parse_args() -> Result<CliCommand, String> {
    let mut args = env::args().skip(1);
    let mut print_only = false;
    let mut root_override = None;
    let mut name_parts: Vec<String> = Vec::new();

    while let Some(arg) = args.next() {
        if let Some(rest) = arg.strip_prefix("--path=") {
            if rest.is_empty() {
                return Err("`--path=` cannot be empty".to_string());
            }
            root_override = Some(rest.to_string());
            continue;
        }

        match arg.as_str() {
            "--help" | "-h" => return Ok(CliCommand::Help),
            "--version" | "-V" => return Ok(CliCommand::Version),
            "--print-script" | "--emit-script" | "--no-shell" | "-p" => {
                print_only = true;
            }
            "--path" => {
                let value = args
                    .next()
                    .ok_or_else(|| "--path requires a directory".to_string())?;
                root_override = Some(value);
            }
            _ => name_parts.push(arg),
        }
    }

    let custom_name = if name_parts.is_empty() {
        None
    } else {
        Some(name_parts.join("-"))
    };

    Ok(CliCommand::Run(ParsedArgs {
        print_only,
        root_override,
        custom_name,
    }))
}

fn print_help() {
    println!(
        "\
try - teleport to a fresh directory

USAGE:
    try [OPTIONS] [NAME...]

Options:
    -p, --print-script    Print a `cd` command instead of launching a shell
    --path <DIR>          Override the destination root (defaults to {DEFAULT_ROOT})
    -h, --help            Show this message
    -V, --version         Print the version

Arguments:
    NAME                  Optional words that form the directory slug (e.g. `api spike`)

Environment:
    TRY_PATH              Sets the default root directory
    TRY_PRINT_ONLY        Forces script printing even when running interactively
"
    );
}

enum CliCommand {
    Run(ParsedArgs),
    Help,
    Version,
}

struct ParsedArgs {
    print_only: bool,
    root_override: Option<String>,
    custom_name: Option<String>,
}
