// A class implementing a generic interface with methods that throw: in TypeScript any method
// may throw, so the interface's say nothing of it.
import type { Queue } from "./generic_iface_throws/queue";
import { StackQueue } from "./generic_iface_throws/stack_queue";

const q: Queue<number> = new StackQueue<number>();
q.enqueue(1);
q.enqueue(2);
console.log(q.dequeue(), q.length());
const s = new StackQueue<string>();
try {
  s.dequeue();
} catch (e) {
  console.log((e as Error).message);
}
