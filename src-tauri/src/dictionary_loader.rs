use std::path::Path;
use calamine::{Reader, open_workbook_auto, DataType};
use crate::word_engine::WordEntry;

pub fn load_dictionary(path: &Path) -> Result<Vec<WordEntry>, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    match ext.as_str() {
        "csv" => load_csv(path),
        "json" => load_json(path),
        "xlsx" => load_xlsx(path),
        _ => Err(format!("Unsupported format: {}", ext)),
    }
}

/// Resolve term and definition column indices from a list of header strings.
/// Accepts: term/word/front and definition/translation/back (case-insensitive).
/// Falls back to columns 0 and 1 if no recognised headers are found.
fn resolve_term_def_indices(headers: &[&str]) -> Option<(usize, usize)> {
    let term_idx = headers.iter().position(|h| {
        matches!(h.to_lowercase().trim(), "term" | "word" | "front")
    });
    let def_idx = headers.iter().position(|h| {
        matches!(h.to_lowercase().trim(), "definition" | "translation" | "back")
    });
    match (term_idx, def_idx) {
        (Some(t), Some(d)) => Some((t, d)),
        _ => {
            if headers.len() >= 2 { Some((0, 1)) } else { None }
        }
    }
}

fn load_csv(path: &Path) -> Result<Vec<WordEntry>, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    let raw_headers = reader.headers().map_err(|e| e.to_string())?.clone();
    let header_strs: Vec<&str> = raw_headers.iter().collect();

    let (term_idx, def_idx) = resolve_term_def_indices(&header_strs)
        .ok_or("CSV must have at least 2 columns")?;

    let mut entries = Vec::new();
    for result in reader.records() {
        if let Ok(record) = result {
            if let (Some(t), Some(d)) = (record.get(term_idx), record.get(def_idx)) {
                let t = t.trim();
                let d = d.trim();
                if !t.is_empty() && !d.is_empty() {
                    entries.push(WordEntry {
                        term: t.to_string(),
                        definition: d.to_string(),
                    });
                }
            }
        }
    }
    Ok(entries)
}

fn load_json(path: &Path) -> Result<Vec<WordEntry>, String> {
    let data = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let entries: Vec<WordEntry> = serde_json::from_str(&data).map_err(|e| e.to_string())?;
    Ok(entries.into_iter().filter(|e| !e.term.trim().is_empty() && !e.definition.trim().is_empty()).collect())
}

fn load_xlsx(path: &Path) -> Result<Vec<WordEntry>, String> {
    let mut workbook = open_workbook_auto(path).map_err(|e| e.to_string())?;
    let sheet_names = workbook.sheet_names();
    if sheet_names.is_empty() {
        return Err("XLSX has no sheets".to_string());
    }
    let sheet_name = sheet_names[0].clone();
    let range = workbook.worksheet_range(&sheet_name)
        .ok_or_else(|| "Sheet not found".to_string())?
        .map_err(|e| e.to_string())?;

    let mut rows = range.rows();
    let header_row = rows.next().ok_or("XLSX is empty")?;

    let header_strs: Vec<String> = header_row.iter().map(|cell| {
        if let DataType::String(s) = cell { s.clone() } else { String::new() }
    }).collect();
    let header_refs: Vec<&str> = header_strs.iter().map(|s| s.as_str()).collect();

    let (term_idx, def_idx) = resolve_term_def_indices(&header_refs)
        .ok_or("XLSX must have at least 2 columns")?;

    let mut entries = Vec::new();
    for row in rows {
        let t = match row.get(term_idx) {
            Some(DataType::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        let d = match row.get(def_idx) {
            Some(DataType::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        if !t.is_empty() && !d.is_empty() {
            entries.push(WordEntry { term: t, definition: d });
        }
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn write_temp(name: &str, content: &[u8]) -> PathBuf {
        let path = std::env::temp_dir().join(name);
        std::fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn parses_valid_csv() {
        let path = write_temp("lw_test_csv_valid.csv", b"term,definition\nhello,\xd0\xbf\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82\nworld,\xd0\xbc\xd0\xb8\xd1\x80");
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].term, "hello");
        assert_eq!(result[0].definition, "привет");
        assert_eq!(result[1].term, "world");
    }

    #[test]
    fn parses_legacy_word_translation_csv() {
        let path = write_temp("lw_test_csv_legacy.csv", b"word,translation\napple,\xd1\x8f\xd0\xb1\xd0\xbb\xd0\xbe\xd0\xba\xd0\xbe\ncat,\xd0\xba\xd0\xbe\xd1\x82");
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].term, "apple");
        assert_eq!(result[0].definition, "яблоко");
    }

    #[test]
    fn parses_front_back_csv() {
        let path = write_temp("lw_test_csv_front_back.csv", b"front,back\nQ1,A1\nQ2,A2");
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].term, "Q1");
        assert_eq!(result[0].definition, "A1");
    }

    #[test]
    fn parses_valid_json() {
        let content = br#"[{"term":"apple","definition":"\u044f\u0431\u043b\u043e\u043a\u043e"},{"term":"cat","definition":"\u043a\u043e\u0442"}]"#;
        let path = write_temp("lw_test_json_valid.json", content);
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].term, "apple");
        assert_eq!(result[0].definition, "яблоко");
        assert_eq!(result[1].definition, "кот");
    }

    #[test]
    fn parses_legacy_json_word_translation() {
        let content = br#"[{"word":"apple","translation":"\u044f\u0431\u043b\u043e\u043a\u043e"},{"word":"cat","translation":"\u043a\u043e\u0442"}]"#;
        let path = write_temp("lw_test_json_legacy.json", content);
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].term, "apple");
        assert_eq!(result[0].definition, "яблоко");
    }

    #[test]
    fn error_for_unsupported_extension() {
        let path = write_temp("lw_test_unsupported.txt", b"term,definition\nhello,hi");
        let result = load_dictionary(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported format"));
    }

    #[test]
    fn error_for_single_column_csv() {
        let path = write_temp("lw_test_csv_one_col.csv", b"term\nhello");
        let result = load_dictionary(&path);
        assert!(result.is_err());
    }

    #[test]
    fn cyrillic_characters_not_garbled() {
        let content = "term,definition\nпривет,hello\nмир,world".as_bytes();
        let path = write_temp("lw_test_csv_cyrillic.csv", content);
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result[0].term, "привет");
        assert_eq!(result[1].term, "мир");
    }
}
