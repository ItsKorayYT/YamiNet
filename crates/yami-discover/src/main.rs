//! Local CLI for demonstrating, authoring, validating and searching YamiNet
//! website directories.
//!
//! The `demo` command uses the three sample directory documents compiled into
//! the binary. Authoring commands write only local YLang files. This tool
//! performs no network access and no crawling.
use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
};
use yami_discover::{
    Catalog, Directory, DirectoryEntry, WebsiteAddress, MAX_RESULTS, MAX_SOURCE_LABEL_CHARS,
};

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
yami-discover 0.4 — local, untrusted website discovery and authoring

Usage:
  yami-discover demo [QUERY]
  yami-discover search --directory FILE... QUERY
  yami-discover list --directory FILE...
  yami-discover validate --directory FILE...

  yami-discover create FILE --title TITLE
  yami-discover add FILE --title TITLE --description TEXT --address YAMI_URL [--category CATEGORY]
  yami-discover remove FILE --address YAMI_URL
  yami-discover edit FILE --address YAMI_URL [--title TITLE] [--description TEXT] [--category CATEGORY | --clear-category]
  yami-discover show FILE

Commands:
  demo       Import the bundled sample directories and search them (default
             query: yami). No network access.
  search     Import local YLang directory documents and search locally.
  list       Import local directory documents and list every merged entry.
  validate   Parse, validate and merge local directory documents, then report
             source and duplicate counts.
  create     Create a new empty version-1 directory. Refuses to overwrite.
  add        Append a website entry. Rejects duplicate canonical addresses.
  remove     Remove the entry with the given canonical address.
  edit       Change an existing entry's title, description or category.
  show       Display one directory's metadata and entries.

Options:
  --directory FILE   Directory document to import (repeatable; --dir also works)
  --title TITLE      Website or directory title
  --description TEXT Short plain-text description
  --address URL      Canonical yami:// node identity and document key
  --category NAME    Optional category (set at most once)
  --clear-category   Remove the category during edit
  -h, --help         Show this help

Authoring writes atomically through a temporary file. Entries keep document
order: add appends, edit replaces in place, remove deletes in place. Generated
descriptions are escaped so reserved YLang prefixes and brackets round-trip.

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
        "create" => create(&args[1..]),
        "add" => add(&args[1..]),
        "remove" => remove(&args[1..]),
        "edit" => edit(&args[1..]),
        "show" => show(&args[1..]),
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

fn create(args: &[String]) -> Result<(), String> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_command_options(args, &["--title"], &[], "create")?;
    let file = options.file("create")?;
    let directory = Directory::new(options.required("create", "--title")?, Vec::new())
        .map_err(|error| error.to_string())?;
    directory
        .save_new(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    println!("Created {} (0 entries).", file.display());
    Ok(())
}

fn add(args: &[String]) -> Result<(), String> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_command_options(
        args,
        &["--title", "--description", "--address", "--category"],
        &[],
        "add",
    )?;
    let file = options.file("add")?;
    let address = parse_address(options.required("add", "--address")?)?;
    let canonical = address.to_string();
    let title = options.required("add", "--title")?;
    let entry = DirectoryEntry::new(
        title,
        options.required("add", "--description")?,
        address,
        options.values.get("--category").cloned(),
    )
    .map_err(|error| error.to_string())?;
    let mut directory =
        Directory::load(file).map_err(|error| format!("{}: {error}", file.display()))?;
    directory
        .add_entry(entry)
        .map_err(|error| error.to_string())?;
    directory
        .save(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    println!(
        "Added {title:?} ({canonical}); {} entries.",
        directory.entries().len()
    );
    Ok(())
}

