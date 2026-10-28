(() => {
  "use strict";
  /// Học thông minh: the exercise that fits how well a word is known. New and shaky words (under two reviews, a
  /// memory of under two days, or just forgotten) are picked out of four; then written from their meaning, then from
  /// hearing them; words kept three weeks and more are said aloud or used in a sentence. What is missing steps aside:
  /// choices need four words, speaking the course's Whisper model, a sentence an AI.
  function smartKind(w) {
    const speak = !!state.models[courseOf() === "zh" ? "sttzh" : "stt"]?.ready, write = PROVIDERS.some(ready);
    if (w.reps < 2 || w.stability < 2 || w.interval === 0) return practice.all.length >= 4 ? "choice" : "fill";
    if (w.stability < 7) return "fill";
    if (w.stability < 21 || !(speak || write)) return "dict";
    return speak && (!write || Math.random() < 0.5) ? "speak" : "write";
  }
  exercises.smart = { show: (w, card, p) => exercises[smartKind(w)].show(w, card, p) };
})();
