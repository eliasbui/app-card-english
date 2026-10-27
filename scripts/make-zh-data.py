"""Builds the two Chinese files of assets from the downloads:
  cedict.txt   "traditional simplified [pin1 yin1]" per line, CC-CEDICT without the English definitions
  hanviet.txt  "字 tự" per line, the Sino-Vietnamese reading of a character
Usage: make_zh.py <cedict.txt.gz> <Unihan.zip> <TudienThienChuu.tab> <out dir>

A reading comes from Thiều Chửu (1942, public domain; CC0 as catusf/tudien), or else from Unihan kVietnamese when
that is a syllable Thiều Chửu also reads: Unihan mixes in readings of chữ Nôm, some of them vulgar, which are not
Sino-Vietnamese syllables. A simplified character
reads as its traditional form (儿 as 兒, nhi, not as the radical 儿, nhân), and a character without a reading of its
own takes that of a variant (告 as 吿)."""
import gzip
import pathlib
import sys
import unicodedata
import zipfile

cedict_gz, unihan_zip, thieu_chuu, out = sys.argv[1], sys.argv[2], sys.argv[3], pathlib.Path(sys.argv[4])

lines, seen, header = [], set(), []
with gzip.open(cedict_gz, "rt", encoding="utf-8") as f:
    for line in f:
        line = line.rstrip("\n")
        if line.startswith("#"):
            header.append(line)
            continue
        head, sep, _ = line.partition("/")
        head = head.strip()
        if not sep or "[" not in head or not head.endswith("]"):
            continue
        if head not in seen:
            seen.add(head)
            lines.append(head)
(out / "cedict.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
print("cedict entries", len(lines), "bytes", (out / "cedict.txt").stat().st_size)
print("\n".join(h for h in header if "date=" in h or "license=" in h))


def char(code):
    return chr(int(code.split("<")[0][2:], 16))


own, nom, variants = {}, {}, {"kTraditionalVariant": {}, "kZVariant": {}, "kSemanticVariant": {}}
with zipfile.ZipFile(unihan_zip) as z:
    for name in ("Unihan_Readings.txt", "Unihan_Variants.txt"):
        for line in z.read(name).decode("utf-8").splitlines():
            if line.startswith("#") or not line.strip():
                continue
            code, field, value = line.split("\t", 2)
            if field == "kVietnamese":
                nom[char(code)] = value.split()[0]
            elif field in variants:
                variants[field][char(code)] = [char(v) for v in value.split()]
chuu = 0
for line in pathlib.Path(thieu_chuu).read_text(encoding="utf-8").splitlines():
    head, _, rest = line.partition("\t")
    # Some characters are written as compatibility ideographs, which NFC turns into the usual ones.
    head = unicodedata.normalize("NFC", head)
    # The first word: some readings carry the number of strokes, "đại (5 n)".
    first = (rest.split("<br>")[0].split("|")[0].split() or [""])[0].lower().replace("ð", "đ")
    if len(head) == 1 and first and all(c.isalpha() for c in first):
        chuu += 1
        own[head] = first
syllables = set(own.values())
for c, r in nom.items():
    if c not in own and r in syllables:
        own[c] = r
own["儿"] = "nhi"
trad, z_var, sem = variants["kTraditionalVariant"], variants["kZVariant"], variants["kSemanticVariant"]


def pick(c):
    forms = trad.get(c, [])
    if forms and c not in forms:
        for t in forms:
            if t in own:
                return own[t]
    if c in own:
        return own[c]
    for v in forms + z_var.get(c, []) + sem.get(c, []):
        if v in own:
            return own[v]
    return None


chars = set(own) | set(trad) | set(z_var) | set(sem)
for head in lines:
    t, s, _ = head.split(" ", 2)
    chars.update(t + s)
reading = {c: r for c in chars if "\u3400" <= c and (r := pick(c))}
rows = sorted(reading.items(), key=lambda kv: ord(kv[0]))
(out / "hanviet.txt").write_text("".join(f"{c} {r}\n" for c, r in rows), encoding="utf-8")
print("hanviet: thieu chuu", chuu, "written", len(rows), "bytes", (out / "hanviet.txt").stat().st_size)
print("check:", " ".join(f"{c}={reading.get(c)}" for c in "儿学爱电这告值面干后发吧你了们里"))
