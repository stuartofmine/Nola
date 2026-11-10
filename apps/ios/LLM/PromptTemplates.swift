// SPDX-License-Identifier: Apache-2.0
// File Overview:
// - Purpose: Implements PromptTemplates logic in the iOS LLM integration layer.
// - Interfaces:
//   - PromptTemplates
// - APIs:
//   - none
//
//
//  PromptTemplates.swift
//  Nola
//
//  Centralized prompt templates used by LLM providers and diagnostics (echo test).
//

import Foundation

enum PromptTemplates {
    nonisolated private static func loadSharedPrompt(named name: String) -> String {
        let bundledCandidates = [
            Bundle.main.url(forResource: name, withExtension: "txt"),
            Bundle.main.url(forResource: name, withExtension: "txt", subdirectory: "SharedPrompts"),
        ]
        for bundled in bundledCandidates.compactMap({ $0 }) {
            if let text = try? String(contentsOf: bundled, encoding: .utf8) {
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { return trimmed }
            }
        }

        let currentDirectory = URL(fileURLWithPath: FileManager.default.currentDirectoryPath)
        let sourceCandidates = [
            currentDirectory.appendingPathComponent("apps/ios/SharedPrompts/\(name).txt"),
            currentDirectory.appendingPathComponent("../apps/ios/SharedPrompts/\(name).txt")
        ]
        for url in sourceCandidates {
            if let text = try? String(contentsOf: url, encoding: .utf8) {
                let trimmed = text.trimmingCharacters(in: .whitespacesAndNewlines)
                if !trimmed.isEmpty { return trimmed }
            }
        }

        preconditionFailure("Required SharedPrompts resource is missing: \(name).txt")
    }

    // System prompt for summarization
    static var summarizationSystem: String {
        return """
You are a data-extraction assistant. You will receive an INPUT_JSON object with fields: `candidates` (array of items), optional `userPrefs` (may contain `prompt`, `topN`, or other constraints), and `dateRange` (context).

Goal: produce a single, machine-parseable JSON object (no prose) that summarizes the most relevant actionables found in `candidates` according to `userPrefs` and time-sensitivity.

Output schema (MUST be exactly this shape):
{
  "items": [
    {
      "id": "string - stable id used for dedupe",
      "text": "short actionable text (<=300 chars)",
      "priority": number (0.0-1.0),
      "dedupeKey": "string - stable dedupe key",
      "source": "optional short source label or url",
      "confidence": number (0.0-1.0)
    }
  ],
  "explain": "{{user-facing summary text}}"
}

Hard rules (enforced):
- Return ONLY valid JSON matching the schema above. Do not include any other text, comments, or markdown.
- The `explain` value MUST be wrapped exactly with double braces as shown (app extracts inner text): `{{...}}`.
- If `userPrefs.prompt` exists, treat it as the primary instruction and focus extraction to satisfy it.
- If `userPrefs.topN` is provided, return at most that many `items` (otherwise return up to 10).
- Sort `items` by `priority` descending.
- `text` must be concise and actionable; <=300 characters.
- `dedupeKey` should be deterministic (e.g., combination of source id, type, and normalized timestamp).
- `confidence` reflects parse/extraction certainty (0.0-1.0). For ambiguous cases set confidence < 0.8 and include best-effort result.
- If no actionable items are found, return `"items": []` and put an explicit explanation inside `explain` (still wrapped in `{{}}`).
- Do not hallucinate facts; only extract information present in `candidates`.

Source handling:
- When `candidates` include URLs or content with clear origins, set `source` to a short label (hostname or id) to help traceability.

Language & format:
- Respect language implicit in input; answer in same language unless `userPrefs` requests another.
- Use ISO8601 datetimes if including dates in `text` or `source` fields.

Examples and edge-cases:
- If asked for a "top 3" by `userPrefs.prompt`, include three named entities in `explain` and `items` if available; otherwise explain missing data.

Be deterministic and concise.
"""
    }

    // Stable product identity used as one fragment inside Core Agent composition.
    // Agent safety/tool policy is owned by Core and must not be loaded here.
    static var agentIdentity: String {
        "The active user-facing surface is Nola for iPhone and iPad."
    }

    nonisolated static func agentResponseStyle(_ style: PromptResponseStyle) -> String {
        responseStylePrompt(style)
    }

