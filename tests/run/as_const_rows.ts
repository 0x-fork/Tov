function f(n: number, b: boolean) { return `${n}${b}`; }
for (const row of [[0, false], [1, true], [4, true]] as const) {
  const a = row[0]; const b = row[1];
  console.log(f(a, b), JSON.stringify(row[0]), `${JSON.stringify(row)}`);
}
const objs = [{ input: [1, 2], expected: 3 }, { input: [], expected: 0 }];
for (const row of objs) { const { input, expected } = row; console.log(input.length, expected, JSON.stringify(row.input)); }
