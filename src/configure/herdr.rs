use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

const QUOTA_ROW_MARKERS: [&str; 10] = [
    "$quota_badge",
    "$quota_state",
    "$quota_icon",
    "$quota_provider",
    "$quota_status",
    "$quota_summary",
    "$quota_topic",
    "$quota_5h",
    "$quota_week",
    "$quota_header",
];
const ROW_GAP_MARKER: &str = "herdr-agent-quota";

pub fn check() -> Result<()> {
    let path = config_path()?;
    let original = fs::read_to_string(&path).unwrap_or_default();
    let updated = add_quota_row(&original)?;
    if updated == original {
        println!(
            "Herdr sidebar already contains quota tokens: {}",
            path.display()
        );
    } else {
        println!("Herdr sidebar preview for {}:", path.display());
        print_diff_hint();
    }
    Ok(())
}

pub fn apply() -> Result<()> {
    let path = config_path()?;
    let original = fs::read_to_string(&path).unwrap_or_default();
    let updated = add_quota_row(&original)?;
    if updated == original {
        return Ok(());
    }
    if !original.is_empty() {
        if let Some(backup) = backup_path()? {
            if let Some(parent) = backup.parent() {
                fs::create_dir_all(parent).context("create plugin state directory")?;
            }
            if !backup.exists() {
                fs::write(backup, &original).context("write Herdr config backup")?;
            }
        }
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("create Herdr config directory")?;
    }
    fs::write(&path, updated).context("write Herdr config")?;
    println!("Added quota sidebar row to {}", path.display());
    Ok(())
}

pub fn uninstall() -> Result<()> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(());
    }
    let original = fs::read_to_string(&path).context("read Herdr config")?;
    let updated = remove_quota_row(&original)?;
    if updated != original {
        fs::write(&path, updated).context("remove quota sidebar row")?;
        println!("Removed quota sidebar row from {}", path.display());
    }
    if let Some(backup) = backup_path()? {
        if backup.exists() {
            fs::remove_file(backup).context("remove Herdr config backup")?;
        }
    }
    Ok(())
}

pub fn config_path() -> Result<PathBuf> {
    if let Some(path) = std::env::var_os("HERDR_CONFIG_FILE") {
        return Ok(PathBuf::from(path));
    }
    let home = std::env::var_os("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config/herdr/config.toml"))
}

fn backup_path() -> Result<Option<PathBuf>> {
    let state = std::env::var_os("HERDR_PLUGIN_STATE_DIR");
    Ok(state.map(|directory| PathBuf::from(directory).join("herdr-config.original.toml")))
}

pub fn add_quota_row(input: &str) -> Result<String> {
    let mut document = if input.trim().is_empty() {
        DocumentMut::new()
    } else {
        input
            .parse::<DocumentMut>()
            .context("parse Herdr TOML config")?
    };
    let agents = ensure_table(&mut document, &["ui", "sidebar", "agents"])?;
    if !agents.contains_key("row_gap") {
        let mut row_gap = Value::from(1);
        row_gap
            .decor_mut()
            .set_suffix(format!(" # {ROW_GAP_MARKER}"));
        agents.insert("row_gap", Item::Value(row_gap));
    }
    let rows = agents["rows"].or_insert(Item::Value(Value::Array(Array::new())));
    let rows = rows
        .as_array_mut()
        .context("Herdr ui.sidebar.agents.rows must be an array")?;
    let mut updated_rows = Array::new();
    for row in rows.iter() {
        let cleaned = normalize_official_row(strip_quota_tokens(row));
        if !cleaned.is_empty() {
            updated_rows.push(Value::Array(cleaned));
        }
    }

    // If an older version replaced every row with quota-only rows, restore
    // Herdr's official state/tab row before adding provider, usage, and topic.
    if updated_rows.is_empty() {
        updated_rows.push(Value::Array(default_state_row()));
    }
    append_quota_rows(&mut updated_rows);
    *rows = updated_rows;
    Ok(document.to_string())
}

pub fn remove_quota_row(input: &str) -> Result<String> {
    if input.trim().is_empty() {
        return Ok(input.to_string());
    }
    let mut document = input
        .parse::<DocumentMut>()
        .context("parse Herdr TOML config")?;
    let Some(agents) = document
        .get_mut("ui")
        .and_then(Item::as_table_mut)
        .and_then(|ui| ui.get_mut("sidebar"))
        .and_then(Item::as_table_mut)
        .and_then(|sidebar| sidebar.get_mut("agents"))
        .and_then(Item::as_table_mut)
    else {
        return Ok(input.to_string());
    };
    let Some(rows) = agents.get_mut("rows").and_then(Item::as_array_mut) else {
        return Ok(input.to_string());
    };
    let mut retained = Array::new();
    for row in rows.iter() {
        let cleaned = strip_quota_tokens(row);
        if cleaned.len() == 1
            && matches!(
                cleaned.iter().next().and_then(Value::as_str),
                Some("terminal_title_stripped") | Some("$quota_topic")
            )
        {
            continue;
        }
        if !cleaned.is_empty() {
            retained.push(Value::Array(cleaned));
        }
    }
    agents["rows"] = Item::Value(Value::Array(retained));
    let managed_row_gap = agents
        .get("row_gap")
        .and_then(Item::as_value)
        .and_then(|value| value.decor().suffix())
        .and_then(|suffix| suffix.as_str())
        .is_some_and(|suffix| suffix.contains(ROW_GAP_MARKER));
    if managed_row_gap {
        agents.remove("row_gap");
    }
    Ok(document.to_string())
}