    static var chatSystem: String {
        get throws {
            try chatSystem(responseStyle: .casual, customSystemPrompt: nil)
        }
    }

    static func chatSystem(responseStyle: PromptResponseStyle, customSystemPrompt: String? = nil) throws -> String {
        let style = responseStylePrompt(responseStyle)
        var fragments: [AgentCorePromptFragment] = [
            .init(
                id: "ios:host-identity",
                layer: .identity,
                trust: .hostFact,
                cacheable: true,
                content: agentIdentity
            ),
            .init(
                id: "ios:user-response-style",
                layer: .responseStyle,
                trust: .userInstruction,
                cacheable: false,
                content: style
            )
        ]
        if let custom = normalizedCustomSystemPrompt(customSystemPrompt), !custom.isEmpty {
            fragments.append(.init(
                id: "ios:user-custom-instructions",
                layer: .executionPolicy,
                trust: .userInstruction,
                cacheable: false,
                content: custom
            ))
        }
        return try AgentCorePromptComposer.compose(fragments).rendered
    }

    nonisolated private static func responseStylePrompt(_ style: PromptResponseStyle) -> String {
        let text = loadSharedPrompt(named: "nola_response_styles")
        return styleSection(from: text, style: style) ?? style.promptInstruction
    }

    nonisolated private static func styleSection(from text: String, style: PromptResponseStyle) -> String? {
        let marker = "[\(style.rawValue)]"
        var capture = false
        var lines: [String] = []
        for line in text.components(separatedBy: .newlines) {
            let trimmed = line.trimmingCharacters(in: .whitespacesAndNewlines)
            if trimmed.hasPrefix("[") && trimmed.hasSuffix("]") {
                if capture { break }
                capture = trimmed == marker
                continue
            }
            if capture { lines.append(line) }
        }
        let section = lines.joined(separator: "\n").trimmingCharacters(in: .whitespacesAndNewlines)
        return section.isEmpty ? nil : section
    }

    private static func normalizedCustomSystemPrompt(_ value: String?) -> String? {
        guard let value else { return nil }
        let trimmed = value.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !trimmed.isEmpty else { return nil }
        if trimmed.count <= 6_000 {
            return trimmed
        }
        let end = trimmed.index(trimmed.startIndex, offsetBy: 6_000)
        return String(trimmed[..<end]).trimmingCharacters(in: .whitespacesAndNewlines)
    }

    static var complexAgentMode: String {
        loadSharedPrompt(named: "nola_complex_agent_mode")
    }

    // User content prefix for summarization; the JSON data (ModelInput) will be appended
    static let summarizationUserPrefix = "INPUT_JSON="

    // Echo test prompt; token will be interpolated
    static func echoUser(token: String) -> String {
        return "Please respond ONLY with the exact token: \(token)"
    }

    // Decision prompt: ask the LLM to return a strict JSON actions response for a single candidate
    static func decisionPrompt(candidateJSON: String, userPrefsJSON: String? = nil) -> String {
        var sb = """
You are a decision-making agent. Given the candidate JSON below, decide which actions to take.
Return STRICT JSON (only JSON) matching the following structure:
{
  "actions": [
    {
      "type": "notify|store|ignore|create_task|reschedule",
      "priority": "high|medium|low",
      "dedupeKey": "string",
      "scheduledAt": "ISO8601 string or null",
      "contentSummary": "short text (<=300 chars)",
      "reason": "brief reason",
      "confidence": number (0.0-1.0)
    }
  ],
  "summary": "optional short summary"
}

Rules:
- Return ONLY valid JSON. Do not include commentary or explanations outside the JSON object.
- If you must include natural language, still ensure there is a single top-level JSON object that can be parsed.
- Ensure each action has a non-empty `dedupeKey`.
- For "notify" actions, ALWAYS include a "scheduledAt" field with an ISO8601 timestamp indicating when the notification should be triggered. Consider the event's due date, urgency, and user's timezone. If no specific time is needed, use null.
- The user can manually adjust the notification time after creation.
"""

        sb += "\nCandidate:\n"
        sb += candidateJSON
        if let prefs = userPrefsJSON {
            sb += "\nUserPrefs:\n"
            sb += prefs
        }
        sb += "\n\nRespond with JSON now."
        return sb
    }

