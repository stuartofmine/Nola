// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements LLMContext logic in the project module.
// - Interfaces:
//   - LLMContext
// - APIs:
//   - none
//
export interface LLMContext {
  infer(prompt: string, opts?: { maxTokens?: number, stream?: boolean }): Promise<any>;
  streamInfer?(prompt: string, opts: { maxTokens?: number }, callback: (chunk: string) => void): void;
  downloadModel?(version: string): Promise<void>;
}

export default LLMContext;
