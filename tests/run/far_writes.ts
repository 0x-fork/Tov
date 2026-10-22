const keys: number[] = []
keys[3] = 7
keys[keys.length] = 9
console.log(keys.length, keys[3], keys[4])
const count: number[] = []
for (const x of [3, 0, 2, 5, 3]) count[x] = (count[x] || 0) + 1
console.log(count.length, count[3], count[5])
