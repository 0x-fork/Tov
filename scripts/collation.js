// Derives the collation tables in runtime/tov.c (tv_coll_ranges, tv_coll_idx, tv_coll_ce) from
// ICU's root collation, as Node's Intl has it: `node scripts/collation.js > tables.h`. Each code
// point of the covered ranges gets collation elements (primary: its base letter; secondary:
// accents; tertiary: case and variants), found by comparing at each strength. The model is
// checked against localeCompare on random strings before the tables are written.
const variant = new Intl.Collator('en', { sensitivity: 'variant' }).compare;
const accent = new Intl.Collator('en', { sensitivity: 'accent' }).compare;
const base = new Intl.Collator('en', { sensitivity: 'base' }).compare;
const ch = c => String.fromCodePoint(c);

// Covered: Latin, IPA, marks, Greek, Cyrillic, Armenian, Hebrew, Arabic; Devanagari; Hangul
// jamo; Latin and Greek extended; punctuation and symbols; CJK punctuation and kana; emoji.
// (Hangul syllables are their jamo; Han and the rest are ordered by code point, Han last.)
const RANGES = [[0x0000, 0x0700], [0x0900, 0x0980], [0x1100, 0x1200], [0x1e00, 0x2c00], [0x3000, 0x3100], [0x1f000, 0x1fb00]];
const isMark = c => (c >= 0x300 && c <= 0x36f) || (c >= 0x1ab0 && c <= 0x1aff) || (c >= 0x1dc0 && c <= 0x1dff) || (c >= 0x20d0 && c <= 0x20ff);
const cps = [];
for (const [a, b] of RANGES) for (let c = a; c < b; c++) if (!(c >= 0xd800 && c < 0xe000)) cps.push(c);
// (a mark is an accent when it adds no primary weight: the combining letters ͣ..ͯ have one)
const marks = cps.filter(c => isMark(c) && base('a' + ch(c), 'a') === 0);
const accentMark = new Set(marks);
// marks' secondary order, as on 'a'
const markOrder = marks.slice().sort((x, y) => accent('a' + ch(x), 'a' + ch(y)) || x - y);
const markSec = new Map(); let sw = 1;
for (let i = 0; i < markOrder.length; i++) { if (i && accent('a' + ch(markOrder[i - 1]), 'a' + ch(markOrder[i])) !== 0) sw++; markSec.set(markOrder[i], sw + 1); }
// atoms: what doesn't decompose canonically, not marks
const atoms = cps.filter(c => ch(c).normalize('NFD') === ch(c) && !accentMark.has(c));
const ignorable = new Set(atoms.filter(c => variant('a' + ch(c) + 'b', 'ab') === 0));
const live = atoms.filter(c => !ignorable.has(c));
// expansions: a compatibility character that collates as its decomposition (½, ™, ǆ), or one
// that collates as two ASCII letters (ß, æ)
const letters = 'abcdefghijklmnopqrstuvwxyz';
const expand = new Map();
for (const c of live) {
  if (c < 0x80) continue;
  const k = ch(c).normalize('NFKD');
  if (k !== ch(c) && [...k].length > 1 && base(ch(c), k) === 0) { expand.set(c, k); continue; }
  if (base(ch(c), 'a') < 0 || base(ch(c), 'zz') > 0) continue;
  for (const x of letters) for (const y of letters) if (base(ch(c), x + y) === 0) expand.set(c, x + y);
}
const singles = live.filter(c => !expand.has(c));
singles.sort((x, y) => base(ch(x), ch(y)) || accent(ch(x), ch(y)) || variant(ch(x), ch(y)) || x - y);
// primary classes
const prim = new Map(); let p = 0; const rep = new Map();
for (let i = 0; i < singles.length; i++) { const c = singles[i]; if (!i || base(ch(singles[i - 1]), ch(c)) !== 0) { p++; rep.set(p, c); } prim.set(c, p); }
// secondary: equal to its class's first at accent level (common), else a mark it collates as
const sec = new Map();
for (const c of singles) {
  const r = rep.get(prim.get(c));
  if (accent(ch(c), ch(r)) === 0) { sec.set(c, 0); continue; }
  const m = marks.find(m => accent(ch(c), ch(r) + ch(m)) === 0);
  sec.set(c, m === undefined ? -1 : m);
}
// pseudo marks (ð, ŀ): a secondary between the marks', by comparison on the class's letter
const marksSorted = [...markSec].sort((a, b) => a[1] - b[1]);
const pseudo = new Map();
for (const [c, s] of sec) if (s === -1) {
  const r = rep.get(prim.get(c));
  let pos = 1000;
  for (const [m, w] of marksSorted) if (accent(ch(c), ch(r) + ch(m)) < 0) { pos = w - 0.5; break; }
  pseudo.set(c, pos);
}
// tertiary: the rank of its variant among the class's characters of the same accent
const ter = new Map();
const groups = new Map();
for (const c of singles) { const k = prim.get(c) + ':' + sec.get(c); if (!groups.has(k)) groups.set(k, []); groups.get(k).push(c); }
for (const g of groups.values()) { let t = 0; for (let i = 0; i < g.length; i++) { if (i && variant(ch(g[i - 1]), ch(g[i])) !== 0) t++; ter.set(g[i], t); } }

