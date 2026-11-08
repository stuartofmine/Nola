#!/usr/bin/env node
// SPDX-License-Identifier: Apache-2.0
// Validates the Android data-center localization contract across every locale.

const fs = require("fs");
const path = require("path");

const root = path.resolve(__dirname, "..");
const resources = path.join(root, "android", "app", "src", "main", "res");
const localeDirectories = fs
  .readdirSync(resources, { withFileTypes: true })
  .filter((entry) => entry.isDirectory() && /^values(?:-|$)/.test(entry.name))
  .map((entry) => entry.name)
  .filter((name) => name === "values" || /^values-(?:de|es|fr|ja|ru|zh)$/.test(name))
  .sort();

function decodeXml(value) {
  return value
    .replace(/<xliff:g\b[^>]*>([\s\S]*?)<\/xliff:g>/g, "$1")
    .replace(/<[^>]+>/g, "")
    .replace(/&quot;/g, '"')
    .replace(/&apos;/g, "'")
    .replace(/&lt;/g, "<")
    .replace(/&gt;/g, ">")
    .replace(/&amp;/g, "&")
    .trim();
}

function loadStrings(directory) {
  const file = path.join(resources, directory, "strings.xml");
  if (!fs.existsSync(file)) throw new Error(`Missing ${file}`);
  const xml = fs.readFileSync(file, "utf8");
  const values = new Map();
  const pattern = /<string\s+name="([^"]+)"[^>]*>([\s\S]*?)<\/string>/g;
  for (const match of xml.matchAll(pattern)) {
    if (match[1].startsWith("data_sources_")) {
      values.set(match[1], decodeXml(match[2]));
    }
  }
  return values;
}

function placeholders(value) {
  return [...value.matchAll(/%(?:\d+\$)?[a-zA-Z]/g)].map((match) => match[0]).sort();
}

const catalogs = new Map(localeDirectories.map((directory) => [directory, loadStrings(directory)]));
const source = catalogs.get("values");
if (!source || source.size === 0) throw new Error("No data_sources_* strings found in values/strings.xml");

const failures = [];
for (const [directory, catalog] of catalogs) {
  for (const [key, sourceValue] of source) {
    if (!catalog.has(key)) {
      failures.push(`${directory}: missing ${key}`);
      continue;
    }
    const value = catalog.get(key);
    if (!value) failures.push(`${directory}: empty ${key}`);
    if (JSON.stringify(placeholders(value)) !== JSON.stringify(placeholders(sourceValue))) {
      failures.push(`${directory}: placeholder mismatch for ${key}`);
    }
  }
  for (const key of catalog.keys()) {
    if (!source.has(key)) failures.push(`${directory}: unexpected ${key}`);
  }
}

if (failures.length > 0) {
  console.error(failures.join("\n"));
  process.exit(1);
}

console.log(
  `Android data-center localization validation passed: ${source.size} keys, ${localeDirectories.length} locales`,
);
