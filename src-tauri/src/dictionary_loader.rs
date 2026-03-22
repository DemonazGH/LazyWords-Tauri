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

fn load_csv(path: &Path) -> Result<Vec<WordEntry>, String> {
    let mut reader = csv::Reader::from_path(path).map_err(|e| e.to_string())?;
    let headers = reader.headers().map_err(|e| e.to_string())?.clone();
    
    let word_idx = headers.iter().position(|h| h.to_lowercase() == "word")
        .ok_or("CSV must have 'word' column")?;
    let transl_idx = headers.iter().position(|h| h.to_lowercase() == "translation")
        .ok_or("CSV must have 'translation' column")?;

    let mut entries = Vec::new();
    for result in reader.records() {
        if let Ok(record) = result {
            if let (Some(w), Some(t)) = (record.get(word_idx), record.get(transl_idx)) {
                let w = w.trim();
                let t = t.trim();
                if !w.is_empty() && !t.is_empty() {
                    entries.push(WordEntry {
                        word: w.to_string(),
                        translation: t.to_string(),
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
    Ok(entries.into_iter().filter(|e| !e.word.trim().is_empty() && !e.translation.trim().is_empty()).collect())
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
    let headers = rows.next().ok_or("XLSX is empty")?;
    
    let mut word_idx = None;
    let mut transl_idx = None;
    
    for (i, cell) in headers.iter().enumerate() {
        if let DataType::String(s) = cell {
            match s.to_lowercase().trim() {
                "word" => word_idx = Some(i),
                "translation" => transl_idx = Some(i),
                _ => {}
            }
        }
    }
    
    let word_idx = word_idx.ok_or("XLSX must have 'word' column")?;
    let transl_idx = transl_idx.ok_or("XLSX must have 'translation' column")?;

    let mut entries = Vec::new();
    for row in rows {
        let w = match row.get(word_idx) {
            Some(DataType::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        let t = match row.get(transl_idx) {
            Some(DataType::String(s)) => s.trim().to_string(),
            _ => String::new(),
        };
        if !w.is_empty() && !t.is_empty() {
            entries.push(WordEntry { word: w, translation: t });
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
        let path = write_temp("lw_test_csv_valid.csv", b"word,translation\nhello,\xd0\xbf\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82\nworld,\xd0\xbc\xd0\xb8\xd1\x80");
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].word, "hello");
        assert_eq!(result[0].translation, "привет");
        assert_eq!(result[1].word, "world");
    }

    #[test]
    fn parses_valid_json() {
        let content = br#"[{"word":"apple","translation":"\u044f\u0431\u043b\u043e\u043a\u043e"},{"word":"cat","translation":"\u043a\u043e\u0442"}]"#;
        let path = write_temp("lw_test_json_valid.json", content);
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].word, "apple");
        assert_eq!(result[0].translation, "яблоко");
        assert_eq!(result[1].translation, "кот");
    }

    #[test]
    fn error_for_unsupported_extension() {
        let path = write_temp("lw_test_unsupported.txt", b"word,translation\nhello,hi");
        let result = load_dictionary(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported format"));
    }

    #[test]
    fn error_for_missing_word_column_in_csv() {
        let path = write_temp("lw_test_csv_no_word.csv", b"term,translation\nhello,\xd0\xbf\xd1\x80\xd0\xb8\xd0\xb2\xd0\xb5\xd1\x82");
        let result = load_dictionary(&path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("'word'"));
    }

    #[test]
    fn cyrillic_characters_not_garbled() {
        let content = "word,translation\nпривет,hello\nмир,world".as_bytes();
        let path = write_temp("lw_test_csv_cyrillic.csv", content);
        let result = load_dictionary(&path).unwrap();
        assert_eq!(result[0].word, "привет");
        assert_eq!(result[1].word, "мир");
    }
}