fn remove(args: &[String]) -> Result<(), String> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_command_options(args, &["--address"], &[], "remove")?;
    let file = options.file("remove")?;
    let address = parse_address(options.required("remove", "--address")?)?;
    let mut directory =
        Directory::load(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let removed = directory
        .remove_entry(&address)
        .map_err(|error| error.to_string())?;
    directory
        .save(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    println!(
        "Removed {:?} ({}); {} entries remain.",
        removed.title(),
        removed.address(),
        directory.entries().len()
    );
    Ok(())
}

fn edit(args: &[String]) -> Result<(), String> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_command_options(
        args,
        &["--address", "--title", "--description", "--category"],
        &["--clear-category"],
        "edit",
    )?;
    let file = options.file("edit")?;
    let address = parse_address(options.required("edit", "--address")?)?;
    let clear_category = options.flag("--clear-category");
    if clear_category && options.values.contains_key("--category") {
        return Err(format!(
            "edit: --category and --clear-category cannot be combined\n\n{USAGE}"
        ));
    }
    let has_change = clear_category
        || ["--title", "--description", "--category"]
            .iter()
            .any(|option| options.values.contains_key(*option));
    if !has_change {
        return Err(format!(
            "edit requires at least one of --title, --description, --category or --clear-category\n\n{USAGE}"
        ));
    }

    let mut directory =
        Directory::load(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let current = directory
        .entry(&address)
        .ok_or_else(|| format!("edit: no entry with address {address}"))?;
    let title = options
        .values
        .get("--title")
        .map(String::as_str)
        .unwrap_or_else(|| current.title());
    let description = options
        .values
        .get("--description")
        .map(String::as_str)
        .unwrap_or_else(|| current.description());
    let category = if clear_category {
        None
    } else if let Some(category) = options.values.get("--category") {
        Some(category.clone())
    } else {
        current.category().map(str::to_string)
    };
    let updated_title = title.to_string();
    let replacement = DirectoryEntry::new(title, description, address.clone(), category)
        .map_err(|error| error.to_string())?;
    directory
        .replace_entry(&address, replacement)
        .map_err(|error| error.to_string())?;
    directory
        .save(file)
        .map_err(|error| format!("{}: {error}", file.display()))?;
    println!(
        "Updated {updated_title:?} ({address}); {} entries.",
        directory.entries().len()
    );
    Ok(())
}

fn show(args: &[String]) -> Result<(), String> {
    if wants_help(args) {
        println!("{USAGE}");
        return Ok(());
    }
    let options = parse_command_options(args, &[], &[], "show")?;
    let file = options.file("show")?;
    let directory =
        Directory::load(file).map_err(|error| format!("{}: {error}", file.display()))?;
    let language = directory
        .language()
        .map(|language| format!(", language {language}"))
        .unwrap_or_default();
    println!(
        "\n{} — {} (format {}, {} entries{language})",
        file.display(),
        directory.name(),
        directory.version(),
        directory.entries().len()
    );
    for (index, entry) in directory.entries().iter().enumerate() {
        println!("{}. {}", index + 1, entry.title());
        println!("   {}", entry.address());
        println!("   {}", entry.description());
        if let Some(category) = entry.category() {
            println!("   category: {category}");
        }
    }
    println!("\n{TRUST_NOTE}");
    Ok(())
}

fn parse_address(text: &str) -> Result<WebsiteAddress, String> {
    WebsiteAddress::parse(text).map_err(|error| format!("invalid --address: {error}"))
}

#[derive(Debug, Default)]
struct CommandOptions {
    values: HashMap<String, String>,
    flags: HashSet<String>,
    positionals: Vec<String>,
}

impl CommandOptions {
    fn required(&self, command: &str, option: &str) -> Result<&str, String> {
        self.values
            .get(option)
            .map(String::as_str)
            .ok_or_else(|| format!("{command} requires {option}\n\n{USAGE}"))
    }

    fn flag(&self, option: &str) -> bool {
        self.flags.contains(option)
    }

    fn file(&self, command: &str) -> Result<&Path, String> {
        match self.positionals.as_slice() {
            [file] => Ok(Path::new(file)),
            [] => Err(format!("{command} requires exactly one FILE\n\n{USAGE}")),
            _ => Err(format!(
                "{command} accepts exactly one FILE, not {}\n\n{USAGE}",
                self.positionals.len()
            )),
        }
    }
}

fn wants_help(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("-h" | "--help"))
}

