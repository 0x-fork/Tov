interface Path { oldPos: number; last: string | undefined }
function f(n: number): number {
  const best: Path[] = new Array(n + 3);
  best[1] = { oldPos: 4, last: undefined };
  best[3] = { oldPos: 9, last: "x" };
  let total = 0;
  for (let k = 0; k < n; k += 2) {
    const removePath = best[k + 1],
          addPath = best[k + 3];
    const canAdd = addPath && addPath.oldPos < 100;
    const canRemove = removePath && removePath.oldPos + 1 < 50;
    if (!canAdd && !canRemove) continue;
    if (!canRemove || (canAdd && removePath.oldPos < addPath.oldPos)) total += addPath.oldPos;
    else total += removePath.oldPos;
  }
  return total;
}
console.log(f(2), f(4));
