function main() {
  const lines = ["a", "b", "> c"];
  let i;
  for (i = 0; i < lines.length; i++) {
    if (lines[i].startsWith(">")) break;
  }
  console.log(i, lines.slice(i).join("|"));
  let tempStart;
  let best = Infinity;
  for (const n of [5, 3, 9]) {
    tempStart = n * 2;
    if (tempStart < best) best = tempStart;
  }
  console.log(best, tempStart);
}
main();
