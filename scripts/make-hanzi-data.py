"""Builds assets/hanzi-parts.txt from Make Me a Hanzi's dictionary.txt (LGPL 3 or later, see assets/hanzi-parts-LICENSE):
  "字<tab>parts<tab>radical" per line, the parts being the components of the character's decomposition with the
  ideographic description characters taken out; ？ stands for a part Make Me a Hanzi does not know.
Only the characters of the words of assets/cedict.txt are kept.
Usage: make-hanzi-data.py <dictionary.txt> <assets dir>
dictionary.txt: https://raw.githubusercontent.com/skishore/makemeahanzi/master/dictionary.txt"""
import json
import pathlib
import sys

dictionary, out = pathlib.Path(sys.argv[1]), pathlib.Path(sys.argv[2])
wanted = set()
for line in (out / "cedict.txt").read_text(encoding="utf-8").splitlines():
    wanted.update(c for c in line.split(" [")[0] if c >= "㐀")
rows = []
for line in dictionary.read_text(encoding="utf-8").splitlines():
    entry = json.loads(line)
    c = entry["character"]
    if c not in wanted:
        continue
    parts = "".join(p for p in entry.get("decomposition", "") if not "⿰" <= p <= "⿿" and p != c)
    rows.append(f"{c}\t{'' if parts == '？' else parts}\t{entry.get('radical', '')}\n")
rows.sort(key=lambda r: ord(r[0]))
(out / "hanzi-parts.txt").write_text("".join(rows), encoding="utf-8")
print("hanzi-parts:", len(rows), "characters,", (out / "hanzi-parts.txt").stat().st_size, "bytes")
