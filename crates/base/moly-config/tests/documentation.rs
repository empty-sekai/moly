//! Keeps `docs/configuration.md` and `moly.example.toml` in step with `KEYS`:
//! every key is documented once, under its section, with its environment
//! variable; the documentation lists no key that is not declared; and the
//! example holds every key, commented out, with a value of the right kind.

use std::collections::{BTreeMap, BTreeSet};

use moly_config::{keys, Kind, KEYS, Key};

const DOC: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../docs/configuration.md");
const EXAMPLE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../../moly.example.toml");

/// The Environment variable cell of a key without an environment variable.
const NO_ENV: &str = "none";

fn key_section(key: &Key) -> &'static str {
    key.path.split_once('.').map_or(key.path, |(section, _)| section)
}

fn key_name(key: &Key) -> &'static str {
    key.path.split_once('.').map_or(key.path, |(_, name)| name)
}

/// The heading of a section's table in the documentation.
fn heading(section: &str) -> String {
    format!("### `[{section}]`")
}

/// A key, with its environment variable in place, so that what is documented
/// is compared with what `KEYS` declares.
fn label(key: &Key) -> String {
    match key.env {
        Some(env) => format!("{} ({})", key.path, env),
        None => key.path.to_owned(),
    }
}

fn kind_name(kind: Kind) -> &'static str {
    match kind {
        Kind::Text => "string",
        Kind::Path => "path",
        Kind::Url => "address",
        Kind::Flag => "true or false",
        Kind::Integer => "whole number",
    }
}

/// Splits a table row on its pipes, without the outer ones. A pipe written
/// `\|` is part of a cell, not a separator.
fn split_row(line: &str) -> Vec<String> {
    let mut inner = line.trim();
    inner = inner.strip_prefix('|').unwrap_or(inner);
    inner = inner.strip_suffix('|').unwrap_or(inner);
    let mut cells = vec![String::new()];
    let mut characters = inner.trim().chars().peekable();
    while let Some(character) = characters.next() {
        if character == '\\' && characters.peek() == Some(&'|') {
            characters.next();
            cells.last_mut().expect("a cell is open").push('|');
        } else if character == '|' {
            cells.push(String::new());
        } else {
            cells.last_mut().expect("a cell is open").push(character);
        }
    }
    cells.into_iter().map(|cell| cell.trim().to_owned()).collect()
}

fn is_rule(line: &str) -> bool {
    let line = line.trim();
    line.starts_with('|')
        && split_row(line).iter().all(|cell| {
            !cell.is_empty() && cell.chars().all(|character| matches!(character, '-' | ':'))
        })
}

/// The section a `### `[name]`` heading names, in the form `### name`.
fn heading_section(line: &str) -> Option<String> {
    let rest = line.trim().strip_prefix("### ")?;
    let rest = rest.strip_prefix('`')?;
    let (section, _) = rest.strip_prefix('[')?.split_once(']')?;
    Some(format!("### {section}"))
}

/// The backticked words of a cell.
fn quoted(cell: &str) -> Option<&str> {
    cell.strip_prefix('`')?.strip_suffix('`')
}

/// The key tables of the documentation, in order: the section and the rows of
/// every heading of the form `### `[section]``.
fn doc_tables(text: &str) -> Vec<(String, Vec<Vec<String>>)> {
    let mut tables: Vec<(String, Vec<Vec<String>>)> = Vec::new();
    let mut in_table = false;
    for line in text.lines() {
        // A heading at column 0 (`#`, `##`, `###`) ends the table before it.
        if line.starts_with('#') {
            let section = heading_section(line).unwrap_or_else(|| line.trim().to_owned());
            tables.push((section, Vec::new()));
            in_table = false;
        } else if line.trim_start().starts_with('|') {
            if !in_table {
                in_table = true;
                continue;
            }
            if !is_rule(line) {
                if let Some((_, rows)) = tables.last_mut() {
                    rows.push(split_row(line));
                }
            }
        } else {
            in_table = false;
        }
    }
    tables
}

/// The example's sections, in order, with their `# name = value` lines: line
/// number, name and value.
fn example_sections(text: &str) -> Vec<(String, Vec<(usize, String, String)>)> {
    let mut sections: Vec<(String, Vec<(usize, String, String)>)> = Vec::new();
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if let Some(header) = line.strip_prefix('[') {
            let section = header.strip_suffix(']').map(str::trim).unwrap_or(header);
            sections.push((section.to_owned(), Vec::new()));
            continue;
        }
        let Some(rest) = line.strip_prefix("# ") else { continue };
        let Some((assignment, value)) = rest.split_once(" = ") else { continue };
        if assignment.is_empty()
            || !assignment.chars().all(|character| character.is_ascii_alphanumeric() || character == '_')
        {
            continue;
        }
        if let Some((_, rows)) = sections.last_mut() {
            rows.push((index + 1, assignment.to_owned(), value.to_owned()));
        }
    }
    sections
}

/// A bare key name: letters, digits and underscores.
fn is_bare_name(name: &str) -> bool {
    !name.is_empty() && name.chars().all(|character| character.is_ascii_alphanumeric() || character == '_')
}

