"""Builds the Chinese files of assets from the downloads:
  cedict.txt           "traditional simplified [pin1 yin1]" per line, CC-CEDICT without the English definitions
  cedict-gloss.txt.gz  a short English gloss for each line of cedict.txt, in the same order (gzip, rebuilt byte for
                       byte by the same Python)
  hanviet.txt          "字 tự" per line, the Sino-Vietnamese reading of a character
Usage: make_zh.py <cedict.txt.gz> <Unihan.zip> <TudienThienChuu.tab> <out dir>
CC-CEDICT comes from https://www.mdbg.net/chinese/export/cedict/cedict_1_0_ts_utf-8_mdbg.txt.gz; the version in
assets is the one of 2026-10-27 (assets/cedict-LICENSE).

A gloss is the first senses of the entry that fit in 80 characters (two words of each when there are more), without
classifiers, pronunciation notes and references to other entries (个 for 個|个[ge4]); an entry that is only a variant
of another has the gloss of that one, and an entry that is only a reference keeps it ("used in 吗啡").

A reading comes from Thiều Chửu (1942, public domain; CC0 as catusf/tudien), or else from Unihan kVietnamese when
that is a syllable Thiều Chửu also reads: Unihan mixes in readings of chữ Nôm, some of them vulgar, which are not
Sino-Vietnamese syllables. A simplified character
reads as its traditional form (儿 as 兒, nhi, not as the radical 儿, nhân), and a character without a reading of its
own takes that of a variant (告 as 吿)."""
import gzip
import pathlib
import re
import sys
import unicodedata
import zipfile

cedict_gz, unihan_zip, thieu_chuu, out = sys.argv[1], sys.argv[2], sys.argv[3], pathlib.Path(sys.argv[4])

lines, senses, header = [], {}, []
with gzip.open(cedict_gz, "rt", encoding="utf-8") as f:
    for line in f:
        line = line.rstrip("\n")
        if line.startswith("#"):
            header.append(line)
            continue
        head, sep, rest = line.partition("/")
        head = head.strip()
        if not sep or "[" not in head or not head.endswith("]"):
            continue
        if head not in senses:
            senses[head] = []
            lines.append(head)
        senses[head] += [s for s in rest.split("/") if s.strip()]
(out / "cedict.txt").write_text("\n".join(lines) + "\n", encoding="utf-8")
print("cedict entries", len(lines), "bytes", (out / "cedict.txt").stat().st_size)
print("\n".join(h for h in header if "date=" in h or "license=" in h))

GLOSS = 80
# 個|个[ge4] or 得[de2]: the traditional form when it differs, the simplified one and the pinyin.
REF = re.compile(r"(?:([^\s\[\]|,;()]+)\|)?([^\s\[\]|,;()]+)\[([^\]]*)\]")
DROP = re.compile(r"CL:|(?:Taiwan |also )?pr\. ")
XREF = re.compile(r"see |used in |(?:\w+ )*variant of |same as |also written |erhua form of ")
VARIANT = re.compile(r"(?:\w+ )*variant of |same as |also written ")
NOTE = re.compile(r"\s*\((?:CL:|(?:Taiwan |also )?pr\. )[^)]*\)")
LEAD = re.compile(r"\([^()]*\)\s*(?=\S)")


def plain(sense):
    sense = REF.sub(r"\2", NOTE.sub("", sense))
    # A reference may come without its pinyin, and pinyin without its word.
    sense = re.sub(r"[^\s\[\]|,;()]+\|([^\s\[\]|,;()]+)", r"\1", sense)
    return re.sub(r"\s*\[[^\]]*\]", "", sense).strip()


def trim(part):
    # A sense too long to show loses the note in front of it: "(used to describe sb ...) intense; severe".
    lead = LEAD.match(part)
    return part[lead.end():] if lead and len(part) > GLOSS else part


def short(parts):
    # Two words of each sense, when there are more senses, leave room for them.
    if len(parts) > 1:
        parts = ["; ".join(p.split("; ")[:2]) for p in parts]
    text = ""
    for part in map(trim, parts[:3]):
        if text and len(text) + 2 + len(part) > GLOSS:
            break
        text = f"{text}; {part}" if text else part
    if len(text) > GLOSS:
        text = text[: GLOSS - 1].rsplit(" ", 1)[0].rstrip(",;:") + "…"
    return text


# A reference names the traditional form, or the one form when both are the same.
entry = {}
for head in lines:
    t, _, p = head.split(" ", 2)
    entry.setdefault((t, p[1:-1]), head)
meant = {h: [p for s in senses[h] if not DROP.match(s) and not XREF.match(s) and (p := plain(s))] for h in lines}


def gloss(head):
    if meant[head]:
        return short(meant[head])
    first = senses[head][0] if senses[head] else ""
    ref = REF.search(first) if VARIANT.match(first) else None
    target = ref and entry.get((ref.group(1) or ref.group(2), ref.group(3)))
    if target and meant[target]:
        return short(meant[target])
    return short([p for s in senses[head] if not DROP.match(s) and (p := plain(s))])


glosses = "".join(gloss(h) + "\n" for h in lines).encode("utf-8")
(out / "cedict-gloss.txt.gz").write_bytes(gzip.compress(glosses, compresslevel=9, mtime=0))
print("glosses bytes", len(glosses), "gzip", (out / "cedict-gloss.txt.gz").stat().st_size)


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
