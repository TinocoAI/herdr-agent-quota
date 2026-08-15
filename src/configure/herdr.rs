use anyhow::{Context, Result};
use std::fs;
use std::path::PathBuf;
use toml_edit::{Array, DocumentMut, Item, Table, Value};

const QUOTA_ROW_MARKER: &str = "$quota_badge";

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
    let rows = agents["rows"].or_insert(Item::Value(Value::Array(Array::new())));
    let rows = rows
        .as_array_mut()
        .context("Herdr ui.sidebar.agents.rows must be an array")?;
    if rows.iter().any(row_contains_quota_marker) {
        return Ok(document.to_string());
    }
    let mut row = Array::new();
    row.push("$quota_badge");
    row.push("$quota_state");
    row.push("$quota_summary");
    rows.push(Value::Array(row));
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
        if !row_contains_quota_marker(row) {
            retained.push(row.clone());
        }
    }
    agents["rows"] = Item::Value(Value::Array(retained));
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

fn row_contains_quota_marker(row: &Value) -> bool {
    row.as_array()
        .map(|items| {
            items
                .iter()
                .any(|item| item.as_str() == Some(QUOTA_ROW_MARKER))
        })
        .unwrap_or(false)
}

fn print_diff_hint() {
    println!("  add one Agent row containing $quota_badge, $quota_state, $quota_summary");
    println!("  existing Herdr rows remain unchanged");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adds_quota_row_without_replacing_existing_rows() {
        let original = r#"[ui.sidebar.agents]
rows = [["state_icon", "agent"]]
"#;
        let updated = add_quota_row(original).unwrap();
        assert!(updated.contains("$quota_badge"));
        assert!(updated.contains("state_icon"));
        assert_eq!(add_quota_row(&updated).unwrap(), updated);
    }

    #[test]
    fn removes_only_plugin_owned_row() {
        let original = r#"[ui.sidebar.agents]
rows = [["state_icon", "agent"], ["$quota_badge", "$quota_state", "$quota_summary"]]
"#;
        let updated = remove_quota_row(original).unwrap();
        assert!(updated.contains("state_icon"));
        assert!(!updated.contains("$quota_badge"));
    }
}
