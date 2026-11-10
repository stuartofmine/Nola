// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PromptTemplateDefaults logic in the iOS service layer.
// - Interfaces:
//   - BuiltInPromptTemplates
// - APIs:
//   - none
//
//
//  PromptTemplateDefaults.swift
//  Nola
//
//  Centralized built-in prompt templates for easier manual editing.
//

import Foundation

nonisolated public enum BuiltInPromptTemplates {
    public static func templates() -> [PromptTemplate] {
        // Load built-ins from centralized PromptTemplates
        return PromptTemplates.builtInTemplates()
    }
}
