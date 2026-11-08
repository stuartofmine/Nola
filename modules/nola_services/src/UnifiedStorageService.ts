// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements UnifiedStorageService logic in the project module.
// - Interfaces:
//   - UnifiedStorageService
// - APIs:
//   - none
//
import { PersistentStore } from 'persistence/src/PersistentStore';

export class UnifiedStorageService {
  private store: PersistentStore;

  constructor() {
    this.store = new PersistentStore('nola_unified');
  }

  async saveString(key: string, value: string): Promise<void> {
    await this.store.storeString(key, value);
  }

  async fetchString(key: string): Promise<string | null> {
    return await this.store.fetchString(key);
  }

  async remove(key: string): Promise<void> {
    await this.store.remove(key);
  }
}
