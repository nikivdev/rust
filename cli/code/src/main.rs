use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use clap::Parser;
use regex::Regex;

fn main() {
    if let Err(err) = try_main() {
        eprintln!("Error: {err}");
        std::process::exit(1);
    }
}

fn try_main() -> Result<()> {
    let args = Cli::parse();
    if args.line == 0 {
        bail!("line number must be 1 or greater");
    }
    if args.depth == 0 {
        bail!("depth must be 1 or greater");
    }

    let source = SourceFile::load(&args.file)?;
    if args.line > source.line_count() {
        bail!(
            "line {} is out of bounds for {} (max {})",
            args.line,
            args.file.display(),
            source.line_count()
        );
    }

    let target_line = args.line - 1;
    let primary_range = source
        .block_for_line(target_line)
        .unwrap_or(LineRange::new(target_line, target_line));

    println!("{}:{}\n", args.file.display(), args.line);
    print_range(&source, &primary_range);

    let mut seen = HashSet::new();
    if let Some(name) = source.infer_name(&primary_range) {
        seen.insert(name);
    }

    follow_references(&source, &primary_range, args.depth, 1, &mut seen);
    Ok(())
}

#[derive(Parser)]
#[command(
    name = "code",
    version,
    about = "Print an expanded view of the code around a given line."
)]
struct Cli {
    /// 1-based line number to expand.
    line: usize,
    /// Path to the file that contains the line.
    file: PathBuf,
    /// How many levels of referenced definitions to expand.
    #[arg(long, default_value_t = 1)]
    depth: usize,
}

fn follow_references(
    source: &SourceFile,
    range: &LineRange,
    max_depth: usize,
    current_depth: usize,
    seen: &mut HashSet<String>,
) {
    if current_depth > max_depth {
        return;
    }

    let names = referenced_symbols(source, range);
    for name in names {
        if !seen.insert(name.clone()) {
            continue;
        }

        if let Some(def_range) = source.find_definition(&name, Some(range)) {
            println!("\n{name}:\n");
            print_range(source, &def_range);

            follow_references(source, &def_range, max_depth, current_depth + 1, seen);
        }
    }
}

