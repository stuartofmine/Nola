// SPDX-License-Identifier: Apache-2.0
// Canonical Monochrome Obsidian & Porcelain surface tokens synchronized with shared/design-tokens/tokens.json.

export interface DesktopThemeTokens {
    readonly canvas: string;
    readonly surface: string;
    readonly surfaceSubtle: string;
    readonly surfaceElevated: string;
    readonly ink: string;
    readonly inkSecondary: string;
    readonly inkMuted: string;
    readonly border: string;
    readonly outline: string;
    readonly accent: string;
    readonly onAccent: string;
    readonly accentSubtle: string;
    readonly success: string;
    readonly warning: string;
    readonly danger: string;
    readonly info: string;
}

export const LIGHT_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#fafafa",
    surface: "#ffffff",
    surfaceSubtle: "#f2f2f4",
    surfaceElevated: "#ffffff",
    ink: "#111113",
    inkSecondary: "#52525b",
    inkMuted: "#63636e",
    border: "#e4e4e7",
    outline: "#71717a",
    accent: "#18181b",
    onAccent: "#ffffff",
    accentSubtle: "#ebebef",
    success: "#147537",
    warning: "#925605",
    danger: "#c81e1e",
    info: "#2563eb",
};

export const DARK_THEME_TOKENS: DesktopThemeTokens = {
    canvas: "#0b0b0c",
    surface: "#131315",
    surfaceSubtle: "#1a1a1d",
    surfaceElevated: "#242428",
    ink: "#f5f5f7",
    inkSecondary: "#a1a1aa",
    inkMuted: "#8e8e93",
    border: "#26262a",
    outline: "#7a7a82",
    accent: "#f5f5f7",
    onAccent: "#0b0b0c",
    accentSubtle: "#202024",
    success: "#5cd68d",
    warning: "#f0b85a",
    danger: "#f28080",
    info: "#78b4f8",
};
