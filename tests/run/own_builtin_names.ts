// A module's own types may take a built-in type's name (`interface Map`), as in TypeScript:
// there, the name means the module's type; values (`new Map()`) are still the built-ins.
import type { Map } from "./own_builtin_names/map";

type Partial = { done: boolean };

class ListMap<K, V> implements Map<K, V> {
  private keys: K[] = [];
  private values: V[] = [];
  get(key: K): V | undefined {
    const i = this.keys.indexOf(key);
    return i < 0 ? undefined : this.values[i];
  }
  set(key: K, value: V): void {
    const i = this.keys.indexOf(key);
    if (i < 0) {
      this.keys.push(key);
      this.values.push(value);
    } else {
      this.values[i] = value;
    }
  }
  size(): number {
    return this.keys.length;
  }
}

const m: Map<string, number> = new ListMap<string, number>();
m.set("a", 1);
m.set("b", 2);
m.set("a", 3);
const real = new Map<string, number>([["z", 9]]);
const p: Partial = { done: true };
console.log(m.get("a"), m.get("c"), m.size(), p.done, real.get("z"), real.size);
