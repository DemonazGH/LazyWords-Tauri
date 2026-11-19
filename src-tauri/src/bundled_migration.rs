use std::{collections::HashSet, fs, io, path::Path};

use crate::{dictionary_loader::load_dictionary, learned::Learned, scheduler::card_id};

// Learned::order_token over the parsed CSVs shipped in v1.0.2 (fb194ee).
// Do not regenerate from the current bundle: content AND order are the evidence.
const RELEASE_ORDERS: &[(&str, &str)] = &[
    (
        "en-ru",
        "01f1072cfb69d9419c8145ed781b3d18ab4eda7db5ca6c7535d9a2182b9d654a",
    ),
    (
        "en-definitions",
        "72fbfb9d50724005cc026814d349d3c7ec30fadb960354df459f46103000ce7a",
    ),
    (
        "en-irregular-verbs",
        "860815427e0f4fb39761bba7fe7f9920079c2fa4eacebd1472f030f0800bcd9b",
    ),
];

fn imported_names(dir: &Path) -> io::Result<HashSet<String>> {
    match fs::symlink_metadata(dir) {
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(HashSet::new()),
        Err(e) => return Err(e),
        Ok(_) => {}
    }
    let mut names = HashSet::new();
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        let ext = path.extension().and_then(|s| s.to_str()).unwrap_or("");
        if ["csv", "json", "xlsx"]
            .iter()
            .any(|s| ext.eq_ignore_ascii_case(s))
        {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                names.insert(stem.to_ascii_lowercase());
            }
        }
    }
    Ok(names)
}

pub fn link_verified(learned: &mut Learned, bundled: &Path, imports: &Path) {
    // An unreadable imports directory cannot establish the absence of an override.
    let Ok(overrides) = imported_names(imports) else {
        return;
    };
    for &(deck, order) in RELEASE_ORDERS {
        if learned.pending(deck).is_empty() || overrides.contains(deck) {
            continue;
        }
        if let Ok(words) = load_dictionary(&bundled.join(format!("{deck}.csv"))) {
            let ids: Vec<_> = words.iter().map(card_id).collect();
            learned.resolve(deck, &ids, order);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    #[test]
    fn exact_ru_links_but_same_name_import_changed_order_and_unknown_sources_do_not() {
        let temp = tempfile::tempdir().unwrap();
        let shipped = Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/dictionaries");
        let imports = temp.path().join("imports");
        let pristine = Learned::from_legacy(HashMap::from([("en-ru".into(), vec![0])]));
        let check = |bundled: &Path, imports: &Path, expected: bool| {
            let mut learned = pristine.clone();
            link_verified(&mut learned, bundled, imports);
            assert_eq!(learned.ids("en-ru").len(), usize::from(expected));
            assert_eq!(learned.pending("en-ru").is_empty(), expected);
        };
        check(&shipped, &imports, true);
        fs::create_dir(&imports).unwrap();
        fs::copy(shipped.join("en-ru.csv"), imports.join("en-ru.csv")).unwrap();
        check(&shipped, &imports, false); // Even byte-identical imports are not bundles.
        let absent = temp.path().join("no-imports");
        let text = fs::read_to_string(shipped.join("en-ru.csv")).unwrap();
        let mut lines: Vec<_> = text.lines().collect();
        lines.swap(1, 2);
        fs::write(temp.path().join("en-ru.csv"), lines.join("\n")).unwrap();
        check(temp.path(), &absent, false);
        check(&absent, &absent, false);
        check(&shipped, &temp.path().join("en-ru.csv"), false); // Cannot enumerate imports.
    }
}