fn parse_command_options(
    args: &[String],
    value_options: &[&str],
    flag_options: &[&str],
    command: &str,
) -> Result<CommandOptions, String> {
    let mut parsed = CommandOptions::default();
    let mut literal = false;
    let mut index = 0;
    while index < args.len() {
        let argument = &args[index];
        if !literal && argument == "--" {
            literal = true;
        } else if !literal && argument.starts_with("--") {
            let (option, inline) = match argument.split_once('=') {
                Some((option, value)) => (option, Some(value)),
                None => (argument.as_str(), None),
            };
            if flag_options.contains(&option) {
                if inline.is_some() {
                    return Err(format!(
                        "{command}: {option} does not take a value\n\n{USAGE}"
                    ));
                }
                if !parsed.flags.insert(option.to_string()) {
                    return Err(format!(
                        "{command}: {option} was given more than once\n\n{USAGE}"
                    ));
                }
            } else if value_options.contains(&option) {
                let value = match inline {
                    Some(value) => value.to_string(),
                    None => {
                        index += 1;
                        args.get(index).cloned().ok_or_else(|| {
                            format!("{command}: {option} requires a value\n\n{USAGE}")
                        })?
                    }
                };
                if parsed.values.insert(option.to_string(), value).is_some() {
                    return Err(format!(
                        "{command}: {option} was given more than once\n\n{USAGE}"
                    ));
                }
            } else {
                return Err(format!("{command}: unknown option {option:?}\n\n{USAGE}"));
            }
        } else {
            parsed.positionals.push(argument.clone());
        }
        index += 1;
    }
    Ok(parsed)
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

    macro_rules! command {
        ($($argument:expr),* $(,)?) => {
            vec![$($argument.to_string()),*]
        };
    }

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

    #[test]
    fn command_options_support_inline_values_and_boolean_flags() {
        let args = vec![
            "file.ylang".to_string(),
            "--title=Café".to_string(),
            "--category".to_string(),
            "Journal".to_string(),
            "--clear-category".to_string(),
        ];
        let options = parse_command_options(
            &args,
            &["--title", "--category"],
            &["--clear-category"],
            "edit",
        )
        .unwrap();
        assert_eq!(options.values.get("--title").unwrap(), "Café");
        assert_eq!(options.values.get("--category").unwrap(), "Journal");
        assert!(options.flag("--clear-category"));
        assert_eq!(options.positionals, ["file.ylang"]);

        assert!(
            parse_command_options(&["--unknown".to_string()], &["--title"], &[], "create").is_err()
        );
        assert!(parse_command_options(
            &["--clear-category=yes".to_string()],
            &[],
            &["--clear-category"],
            "edit"
        )
        .is_err());
        assert!(wants_help(&["--help".to_string()]));
        assert!(!wants_help(&["file.ylang".to_string()]));
    }

    #[test]
    fn authoring_workflow_round_trips_and_preserves_order() {
        const ID: &str = "yn1-tiqub4vxm4zslzlrhlfhc5k6b7vvkealot25nodtcixooe6xfzmxfsm6re";
        let directory = std::env::temp_dir().join(format!(
            "yami-discover-cli-authoring-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).unwrap();
        let file = directory.join("community.ylang");
        let file_arg = file.to_string_lossy().into_owned();
        let first = format!("yami://{ID}/home");
        let second = format!("yami://{ID}/about");

        run(command!["create", &file_arg, "--title", "Community"]).unwrap();
        run(command![
            "add",
            &file_arg,
            "--title",
            "Café [notes]",
            "--description",
            ":: literal [strong: text] 日本語",
            "--address",
            &first,
            "--category",
            "Personal",
        ])
        .unwrap();
        run(command![
            "add",
            &file_arg,
            "--title",
            "Second",
            "--description",
            "Second description",
            "--address",
            &second,
        ])
        .unwrap();

        let loaded = Directory::load(&file).unwrap();
        assert_eq!(loaded.entries().len(), 2);
        assert_eq!(loaded.entries()[0].title(), "Café [notes]");
        assert_eq!(
            loaded.entries()[0].description(),
            ":: literal [strong: text] 日本語"
        );
        assert_eq!(loaded.entries()[0].category(), Some("Personal"));
        assert_eq!(loaded.entries()[1].title(), "Second");

        assert!(run(command![
            "add",
            &file_arg,
            "--title",
            "Duplicate",
            "--description",
            "Duplicate",
            "--address",
            &first,
        ])
        .is_err());
        assert_eq!(Directory::load(&file).unwrap().entries().len(), 2);
        assert!(run(command![
            "add",
            &file_arg,
            "--title",
            "Bad",
            "--description",
            "Bad",
            "--address",
            "https://example.com",
        ])
        .is_err());

        run(command![
            "edit",
            &file_arg,
            "--address",
            &first,
            "--title",
            "Café renamed",
            "--description",
            "Updated [literal] text",
            "--category",
            "Journal",
        ])
        .unwrap();
        let edited = Directory::load(&file).unwrap();
        assert_eq!(edited.entries()[0].title(), "Café renamed");
        assert_eq!(edited.entries()[0].description(), "Updated [literal] text");
        assert_eq!(edited.entries()[0].category(), Some("Journal"));
        assert_eq!(edited.entries()[1].title(), "Second");

        run(command![
            "edit",
            &file_arg,
            "--address",
            &first,
            "--clear-category",
        ])
        .unwrap();
        assert_eq!(
            Directory::load(&file).unwrap().entries()[0].category(),
            None
        );
        assert!(run(command!["edit", &file_arg, "--address", &first]).is_err());

        run(command!["remove", &file_arg, "--address", &second]).unwrap();
        let remaining = Directory::load(&file).unwrap();
        assert_eq!(remaining.entries().len(), 1);
        assert_eq!(remaining.entries()[0].title(), "Café renamed");

        run(command!["show", &file_arg]).unwrap();
        run(command!["list", "--directory", &file_arg]).unwrap();
        run(command!["validate", "--directory", &file_arg]).unwrap();

        let mut catalog = Catalog::new();
        catalog.import_file("community", &file).unwrap();
        assert_eq!(catalog.search("updated").unwrap().len(), 1);

        assert!(run(command!["create", &file_arg, "--title", "Other"]).is_err());
        assert_eq!(Directory::load(&file).unwrap().name(), "Community");

        let bad = directory.join("bad.ylang");
        std::fs::write(&bad, "not a directory").unwrap();
        let bad_arg = bad.to_string_lossy().into_owned();
        assert!(run(command![
            "add",
            &bad_arg,
            "--title",
            "Bad",
            "--description",
            "Bad",
            "--address",
            &first,
        ])
        .is_err());
        assert_eq!(std::fs::read_to_string(&bad).unwrap(), "not a directory");

        let _ = std::fs::remove_dir_all(&directory);
    }
}
