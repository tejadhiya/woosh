/**
 * Fixed-capacity circular ring buffer for 60 Hz telemetry feeds.
 *
 * Avoids copying and allocating up to 600 objects on every 60 Hz frame.
 * Implements Iterable and uses a Svelte 5 reactive version signal so templates
 * re-render cleanly without recreating arrays.
 */
export class RingBuffer<T> {
  private buffer: (T | undefined)[];
  private head = 0;
  private count = 0;
  version = $state(0);
  readonly capacity: number;

  constructor(capacity = 600) {
    this.capacity = capacity;
    this.buffer = new Array(capacity);
  }

  push(item: T) {
    this.buffer[this.head] = item;
    this.head = (this.head + 1) % this.capacity;
    if (this.count < this.capacity) {
      this.count++;
    }
    this.version++;
  }

  pushMany(items: T[]) {
    if (!items || items.length === 0) return;
    for (let i = 0; i < items.length; i++) {
      this.buffer[this.head] = items[i];
      this.head = (this.head + 1) % this.capacity;
      if (this.count < this.capacity) {
        this.count++;
      }
    }
    this.version++;
  }

  clear() {
    this.head = 0;
    this.count = 0;
    this.buffer.fill(undefined);
    this.version++;
  }

  get length(): number {
    void this.version;
    return this.count;
  }

  *[Symbol.iterator](): Iterator<T> {
    void this.version;
    const start = this.count < this.capacity ? 0 : this.head;
    for (let i = 0; i < this.count; i++) {
      yield this.buffer[(start + i) % this.capacity] as T;
    }
  }

  toArray(): T[] {
    void this.version;
    const res: T[] = new Array(this.count);
    const start = this.count < this.capacity ? 0 : this.head;
    for (let i = 0; i < this.count; i++) {
      res[i] = this.buffer[(start + i) % this.capacity] as T;
    }
    return res;
  }
}

