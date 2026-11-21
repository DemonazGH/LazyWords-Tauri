const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

window.api = {
  getSettings: () => invoke('get_settings'),
  saveSettings: (patch) => invoke('save_settings', { newSettings: patch }),
  completeOnboarding: (dictId) => invoke('complete_onboarding', { dictId }),
  getLocale: () => invoke('get_locale'),
  getDictionaries: () => invoke('get_dictionaries'),
  importDictionary: () => invoke('import_dictionary'),
  deleteDictionary: (id) => invoke('delete_dictionary', { id }),
  getStats: () => invoke('get_stats'),
  getLearnedList: () => invoke('get_learned_list'),
  previewLegacyLearned: dictId => invoke('preview_legacy_learned', { dictId }),
  markLearned: (cardId, dictId) => invoke('mark_learned', { cardId, dictId }),
  takeQuickAddDraft: () => invoke('take_quick_add_draft'),
  saveQuickCard: (term, definition) => invoke('save_quick_card', { term, definition }),
  getMyWords: () => invoke('get_my_words'),
  editMyWord: (cardId, term, definition) => invoke('edit_my_word', { cardId, term, definition }),
  deleteMyWord: (cardId) => invoke('delete_my_word', { cardId }),
  closeQuickAdd: () => invoke('close_quick_add'),
  getCardLayout: () => invoke('get_card_layout'),
  resizeCard: (height) => invoke('resize_card', { height }),
  removeLearned: (cardId, dictId) => invoke('remove_learned', { cardId, dictId }),
  clearLearned: (dictId) => invoke('clear_learned', { dictId }),
  resolveLegacyLearned: (dictId, orderToken) => invoke('resolve_legacy_learned', { dictId, orderToken }),

  onShowWord: (cb) => listen('show-word', (e) => cb(e.payload)),
  onQuickAddOpened: (cb) => listen('quick-add-opened', () => cb()),
  onMarkKnown: (cb) => listen('mark-known-shortcut', () => cb()),
  onUpdateSettings: (cb) => listen('update-settings', (e) => cb(e.payload)),
  onAllLearned: (cb) => listen('show-all-learned', (e) => cb(e.payload)),
  onSwitchedDict: (cb) => listen('show-switched-dict', (e) => cb(e.payload)),
  onAllDictsLearned: (cb) => listen('show-all-dicts-learned', (e) => cb(e.payload)),
  
  onLocale: (cb) => listen('locale-data', (e) => cb(e.payload)),
  onStatsUpdated: (cb) => listen('stats-updated', () => cb()),
  onDictAutoSwitched: (cb) => listen('dict-auto-switched', (e) => cb(e.payload)),
  onLocaleUpdated: (cb) => listen('locale-updated', (e) => cb(e.payload)),
  onDictListUpdated: (cb) => listen('dict-list-updated', () => cb())
};
