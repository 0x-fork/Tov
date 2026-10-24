// A function whose result holds its own results has a recursive result type, as TypeScript's.
function createTree(depth: number) {
  return depth-- > 0
    ? { left: createTree(depth), right: createTree(depth) }
    : { left: null, right: null };
}
function checksum(node: ReturnType<typeof createTree>): number {
  if (!node.left) return 1;
  return 1 + checksum(node.left) + checksum(node.right!);
}
interface TreeNode {
  left: TreeNode | null;
  right: TreeNode | null;
}
function count(node: TreeNode): number {
  return 1 + (node.left ? count(node.left) : 0) + (node.right ? count(node.right) : 0);
}
const t = createTree(5);
console.log(checksum(t), count(t));
