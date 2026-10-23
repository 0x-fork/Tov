// An untyped `{}` stored into by key is a dictionary, as JavaScript's object of keys; assigning a
// field narrows it to what's stored (`current.next = node; current = current.next`).

function twoSum(nums: number[], target: number): number[] | null {
  const seen: any = {};
  for (let i = 0; i < nums.length; i++) {
    if (seen[target - nums[i]] || seen[target - nums[i]] === 0) return [seen[target - nums[i]], i];
    seen[nums[i]] = i;
  }
  return null;
}

function longest(s: string): number {
  const last = {};
  let left = 0;
  let best = 0;
  for (let i = 0; i < s.length; i++) {
    const c = s[i];
    if (typeof last[c] !== "undefined" && last[c] >= left) left = last[c] + 1;
    last[c] = i;
    best = Math.max(best, i - left + 1);
  }
  return best;
}

class ListNode {
  next: ListNode | null = null;
  constructor(public val: number) {}
}
function build(vals: number[]): ListNode | null {
  const head = new ListNode(0);
  let current = head;
  for (const v of vals) {
    current.next = new ListNode(v);
    current = current.next;
  }
  return head.next;
}

const list = build([1, 2, 3]);
console.log(twoSum([2, 7, 11, 15], 9), twoSum([1], 5), longest("abcabcbb"), longest("pwwkew"), list?.next?.val);
