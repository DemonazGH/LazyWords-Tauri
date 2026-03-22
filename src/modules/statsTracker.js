// statsTracker.js — Daily stats and streak tracking (stub for stage 7)

class StatsTracker {
  constructor(stats = {}) {
    this.stats = stats;
  }

  today() {
    return new Date().toISOString().slice(0, 10);
  }

  recordShown() {
    const key = this.today();
    if (!this.stats[key]) this.stats[key] = { shown: 0, learned: 0 };
    this.stats[key].shown++;
  }

  recordLearned() {
    const key = this.today();
    if (!this.stats[key]) this.stats[key] = { shown: 0, learned: 0 };
    this.stats[key].learned++;
  }

  decrementLearned() {
    const key = this.today();
    if (this.stats[key] && this.stats[key].learned > 0) {
      this.stats[key].learned--;
    }
  }

  getStreak() {
    let streak = 0;
    const d = new Date();
    while (true) {
      const key = d.toISOString().slice(0, 10);
      if (!this.stats[key] || this.stats[key].shown === 0) break;
      streak++;
      d.setDate(d.getDate() - 1);
    }
    return streak;
  }

  getLast7Days() {
    const result = [];
    const d = new Date();
    for (let i = 6; i >= 0; i--) {
      const day = new Date(d);
      day.setDate(d.getDate() - i);
      const key = day.toISOString().slice(0, 10);
      result.push({ date: key, ...(this.stats[key] ?? { shown: 0, learned: 0 }) });
    }
    return result;
  }
}

module.exports = StatsTracker;