// The model: a string's collation elements, compared level by level.
const HAN = c => (c >= 0x3400 && c <= 0x4dbf) || (c >= 0x4e00 && c <= 0x9fff) || (c >= 0xf900 && c <= 0xfaff) || (c >= 0x20000 && c <= 0x3ffff);
const covered = c => RANGES.some(([a, b]) => c >= a && c < b);
function ces(s, out) {
  for (const cc of s.normalize('NFD')) {
    const c = cc.codePointAt(0);
    if (covered(c)) {
      if (ignorable.has(c)) continue;
      if (markSec.has(c)) { out.push([0, markSec.get(c), 1]); continue; }
      if (expand.has(c)) {
        const n0 = out.length; ces(expand.get(c), out);
        for (let i = n0; i < out.length; i++) out[i][2] += 0.5;
        continue;
      }
      if (prim.has(c)) {
        out.push([prim.get(c), 1, 1 + ter.get(c)]);
        const m = sec.get(c);
        if (m > 0) out.push([0, markSec.get(m), 1]);
        else if (m === -1) out.push([0, pseudo.get(c), 1]);
        continue;
      }
    }
    // (outside the tables: by code point, Han last)
    out.push([(HAN(c) ? 0x2000000 : 0x1000000) + c, 1, 1]);
  }
  return out;
}
function cmp(a, b) {
  const x = ces(a, []), y = ces(b, []);
  for (const lvl of [0, 1, 2]) {
    const xs = x.map(e => e[lvl]).filter(v => v > 0), ys = y.map(e => e[lvl]).filter(v => v > 0);
    const n = Math.min(xs.length, ys.length);
    for (let i = 0; i < n; i++) if (xs[i] !== ys[i]) return xs[i] < ys[i] ? -1 : 1;
    if (xs.length !== ys.length) return xs.length < ys.length ? -1 : 1;
  }
  return 0;
}

// Checked against ICU on random strings: common characters, and any of the covered ones.
const common = [..."aAbBcCeEnNoOsSzZ -_.,'0129éÉèêëÇçñÑöÖøØåÅßæÆœŒłŁđðÐıİžŽšĳǆſŀαΑβяЯёạẠắ€—“…½①™ⓐ²→★😀🎉한글あア中文́̈\u0001\t"];
const all = [...prim.keys(), ...expand.keys(), ...markSec.keys()].map(ch);
let s = 1; const rnd = n => { s = (s * 1103515245 + 12345) % 2147483648; return s % n; };
const pick = () => rnd(3) ? common[rnd(common.length)] : all[rnd(all.length)];
const word = () => { let w = ''; const n = 1 + rnd(6); for (let i = 0; i < n; i++) w += pick(); return w; };
let bad = 0; const ex = [];
const N = +(process.env.N || 400000);
for (let i = 0; i < N; i++) {
  const a = word(), b = rnd(2) ? word() : [...a].slice(0, -1).join('') + pick();
  const want = Math.sign(a.localeCompare(b)), got = cmp(a, b);
  if (want !== got) { bad++; if (ex.length < 30) ex.push(JSON.stringify(a) + ' ' + JSON.stringify(b) + ' want ' + want + ' got ' + got); }
}
if (bad) { console.error('mismatches', bad, 'of', N); console.error(ex.join('\n')); process.exit(1); }

// The tables: ranks of the weights (0: none at that level), each code point's elements.
const list = [];
for (const [a, b] of RANGES) for (let c = a; c < b; c++) list.push([c, c >= 0xd800 && c < 0xe000 ? null : ces(ch(c), [])]);
const prims = new Set(), secs = new Set(), ters = new Set();
for (const [, e] of list) if (e) for (const [p, s, t] of e) { if (p < 0x1000000) prims.add(p); secs.add(s); ters.add(t); }
const rank = set => new Map([...set].sort((a, b) => a - b).map((x, i) => [x, i]));
const pr = rank(prims), sr = rank(secs), tr = rank(ters);
const P = p => p === 0 ? 0 : pr.get(p) + (pr.has(0) ? 0 : 1);
const ceArr = [], idx = [];
for (const [, e] of list) {
  if (!e || e.some(x => x[0] >= 0x1000000)) { idx.push(0xffffffff); continue; }
  if (e.length > 15) throw new Error('too many elements');
  idx.push(ceArr.length | (e.length << 24));
  for (const [p, s, t] of e) ceArr.push([P(p), sr.get(s) + 1, tr.get(t) + 1]);
}
const maxP = Math.max(...ceArr.map(x => x[0]));
let off = 0, rtab = [];
for (const [a, b] of RANGES) { rtab.push(`{0x${a.toString(16)}, 0x${b.toString(16)}, ${off}}`); off += b - a; }
let out = `/* Generated by scripts/collation.js from ICU's root collation (Node's Intl): each covered code
 * point's collation elements (primary, secondary, tertiary). A slot is an offset into
 * tv_coll_ce | its count << 24 (0 elements: ignorable); 0xffffffff: not covered (ordered by code
 * point after every covered primary, Han last). */\n`;
out += `#define TV_COLL_MAXP ${maxP}\n#define TV_COLL_SEC0 ${sr.get(1) + 1}\n#define TV_COLL_TER0 ${tr.get(1) + 1}\n`;
out += `static const struct { uint32_t lo, hi, off; } tv_coll_ranges[${RANGES.length}] = {${rtab.join(', ')}};\n`;
out += `static const uint32_t tv_coll_idx[${idx.length}] = {` + idx.map((v, i) => (i % 12 ? '' : '\n    ') + (v === 0xffffffff ? '~0u' : v) + ',').join('') + '\n};\n';
out += `static const uint16_t tv_coll_ce[${ceArr.length}][3] = {` + ceArr.map(([p, s, t], i) => (i % 8 ? ' ' : '\n    ') + `{${p}, ${s}, ${t}},`).join('') + '\n};\n';
process.stdout.write(out);
console.error('elements', ceArr.length, 'primaries', maxP, 'secondaries', sr.size, 'tertiaries', tr.size, 'expansions', expand.size, 'ignorable', ignorable.size);
