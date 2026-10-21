// `Promise<T>` may reject, as in TypeScript: an async function declared to return one can throw.
interface User { id: number; name: string }
async function load(id: number): Promise<User> {
  await Promise.resolve(0)
  if (id < 0) throw new Error(`no user ${id}`)
  return { id, name: `u${id}` }
}
async function main(): Promise<void> {
  const u = await load(1)
  console.log(u.name)
  try {
    await load(-1)
  } catch (e) {
    console.log((e as Error).message)
  }
  const ps: Promise<User>[] = [load(2), load(3)]
  const all = await Promise.all(ps)
  console.log(all.map((x) => x.name).join(","))
}
main()
