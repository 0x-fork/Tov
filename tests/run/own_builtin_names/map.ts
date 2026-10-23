export interface Map<K, V> {
  get(key: K): V | undefined;
  set(key: K, value: V): void;
  size(): number;
}
