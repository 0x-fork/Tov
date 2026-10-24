// Assigning through `x!` checks x is there, as reading it does.
class A { xs: number[] | null = null; set(i: number) { this.xs![i] = 1; } }
const a = new A(); a.xs = [0, 0]; a.set(1); console.log(a.xs);
const b = new A(); b.set(0);