fn print_range(source: &SourceFile, range: &LineRange) {
    for line in range.start..=range.end {
        if let Some(text) = source.line(line) {
            println!("{text}");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LineRange {
    start: usize,
    end: usize,
}

impl LineRange {
    fn new(start: usize, end: usize) -> Self {
        Self { start, end }
    }

    fn contains(&self, line: usize) -> bool {
        line >= self.start && line <= self.end
    }
}

struct SourceFile {
    text: String,
    lines: Vec<String>,
    line_offsets: Vec<usize>,
}

impl SourceFile {
    fn load(path: &Path) -> Result<Self> {
        let text = fs::read_to_string(path)
            .with_context(|| format!("Unable to read {}", path.display()))?;
        let mut line_offsets = Vec::new();
        let mut offset = 0usize;
        let mut lines = Vec::new();

        for line in text.split_inclusive('\n') {
            line_offsets.push(offset);
            offset += line.len();
            lines.push(line.trim_end_matches('\n').to_string());
        }

        if text.is_empty() {
            bail!("{} is empty", path.display());
        }

        Ok(Self {
            text,
            lines,
            line_offsets,
        })
    }

    fn line_count(&self) -> usize {
        self.lines.len()
    }

    fn line(&self, idx: usize) -> Option<&str> {
        self.lines.get(idx).map(|s| s.as_str())
    }

    fn line_for_offset(&self, offset: usize) -> usize {
        match self
            .line_offsets
            .binary_search_by(|probe| probe.cmp(&offset))
        {
            Ok(exact) => exact,
            Err(insert) => insert.saturating_sub(1),
        }
    }

    fn block_for_line(&self, line: usize) -> Option<LineRange> {
        let line_start = *self.line_offsets.get(line)?;
        let mut offsets_to_try = vec![line_start];
        if let Some(text) = self.line(line) {
            if let Some(pos) = text.find('{') {
                offsets_to_try.push(line_start + pos);
            }
        }

        for offset in offsets_to_try {
            if let Some((start, end)) = brace_enclosed_block(&self.text, offset) {
                let start_line = self.line_for_offset(start);
                let end_line = self.line_for_offset(end);
                return Some(LineRange::new(start_line, end_line));
            }
        }

        Some(self.indent_block(line))
    }

    fn indent_block(&self, line: usize) -> LineRange {
        let current_indent = leading_whitespace(self.line(line).unwrap_or_default());
        let mut start = line;
        let mut end = line;

        // Walk backwards until indentation decreases meaningfully.
        while start > 0 {
            let prev = start - 1;
            let text = self.line(prev).unwrap_or_default();
            if text.trim().is_empty() {
                start = prev;
                continue;
            }

            if leading_whitespace(text) < current_indent {
                start = prev;
                break;
            }
            start = prev;
        }

        // Walk forwards until indentation drops.
        while end + 1 < self.lines.len() {
            let next = end + 1;
            let text = self.line(next).unwrap_or_default();
            if text.trim().is_empty() {
                end = next;
                continue;
            }

            if leading_whitespace(text) < current_indent {
                break;
            }
            end = next;
        }

        LineRange::new(start, end)
    }

    fn infer_name(&self, range: &LineRange) -> Option<String> {
        let header_line = self.line(range.start)?.trim();
        let patterns = [
            r#"(?m)^async\s+function\s+([A-Za-z_][A-Za-z0-9_]*)"#,
            r#"(?m)^function\s+([A-Za-z_][A-Za-z0-9_]*)"#,
            r#"(?m)^(?:pub\s+)?(?:async\s+)?fn\s+([A-Za-z_][A-Za-z0-9_]*)"#,
            r#"(?m)^class\s+([A-Za-z_][A-Za-z0-9_]*)"#,
            r#"(?m)^(?:const|let|var)\s+([A-Za-z_][A-Za-z0-9_]*)\s*="#,
        ];

        for pat in patterns {
            let re = Regex::new(pat).expect("valid regex");
            if let Some(caps) = re.captures(header_line) {
                if let Some(name) = caps.get(1) {
                    return Some(name.as_str().to_string());
                }
            }
        }

        None
    }

    fn find_definition(&self, name: &str, exclude: Option<&LineRange>) -> Option<LineRange> {
        let mut patterns = Vec::new();
        let escaped = regex::escape(name);
        patterns.push(format!(
            r"(?m)^[ \t]*(?:export\s+)?(?:async\s+)?function\s+{}\b",
            escaped
        ));
        patterns.push(format!(
            r"(?m)^[ \t]*(?:const|let|var)\s+{}\s*=\s*(?:async\s+)?function\b",
            escaped
        ));
        patterns.push(format!(
            r"(?m)^[ \t]*(?:const|let|var)\s+{}\s*=\s*[^=]*=>",
            escaped
        ));
        patterns.push(format!(
            r"(?m)^[ \t]*(?:pub\s+)?(?:async\s+)?fn\s+{}\b",
            escaped
        ));
        patterns.push(format!(r"(?m)^[ \t]*class\s+{}\b", escaped));

        for pat in patterns {
            let re = Regex::new(&pat).ok()?;
            for m in re.find_iter(&self.text) {
                if let Some(range) = self.capture_definition_range(m.start()) {
                    if let Some(excluded) = exclude {
                        if excluded.contains(range.start) {
                            continue;
                        }
                    }
                    return Some(range);
                }
            }
        }

        None
    }

    fn capture_definition_range(&self, offset: usize) -> Option<LineRange> {
        if let Some(open_pos) = self.text[offset..].find('{').map(|rel| offset + rel) {
            if let Some((start, end)) = brace_enclosed_block(&self.text, open_pos) {
                let start_line = self.line_for_offset(start);
                let end_line = self.line_for_offset(end);
                return Some(LineRange::new(start_line, end_line));
            }
        }

        let line = self.line_for_offset(offset);
        Some(LineRange::new(line, line))
    }
}

fn referenced_symbols(source: &SourceFile, range: &LineRange) -> Vec<String> {
    let mut names = Vec::new();
    let mut seen = HashSet::new();
    let re = Regex::new(r"(?m)(?<![\w.:])([A-Za-z_][A-Za-z0-9_]*)\s*\(").expect("valid call regex");
    let reserved: HashSet<&'static str> = [
        "if", "for", "while", "switch", "return", "match", "loop", "catch", "try", "await",
        "async", "fn", "function",
    ]
    .into_iter()
    .collect();

    for line in range.start..=range.end {
        if let Some(text) = source.line(line) {
            for caps in re.captures_iter(text) {
                if let Some(m) = caps.get(1) {
                    let name = m.as_str();
                    if reserved.contains(name) {
                        continue;
                    }
                    if seen.insert(name.to_string()) {
                        names.push(name.to_string());
                    }
                }
            }
        }
    }

    names
}

fn brace_enclosed_block(text: &str, target_offset: usize) -> Option<(usize, usize)> {
    let mut stack = Vec::new();

    for (idx, ch) in text.char_indices() {
        match ch {
            '{' => stack.push(idx),
            '}' => {
                let start = stack.pop()?;
                if start <= target_offset && target_offset <= idx {
                    return Some((start, idx));
                }
            }
            _ => {}
        }
    }

    None
}

fn leading_whitespace(text: &str) -> usize {
    text.chars()
        .take_while(|c| c.is_whitespace() && *c != '\n')
        .count()
}