fn ensure_table<'a>(document: &'a mut DocumentMut, path: &[&str]) -> Result<&'a mut Table> {
    let mut item: &mut Item = document.as_item_mut();
    for key in path {
        let table = item
            .as_table_mut()
            .context("Herdr config section is not a table")?;
        item = table.entry(key).or_insert(Item::Table(Table::new()));
    }
    item.as_table_mut()
        .context("Herdr config section is not a table")
}

fn strip_quota_tokens(row: &Value) -> Array {
    let mut cleaned = Array::new();
    if let Some(items) = row.as_array() {
        for item in items {
            let is_quota_token = item
                .as_str()
                .is_some_and(|value| QUOTA_ROW_MARKERS.contains(&value));
            if !is_quota_token {
                cleaned.push(item.clone());
            }
        }
    }
    cleaned
}

fn default_state_row() -> Array {
    let mut row = Array::new();
    row.push("state_icon");
    row.push("tab");
    row
}

fn normalize_official_row(row: Array) -> Array {
    let has_state_icon = row.iter().any(|item| item.as_str() == Some("state_icon"));
    if !has_state_icon || row.iter().any(|item| item.as_str() == Some("agent")) {
        return row;
    }
    let mut normalized = Array::new();
    let mut has_tab = false;
    for item in row {
        match item.as_str() {
            Some("workspace") | Some("pane") => {
                if !has_tab {
                    normalized.push("tab");
                    has_tab = true;
                }
            }
            Some("tab") => {
                if !has_tab {
                    has_tab = true;
                    normalized.push(item);
                }
            }
            Some("terminal_title_stripped") => {}
            _ => normalized.push(item),
        }
    }
    if !has_tab {
        let insert_at = normalized
            .iter()
            .position(|item| item.as_str() == Some("terminal_title_stripped"))
            .unwrap_or(normalized.len());
        normalized.insert(insert_at, "tab");
    }
    normalized
}

fn append_quota_rows(rows: &mut Array) {
    // Keep the official state row, but split each quota window onto its own
    // row. Herdr elides rows whose custom token is absent, so weekly-only
    // providers do not gain a blank five-hour row.
    for row in rows.iter_mut() {
        let Some(items) = row.as_array_mut() else {
            continue;
        };
        let has_state_icon = items.iter().any(|item| item.as_str() == Some("state_icon"));
        let mut cleaned = Array::new();
        for item in items.iter() {
            match item.as_str() {
                Some("terminal_title_stripped") | Some("$quota_topic") => {}
                Some("agent") if !has_state_icon => {}
                _ => cleaned.push(item.clone()),
            }
        }
        *items = cleaned;
    }

    let mut compacted_rows = Array::new();
    for row in rows.iter() {
        if row.as_array().is_some_and(|items| !items.is_empty()) {
            compacted_rows.push(row.clone());
        }
    }
    *rows = compacted_rows;

    let official_index = rows.iter().position(|row| {
        row.as_array().is_some_and(|items| {
            items.iter().any(|item| item.as_str() == Some("state_icon"))
                && !items.iter().any(|item| item.as_str() == Some("agent"))
        })
    });

    if let Some(index) = official_index {
        if let Some(row) = rows.get_mut(index).and_then(Value::as_array_mut) {
            if !row
                .iter()
                .any(|item| item.as_str() == Some("$quota_provider"))
            {
                row.push("$quota_provider");
            }
        }
    }

    let mut five_hour_row = Array::new();
    five_hour_row.push("$quota_5h");
    rows.push(Value::Array(five_hour_row));

    let mut weekly_row = Array::new();
    weekly_row.push("$quota_week");
    rows.push(Value::Array(weekly_row));

    let mut topic_row = Array::new();
    topic_row.push("$quota_topic");
    rows.push(Value::Array(topic_row));
}

fn print_diff_hint() {
    println!("  keep Herdr's official state icon and plane tab");
    println!("  add provider, separate 5h/week rows, and a live user-prompt row");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_quota_rows_without_replacing_official_rows() {
        let original = r#"[ui.sidebar.agents]
rows = [["state_icon", "agent"]]
"#;
        let updated = add_quota_row(original).unwrap();
        assert!(updated.contains("$quota_5h"));
        assert!(updated.contains("$quota_week"));
        assert!(updated.contains("state_icon"));
        assert!(updated.contains("agent"));
        assert_eq!(updated.matches("[\"").count(), 4);
        assert_eq!(add_quota_row(&updated).unwrap(), updated);
    }

    #[test]
    fn removes_plugin_tokens_but_keeps_the_official_agent_row() {
        let original = r#"[ui.sidebar.agents]
rows = [["state_icon", "pane", "terminal_title_stripped"], ["agent", "$quota_icon", "$quota_5h"], ["$quota_week"]]
"#;
        let updated = remove_quota_row(original).unwrap();
        assert!(updated.contains("state_icon"));
        assert!(updated.contains("agent"));
        assert!(!updated.contains("$quota_summary"));
        assert!(!updated.contains("$quota_icon"));
        assert!(updated.contains("terminal_title_stripped"));
    }

    #[test]
    fn migrates_old_quota_only_rows_and_restores_herdr_state_row() {
        let original = r#"[ui.sidebar.agents]
rows = [["$quota_provider", "$quota_status"], ["$quota_summary"]]
"#;
        let updated = add_quota_row(original).unwrap();
        assert!(updated.contains("state_icon"));
        assert!(updated.contains("$quota_provider"));
        assert!(updated.contains("$quota_5h"));
        assert!(updated.contains("$quota_week"));
        assert_eq!(add_quota_row(&updated).unwrap(), updated);
    }
}
