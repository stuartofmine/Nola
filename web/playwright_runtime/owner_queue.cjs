// SPDX-License-Identifier: Apache-2.0
'use strict';

/** Per-owner FIFO. Waiting operations also protect their owner from eviction. */
class OwnerOperationQueue {
  constructor() { this.owners = new Map(); }
  isBusy(owner) { return (this.owners.get(owner)?.pending || 0) > 0; }
  run(owner, operation) {
    let entry = this.owners.get(owner);
    if (!entry) {
      entry = { tail: Promise.resolve(), pending: 0 };
      this.owners.set(owner, entry);
    }
    entry.pending += 1;
    const result = entry.tail.then(operation);
    entry.tail = result.then(() => undefined, () => undefined);
    return result.finally(() => {
      entry.pending -= 1;
      if (entry.pending === 0 && this.owners.get(owner) === entry) this.owners.delete(owner);
    });
  }
}
module.exports = { OwnerOperationQueue };
