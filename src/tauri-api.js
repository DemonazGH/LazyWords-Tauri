const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.api = {
  getSettings: () => invoke('get_settings'),
  saveSettings: (patch) => invoke('save_settings', { newSettings: patch }),
  getLocale: () => invoke('get_locale'),
  getDictionaries: () => invoke('get_dictionaries'),
  importDictionary: () => invoke('import_dictionary'),
  deleteDictionary: (id) => invoke('delete_dictionary', { id }),
  getStats: () => invoke('get_stats'),
  getLearnedList: () => invoke('get_learned_list'),
  markLearned: (index) => invoke('mark_learned', { index }),
  removeLearned: (index) => invoke('remove_learned', { index }),
  clearLearned: () => invoke('clear_learned'),

  onShowWord: (cb) => listen('show-word', (e) => cb(e.payload)),
  onHideWord: (cb) => listen('hide-word', () => cb()),
  onMarkKnown: (cb) => listen('mark-known-shortcut', () => cb()),
  onUpdateSettings: (cb) => listen('update-settings', (e) => cb(e.payload)),
  onAllLearned: (cb) => listen('show-all-learned', (e) => cb(e.payload)),
  onSwitchedDict: (cb) => listen('show-switched-dict', (e) => cb(e.payload)),
  onAllDictsLearned: (cb) => listen('show-all-dicts-learned', (e) => cb(e.payload)),
  
  onLocale: (cb) => listen('locale-data', (e) => cb(e.payload)),
  onStatsUpdated: (cb) => listen('stats-updated', () => cb()),
  onDictAutoSwitched: (cb) => listen('dict-auto-switched', (e) => cb(e.payload)),
  onLocaleUpdated: (cb) => listen('locale-updated', (e) => cb(e.payload))
};