/// Removes the `# ` from the `# name = value` lines of `text`, leaving every
/// other line as it is.
fn uncommented(text: &str) -> String {
    text.lines()
        .map(|line| {
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            match trimmed.strip_prefix("# ") {
                Some(rest) => match rest.split_once(" = ") {
                    Some((name, _)) if is_bare_name(name) => format!("{indent}{rest}"),
                    _ => line.to_owned(),
                },
                None => line.to_owned(),
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn read(path: &str) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|error| panic!("{path}: {error}"))
}

#[test]
fn each_key_is_documented_under_its_section_with_its_variable_its_kind_and_its_readers() {
    let text = read(DOC);
    let tables = doc_tables(&text);
    for key in KEYS {
        let name = format!("`{}`", key_name(key));
        let wanted = format!("### {}", key_section(key));
        let found = tables.iter().find(|(heading, rows)| *heading == wanted && rows.iter().any(|row| row[0] == name));
        let Some((_, rows)) = found else {
            panic!(
                "docs/configuration.md has no row ``| `{}` |`` under the heading `{}` for `{}`",
                key_name(key),
                heading(key_section(key)),
                label(key)
            );
        };
        let row = rows.iter().find(|row| row[0] == name).expect("the row was found");
        assert_eq!(
            row.len(),
            5,
            "the doc row of `{}` needs the cells Key, Environment variable, Value, Read by and Description",
            label(key)
        );
        let env = key.env.map_or_else(|| NO_ENV.to_owned(), |env| format!("`{env}`"));
        assert_eq!(row[1], env, "the doc row of `{}` must name its environment variable `{env}` in its own cell", label(key));
        assert_eq!(
            row[2],
            kind_name(key.kind),
            "the doc row of `{}` must name its value kind `{}`",
            label(key),
            kind_name(key.kind)
        );
        assert_eq!(row[3], key.readers, "the doc row of `{}` must name its readers `{}`", label(key), key.readers);
        assert!(!row[4].is_empty(), "the doc row of `{}` has no description", label(key));
    }
}

#[test]
fn each_key_is_documented_once_and_no_other_key_is() {
    let text = read(DOC);
    let declared: BTreeSet<String> = KEYS.iter().map(|key| key.path.to_owned()).collect();
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (heading, rows) in doc_tables(&text) {
        let Some(section) = heading.strip_prefix("### ") else { continue };
        for row in rows {
            let cell = &row[0];
            let key_name = quoted(cell).unwrap_or(cell.as_str());
            // A key table holds the names of its section's keys; a row named
            // `section.name` names a key outright.
            let path = match key_name.split_once('.') {
                Some((other, _)) => {
                    assert_eq!(
                        other, section,
                        "docs/configuration.md lists `{key_name}` in the `{heading}` table, but it belongs to another section"
                    );
                    key_name.to_owned()
                }
                None => format!("{section}.{key_name}"),
            };
            assert!(
                declared.contains(&path),
                "docs/configuration.md has the row ``| {cell} |`` in the `{heading}` table, but `{path}` is not a configuration key"
            );
            let count = seen.entry(path.clone()).or_insert(0);
            *count += 1;
            assert_eq!(*count, 1, "docs/configuration.md lists `{path}` more than once");
        }
    }
    for key in KEYS {
        assert_eq!(seen.get(key.path).copied(), Some(1), "docs/configuration.md must list `{}` exactly once", label(key));
    }
}

#[test]
fn the_example_lists_each_key_commented_out_under_its_section() {
    let text = read(EXAMPLE);
    let declared: BTreeSet<String> = KEYS.iter().map(|key| key.path.to_owned()).collect();
    let sections = example_sections(&text);
    let mut seen: BTreeMap<String, usize> = BTreeMap::new();
    for (section, rows) in &sections {
        for (line, name, value) in rows {
            let path = format!("{section}.{name}");
            assert!(
                declared.contains(&path),
                "moly.example.toml line {line} lists `{path}`, but it is not a configuration key"
            );
            assert!(!value.trim().is_empty(), "moly.example.toml line {line} gives `{path}` no value");
            let count = seen.entry(path.clone()).or_insert(0);
            *count += 1;
            assert_eq!(*count, 1, "moly.example.toml lists `{path}` more than once (line {line})");
        }
    }
    for key in KEYS {
        let found = sections.iter().any(|(section, rows)| {
            *section == key_section(key) && rows.iter().any(|(_, name, _)| name == key_name(key))
        });
        assert!(
            found,
            "moly.example.toml needs `# {name} = value` under [{section}] for `{path}`",
            name = key_name(key),
            section = key_section(key),
            path = label(key)
        );
    }
}

#[test]
fn the_uncommented_example_holds_every_key_with_a_value_of_the_right_kind() {
    let text = read(EXAMPLE);
    let uncommented = uncommented(&text);
    let config = moly_config::Config::parse(&uncommented, None)
        .unwrap_or_else(|error| panic!("moly.example.toml with the `# ` of its key lines removed must parse: {error}"));
    for key in KEYS {
        assert!(
            config.has(key),
            "`{}` in moly.example.toml must have a value of the kind `{}`",
            label(key),
            kind_name(key.kind)
        );
    }
    let original = moly_config::Config::parse(&text, None)
        .unwrap_or_else(|error| panic!("moly.example.toml itself must parse: {error}"));
    for key in KEYS {
        assert!(!original.has(key), "`{}` in moly.example.toml must be commented out", label(key));
    }
    // The numbers and flags of the example read back as the kinds they
    // declare, not only as values the file check accepts. None of these keys
    // has an environment variable, so the program's environment plays no part.
    assert_eq!(config.integer(&keys::LAUNCHER_JOBS), Ok(Some(16)));
    assert!(config.integer(&keys::SERVER_CONTROL_PORT).is_ok_and(|value| value.is_some()));
    assert_eq!(config.flag(&keys::DEV_WINDOW), Ok(Some(false)));
    assert_eq!(config.file_flag(&keys::HIDE_HUD), Some(false));
    assert_eq!(config.file_flag(&keys::HIDE_PLAYER), Some(false));
    // Every other key of the example that is not a flag is a string the
    // program parses itself, so a quoted value is what the example holds.
    for key in KEYS {
        if matches!(key.kind, Kind::Text | Kind::Path | Kind::Url) {
            assert!(
                config.file_text(key).is_some(),
                "`{}` in moly.example.toml must hold a quoted string",
                label(key)
            );
        }
    }
}

#[test]
fn the_developer_switches_say_that_their_values_are_strings() {
    let text = read(DOC);
    let section = doc_section(&text, "### `[dev]`");
    assert!(!section.is_empty(), "docs/configuration.md needs a `### `[dev]`` section");
    assert!(section.contains("parses itself"), "the `[dev]` section must say that each switch parses its value itself");
    assert!(
        section.contains("environment variable"),
        "the `[dev]` section must say that each switch is also set through its environment variable"
    );
    let flags = ["dev.window", "dev.check_systems"];
    for key in KEYS.iter().filter(|key| key_section(key) == "dev" && !flags.contains(&key.path)) {
        assert!(matches!(key.kind, Kind::Text | Kind::Path), "`{}` must hold a string", label(key));
    }
}

/// The text of a `## heading` section of the documentation, without the
/// sections that follow it.
fn doc_section(text: &str, heading: &str) -> String {
    let level = heading.len() - heading.trim_start_matches('#').len();
    let mut lines = text.lines().skip_while(|line| line.trim() != heading).skip(1);
    lines.by_ref().take_while(|line| !heading_level(line).is_some_and(|other| other <= level)).collect::<Vec<_>>().join("\n")
}

/// The number of `#` of an ATX heading line.
fn heading_level(line: &str) -> Option<usize> {
    let level = line.chars().take_while(|character| *character == '#').count();
    (level > 0 && line[level..].starts_with(' ')).then_some(level)
}

#[test]
fn the_sections_before_the_keys_name_the_file_and_the_variables_that_place_it() {
    let text = read(DOC);
    let file = doc_section(&text, "## The configuration file");
    assert!(!file.is_empty(), "docs/configuration.md needs a `## The configuration file` section");
    for wanted in ["`moly.toml`", "`MOLY_CONFIG`", "launcher.json.imported", "`settings.server.json`", "`settings.local.json`"] {
        assert!(file.contains(wanted), "the `## The configuration file` section must name {wanted}");
    }
    let before = doc_section(&text, "## Before the file");
    assert!(!before.is_empty(), "docs/configuration.md needs a `## Before the file` section");
    for variable in ["`MOLY_DATA_DIR`", "`MOLY_CONFIG`"] {
        assert!(before.contains(variable), "the `## Before the file` section must name {variable}");
    }
    for key in KEYS {
        if let Some(env) = key.env {
            assert!(
                !before.contains(&format!("`{env}`")),
                "`{env}` (the environment variable of `{}`) is a key of the file, not a variable that places it",
                key.path
            );
        }
    }
    let page = doc_section(&text, "## Page parameters (browser version)");
    for parameter in ["`server`", "`renderer=webgl2`", "`player_uid`, `player_region`, `player_api`"] {
        assert!(page.contains(parameter), "the `## Page parameters (browser version)` section must list {parameter}");
    }
}

#[test]
fn every_example_address_uses_a_reserved_name() {
    let text = read(EXAMPLE);
    for (index, line) in text.lines().enumerate() {
        for word in line.split(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '(' | ')' | ',')) {
            let Some((_, rest)) = word.split_once("://") else { continue };
            let host = rest.split(['/', '?', '#']).next().unwrap_or(rest);
            // Drop a user and password, a port and the braces of a template.
            let host = host.rsplit_once('@').map_or(host, |(_, host)| host);
            let host = if host.starts_with('[') {
                host.split_once(']').map_or("", |(host, _)| host)
            } else {
                host.split_once(':').map_or(host, |(host, _)| host)
            };
            let host = host.replace(['{', '}'], "");
            assert!(
                host == "127.0.0.1" || host == "localhost" || host == "[::1]" || host.ends_with(".example"),
                "moly.example.toml line {}: `{word}` does not use a reserved name",
                index + 1
            );
        }
    }
}
