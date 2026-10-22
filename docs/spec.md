# Tov language spec (draft 0)

Tov is TypeScript-shaped. **If code looks like TypeScript it behaves like TypeScript, or it is a compile error with a code and a fix.** This spec lists only what differs from, or is removed from, TypeScript; everything else means what it means in TS.

Status tags: **[M0]** checked today · **[Mn]** planned for milestone n.

## 1. Files, modules, entry point
- Source files end in `.tov` (`.ts` works too). One file = one module.
- `import { a, b as c } from "./other"`, `import * as ns from "./other"`, `import d from "./other"` (its `export default`), `import type { T }`, and `import * as fs from "std/fs"`. A TypeScript module is found as `tsc` and Bun find it: `./util` is `util.ts` (or `util.tsx`, or `util/index.ts`), `./util.js` is the `util.ts` it compiles to (NodeNext style), and `./util.ts` is itself; a `.tov` module is named without its extension. A Node.js built-in's default import is the module (`import path from "node:path"`). No side-effect imports. **[M0]**
- `export` goes on declarations (`export function`, `export type`, `export interface`, `export const`, `export class`), or lists them: `export { a, b as c }`, `export default f` (or `export default function f() {}`, or any value); and passes on another module's: `export * from "./m"`, `export * as ns from "./m"`, `export { a as b } from "./m"`, `export type { T } from "./m"` [M2].
- A constant can be used by a closure in its own initializer when its type is known without the closure (it's annotated, or it's a call whose signature fixes its type): `const server = Bun.serve({ websocket: { message(ws, m) { server.publish("room", m) } } })`.
- **Entry point.** The program's entry file is either a **script** — top-level statements run in source order, with its module `const`/`let` initialized where they're declared — or has `function main()` (optionally `main(): int` for an exit code). Imported modules hold only declarations, `const`/`let` bindings and `test(...)` calls: importing a module has no side effects.
- **Module state.** Module-level `let` can be reassigned in its module; a module `const` can't be rebound, but its contents can change (`todos.push(t)`), as in TS. Imported bindings are read-only.

## 2. Lexical
- Comments `//`, `/* */`. Identifiers as in TS (ASCII + `_` + `$`).
- Integer literals: `42`, `1_000`, `0xff`, `0b1010`, `0o17`. Float literals contain `.` or an exponent: `1.0`, `.5`, `2e10`.
- Strings: `"..."` or `'...'` with JS escapes. Template literals `` `a ${expr} b` ``.
- Semicolons are optional. A line break ends a statement unless the next line starts with `.`, `?.`, or a binary operator, or the break is inside `()`/`[]`. A line starting with `(` or `[` never continues the previous line.

## 3. Types **[M0]**
| Type | Meaning |
|---|---|
| `int` | 64-bit signed integer (overflow traps) |
| `f64`, `f32` | IEEE floats (`number`: see below) |
| `i8 i16 i32 u8 u16 u32 u64` | sized integers |
| `bool` (alias `boolean`), `string` | UTF-8 immutable string |
| `bigint` | arbitrary-precision integer, as in TS (`123n`) |
| `decimal` | exact base-10 number, 34 significant digits (Tov's own: `19.99m`, or `19.99` where a `decimal` is expected) |
| `undefined`, `null`, `void`, `never`, `unknown` | as TS (`??`, `?.` and `!` treat `null` like `undefined`; `x == null` matches both) |
| `T[]` / `Array<T>` | growable array (value); `Uint8Array`, `Int32Array`, `Float64Array` and the other typed arrays are arrays of `u8`, `i32`, `f64`, ... |
| `Map<K, V>`, `Set<T>` | hash map/set (value) |
| `{ a: T, b?: U }` | exact record (value) |
| `A \| B` | union; `"lit"` string-literal types and `true`/`false` (discriminants: `{ ok: true, value: T } \| { ok: false, error: E }`, narrowed by `if (r.ok)`); numeric literal types (`1 \| 2 \| 3`) are `number` |
| `A & B` | intersection of record types: one record with the fields of each |
| `(a: T) => U` | function type (`(a: T) => U throws E` if it can throw, §7) |
| `Record<string, V>` | string-keyed map built with an object literal: `{ "Content-Type": "text/plain" }`; read with `.get(k)` (it is a `Map<string, V>`) |
| `Promise<T>`, `Promise<T, E>` | a promise of a `T` that never rejects, or can reject with `E` (an `Error` class); see §7b |

- Tuples `[A, B]` are records whose fields are `0`, `1`, ... (`t[0]`, `t.length`, printed and written to JSON as arrays). Destructuring works in `let`/`const` (`const [a, b] = t`, `const { x, y: py, z = 0 } = p`, holes and `...rest` in arrays), `for...of` (`for (const [k, v] of map)`), parameters and `[a, b] = [b, a]`; defaults there and on parameters (`f(n: Node | null = root)`) apply only to `undefined`, not `null`, as in JavaScript; a missing array element traps, where JavaScript gives `undefined`. `map.entries()`, `xs.entries()`, `new Map(entries)` and `Object.keys/values/entries` give and take `[key, value]` tuples.
- Removed: `any`, `object`, `symbol`, conditional/mapped types, namespaces, decorators. `enum`s are a type (a number, or the union of their string values) and a constant holding the members (`Color.Red`); a numeric enum numbered from 0 maps back to names (`Color[0]`), and a `switch` with a case for every member is exhaustive.
- `boolean` is exactly `bool`. `number` is TypeScript's number: each `number` written in an annotation is an `int` when only integers ever reach it (from the program's own code: data parsed from JSON or received from JavaScript keeps it a float), and an `f64` otherwise. The program means the same either way, except that an integer past 2^63 traps where a float would round (as an `int`'s does); `fib(n: number): number` runs as fast as with `int`.

### Numbers
- An integer literal is `int` unless context expects a float type. An unannotated `let x = 0` becomes `f64` if any later use in the function requires it (`x += 0.5`, `return x` from an `f64` function).
- `int` widens implicitly to `f64` (in arithmetic and assignment). `f64` → `int` is never implicit: use `Math.trunc/floor/ceil/round(x)`, which return `int` (trap on NaN/overflow).
- Float arithmetic is IEEE 754 with every operation rounded separately, exactly as in JavaScript: `a * b + c` is never fused into one multiply-add.
- Bitwise operators on `int` and floats are JavaScript's: both sides wrap to 32 bits (ToInt32), shift counts are taken mod 32 and `>>>` is unsigned, giving an `int` (`(h * 31 + c) | 0`, `x | 0` to truncate). Sized integers keep their own width.
- Sized integers compute as `int` (`u8 + u8` is an `int`, as C promotes them; not `u64`), and a number stored into an element of a sized array wraps into it, as into a JavaScript typed array (`bytes[i] = 300` stores 44).
- `/` always produces `f64` (as in TS). `%`, `+ - *` on two ints produce `int`. Sized integers don't mix implicitly; convert with `u8(x)`, `i32(x)` etc. (trap if out of range).

### bigint
- `bigint` is JavaScript's BigInt: `123n`, `0xffn`, `+ - * / % **` (`/` truncates), `& | ^ ~ << >>` (two's complement), `===`, `<` and friends, `++`/`--`, `typeof x === "bigint"`. `BigInt(x)` converts an integer (exactly), a string (`try`: `SyntaxError` unless it's one) or a number (`try`: `RangeError` unless it's an integer); `Number(x)` is the nearest number; `int(x)`, `u64(x)`, `i32(x)`... trap if it doesn't fit. `x.toString(radix?)`, `BigInt.asIntN(bits, x)`, `BigInt.asUintN(bits, x)`.
- A `bigint` never mixes with numbers: `x + 1` is an error with the fix `x + 1n` (JavaScript throws a TypeError); comparing with `<` against an integer works, as in JS. `>>>` is an error (a bigint has no width). Dividing by `0n` and a negative `**` exponent trap. `JSON.stringify` rejects bigints, as JavaScript does.
- Values below 2^62 in magnitude live in one word (no allocation); larger ones are reference-counted, and `x += y`, `x *= y`, `x = (x + a * b) * c` update `x` in place when nothing else holds it.

