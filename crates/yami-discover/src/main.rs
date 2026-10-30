//! Minimal local CLI demonstration for YamiNet website discovery.
//!
//! The `demo` command uses the three sample directory documents compiled into
//! the binary. It performs no network access and no crawling.
use std::{
    collections::HashSet,
    path::{Path, PathBuf},
};
use yami_discover::{Catalog, MAX_RESULTS, MAX_SOURCE_LABEL_CHARS};

const SAMPLES: &[(&str, &str)] = &[
    (
        "community",
        include_str!("../../../sites/directories/community.ylang"),
    ),
    (
        "garden",
        include_str!("../../../sites/directories/garden.ylang"),
    ),
    ("lab", include_str!("../../../sites/directories/lab.ylang")),
];

const USAGE: &str = "\
yami-discover 0.4 — local, untrusted website discovery

Usage:
  yami-discover demo [QUERY]
  yami-discover search --directory FILE... QUERY
  yami-discover list --directory FILE...
  yami-discover validate --directory FILE...

Commands:
  demo       Import the bundled sample directories and search them (default
             query: yami). No network access.
  search     Import local YLang directory documents and search locally.
  list       Import local directory documents and list every merged entry.
  validate   Parse, validate and merge local directory documents, then report
             source and duplicate counts.

Options:
  --directory FILE   Directory document to import (repeatable; --dir also works)
  -h, --help         Show this help

Entries are untrusted hints. Opening a listed site must still authenticate its
yn1- identity through the normal YamiNet TLS 1.3 stack. This tool opens no
connections, runs no crawler and sends no query to any directory operator.";

const TRUST_NOTE: &str = "Entries are untrusted hints: opening one still requires the existing TLS 1.3 identity check. This tool performs no network access.";

fn main() {
    if let Err(error) = run(std::env::args().skip(1).collect()) {
        eprintln!("yami-discover: {error}");
        std::process::exit(1);
    }
}

fn run(args: Vec<String>) -> Result<(), String> {
    let Some(command) = args.first().map(String::as_str) else {
        println!("{USAGE}");
        return Ok(());
    };
    match command {
        "-h" | "--help" | "help" => {
            println!("{USAGE}");
            Ok(())
        }
        "demo" => demo(&args[1..]),
        "search" => search(&args[1..]),
        "list" => list(&args[1..]),
        "validate" => validate(&args[1..]),
        other => Err(format!("unknown command {other:?}\n\n{USAGE}")),
    }
}

fn demo(args: &[String]) -> Result<(), String> {
    if args.len() > 1 {
        return Err(format!("demo accepts at most one QUERY\n\n{USAGE}"));
    }
    let query = args.first().map(String::as_str).unwrap_or("yami");
    let mut catalog = Catalog::new();
    println!("Bundled sample directories (no network):");
    let mut duplicates = 0;
    for (label, source) in SAMPLES {
        let report = catalog
            .import_text(label, source)
            .map_err(|error| format!("sample {label:?}: {error}"))?;
        duplicates += report.source.duplicates;
        println!(
            "  {label:<10} {} — {} entries ({} added, {} duplicate(s))",
            report.source.name,
            report.source.entries,
            report.source.added,
            report.source.duplicates
        );
    }
    println!(
        "\nMerged: {} unique entries from {} independent sources ({} duplicate listing(s) removed).",
        catalog.len(),
        catalog.sources().len(),
        duplicates
    );
    print_hits(&catalog, query)?;
    println!("\n{TRUST_NOTE}");
    Ok(())
}

fn search(args: &[String]) -> Result<(), String> {
    let (paths, positional) = parse_directories(args)?;
    if paths.is_empty() {
        return Err(format!(
            "search requires at least one --directory FILE\n\n{USAGE}"
        ));
    }
    if positional.is_empty() {
        return Err(format!("search requires QUERY\n\n{USAGE}"));
    }
    let query = positional.join(" ");
    let catalog = import_paths(&paths)?;
    print_hits(&catalog, &query)?;
    println!("\n{TRUST_NOTE}");
    Ok(())
}

fn list(args: &[String]) -> Result<(), String> {
    let (paths, positional) = parse_directories(args)?;
    if !positional.is_empty() {
        return Err(format!("list does not accept a query\n\n{USAGE}"));
    }
    if paths.is_empty() {
        return Err(format!(
            "list requires at least one --directory FILE\n\n{USAGE}"
        ));
    }
    let catalog = import_paths(&paths)?;
    let mut entries: Vec<_> = catalog.entries().iter().collect();
    entries.sort_by_key(|entry| entry.title().to_lowercase());
    println!("\n{} unique entries:", entries.len());
    for (index, entry) in entries.iter().enumerate() {
        println!("{}. {}", index + 1, entry.title());
        println!("   {}", entry.address());
        println!("   {}", entry.description());
        let category = entry
            .category()
            .map(|category| format!(" · {category}"))
            .unwrap_or_default();
        println!("   source: {}{category}", entry.source());
    }
    println!("\n{TRUST_NOTE}");
    Ok(())
}

