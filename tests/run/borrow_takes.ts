// References a local takes from a field (`const r = n.left!; ...; n.left = t`) and parameters an
// arrow reassigns: no object freed while still held, no caller's string changed in place.
class Node {
  data: number;
  left: Node | null = null;
  right: Node | null = null;
  height = 1;
  constructor(d: number) {
    this.data = d;
  }
}
function rotateRight(node: Node): Node {
  const newRoot = node.left!;
  const temp = newRoot.right;
  newRoot.right = node;
  node.left = temp;
  return newRoot;
}
function rotateLeft(node: Node): Node {
  const newRoot = node.right!;
  const temp = newRoot.left;
  newRoot.left = node;
  node.right = temp;
  return newRoot;
}
const h = (n: Node | null) => (n ? n.height : 0);
function fix(n: Node): Node {
  n.height = 1 + Math.max(h(n.left), h(n.right));
  const bal = h(n.left) - h(n.right);
  if (bal > 1) {
    if (h(n.left!.left) < h(n.left!.right)) n.left = rotateLeft(n.left!);
    return rotateRight(n);
  }
  if (bal < -1) {
    if (h(n.right!.right) < h(n.right!.left)) n.right = rotateRight(n.right!);
    return rotateLeft(n);
  }
  return n;
}
function insert(n: Node | null, v: number): Node {
  if (!n) return new Node(v);
  if (v < n.data) n.left = insert(n.left, v);
  else n.right = insert(n.right, v);
  return fix(n);
}
function inorder(n: Node | null, out: number[]): number[] {
  if (n) {
    inorder(n.left, out);
    out.push(n.data);
    inorder(n.right, out);
  }
  return out;
}
let root: Node | null = null;
for (const v of [11, 7, 5, 9, 3, 1, 20, 15, 30, 25, 2, 4, 6, 8, 10]) root = insert(root, v);
const fresh = new Node(99);
console.log(inorder(root, []).join(" "), root!.data, root!.height, fresh.data);

const appendComment = (s: string) => {
  s = `${s} {comment}`;
  return s;
};
let ms = "x";
for (let i = 0; i < 3; i++) ms = ms + i;
const r = appendComment(ms);
const shout = (s: string) => {
  s += "!";
  s += "!";
  return s;
};
console.log(JSON.stringify(ms), JSON.stringify(r), shout(ms), ms);
