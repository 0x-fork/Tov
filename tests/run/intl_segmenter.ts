const seg = new Intl.Segmenter("en", { granularity: "word" });
for (const s of ["Hello, world!  It's 3.14 here", "foo_bar baz-qux", "a\nb\tc", "don't stop", "x  y"]) {
  console.log(JSON.stringify(Array.from(seg.segment(s)).map(p => [p.segment, p.index, p.isWordLike])));
}
const g = new Intl.Segmenter("en", { granularity: "grapheme" });
console.log(JSON.stringify(Array.from(g.segment("abc")).map(p => p.segment)), seg.resolvedOptions().granularity);
