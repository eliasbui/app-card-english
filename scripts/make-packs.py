"""Builds the word packs of assets from the downloads:
  ngsl.txt   NGSL 1.2, one word per line by frequency rank   (NGSL_12_stats.csv)
  nawl.txt   NAWL 1.2, the same                              (NAWL_12_stats.csv)
  tsl.txt    TSL 1.2, the same                               (TSL_12_stats.csv)
  hsk30.txt  HSK 3.0 (2021), "level word" per line, 7 for the levels 7 to 9   (hsk30-expanded.csv)
Usage: make-packs.py <download dir> <out dir>

The three English lists come from https://www.newgeneralservicelist.com/s/<name>, the HSK list from
https://raw.githubusercontent.com/ivankra/hsk30/master/hsk30-expanded.csv. Of an HSK term only its first variant is
kept, never the examples of an affix (们, not 朋友们), and a word listed at two levels only at the lower one."""
import csv
import pathlib
import sys

dl, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])


def ranked(name, word_col, rank_col):
    raw = (dl / name).read_bytes()
    try:
        text = raw.decode("utf-8-sig")
    except UnicodeDecodeError:
        # The TSL file is in Windows-1252 (résumé).
        text = raw.decode("cp1252")
    rows = sorted((int(r[rank_col]), r[word_col].strip()) for r in csv.DictReader(text.splitlines()))
    words, seen = [], set()
    for _, w in rows:
        if w and w.lower() not in seen:
            seen.add(w.lower())
            words.append(w)
    return words


for name, src, word_col, rank_col in [
    ("ngsl", "NGSL_12_stats.csv", "Lemma", "SFI Rank"),
    ("nawl", "NAWL_12_stats.csv", "Word", "Rank"),
    ("tsl", "TSL_12_stats.csv", "Word", "TSL Rank"),
]:
    (out / f"{name}.txt").write_text("\n".join(ranked(src, word_col, rank_col)) + "\n", encoding="utf-8")

lines, seen, ids = [], set(), set()
with open(dl / "hsk30-expanded.csv", encoding="utf-8") as f:
    for r in csv.DictReader(f):
        if r["Example"] or r["ID"] in ids:
            continue
        ids.add(r["ID"])
        word, level = r["Simplified"].strip(), r["Level"].split("-")[0]
        if word not in seen:
            seen.add(word)
            lines.append(f"{level} {word}")
(out / "hsk30.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
