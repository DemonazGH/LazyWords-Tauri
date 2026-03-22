// wordEngine.js — Word Engine module (stub for stage 1, implemented in stage 3)

class WordEngine {
  constructor() {
    this.dictionary = [];
    this.learnedIndices = new Set();
  }

  loadDictionary(words) {
    this.dictionary = words;
  }

  setLearned(indices) {
    this.learnedIndices = new Set(indices);
  }

  // Returns {word, translation, index} or null if all words are learned
  getRandomWord() {
    const active = this.dictionary
      .map((entry, i) => ({ ...entry, index: i }))
      .filter(e => !this.learnedIndices.has(e.index));
    if (active.length === 0) return null;
    return active[Math.floor(Math.random() * active.length)];
  }
}

module.exports = WordEngine;