    // Variation: encourage JSON inside a ```json code block to help some models
    static func decisionPromptWithCodeFence(candidateJSON: String, userPrefsJSON: String? = nil) -> String {
        var p = decisionPrompt(candidateJSON: candidateJSON, userPrefsJSON: userPrefsJSON)
        p += "\nIf helpful, you may wrap the JSON in a triple-backtick json block: ```json <your-json> ```"
        return p
    }

    // A recovery prompt that asks the model to re-output only the JSON if its prior response included extra text.
    static func recoveryPromptExpectJSON(previousResponse: String) -> String {
        return """
The previous response may have included extra explanatory text. Extract and return ONLY the top-level JSON object that was present in the previous response.
If the previous response included no JSON, return the string: {"actions": []}

PreviousResponse:
""" + previousResponse
    }

    // Task parsing system prompt template.
    // The result is submitted through a required native tool and validated
    // locally against a complete JSON Schema.
    // The prompt expects naive local ISO-like datetimes (yyyy-MM-dd'T'HH:mm:ss) without timezone suffix.
    static func taskParsingSystem(replacements: [String: String] = [:]) -> String {
      var base = """
You are a task-parsing assistant. Submit exactly one complete result through `submit_task_parse`.

Rules:
- Do not emit prose, marker wrappers, or a second result outside the required tool call.
- If the user wants the app to periodically fetch/summarize/watch a URL, RSS, feed, webpage, or API, output a `scheduled_action` intent instead of a normal reminder/task.
- Current device local datetime is __EX_CURRENT_LOCAL_DATETIME__ (__EX_TIMEZONE__). Use this as the reference for every relative date/time.
- Use the device's LOCAL time and return naive ISO-like datetimes without timezone (format: "yyyy-MM-dd'T'HH:mm:ss").
- For relative expressions (today/tomorrow/next week/etc), compute the next logical datetime; if time is not provided, default = 09:00 local.
- For scheduled actions, `scheduledTimeISO` should be canonical `HH:mm` local time.
- For scheduled actions, infer `format` as one of `html|rss|json|icalendar`.
- For scheduled actions, infer `scheduleFrequency` as `daily` or `weekly`.
- For scheduled actions, set `scheduledWeekday` only when frequency is `weekly` (1=Sunday ... 7=Saturday).
- When ambiguous, choose a reasonable default, set `confidence` < 0.8, and include a brief `reason` field in the JSON explaining ambiguity.
- If the user message is not requesting task creation/update or scheduled automation (for example, plain QA/chitchat), return `intent: "none"` with `task` fields empty/null and include `reason`.
- Immediate requests directed at the assistant are NOT tasks. If the user asks the assistant to answer, recite, translate, explain, write, summarize now, calculate now, role-play, or continue a conversation, return `intent: "none"` unless the user explicitly asks Nola to save, remind, schedule, notify, create a task, or do it later/at a time.
- Bare imperatives such as "背诵赤壁赋", "翻译这段话", "解释量子力学", "帮我写封邮件" are normal chat requests, not reminders/tasks. Do not invent a default 09:00 reminder for them.
- Do not return `null` for `targetDate` when the user gave a time reference; instead compute the datetime.

Expected JSON schema (example fields):
{
  "intent": "none|user_task|scheduled_action",
  "type": "reminder|task|event",
  "title": "string",
  "description": string|null,
  "targetDate": "yyyy-MM-dd'T'HH:mm:ss",
  "isRecurring": boolean,
  "recurrenceRule": { "frequency": "daily|weekly|monthly|yearly", "interval": 1 }|null,
  "relatedPerson": string|null,
  "amount": number|null,
  "currency": string|null,
  "dueDate": string|null,
  "scheduledAction": {
    "name": "string",
    "url": "https://...",
    "format": "html|rss|json|icalendar",
    "scheduleFrequency": "daily|weekly",
    "scheduledTimeISO": "HH:mm",
    "scheduledWeekday": 1,
    "userPrompt": "string|null"
  },
  "confidence": number (0.0-1.0),
  "reason": string (optional for ambiguity)
}

Example:
{ "intent": "user_task", "type": "reminder", "title": "Have dinner", "description": null, "targetDate": "__EX_DAY_AFTER_TOMORROW_19__", "isRecurring": false, "recurrenceRule": null, "relatedPerson": null, "amount": null, "currency": null, "dueDate": null, "scheduledAction": null, "confidence": 0.9, "reason": null }

Example for an immediate assistant request:
{ "intent": "none", "type": null, "title": null, "description": null, "targetDate": null, "isRecurring": null, "recurrenceRule": null, "relatedPerson": null, "amount": null, "currency": null, "dueDate": null, "scheduledAction": null, "confidence": 0.96, "reason": "Immediate assistant response request, not a task creation request." }

Example for a scheduled action:
{ "intent": "scheduled_action", "type": null, "title": "Hacker News Morning Brief", "description": "Summarize the top stories and send a concise digest.", "targetDate": null, "isRecurring": null, "recurrenceRule": null, "relatedPerson": null, "amount": null, "currency": null, "dueDate": null, "scheduledAction": { "name": "Hacker News Morning Brief", "url": "https://news.ycombinator.com/", "format": "html", "scheduleFrequency": "daily", "scheduledTimeISO": "08:00", "scheduledWeekday": null, "userPrompt": "Summarize the top stories and send a concise digest." }, "confidence": 0.92, "reason": null }
"""

      for (k, v) in replacements {
        base = base.replacingOccurrences(of: "__EX_\(k)__", with: v)
      }

      return base
    }

