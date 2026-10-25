interface Ch { added: boolean; value: string }
function f(changes: Ch[]): string {
  let out = "";
  let change;
  for (let i = 0; i < changes.length; i++) {
    change = changes[i];
    out += change.value;
  }
  return out;
}
function g(changes: Ch[]): string {
  let change;
  change = changes[0];
  return change.value;
}
console.log(f([{ added: true, value: "a" }]), g([{ added: true, value: "b" }]));