fn validate(args: &[String]) -> Result<(), String> {
    let (paths, positional) = parse_directories(args)?;
    if !positional.is_empty() {
        return Err(format!("validate does not accept a query\n\n{USAGE}"));
    }
    if paths.is_empty() {
        return Err(format!(
            "validate requires at least one --directory FILE\n\n{USAGE}"
        ));
    }
    let catalog = import_paths(&paths)?;
    println!(
        "\nValid: {} source(s), {} unique entries, no malformed documents.",
        catalog.sources().len(),
        catalog.len()
    );
    Ok(())
}

fn import_paths(paths: &[PathBuf]) -> Result<Catalog, String> {
    let mut catalog = Catalog::new();
    let mut used = HashSet::new();
    println!("Importing local directory documents:");
    for path in paths {
        let label = unique_label(path, &mut used);
        let report = catalog
            .import_file(&label, path)
            .map_err(|error| format!("{}: {error}", path.display()))?;
        println!(
            "  {label:<16} {} — {} entries ({} added, {} duplicate(s))",
            report.source.name,
            report.source.entries,
            report.source.added,
            report.source.duplicates
        );
    }
    println!(
        "Merged: {} unique entries from {} independent sources.",
        catalog.len(),
        catalog.sources().len()
    );
    Ok(catalog)
}

fn unique_label(path: &Path, used: &mut HashSet<String>) -> String {
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("directory");
    let base: String = stem
        .trim()
        .chars()
        .filter(|character| !character.is_control())
        .take(64)
        .collect();
    let base = if base.is_empty() {
        "directory".to_string()
    } else {
        base
    };
    let mut label = base.clone();
    let mut suffix = 2;
    while used.contains(&label) {
        let suffix_text = format!("-{suffix}");
        let keep = MAX_SOURCE_LABEL_CHARS.saturating_sub(suffix_text.chars().count());
        label = format!(
            "{}{suffix_text}",
            base.chars().take(keep).collect::<String>()
        );
        suffix += 1;
    }
    used.insert(label.clone());
    label
}

fn print_hits(catalog: &Catalog, query: &str) -> Result<(), String> {
    let hits = catalog.search(query).map_err(|error| error.to_string())?;
    if hits.is_empty() {
        println!("\nNo entries matched {query:?}.");
        return Ok(());
    }
    let limit_note = if hits.len() == MAX_RESULTS {
        format!(" (showing at most {MAX_RESULTS})")
    } else {
        String::new()
    };
    println!("\nSearch {query:?}: {} result(s){limit_note}", hits.len());
    for (index, hit) in hits.iter().enumerate() {
        let entry = hit.entry;
        println!("{}. {}", index + 1, entry.title());
        println!("   {}", entry.address());
        println!("   {}", entry.description());
        let category = entry
            .category()
            .map(|category| format!(" · {category}"))
            .unwrap_or_default();
        println!("   source: {}{category}", entry.source());
    }
    Ok(())
}

fn parse_directories(args: &[String]) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let mut paths = Vec::new();
    let mut positional = Vec::new();
    let mut literal = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if !literal && argument == "--" {
            literal = true;
        } else if !literal && matches!(argument.as_str(), "--directory" | "--dir") {
            index += 1;
            let path = args
                .get(index)
                .ok_or_else(|| format!("{} requires a file path\n\n{USAGE}", argument))?;
            paths.push(PathBuf::from(path));
        } else if !literal && argument.starts_with("--") {
            return Err(format!("unknown option {argument:?}\n\n{USAGE}"));
        } else {
            positional.push(argument.clone());
        }
        index += 1;
    }
    Ok((paths, positional))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn directory_options_and_positional_arguments_are_separated() {
        let args = vec![
            "--directory".to_string(),
            "one.ylang".to_string(),
            "--dir".to_string(),
            "two.ylang".to_string(),
            "café".to_string(),
            "notes".to_string(),
        ];
        let (paths, positional) = parse_directories(&args).unwrap();
        assert_eq!(paths.len(), 2);
        assert_eq!(positional, ["café", "notes"]);
        assert!(parse_directories(&["--directory".to_string()]).is_err());
        assert!(parse_directories(&["--unknown".to_string()]).is_err());
    }

    #[test]
    fn labels_are_unique_and_bounded() {
        let mut used = HashSet::new();
        let first = unique_label(Path::new("a/community.ylang"), &mut used);
        let second = unique_label(Path::new("b/community.ylang"), &mut used);
        assert_eq!(first, "community");
        assert_eq!(second, "community-2");
        assert!(second.chars().count() <= MAX_SOURCE_LABEL_CHARS);
    }
}
