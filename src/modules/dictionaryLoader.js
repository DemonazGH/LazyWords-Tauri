// dictionaryLoader.js — Parses CSV, JSON, XLSX dictionaries
const fs = require('fs');
const path = require('path');

// Proper CSV line tokenizer — handles quoted fields with commas inside
function parseCsvLine(line) {
  const fields = [];
  let current = '';
  let inQuotes = false;
  for (let i = 0; i < line.length; i++) {
    const ch = line[i];
    if (ch === '"') {
      if (inQuotes && line[i + 1] === '"') { current += '"'; i++; } // escaped quote
      else inQuotes = !inQuotes;
    } else if (ch === ',' && !inQuotes) {
      fields.push(current.trim());
      current = '';
    } else {
      current += ch;
    }
  }
  fields.push(current.trim());
  return fields;
}

function loadCsv(filePath) {
  const content = fs.readFileSync(filePath, 'utf8');
  const lines = content.trim().split(/\r?\n/);
  const headers = parseCsvLine(lines[0]).map(h => h.toLowerCase());
  const wordIdx = headers.indexOf('word');
  const translIdx = headers.indexOf('translation');

  if (wordIdx === -1 || translIdx === -1) {
    throw new Error(`CSV must have 'word' and 'translation' columns. Found: ${headers.join(', ')}`);
  }

  return lines.slice(1)
    .map(line => {
      const cols = parseCsvLine(line);
      return { word: cols[wordIdx], translation: cols[translIdx] };
    })
    .filter(e => e.word && e.translation);
}

function loadJson(filePath) {
  const data = JSON.parse(fs.readFileSync(filePath, 'utf8'));
  if (!Array.isArray(data)) throw new Error('JSON dictionary must be an array');
  return data.filter(e => e.word && e.translation);
}

function loadXlsx(filePath) {
  const XLSX = require('xlsx');
  const wb = XLSX.readFile(filePath);
  const sheet = wb.Sheets[wb.SheetNames[0]];
  const rows = XLSX.utils.sheet_to_json(sheet, { defval: '' });

  // Normalize header names to lowercase
  const entries = rows.map(row => {
    const normalized = {};
    for (const key of Object.keys(row)) {
      normalized[key.toLowerCase().trim()] = String(row[key]).trim();
    }
    return normalized;
  });

  if (entries.length === 0 || !('word' in entries[0]) || !('translation' in entries[0])) {
    throw new Error("XLSX must have 'word' and 'translation' columns");
  }

  return entries.filter(e => e.word && e.translation)
    .map(e => ({ word: e.word, translation: e.translation }));
}

function loadDictionary(filePath) {
  const ext = path.extname(filePath).toLowerCase();
  if (ext === '.csv') return loadCsv(filePath);
  if (ext === '.json') return loadJson(filePath);
  if (ext === '.xlsx') return loadXlsx(filePath);
  throw new Error(`Unsupported format: ${ext}`);
}

module.exports = { loadDictionary };
