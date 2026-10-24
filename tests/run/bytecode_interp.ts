// A small stack-machine interpreter: a switch dispatch loop over an opcode array.
const enum Op { Push, Add, Sub, Mul, Lt, Jz, Jmp, Load, Store, Halt }
function run(code: number[], locals: number[]): number {
  const stack: number[] = [];
  let pc = 0;
  for (;;) {
    const op = code[pc++];
    switch (op) {
      case Op.Push: stack.push(code[pc++]); break;
      case Op.Add: { const b = stack.pop()!, a = stack.pop()!; stack.push(a + b); break; }
      case Op.Sub: { const b = stack.pop()!, a = stack.pop()!; stack.push(a - b); break; }
      case Op.Mul: { const b = stack.pop()!, a = stack.pop()!; stack.push((a * b) % 1000003); break; }
      case Op.Lt: { const b = stack.pop()!, a = stack.pop()!; stack.push(a < b ? 1 : 0); break; }
      case Op.Jz: { const t = code[pc++]; if (stack.pop()! === 0) pc = t; break; }
      case Op.Jmp: pc = code[pc]; break;
      case Op.Load: stack.push(locals[code[pc++]]); break;
      case Op.Store: locals[code[pc++]] = stack.pop()!; break;
      case Op.Halt: return stack.length > 0 ? stack[stack.length - 1] : locals[1];
    }
  }
}
// i = 0; acc = 1; while (i < N) { acc = acc * 3 + i; i = i + 1 }
const N = 3000000;
const code = [
  Op.Push, 0, Op.Store, 0, Op.Push, 1, Op.Store, 1,
  Op.Load, 0, Op.Push, N, Op.Lt, Op.Jz, 33,
  Op.Load, 1, Op.Push, 3, Op.Mul, Op.Load, 0, Op.Add, Op.Store, 1,
  Op.Load, 0, Op.Push, 1, Op.Add, Op.Store, 0, Op.Jmp, 8,
  Op.Halt,
];
code[14] = 34;
console.log(run(code, [0, 0]));