### decimal
- `decimal` holds money and other base-10 quantities exactly: `0.1m + 0.2m === 0.3m`. A literal is a decimal with an `m` suffix, or wherever a `decimal` is expected (`const price: decimal = 19.99`, `total * 0.0825`); an `int` converts to one implicitly (exactly), a float never does (`decimal(x)` converts a number through its shortest form, so `decimal(0.1)` is `0.1`; NaN and the infinities trap). `decimal("12.50")` parses (`try`: `SyntaxError`, or `RangeError` past 34 digits before the point); `Number(d)` is the nearest number, `int(d)` traps unless it's a whole number that fits.
- `+ - *` are exact and keep every digit: the scale of a sum is the larger of the two (`1.10 + 2.205` is `3.305`), of a product their sum (`1.10 * 3` is `3.30`). Past 34 digits a result rounds half to even; 34 digits before the point traps. `/` gives 34 significant digits, half to even (`1m / 3m` is `0.3333333333333333333333333333333333`); an exact quotient keeps the dividend's scale less the divisor's (`10.00m / 4` is `2.50`). `%` is exact. `===` and `<` compare values (`1.10m === 1.1m`); `toString()` keeps the scale (`"1.10"`).
- `d.round(places?, mode?)` (modes as the TC39 Decimal proposal names them: `"halfEven"`, the default, `"halfExpand"`, `"halfTrunc"`, `"halfCeil"`, `"halfFloor"`, `"ceil"`, `"floor"`, `"trunc"`, `"expand"`), `floor()`, `ceil()`, `trunc()`, `abs()`, `toFixed(digits)` (half away from zero, as Number's). `JSON.stringify` writes a decimal as a number with its digits (`3.10`), and `JSON.parse` reads a number into a decimal exactly.
- No `**`, bitwise operators or `Math` functions: decimals are for exact arithmetic.

### Records
- `type P = { x: f64, y: f64 }` declares an exact record. Two record types with the same fields are the same type.
- An object literal must provide every required field and no others. Optional fields (`b?: U`) may be omitted and have type `U | undefined`.
- Records, arrays, maps, sets, and strings are **values**: assignment copies (cheaply, copy-on-write). See §6.

### Unions and narrowing
- A **discriminated union** is a union of records sharing a string-literal field (e.g. `kind`). It compiles to a tagged union.
- Narrowing works on: `switch (x.kind)`, `if (x.kind === "a")`, `x === undefined` / `x !== undefined`, `typeof x === "string"` (for unions of primitives), and early exits (`if (x === undefined) return` narrows the rest of the block).
- Field paths of locals narrow too: `if (t.left !== undefined) use(t.left)`. So does a module `const` (`const port = process.env.PORT; if (port !== undefined) ...`), though not its fields (they can change elsewhere). `Array.isArray(x)` narrows a union to its array members. A type guard given to `filter`, `find` or `findLast` narrows what they return: declared (`(x): x is Cat => ...`, or a function `isCat(p: Pet): p is Cat`), or inferred as TypeScript 5.5 does, from a body that is true exactly when the parameter is of part of its type (`xs.filter((x) => x !== undefined)` is a `T[]`, `pets.filter((p) => p.kind === "cat")` a `Cat[]`). Assigning to the path (or any prefix of it), or passing it as `inout`, forgets the narrowing. Inside a closure, narrowings of the enclosing function's `let` variables don't apply. A call can change a narrowed path that goes through an object (`this.head`), so in `.tov` the call forgets the narrowing; in TypeScript it is kept, as `tsc` does, and reading the path after a call checks the value still has the narrowed type (a trap if a callback changed it).
- A `switch` over a union discriminant or literal union must be exhaustive or have `default`.

### Interfaces **[M2]**
- `interface` is structural and implicit: any record or class with the members (and matching types) satisfies it. As a generic bound it is statically dispatched; as a value type (`function f(x: Named)`, `Shape[]`) it holds any satisfying value. A record behind an interface is a copy (records stay values); a class instance is shared.
- Methods in interfaces (`area(): f64`) are function-typed members: a class satisfies them with methods, a record with fields holding functions.

### Generics
- `function f<T>(x: T): T`, `function g<T extends I>(...)`, `type Box<T> = { value: T }`, defaults (`<T = undefined>`: the type when nothing fixes it, and what `Box` means without arguments), generic classes, and generic methods (`static of<T>(x: T)`, `json<T>(): Promise<T, SyntaxError>`; a generic method can't be overridden yet). Type arguments are inferred from arguments and from the expected type (`const t: Todo = try await req.json()`); closures are checked after everything else, so `connect({ data: { n: 1 }, open: (s) => s.data.n })` infers `T` from `data` and then types `s`. Bodies are checked once, at definition: a `T` can only be used through its bound.

## 4. Declarations and statements **[M0]**
- `function`, `const`, `let`, `type`, `interface`, arrow functions `(x: T) => expr` / `=> { ... }`, and methods in object literals (`{ fetch(req) { ... } }`, a field holding an arrow function).
- Functions declared inside a body are closures (`function walk(n: number): void { ... }`), hoisted as in JavaScript: each is made just before the first statement that uses it. One that calls itself needs its parameters annotated, and its result too when that can't be told from the returns that don't call it (as in TypeScript); functions side by side that call each other need both annotated; they share one environment, so no reference cycle forms. `function (x) { ... }` expressions are arrow functions (without their own `this` or `arguments`). Generic nested functions aren't supported yet.
- Exported functions must annotate every parameter and the return type (unannotated return means `void`). Local and unexported functions may omit the return type when it can be inferred.
- `if/else`, `while`, `do/while`, `for (init; cond; step)`, `for (const x of xs)`, `switch`, `break`, `continue`, `return`, blocks.
- Conditions, `!`, `&&` and `||` use JavaScript's truthiness: `false`, `0`, `NaN`, `""`, `0n` and `undefined` are false, everything else (objects, even empty arrays) true. `a || b` and `a && b` are values unless both are `bool` (`name || "anonymous"`, `node && node.next`); `if (x)` narrows `x` to defined.
- A function with a non-`void` return type must return on every path.
- `var` is `let`; `for (const k in obj)` iterates `Object.keys(obj)`; `==` and `!=` are `===` and `!==` (TypeScript already rejects comparing different types), except that `x == null` and `x == undefined` match both.
- Removed: `with`, labels, `delete`, `arguments`, `eval`, the comma operator (but for steps: `i++, j--`).

## 5. Expressions **[M0]**
- Operators and precedence as in TS. Equality on records/arrays compares by value.
- `+` concatenates when either side is a `string` (numbers/bools format as in JS).
- `a?.b` (skipping the rest of the chain when `a` is absent: `a?.b.c(x)`, `a?.[i]`, `f?.()()`), `a ?? b`, `c ? a : b`, `x as T` (checked cast from `unknown` or a union; traps on failure), `x!` (non-undefined assertion; traps).
- Indexing `xs[i]` requires an integer `i` and has type `T`, as in TypeScript, trapping when out of bounds; where an absent value is expected (`xs[i] === undefined`, `xs[i] ?? d`, `xs[i]?.x`, a `T | undefined` binding or parameter) it is `T | undefined` instead. The same goes for `s[i]` on strings and `m[k]` on `Record`s. Index assignment `xs[i] = v` traps when out of bounds.
- Strings are JavaScript's: `s.length`, `s[i]`, `charAt`, `charCodeAt`, `codePointAt`, `at`, `indexOf`, `lastIndexOf`, `slice`, `substring`, `substr`, `padStart`/`padEnd` and `split("")` count UTF-16 code units, and `for (const c of s)` walks code points. They're stored as UTF-8: an ASCII string's indexes are its byte offsets (no cost), and others convert from a cached position, so a loop over `s.charCodeAt(i)` walks the string once. Differences: `s[i]` is `string | undefined` (like an array's); `charCodeAt` returns an `int`, `-1` out of range where JavaScript gives `NaN` (both fail `c >= 48 && c <= 57` and `c === 10`); a lone half of a surrogate pair (cut out by `slice`, `s[i]`, `split("")` or `String.fromCharCode`) becomes U+FFFD, since UTF-8 can't hold one — printed, it's what Node prints. Tov adds `s.byteLength` (UTF-8 bytes) and `s.chars()` (an array of code points).
- `console.log(...args)` prints (Node's formatting); `Math.*` as in JS (with the int-returning rounders above); `NaN`, `Infinity`, `isFinite`, and `Number`'s constants (`MAX_SAFE_INTEGER`, `EPSILON`, ...) and predicates (`Number.isInteger`, `isSafeInteger`, `isFinite`, `isNaN`).
- `map.keys()`, `map.values()` and `set.values()` return arrays (not iterators). `new Array<T>(n)` holds `n` zeros (`""`, `false`) where JavaScript has holes; of objects, it must be filled in order by the loop after it (`const out: P[] = new Array(n); for (let i = 0; i < n; i++) { ...; out[i] = v; }`, `out` used nowhere else before), and each store adds the next element; `new Array(n).fill(v)`, `Array.from(xs, f)`, `Array.from({ length: n }, (_, i) => ...)`, `Array.of(...)` and `[...xs]` work as in JavaScript.
- `parseInt(s)`, `parseFloat(s)` and `Number(s)` return a number, `NaN` when the text isn't one, as in JavaScript (`parseFloat` reads the longest numeric prefix; `Number("")` is 0; `Number("0x1f")` is 31). A float can index an array (it must be a whole number, else it traps), and the built-ins' integer parameters (positions, counts, lengths, digits) take numbers, truncated as JavaScript does.

## 6. Values, mutation, `inout` **[M0]**
- `let` bindings can be reassigned; `const` bindings cannot. Both can be mutated in place (`xs.push(1)`, `p.x = 2`) — same as TS.
- Arrays, maps and sets are shared, as in TypeScript: `const b = a` makes `b` the same array, and a change through any reference (`b.push(1)`, `b[0] = 2`, a function changing its parameter, `m.get(k)!.push(x)`, `const row = grid[0]; row.push(9)`, `seen.add(x)` through a parameter) is seen through all of them. `===` compares them by identity; `slice()`, `[...a]`, `Array.from(a)`, `concat` and `new Map(m)` make new ones.
- Records (object literals) are still values: a function that changes a record parameter in place (and never reassigns it) changes the caller's, as TypeScript's shared objects would: the parameter is `inout`, and arguments are passed as if with `&` (a `const` passed this way becomes a `let`). Written explicitly, `inout` lets the callee replace the whole value (for arrays too):
  ```ts
  function addOne(inout xs: int[]) { xs = [...xs, 1] }
  addOne(&items)
  ```
- Copy-then-mutate of a record where the original is read again (`const q = p; q.x = 1; use(p)`) is an error [M1], because TS would share and Tov copies.
- `class` instances are shared references (reference-counted): assignment shares the object, as in TS, and changes through one reference are seen through all. Their fields can be changed through any reference (`p.x = 1` works on a `const p` or a parameter). See §6a.
- A value is freed as soon as nothing can use it again: a local's value is released after the last statement of its block that uses it (directly or through a local that may borrow from it), not at the end of the block.

## 6a. Classes **[M2]**
```ts
abstract class Shape {
  constructor(readonly name: string) {}     // parameter property: a readonly field
  abstract area(): f64
  describe(): string { return `${this.name}: ${this.area()}` }
}
class Circle extends Shape {
  private hits = 0
  static readonly UNIT = 1
  constructor(private r: f64) { super("circle") }
  area(): f64 { return Math.PI * this.r * this.r }
  get diameter(): f64 { return 2 * this.r }
}
```
- Single inheritance (`extends`, `super(...)`, `super.method(...)`), `abstract` classes and methods, `implements` (checked structurally), `static` methods and fields, getters and setters (`set name(v)`: `x.name = v`, `x.name += 1`), `x.constructor.name`, `public`/`protected`/`private` and `#private` members, `readonly` fields (set only by their initializer or the constructor), parameter properties. No generic methods yet (make the class generic, or use a top-level generic function).
- Every field needs a value: an initializer, an assignment at the top level of the constructor, or `?` (optional). In a constructor, `this` is only used to set fields until all are set. A derived class's constructor starts with `super(...)`.
- An override takes the same parameters (or wider types) and returns the same type (or a narrower one). Methods nobody overrides are called directly.
- `x instanceof C` narrows unions and base-class types; `x as C` is a checked downcast (traps if wrong). `===` on instances is identity. `console.log` prints `ClassName { field: value }`, and `${x}` uses a `toString(): string` method if the class has one.
- `C | undefined` fields and variables are nullable pointers (no tag word).
- **Memory.** Instances are reference-counted and freed as soon as the last reference goes. Reference counting can't free *cycles*, so the checker rejects class graphs where instances could form a cycle through strong fields, and offers three fixes: `weak parent: Node | undefined` (a back-reference that doesn't keep its target alive and reads as `undefined` once the target is freed); `readonly` references set in the constructor (objects then only point to older objects, so no cycle can form); or `cyclic class Node`, which also enrolls its instances (and those of classes on the same cycles, and subclasses) in a cycle collector. Only cyclic classes pay for it. Closures stored in fields that capture `this` are not traced: avoid storing such closures on the object they capture.

## 7. Errors **[M3]**
```ts
class NotFound extends Error {
  constructor(readonly key: string) { super(`no such key: ${key}`); this.name = "NotFound" }
}
function lookup(m: Map<string, int>, k: string): int throws NotFound {
  const v = m.get(k)
  if (v === undefined) throw new NotFound(k)
  return v
}
function total(m: Map<string, int>): int {   // throws NotFound (inferred)
  return lookup(m, "a") + lookup(m, "b")
}
try {
  console.log(total(m))
} catch (e) {                                  // e: NotFound
  console.log(e.message)
} finally {
  cleanup()
}
```
- Only `Error` objects (the built-in `Error`, `TypeError`, `RangeError`, `SyntaxError`, or classes extending them) are thrown. `e.message`, `e.name`, and `String(e)` / `${e}` (`"Name: message"`) work as in JavaScript.
- As in TypeScript, an error a call throws passes on to the caller unless a `try { }` block catches it. `try f(x)` marks such a call (optional: it documents that the error passes on). `throws E` declares what a function can throw; without it, functions and methods infer it, as they infer their return types.
- `catch (e)` gives `e` the most specific class covering everything the block can throw (else `Error`); narrow with `instanceof`. `finally` runs on every path out of the block (no `return`/`break` inside a `try` that has `finally` yet).
- A named closure (`const f = () => ...`, a nested function) passes on what its body throws, as a function declaration does, and so does a callback given to a built-in method (`xs.map((s) => parse(s))`: the method stops at the first error and passes it on; a sort's comparator that throws leaves the array in some order). Any other closure can pass errors on too, as in TypeScript: a function type written without `throws` (`type Handler = (req: Request) => Response`) throws `Error` once a function that throws is given to it, and calls through a value of that type are then calls that can throw (where nothing that throws is given to it, calls through it cost nothing extra). `throws E` on a function type says what it throws. A function used as a value keeps its `throws` (`const h = f; h(x)` passes `f`'s error on). An error that escapes a script or `main` prints `uncaught Name: message` to stderr and exits with status 1; in a test it fails the test.
- A JavaScript property read (`ctx.params`, `o[k]`) can throw, from a getter or a Proxy: like a JavaScript call, it passes a `JsError` on unless a `try { }` block catches it. In a closure, either makes its function type throw (as above). Where errors can't pass on (a module constant's initializer), a read that throws is a trap, while a JavaScript call there is refused (T0832).
- A function handed to JavaScript (an npm package's callback or handler) takes `Js` parameters and may pass errors on: they're thrown into JavaScript, a `JsError` as the value JavaScript threw, any other error as a JavaScript `Error` with its name and message. An async function hands JavaScript a promise, rejected with its error.
- Errors are return values underneath (no unwinding): a call that can throw costs one branch.
- Bugs (index out of range with `!`, integer overflow, failed `as`) are traps, not errors: they can't be caught, in a function JavaScript called too.
- Effects (`uses fs | net`) are planned.

## 7a. Standard library **[M3]**
Node's names, so existing habits work:
- `import { readFileSync, readFileBytesSync, writeFileSync, appendFileSync, existsSync, readdirSync, mkdirSync, rmSync, unlinkSync, statSync, accessSync } from "node:fs"` (also `"fs"`, `"std/fs"`): the synchronous API; failures throw `Error` with Node's messages (`ENOENT: no such file or directory, open 'x'`). `readdirSync` returns sorted names. `readFileBytesSync` returns the file's `u8[]` (what Node's `readFileSync` gives as a Buffer), and `writeFileSync`/`appendFileSync` take `string | u8[]`. `statSync` returns a `Stats` (`size`, `mtimeMs`, `isFile()`, `isDirectory()`); `accessSync(path, mode?)` takes Node's mode flags (0 exists, 4 read, 2 write, 1 execute).
- `import * as path from "node:path"`: `join`, `resolve`, `normalize`, `basename`, `dirname`, `extname`, `isAbsolute`, `sep` (POSIX).
- `process.argv` (`[binary, binary, ...args]`, so `process.argv.slice(2)` are the arguments), `process.env.NAME` (`string | undefined`), `process.exit(code?)`, `process.cwd()`, `process.platform`, `process.arch`, `process.pid`, `process.stdout.write(s)`, `process.stderr.write(s)`; `console.error` / `console.warn` write to stderr.
- `Date.now()` and `performance.now()` (milliseconds).
- `JSON.stringify(value, undefined, indent?)` as in JavaScript (maps with string keys are written as objects, where JavaScript writes `{}`). `JSON.parse(text)` takes its type from context — `const c: Config = JSON.parse(text)` or `JSON.parse(text) as Config` — and **checks** the input against it (exact integers, string literals, discriminants, required fields; extra keys ignored, `null` and missing keys read as `undefined`), throwing `SyntaxError` otherwise.
- Rest parameters: `function f(first: T, ...rest: U[])`, in methods too. Default values: `function f(x: T = d)` (an unannotated `n = 10` takes the literal's type); `d` is evaluated when the argument is missing or `undefined`, as in JavaScript (not yet for `inout`, rest and constructor parameter properties).
- Regular expressions: literals (`/(\d+)-(?<w>\w+)/g`) and `new RegExp(pattern, flags?)`, run by QuickJS's engine (vendored, linked only into programs that use them), with JavaScript's syntax, flags and UTF-16 positions. `test`, `exec`, `lastIndex`, `source`, `flags`; a match (`RegExpExecArray`) has `m[i]`, `index`, `input`, `groups`, `length`. Strings take them in `replace`/`replaceAll` (with `$1`, `$<name>`, `$&` or a function), `split`, `match`, `matchAll` and `search`. An invalid pattern throws `SyntaxError`.
- Web servers: see §7c.

## 7b. Async
`async` functions, arrows and methods (class, static and object-literal), and `await`, work as in JavaScript, including the order things run in: microtasks run in order after each timer or request, and an `await` takes the same number of ticks as in Node and Bun (`tests/async` compares every program's output with Node's). A script's top level can `await`, and `main` can be `async`. Not yet: `for await`, `Promise.allSettled`/`any`, `.then`/`.catch`/`.finally`.
- **Promises.** An async function returns `Promise<T>`; its errors reject the promise instead of being thrown at the call: `async function load(id: string): Promise<User> throws NotFound` has type `(id: string) => Promise<User, NotFound>`. `await p` gives the `T`; if `p` can reject, `await` throws, like a call: `const u = await load(id)`. Awaiting a value that isn't a promise gives the value. `new Promise<T>((resolve, reject) => ...)` (it can reject only if the executor takes `reject`), `Promise.resolve(v)` and `Promise.reject(e)` make promises, `Promise.all(ps)` (every value, in order; the first rejection) and `Promise.race(ps)` (the first to settle) combine an array of them (their types merge: `Promise<A, E1> | Promise<B, E2>` is `Promise<A | B, E1 | E2>`); a rejection nobody awaits is reported like an uncaught error.
- **Timers.** `setTimeout(f, ms)` and `setInterval(f, ms)` return an id for `clearTimeout`/`clearInterval`; `queueMicrotask(f)`. Timers keep time as Node's do (whole milliseconds on the loop's clock) and fire within about 0.1 ms of it.
- **Running.** An async function runs until its first `await` that has to wait. Awaiting a call to an async function costs no allocation (its state lives inside the caller's); a call that isn't awaited runs as a task of its own. The program exits when nothing is left to run: no timers, tasks waiting on them, or servers.

## 7c. Web servers (Bun's API)
`Bun.serve`, `Request`, `Response`, `Headers`, `URL` and `URLSearchParams` are globals, as in Bun (also `import { serve, ... } from "std/http"`). A Bun server ports with Tov's usual edit only — `try` on calls that can throw; `tests/parity` runs a Bun server and its port side by side and compares every response.
```ts
const server = Bun.serve({
  port: Number(process.env.PORT ?? 3000),
  routes: {
    "/": new Response("Welcome"),
    "/users/:id": (req) => Response.json({ id: req.params.id }),   // req.params: { id: string }
    "/todos": {
      GET: () => Response.json(todos),
      POST: async (req) => {
        const t = try (await req.json()) as Todo   // a bad body → SyntaxError → 500
        todos.push(t)
        return Response.json(t, { status: 201 })
      },
    },
  },
  fetch(req) {
    const url = try new URL(req.url)
    return new Response(`Not Found: ${url.pathname}`, { status: 404 })
  },
})
console.log(`Listening on ${server.url}`)
```
- **Serving.** `Bun.serve` binds right away (an unusable port prints the error and exits, as an uncaught error does in Bun) and returns a `Server` (`port`, `hostname`, `url`, `stop(closeActive?)`); requests are served once the script's top level (or `main`) has finished, until every server is stopped. `workers: n` (Tov only) forks `n` processes sharing the sockets, supervised: a worker that dies is replaced, and SIGTERM/SIGINT stop them all.
- **Routing** as in Bun: exact paths first, then `:param` and `/*` routes; a route is a `Response`, a handler, or an object of per-method handlers (`GET`, `POST`, ...; others fall through to `fetch`). `req.params` is typed from the route. Unmatched requests go to `fetch`, else `404`.
- **Handlers** return a `Response`, or (`async`) a promise of one: their type is `(req, server) => Response | Promise<Response, Error> throws Error`. A synchronous response is written at once; an async one when its promise settles, and responses on a connection still go out in request order (pipelining works either way).
- **Errors.** A handler that throws (or whose promise rejects) is logged to stderr and answered by `error(err)` if given, else `500 Something went wrong!`, like Bun in production.
- **Types.** `Request`: `method`, `url`, `headers`, `text()` (a `Promise<string>`), `json<T>()` (a `Promise<T, SyntaxError>`, parsed against the expected type). `Response(body?, { status?, statusText?, headers? })`, `Response.json(value, init?)`, `Response.redirect(url, status?)`, `status`, `ok`, `headers`, `text()`, `json<T>()` (promises, as on `Request`). `Headers(init?)`: `get`/`has`/`set`/`append`/`delete`/`forEach`/`toJSON`, case-insensitive; CR/LF in names and values are dropped. `URL(input, base?)` (throws `TypeError`): `href`, `protocol`, `host`, `hostname`, `port`, `pathname`, `search`, `hash`, `origin`, `searchParams`; `URL.parse`, `URL.canParse`. `URLSearchParams`: `get`/`getAll`/`has`/`set`/`append`/`delete`/`size`/`forEach`/`keys`/`values`/`toString`. `Request` also has `formData()`, `blob()`, `bytes()` and `arrayBuffer()`; a `Response` body can be a `Blob` or a `FormData`.
- **Protocol.** HTTP/1.1 with keep-alive, pipelining, `Content-Length` and chunked request bodies, `Expect: 100-continue` and `HEAD`; `content-length` and `date` are added, and `content-type: text/plain;charset=utf-8` when a body has none. Malformed or oversized requests get `400`/`413`/`431` and are closed.
- Not yet: TLS for `Bun.serve`, HTTP/2, streaming bodies, `Bun.file`.
- **WebSockets** as in Bun: `Bun.serve<T>({ fetch(req, server) { if (server.upgrade(req, { data, headers })) return; ... }, websocket: { open, message, close, drain, ping, pong, maxPayloadLength } })`. A handler that upgrades returns nothing. `ServerWebSocket<T>`: `data` (`T | undefined`), `send(text | bytes)`, `sendText`, `sendBinary`, `ping`, `pong`, `close(code?, reason?)`, `terminate()`, `subscribe`, `unsubscribe`, `isSubscribed`, `subscriptions`, `publish(topic, data)` (to the other subscribers; `publishToSelf` includes it), `readyState`, `remoteAddress`; `server.publish(topic, data)` and `server.subscriberCount(topic)`. `message` gets a `string` for text and `u8[]` for binary. Frames are checked as RFC 6455 says (Autobahn's test suite passes); text must be UTF-8; a message over `maxPayloadLength` (16 MB) closes with 1009. Compression (`perMessageDeflate`) and idle timeouts aren't there yet.
- The **`WebSocket` client**: `new WebSocket(url, protocols? | { protocols?, headers?, tls? })` (`ws:`, `wss:`; `try`: a bad URL throws a `SyntaxError`), `onopen`/`onmessage`/`onclose`/`onerror` and `addEventListener`, `send` (messages sent before it opens go out once it does), `close(code?, reason?)`, `readyState`, `protocol`. Events are `WebSocketEvent`s: `data` (`string | u8[]`), `code`, `reason`, `wasClean`.

## 7d. fetch (Bun's client)
`fetch(input, init?)` is a global, as in Bun: `input` is a URL string, a `URL` or a `Request`; `init` is `{ method?, headers?, body?, redirect?, signal?, decompress?, tls?, unix?, proxy?, keepalive? }`. It returns `Promise<Response, Error>`: it resolves once the response's head has arrived (an HTTP error status still resolves, `ok` is false) and the body follows, and rejects with a `FetchError` (a `TypeError` with Bun's `message` and `code`: `ConnectionRefused`, `ENOTFOUND`, `ECONNRESET`, `TooManyRedirects`, `UnexpectedRedirect`, `Malformed_HTTP_Response`, `ZlibError`, `BrotliDecompressionError`, `ZstdDecompressionError`, `ERR_INVALID_URL`, ...) or, when its `signal` aborts, with the signal's reason.
```ts
type User = { id: number, name: string }
const res = try await fetch("http://api.local/users/1", { headers: { Accept: "application/json" }, signal: AbortSignal.timeout(5000) })
if (!res.ok) throw new Error(`HTTP ${res.status}`)
const user = try (await res.json()) as User
```
- **Requests.** Methods are sent upper-cased; every method but `GET`/`HEAD` sends `Content-Length` (a `GET`/`HEAD` with a body is an error). Bodies: `string`, `URLSearchParams` (sent as `application/x-www-form-urlencoded;charset=UTF-8`), bytes (`u8[]`), a `Blob` or `File` (its `type` is the `Content-Type`), or a `FormData` (sent as `multipart/form-data` with a random boundary). The defaults follow the caller's headers, as Bun sends them: `Connection: keep-alive`, `User-Agent`, `Accept: */*`, `Host`, `Accept-Encoding: gzip, deflate, br, zstd`; credentials in the URL become `Authorization: Basic`.
- **Responses.** `status`, `statusText`, `ok`, `headers`, `url` (after redirects, without the fragment), `redirected`, `type`, `bodyUsed`; `text()` (UTF-8 decoded: a BOM is dropped, invalid bytes become U+FFFD), `json<T>()` (checked against the expected type), `bytes()`/`arrayBuffer()` (`u8[]`), `blob()` (typed by the `Content-Type` as it is), `formData()` (a `multipart/form-data` or `application/x-www-form-urlencoded` body; any other rejects with `ERR_FORMDATA_PARSE_ERROR`), `clone()`. These wait for the whole body and reject if it fails (`ECONNRESET` for a connection that drops mid-body, the signal's reason when it aborts). A body is read once: another read rejects with `ERR_BODY_ALREADY_USED`. A response that's dropped unread frees its request (and closes the connection if the body is still arriving).
- **Streaming.** `res.body` is a `ReadableStream` of `u8[]` chunks as they arrive: `getReader()` → `read()` (`{ done, value }`), `cancel()`, `releaseLock()`, `locked`. `for await (const chunk of stream) { ... }` reads it chunk by chunk (it throws, like `try`, if the body fails; leaving the loop early cancels the stream). A reader that falls behind pauses the connection at 4 MB. A compressed body read this way is decoded as it arrives (gzip and deflate with zlib, brotli, zstd), at most 4 MB ahead of the reader, so a small body that decodes to a huge one is decoded only as it's read; one read whole (`text()` and the like) is decoded at once when it's in (gzip and deflate with libdeflate, twice as fast). A stream that ends before its encoding does fails (`ZlibError`, ...). `TextDecoder` (`decode(bytes, { stream: true })` completes characters cut between chunks; `fatal`, `ignoreBOM`) and `TextEncoder` (`encode(text)`) convert between text and `u8[]`.
- **HTTPS.** TLS 1.3 and 1.2 (BoringSSL, vendored: AES-GCM with the CPU's AES instructions, else ChaCha20-Poly1305; X25519 and post-quantum ML-KEM key shares; ALPN `http/1.1`; SNI), with sessions resumed per origin. Certificates are checked against the system's CA bundle (`SSL_CERT_FILE` overrides it, `NODE_EXTRA_CA_CERTS` adds to it) and the host name; failures reject with Node's codes (`CERT_HAS_EXPIRED`, `DEPTH_ZERO_SELF_SIGNED_CERT`, `SELF_SIGNED_CERT_IN_CHAIN`, `UNABLE_TO_VERIFY_LEAF_SIGNATURE`, `ERR_TLS_CERT_ALTNAME_INVALID`, ...). `tls: { ca?, rejectUnauthorized? }` adds trusted certificates or turns the checks off, as does `NODE_TLS_REJECT_UNAUTHORIZED=0`.
- **Protocol.** HTTP/1.1 on pooled keep-alive connections per origin (a pooled connection the server has closed is replaced and the request retried once); `Content-Length`, chunked (extensions and trailers) and close-delimited bodies, interim `1xx` responses, HTTP/1.0 servers; gzip, deflate, brotli and zstd bodies are decoded unless `decompress: false` (`Accept-Encoding: gzip, deflate, br, zstd`, as Bun sends). DNS lookups run on a small thread pool and are cached for 30 s.
- **Redirects** (`redirect: "follow"`, the default): up to 20, as the Fetch standard says; 303 (and 301/302 after a `POST`) continue as `GET` without the body; a redirect to another origin drops `Authorization`, `Proxy-Authorization` and `Cookie`. `"manual"` returns the 3xx response; `"error"` rejects.
- **Cancelling.** `AbortController` (`signal`, `abort(reason?)`), `AbortSignal` (`aborted`, `reason`, `throwIfAborted()`, `addEventListener("abort", f)`, `onabort`, `AbortSignal.abort(reason?)`, `AbortSignal.timeout(ms)`, `AbortSignal.any(signals)`) and `DOMException` (`name`, `code`). An aborted request's connection is closed. `AbortSignal.timeout`'s timer doesn't keep the program running by itself.
- **Headers** (client and server): `get` joins repeated names with `", "`; `getSetCookie()` lists each cookie; `forEach`, `keys()`, `values()` and `toJSON()` see names lower-cased and sorted, repeated names combined.
- **Unix sockets** (Bun's `unix: "/path/to.sock"`): the request goes over that socket; the URL still gives the path and `Host` (`http://localhost/v1/info` for the Docker API). A socket that can't be opened rejects with `FailedToOpenSocket`.
- **Proxies** (Bun's `proxy: "http://user:pass@host:port"`, else `HTTP_PROXY`/`HTTPS_PROXY` (or lower case) unless `NO_PROXY` names the host): plain HTTP goes to the proxy in absolute form; HTTPS through a `CONNECT` tunnel, with TLS and certificate checks end to end. Credentials in the proxy URL become `Proxy-Authorization`. A proxy that refuses the tunnel (`407`) answers the request, as in Bun.
- **Blob, File, FormData.** `new Blob(parts?, { type? })` joins strings (as UTF-8), `u8[]` and Blobs: `size`, `type` (lower-cased; `""` unless printable ASCII), `slice(start?, end?, type?)` (negative offsets count from the end), `text()`, `json<T>()`, `bytes()`, `arrayBuffer()`, `stream()`. `new File(parts, name, { type?, lastModified? })` adds `name` and `lastModified`. `FormData`: `append`/`set` (a string, or a Blob added as a `File`: named by the third argument, else its own name, `"blob"` for a plain Blob), `get`, `getAll`, `has`, `delete`, `forEach`, `keys()`, `values()`; values are `string | File`. It's written as browsers write it: newlines in names and text values become CRLF, and `"`, CR and LF in names and file names are percent-escaped. Unlike Bun, a file is sent with its own `type` (Bun sends most as `application/octet-stream`), and a plain Blob is named `"blob"` (as the standard and Node do).
- Not yet: request bodies as streams, client certificates, `https:` proxies.

## 7e. TCP and TLS sockets (Bun's API)
`Bun.connect(options)` and `Bun.listen(options)`, as in Bun: `hostname`, `port` (or `unix`), `tls` (`true`, or `{ ca?, rejectUnauthorized?, serverName? }` to connect; `{ cert, key }`, PEM text, to listen), `data` (the sockets' starting `socket.data`), `allowHalfOpen`, and `socket`, the handlers: `open(socket)`, `data(socket, bytes)` (a `u8[]`), `drain(socket)`, `end(socket)` (the peer finished sending), `close(socket, error?)`, `error(socket, error)`, `connectError(socket, error)`, `handshake(socket, success, authorizationError?)`.
```ts
type Session = { name: string }
const server = Bun.listen<Session>({
  hostname: "0.0.0.0",
  port: 7000,
  socket: {
    open(socket) { socket.data = { name: "guest" } },
    data(socket, data) { socket.write(data) },
  },
})
const client = try await Bun.connect({ hostname: "localhost", port: 7000, socket: { data(socket, data) { console.log(data.length) } } })
client.write("hello")
```
- `Bun.connect` resolves once the socket is open (with TLS, once the handshake is done and the certificate checked) and rejects with a `SocketError` (`code`: `ECONNREFUSED`, `ENOTFOUND`, `CERT_HAS_EXPIRED`, ...) when it can't connect. `Bun.listen` returns a `TCPSocketListener` (`port`, `hostname`, `stop(closeActive?)`, `ref`, `unref`); a port that can't be bound prints the error and exits, as an uncaught error does in Bun.
- `Socket`: `write(text | bytes)`, `end(data?)`, `flush()`, `terminate()`, `shutdown()`, `setNoDelay(on?)`, `ref()`, `unref()`, `data`, `remoteAddress`, `remotePort`, `localAddress`, `localPort`, `bufferedAmount` (Tov). Unlike Bun's, `write` takes everything: what the kernel can't take yet is buffered (Bun returns a short count and drops the rest), and `drain` follows once it's out. Writes made in a handler go out together when it returns: one system call for many small writes.
- `end()` closes the socket once its data is out (`close` runs); with `allowHalfOpen` it shuts only the sending side, and the peer can keep sending. A handler that throws hands the error to `error`, else it's logged and the socket closes. A connect's `socket.data` is `T | undefined` (`T` from `data`, or the type argument: `Bun.listen<Session>(...)`).

## 8. Tests **[M0]**
```ts
test("adds", () => {
  expect(add(1, 2)).toBe(3)
})
```
`test` calls are top-level only, or in a suite. Jest's and Vitest's API works as it is: `describe("name", () => { ... })` suites (nested too) whose `it`/`test` calls are tests of their own (`suite > name`), `beforeEach`/`afterEach` (run around each test of the suite and the suites in it), `beforeAll`/`afterAll`, `.skip`/`.todo`/`.only`, and `it.each(table)(name, fn)` / `test.each` (a row is spread over the parameters when it's an array, else it's the one parameter; `%s`, `%i`, `%p`... in the name show its values, `$field` an object row's fields, `%#` its number). A table of array literals has rows of tuples, as `as const` makes them.

`tov test dir` runs every `*.test.ts` / `*.spec.ts` file under `dir` as `bun test` does: each is built (on every core, the build cache keeping unchanged ones) and run on its own, so one that doesn't build doesn't stop the others; the totals come last. (A directory of `.tov` files is one program, as before.)

Matchers: `toBe`, `toEqual`, `toStrictEqual`, `toBeCloseTo`, `toBeUndefined`, `toBeDefined`, `toBeNull`, `toBeTruthy`, `toBeFalsy`, `toBeNaN`, `toBeGreaterThan(OrEqual)`, `toBeLessThan(OrEqual)`, `toHaveLength`, `toContain`, and `toThrow`/`toThrowError` (of a function: `expect(() => parse("")).toThrow()`; a message, class or pattern it's given isn't checked yet); `.not` negates any of them.

## 9. Diagnostics
Every diagnostic has a stable code, a location, the observed problem, **the valid alternatives**, and — where possible — a fix tagged `safe` (apply automatically), `maybe` (review), or `placeholder` (needs input). `tov explain <CODE>` prints details.

Code prefixes: `L` lexing · `P` parsing · `N` names/imports · `T` types · `F` control flow · `V` values/mutation · `X` removed TypeScript feature.
