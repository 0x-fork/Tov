import type { Queue } from './queue'
export class StackQueue<T> implements Queue<T> {
  private items: T[] = []
  enqueue(item: T): void {
    if (this.items.length > 3) throw new Error('full')
    this.items.push(item)
  }
  dequeue(): T {
    if (this.items.length === 0) throw new Error('empty')
    return this.items.shift()!
  }
  length(): number { return this.items.length }
}