    // Example prompt snippet to show a good JSON output example to the model (helps few-shot correctness)
    static let goodJSONExample = """
Example of valid output:
```json
{
  "actions": [
    {
      "type": "notify",
      "priority": "medium",
      "dedupeKey": "url:example.com:post-123:2025-11-16",
      "scheduledAt": "2025-11-16T12:00:00Z",
      "contentSummary": NSLocalizedString("autogen.PromptTemplates.cb75bb1c", comment: "AUTOGEN"),
      "reason": NSLocalizedString("autogen.PromptTemplates.0e286f95", comment: "AUTOGEN"),
      "confidence": 0.85
    }
  ],
  "summary": "Send a noon reminder and store the article."
}
```
"""

    // A helper that returns a decision prompt including a short example (few-shot) to improve JSON-only responses
    static func decisionPromptWithExample(candidateJSON: String, userPrefsJSON: String? = nil) -> String {
        var p = decisionPromptWithCodeFence(candidateJSON: candidateJSON, userPrefsJSON: userPrefsJSON)
        p += "\n\n" + goodJSONExample
        return p
    }

    // Built-in prompt templates
    nonisolated static func builtInTemplates() -> [PromptTemplate] {
      // Summary template (for summarizing text/content)
      let summaryTemplate = PromptTemplate(
        id: UUID(uuidString: "796164f9-6a7f-5527-9bbc-41fefc7d6845")!,
        name: NSLocalizedString("autogen.PromptTemplateService.summary_name", comment: "AUTOGEN"),
        language: "zh-Hans",
        content: """
  你是一个文本摘要助手。请将下面的输入（可能是事件描述、文章段落或多条文本）进行压缩性总结，输出一个简洁的中文摘要。

  输入内容：
  {{events}}

  返回仅为纯文本摘要（无需 JSON），要求：
  1. 概括要点并保留关键信息；
  2. 使用清晰的段落，不超过 5 行；
  3. 若输入包含待办/事件要点，请在摘要中用“建议”一段给出 1-3 条可执行建议（每条不超过 100 字）。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // Reminder template (for generating actionable reminder items from schedules)
      let reminderTemplate = PromptTemplate(
        id: UUID(uuidString: "928b062f-7477-592c-a929-226bbdb43458")!,
        name: NSLocalizedString("autogen.PromptTemplateService.reminder_name", comment: "AUTOGEN"),
        language: "zh-Hans",
        content: """
  你是一个智能日程助理。请分析以下事件列表并生成可用于通知/提醒的结构化输出。

  日期范围：{{date_range}}
  事件列表：
  {{events}}

  返回结果请严格使用下列 JSON 结构（仅返回 JSON）：
  {
    "items": [
      {
        "text": "短且可执行的提醒文案（<=300 字）",
        "priority": 0.0-1.0,
        "action": {"type": "notify|store|create_task|reschedule", "target": "calendar|reminder"},
        "dedupeKey": "唯一字符串标识"
      }
    ],
    "explain": "{{面向用户的简短摘要}}"
  }

  要求：
  1. 仅返回符合结构的 JSON，不要输出其他说明性文字或注释；
  2. items 按 priority 降序排列；
  3. text 不超过 300 字，尽量包含时间点/上下文；
  4. explain 字段用双花括号包裹（{{...}}）。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let birthdayTemplate = PromptTemplate(
        id: UUID(uuidString: "ac96acf5-a057-5665-accb-0f62746eefbd")!,
        name: NSLocalizedString("autogen.PromptTemplateService.dddd7bf6", comment: "AUTOGEN"),
        language: "zh-Hans",
        content: """
  分析以下联系人生日事件，生成温馨的生日提醒。

  日期范围：{{date_range}}
  事件列表：{{events}}

  生成规则：
  - 今天生日：使用庆祝语气 🎂
  - 明天生日：提醒准备礼物 🎁
  - 3-7天内：提前规划提醒 📅

  返回 JSON 格式，包含 text、priority、action、dedupeKey 字段。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.9c2f12d3", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let taskTemplate = PromptTemplate(
        id: UUID(uuidString: "a29935fb-6c71-531d-8c60-93ce52e9a8b9")!,
        name: NSLocalizedString("autogen.PromptTemplateService.1f17e67f", comment: "AUTOGEN"),
        language: "zh-Hans",
        content: """
  分析待办事项，按紧急程度排序。

  日期范围：{{date_range}}
  事件列表：{{events}}

  分类：
  - 今日到期：⚠️ 高优先级
  - 3天内到期：📝 中优先级
  - 7天内到期：📋 低优先级

  返回 JSON 格式的待办清单。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.a85c7a56", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // English variants: summary and reminder
      let summaryTemplate_en = PromptTemplate(
        id: UUID(uuidString: "5fa67aee-ac94-5e70-99ea-2d1e272b4e4a")!,
        name: "Summary Template",
        language: "en",
        content: """
  You are a text summarization assistant. Given the input (could be event descriptions, article paragraphs, or multiple text items), produce a concise English summary.

  Input:
  {{events}}

  Return only a plain-text summary (not JSON). Requirements:
  1. Capture main points and key facts; 2. Use clear paragraphs, max 5 lines; 3. If input contains tasks/events, include a short "Recommendations" section with 1–3 actionable suggestions (<=100 chars each).
  """,
        description: "Summary template for text/content summarization",
        isBuiltIn: true
      )

      let reminderTemplate_en = PromptTemplate(
        id: UUID(uuidString: "1df8b750-734b-5b24-9e50-aacce6bf3d26")!,
        name: "Reminder Template",
        language: "en",
        content: """
  You are a schedule assistant. Analyze the list of events below and generate structured reminder items suitable for notifications.

  Date range: {{date_range}}
  Events:
  {{events}}

  Return ONLY the JSON object with the structure:
  {
    "items": [ { "text": "Short actionable reminder", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"unique-id" } ],
    "explain": "{{short user-facing summary}}"
  }

  Requirements: only JSON, items sorted by priority, text concise and include time/context where appropriate.
  """,
        description: "Template for event reminders and notification generation",
        isBuiltIn: true
      )

      let birthdayTemplate_en = PromptTemplate(
        id: UUID(uuidString: "8e29ecd4-fab5-54fe-8b47-26cbb1044d5c")!,
        name: "Birthday Reminder",
        language: "en",
        content: """
  Analyze the list of contact birthdays and generate friendly birthday reminders.

  Date range: {{date_range}}
  Events: {{events}}

  Rules:
  - If someone has a birthday today: use celebratory tone.
  - If tomorrow: suggest preparing a gift.
  - Within 3–7 days: schedule a planning reminder.

  Return a JSON array of items including text, priority, action, and dedupeKey.
  """,
        description: "Birthday reminder template",
        isBuiltIn: true
      )

      let taskTemplate_en = PromptTemplate(
        id: UUID(uuidString: "eb173d3c-b3ab-58bc-8d6a-b438d55c1f13")!,
        name: "Task Prioritization",
        language: "en",
        content: """
  Analyze todo items and sort them by urgency.

  Date range: {{date_range}}
  Events: {{events}}

  Classification:
  - Due today: high priority
  - Due within 3 days: medium priority
  - Due within 7 days: low priority

  Return a JSON-formatted todo list.
  """,
        description: "Task prioritization template",
        isBuiltIn: true
      )

      // German templates
      let summaryTemplate_de = PromptTemplate(
        id: UUID(uuidString: "d79c23b7-98f4-5da2-9fc9-1f62cc18abc9")!,
        name: "Zusammenfassungs-Vorlage",
        language: "de",
        content: """
  Du bist ein Assistent für Textzusammenfassungen. Gegeben ist ein Input (z. B. Ereignisbeschreibungen, Artikelabschnitte oder mehrere Textblöcke). Erstelle eine prägnante deutsche Zusammenfassung.

  Input:
  {{events}}

  Gib nur eine Klartext-Zusammenfassung zurück (kein JSON). Anforderungen:
  1. Fasse Hauptpunkte und Schlüsselinformationen zusammen; 2. Verwende klare Absätze, maximal 5 Zeilen; 3. Falls der Input Aufgaben/Ereignisse enthält, füge einen Abschnitt "Empfehlungen" mit 1–3 umsetzbaren Vorschlägen (<=100 Zeichen pro Vorschlag) hinzu.
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let reminderTemplate_de = PromptTemplate(
        id: UUID(uuidString: "deb2fd1c-5f93-5b22-8080-3eedef6edd9c")!,
        name: "Erinnerungs-Vorlage",
        language: "de",
        content: """
  Du bist ein Terminassistent. Analysiere die folgende Liste von Ereignissen und erzeuge strukturierte Erinnerungs‑Einträge, die für Benachrichtigungen verwendet werden können.

  Datumsbereich: {{date_range}}
  Ereignisse:
  {{events}}

  Gib AUSSCHLIESSLICH ein JSON‑Objekt zurück mit dieser Struktur:
  {
    "items": [ { "text": "Kurze umsetzbare Erinnerung", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"einzigartige-id" } ],
    "explain": "{{kurze benutzerorientierte Zusammenfassung}}"
  }

  Anforderungen: nur JSON, items nach priority sortiert, text knapp und, falls möglich, zeitlich kontextualisiert.
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // French templates
      let summaryTemplate_fr = PromptTemplate(
        id: UUID(uuidString: "dc1606dd-5c75-5aec-869c-998b262781bf")!,
        name: "Modèle de résumé",
        language: "fr",
        content: """
  Vous êtes un assistant de synthèse de texte. Recevez un input (descriptions d'événements, paragraphes d'article ou plusieurs textes) et produisez un résumé concis en français.

  Entrée :
  {{events}}

  Retournez uniquement un texte résumé (pas de JSON). Exigences : 1) Résumer les points clés ; 2) Utiliser des paragraphes clairs, max 5 lignes ; 3) Si l'entrée contient des tâches/événements, ajouter une section « Recommandations » avec 1–3 actions réalisables (<=100 caractères chacune).
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let reminderTemplate_fr = PromptTemplate(
        id: UUID(uuidString: "875573be-217f-5d82-b307-aa1545dbc0ad")!,
        name: "Modèle de rappel",
        language: "fr",
        content: """
  Vous êtes un assistant d'agenda. Analysez la liste d'événements ci‑dessous et générez des éléments de rappel structurés utilisables pour les notifications.

  Plage de dates : {{date_range}}
  Événements :
  {{events}}

  Retournez UNIQUEMENT un objet JSON avec la structure :
  {
    "items": [ { "text": "Rappel court et actionnable", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"id-unique" } ],
    "explain": "{{brève synthèse destinée à l'utilisateur}}"
  }

  Exigences : uniquement du JSON, items triés par priorité, texte concis et contextualisé si possible.
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // Spanish templates
      let summaryTemplate_es = PromptTemplate(
        id: UUID(uuidString: "fa563909-5af4-5cc9-b028-6889000c95c8")!,
        name: "Plantilla de resumen",
        language: "es",
        content: """
  Eres un asistente de resumen de texto. Se recibe una entrada (descripciones de eventos, párrafos de un artículo o varios textos) y debes producir un resumen conciso en español.

  Entrada:
  {{events}}

  Devuelve solo texto resumido (no JSON). Requisitos: 1) Capturar puntos clave; 2) Usar párrafos claros, máximo 5 líneas; 3) Si hay tareas/eventos, añadir una sección "Recomendaciones" con 1–3 sugerencias accionables (<=100 caracteres cada una).
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let reminderTemplate_es = PromptTemplate(
        id: UUID(uuidString: "6358c694-7fd7-50d3-9f96-edbecea3728d")!,
        name: "Plantilla de recordatorio",
        language: "es",
        content: """
  Eres un asistente de calendario. Analiza la siguiente lista de eventos y genera elementos de recordatorio estructurados aptos para notificaciones.

  Rango de fechas: {{date_range}}
  Eventos:
  {{events}}

  Devuelve SOLO un objeto JSON con la estructura:
  {
    "items": [ { "text": "Recordatorio corto y accionable", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"id-unico" } ],
    "explain": "{{resumen breve para el usuario}}"
  }

  Requisitos: solo JSON, items ordenados por prioridad, texto conciso y con contexto temporal cuando sea posible.
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // Russian templates
      let summaryTemplate_ru = PromptTemplate(
        id: UUID(uuidString: "8f56b4da-3722-5e7f-be37-e6779da388d9")!,
        name: "Шаблон сводки",
        language: "ru",
        content: """
  Вы — помощник по суммированию текста. Получив вход (описания событий, абзацы статьи или несколько текстов), создайте краткую сводку на русском языке.

  Вход:
  {{events}}

  Верните только текстовую сводку (без JSON). Требования: 1) выделить ключевые моменты; 2) ясные абзацы, не более 5 строк; 3) если во входе есть задачи/события, добавьте раздел «Рекомендации» с 1–3 практическими предложениями (<=100 символов каждое).
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let reminderTemplate_ru = PromptTemplate(
        id: UUID(uuidString: "b09c77c5-5e1f-597c-aca9-17818de63c2e")!,
        name: "Шаблон напоминания",
        language: "ru",
        content: """
  Вы — помощник по расписанию. Проанализируйте список событий и создайте структурированные элементы напоминаний, пригодные для уведомлений.

  Диапазон дат: {{date_range}}
  События:
  {{events}}

  Верните ТОЛЬКО JSON‑объект со структурой:
  {
    "items": [ { "text": "Короткое практическое напоминание", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"уник-id" } ],
    "explain": "{{короткое описание для пользователя}}"
  }

  Требования: только JSON, элементы отсортированы по priority, текст краткий и, при возможности, содержит временной контекст.
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      // Japanese templates
      let summaryTemplate_ja = PromptTemplate(
        id: UUID(uuidString: "e3103ebf-c386-5275-a40b-a5e737e9815e")!,
        name: "要約テンプレート",
        language: "ja",
        content: """
  あなたはテキスト要約アシスタントです。入力（イベント記述、記事の段落、複数のテキスト等）を受け取り、日本語で簡潔な要約を作成してください。

  入力:
  {{events}}

  出力はプレーンテキストの要約のみとし（JSON不可）、要件：1) 主要なポイントをまとめる；2) 明確な段落、最大5行；3) 入力にタスク/イベントが含まれる場合は「推奨」セクションを設け、1〜3件の実行可能な提案（各<=100文字）を記載。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.summary_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      let reminderTemplate_ja = PromptTemplate(
        id: UUID(uuidString: "9ff16045-e796-5593-a295-8a88ee990bbc")!,
        name: "リマインダーテンプレート",
        language: "ja",
        content: """
  あなたはスケジュールアシスタントです。次のイベント一覧を分析し、通知に使える構造化されたリマインダー項目を生成してください。

  日付範囲：{{date_range}}
  イベント：
  {{events}}

  必ず以下のJSON構造のみを返してください：
  {
    "items": [ { "text": "短く実行可能なリマインダー", "priority": 0.0-1.0, "action": {"type":"notify|store|create_task|reschedule","target":"calendar|reminder"}, "dedupeKey":"ユニークID" } ],
    "explain": "{{ユーザー向けの短い要約}}"
  }

  要件：JSONのみ、itemsはpriority順、textは簡潔で可能なら時間文脈を含めること。
  """,
        description: NSLocalizedString("autogen.PromptTemplateService.reminder_description", comment: "AUTOGEN"),
        isBuiltIn: true
      )

      return [
        summaryTemplate, reminderTemplate, birthdayTemplate, taskTemplate,
        summaryTemplate_en, reminderTemplate_en, birthdayTemplate_en, taskTemplate_en,
        summaryTemplate_de, reminderTemplate_de,
        summaryTemplate_fr, reminderTemplate_fr,
        summaryTemplate_es, reminderTemplate_es,
        summaryTemplate_ru, reminderTemplate_ru,
        summaryTemplate_ja, reminderTemplate_ja
      ]
    }
}
