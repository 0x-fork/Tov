// Two object literals with the same keys as a conditional's branches: one record, each field
// either branch's.
function leaf(c: boolean) {
  return c ? { left: [1, 2], right: [3] } : { left: null, right: null };
}
const a = leaf(true), b = leaf(false);
console.log(a.left, b.left, a.right?.length ?? 0, b.right === null);
