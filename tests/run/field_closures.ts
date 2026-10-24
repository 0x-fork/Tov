// Calls through a class field of a function type: the closures stored there are all the call
// can run. One that drops the object the call borrows keeps it alive; others borrow it.
class Task {
  constructor(public id: number, public run: (t: Task) => number) {}
}
class Sched {
  cur: Task | null = null;
  tasks: Task[] = [];
  add(id: number, fn: (t: Task) => number): void {
    this.tasks.push(new Task(id, fn));
  }
  step(): number {
    return this.cur!.run(this.cur!);
  }
}
const s = new Sched();
s.add(1, (t) => t.id * 10);
s.add(2, (t) => {
  // drops the running task: it lives on until the call returns
  s.cur = null;
  return t.id + 100;
});
let total = 0;
for (const t of s.tasks) {
  s.cur = t;
  total += s.step();
}
console.log(total);

// A field set again later, through a path the constructor doesn't see.
class Box {
  item: Task | null = null;
  act: () => number = () => 0;
  go(): number {
    return this.item!.run(this.item!) + this.act();
  }
}
const b = new Box();
b.item = new Task(5, (t) => {
  b.item = null;
  return t.id;
});
b.act = () => {
  b.item = null;
  return 1;
};
console.log(b.go(), b.item);
b.item = new Task(6, (t) => t.id * 2);
console.log(b.go());

// A class used as a value: its constructor can be given anything.
class Cb {
  constructor(public f: () => string) {}
}
function make(K: new (f: () => string) => Cb): Cb {
  return new K(() => "made");
}
console.log(make(Cb).f(), new Cb(() => "direct").f());
