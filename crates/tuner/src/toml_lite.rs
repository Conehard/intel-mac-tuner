//! Minimal subset of TOML, enough for `config.toml`:
//! `[a]` / `[a.b]` sections, `key = value` with string, number or boolean, and `#` comments.
//! Deliberately no external dependencies: the daemon runs as root.

use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq)]
pub enum TomlValue {
    Str(String),
    Num(f64),
    Bool(bool),
}

/// Map `"section.key"` -> value.
pub type Table = BTreeMap<String, TomlValue>;

fn strip_comment(line: &str) -> &str {
    let mut in_str = false;
    for (i, c) in line.char_indices() {
        match c {
            '"' => in_str = !in_str,
            '#' if !in_str => return &line[..i],
            _ => {}
        }
    }
    line
}

fn valid_name(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn parse_value(raw: &str) -> Option<TomlValue> {
    let raw = raw.trim();
    if let Some(inner) = raw.strip_prefix('"').and_then(|r| r.strip_suffix('"')) {
        if inner.contains('"') || inner.contains('\\') {
            return None;
        }
        return Some(TomlValue::Str(inner.to_string()));
    }
    match raw {
        "true" => return Some(TomlValue::Bool(true)),
        "false" => return Some(TomlValue::Bool(false)),
        _ => {}
    }
    let num: String = raw.chars().filter(|&c| c != '_').collect();
    num.parse::<f64>()
        .ok()
        .filter(|v| v.is_finite())
        .map(TomlValue::Num)
}

pub fn parse(text: &str) -> Result<Table, String> {
    let mut table = Table::new();
    let mut section = String::new();
    for (n, line) in text.lines().enumerate() {
        let lineno = n + 1;
        let line = strip_comment(line).trim();
        if line.is_empty() {
            continue;
        }
        if let Some(name) = line.strip_prefix('[') {
            let name = name
                .strip_suffix(']')
                .ok_or_else(|| format!("line {lineno}: section without ']'"))?
                .trim();
            if !name.split('.').all(valid_name) {
                return Err(format!("line {lineno}: invalid section name: [{name}]"));
            }
            section = name.to_string();
            continue;
        }
        let (key, value) = line
            .split_once('=')
            .ok_or_else(|| format!("line {lineno}: expected `key = value`"))?;
        let key = key.trim();
        if !valid_name(key) {
            return Err(format!("line {lineno}: invalid key name: {key:?}"));
        }
        let value = parse_value(value)
            .ok_or_else(|| format!("line {lineno}: invalid value for {key}: {}", value.trim()))?;
        let full = if section.is_empty() {
            key.to_string()
        } else {
            format!("{section}.{key}")
        };
        if table.insert(full.clone(), value).is_some() {
            return Err(format!("line {lineno}: {full} defined twice"));
        }
    }
    Ok(table)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_sections_values_and_comments() {
        let t = parse(
            r#"
            # comment
            top = 1
            [fans]
            mode = "sensor"   # mode
            [fans.sensor]
            key = "TC#P"
            temp_min_c = 50.5
            big = 1_000
            on = true
            "#,
        )
        .unwrap();
        assert_eq!(t["top"], TomlValue::Num(1.0));
        assert_eq!(t["fans.mode"], TomlValue::Str("sensor".into()));
        assert_eq!(t["fans.sensor.key"], TomlValue::Str("TC#P".into()));
        assert_eq!(t["fans.sensor.temp_min_c"], TomlValue::Num(50.5));
        assert_eq!(t["fans.sensor.big"], TomlValue::Num(1000.0));
        assert_eq!(t["fans.sensor.on"], TomlValue::Bool(true));
    }

    #[test]
    fn reports_errors_with_line_numbers() {
        assert!(parse("[fans").unwrap_err().contains("line 1"));
        assert!(parse("a = 1\nb").unwrap_err().contains("line 2"));
        assert!(parse("a = nope").is_err());
        assert!(parse("a = 1\na = 2").unwrap_err().contains("twice"));
        assert!(parse("a b = 1").is_err());
        assert!(parse("a = \"x\\\"y\"").is_err());
        assert!(parse("a = inf").is_err());
    }
}
