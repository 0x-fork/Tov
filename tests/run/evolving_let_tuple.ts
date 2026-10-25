interface Ch { added: boolean; removed: boolean; value: string }
function conv(changes: Ch[]): [number, string][] {
  const ret: [number, string][] = [];
  let change,
      operation: number;
  for (let i = 0; i < changes.length; i++) {
    change = changes[i];
    if (change.added) operation = 1;
    else if (change.removed) operation = -1;
    else operation = 0;
    ret.push([operation, change.value]);
  }
  return ret;
}
console.log(JSON.stringify(conv([{ added: true, removed: false, value: "a" }, { added: false, removed: false, value: "b" }])));
