// `m?.[1]` of a match that may be `null`: its group, or `undefined`.

const words = ["key=value", "nothing"];
for (const w of words) {
  const m = /^(\w+)=(\w+)$/.exec(w);
  console.log(m?.[1], m?.[2] ?? "-", m?.[0].length);
}
