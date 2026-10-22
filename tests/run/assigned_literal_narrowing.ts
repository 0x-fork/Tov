function pick(n: number) {
  let label: string | undefined
  if (n > 2) label = "big"
  else label = "small"
  return `${label}`
}
function pick2(n: number) {
  let label: string | undefined
  if (n > 2) label = "big"
  else label = "small"
  return label
}
console.log(pick(1), pick2(1))
