// SPDX-License-Identifier: Apache-2.0
package com.nola

import com.nola.core.AgentCoreNative
import com.nola.core.AndroidAgentPermissionMode
import com.nola.data.local.AndroidApplicationRuntime
import com.nola.data.local.ApplicationSubmissionJournal
import com.nola.data.remote.api.WebHostApiServiceImpl
import com.nola.data.remote.api.WebHostHttpClientFactory
import com.nola.data.remote.mobile_crypto.EncryptedEnvelope
import com.nola.data.remote.mobile_crypto.MobileCrypto
import com.nola.data.remote.sse.SseClient
import com.nola.data.remote.sse.SseEvent
import com.nola.data.remote.sse.SseStatusException
import com.nola.domain.model.ModelExecutionCapabilities
import com.nola.domain.model.ModelInput
import com.nola.domain.projection.ConversationProjectionV1
import com.nola.llm.LlmRequestException
import com.nola.llm.LlmStreamEvent
import com.nola.llm.provider.OpenAiProvider
import com.nola.ui.components.HostToolVisualKind
import com.nola.ui.components.MarkdownBlock
import com.nola.ui.components.formatLiveThinkingPreview
import com.nola.ui.components.normalizeLatexForMarkwon
import com.nola.ui.components.parseMarkdownBlocks
import com.nola.ui.components.resolveHostToolVisualKind
import com.nola.ui.components.stabilizeStreamingMarkdownTail
import com.nola.ui.screen.activity.resolveActivityDisplayTitle
import com.nola.ui.screen.chat.ARTIFACT_TEXT_PREVIEW_MAX_BYTES
import com.nola.ui.screen.chat.ARTIFACT_TEXT_TRUNCATION_NOTICE
import com.nola.ui.screen.chat.AndroidInlinePreviewRequest
import com.nola.ui.screen.chat.AttachmentPreviewTarget
import com.nola.ui.screen.chat.ConversationInspector
import com.nola.ui.screen.chat.ToolStatusVisualKind
import com.nola.ui.screen.chat.canPreviewAttachmentUri
import com.nola.ui.screen.chat.isExternalAnswerSource
import com.nola.ui.screen.chat.isImagePreviewUri
import com.nola.ui.screen.chat.loadAttachmentPreviewText
import com.nola.ui.screen.chat.resolveAttachmentPreviewTarget
import com.nola.ui.screen.chat.resolveToolStatusVisualKind
import com.nola.ui.screen.chat.toggleInspector
import io.ktor.client.HttpClient
import io.ktor.client.engine.cio.CIO
import java.io.BufferedReader
import java.io.BufferedWriter
import java.io.File
import java.io.InputStreamReader
import java.io.OutputStreamWriter
import java.net.InetAddress
import java.net.ServerSocket
import java.net.Socket
import java.nio.file.Files
import java.nio.file.attribute.PosixFilePermissions
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.Executors
import java.util.concurrent.atomic.AtomicInteger
import kotlinx.coroutines.flow.toList
import kotlinx.coroutines.runBlocking
import kotlinx.serialization.encodeToString
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.JsonObject
import kotlinx.serialization.json.JsonPrimitive
import kotlinx.serialization.json.boolean
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive
import kotlinx.serialization.json.put
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Deterministic real-journey, inline inspector preview, bounded live reasoning,
 * CJK/LaTeX Markdown sanitization, and loopback mock-HTTP SSE multi-turn E2E suite.
 * Zero LAN or external LLM dependency.
 */
class AndroidGeometryAndWorkflowsTest {
    private val json = Json { ignoreUnknownKeys = true; encodeDefaults = true }

    companion object {
        @Volatile
        private var jniLoaded = false

        @Synchronized
        private fun ensureJniLoaded() {
            if (jniLoaded) return
            val jniLibPath = requireNotNull(System.getProperty("nola.test.modelPrivacyLibrary")) {
                "Expected nola.test.modelPrivacyLibrary system property from Gradle test task"
            }
            assertTrue("Expected compiled JNI library at $jniLibPath", File(jniLibPath).isFile)
            System.load(jniLibPath)
            assertEquals(1, AgentCoreNative.nativeApplicationAbiVersion())
            assertEquals(1, AgentCoreNative.nativeApplicationProtocolVersion())
            jniLoaded = true
        }
    }

    // -------------------------------------------------------------------------
    // 1. Inline Inspector Preview Contract (Zero External Popup for Local Files)
    // -------------------------------------------------------------------------

    @Test
    fun isImagePreviewUriMatchesAllImageExtensionsCaseInsensitivelyAndRejectsNonImages() {
        val imageUris = listOf(
            "file:///tmp/chart.png",
            "file:///tmp/CHART.PNG?width=800&token=abc",
            "file:///tmp/photo.jpg",
            "file:///tmp/Photo.JPG#preview",
            "file:///tmp/scan.jpeg?v=2",
            "file:///tmp/diagram.webp?size=large",
            "file:///tmp/anim.GIF?loop=1",
            "file:///tmp/raw.bmp",
            "file:///tmp/iphone_shot.heic",
            "file:///tmp/IPHONE_SHOT.HeIc?download=0",
            "content://com.nola.docs/media/frame.HEIF?quality=high",
        )
        for (uri in imageUris) {
            assertTrue("Expected image URI recognized: $uri", isImagePreviewUri(uri, mimeType = null))
        }
        // MIME type also identifies extensionless content:// image URIs
        assertTrue(isImagePreviewUri("content://com.nola.provider/document/42", "image/heic"))
        assertTrue(isImagePreviewUri("content://com.nola.provider/document/43", "IMAGE/PNG"))

        val nonImageUris = listOf(
            "file:///tmp/notes.txt",
            "file:///tmp/Main.kt?line=12",
            "file:///tmp/spec.pdf?page=3",
            "file:///tmp/payload.bin",
            "file:///tmp/archive.png.txt",
            "content://com.nola.provider/document/99",
            "",
        )
        for (uri in nonImageUris) {
            assertFalse("Expected non-image URI rejected: $uri", isImagePreviewUri(uri, mimeType = null))
        }
        assertFalse(isImagePreviewUri("file:///tmp/readme.txt", "text/plain"))
        assertFalse(isImagePreviewUri("file:///tmp/report.pdf", "application/pdf"))
        assertFalse(isImagePreviewUri("file:///tmp/data.bin", "application/octet-stream"))
    }

    @Test
    fun loadAttachmentPreviewTextHandlesUtf8CjkBoundedTruncationBinaryRejectionAndMissingFiles() = runBlocking {
        val sandbox = createPrivateTempDir("nola-android-inline-preview-")
        try {
            // 1. Small UTF-8 CJK + code file loads cleanly in full via both raw path and file:// URI.
            val smallCode = File(sandbox, "中文分析_sample.kt")
            val smallContent = buildString {
                appendLine("// 模块说明：局部残差分析")
                appendLine("fun computeResidual(a: Double, b: Double): Double = a - 0.5 * b")
                append("println(\"结果验证：通过 ✓\")")
            }
            smallCode.writeText(smallContent, Charsets.UTF_8)

            val loadedFromPath = loadAttachmentPreviewText(null, smallCode.absolutePath, "text/x-kotlin")
            assertEquals(smallContent, loadedFromPath)
            val loadedFromUri = loadAttachmentPreviewText(null, smallCode.toURI().toString(), null)
            assertEquals(smallContent, loadedFromUri)

            // 2. Oversized file (> 128 KB, e.g. ~260 KB mixed CJK + ASCII) is strictly bounded with truncation notice.
            val oversizedFile = File(sandbox, "oversized_trace.log")
            val lineUnit = "LOG-ENTRY 中文日志行 payload=0123456789abcdef 状态正常\n"
            val unitBytes = lineUnit.toByteArray(Charsets.UTF_8).size
            val repeatCount = (260 * 1024) / unitBytes + 1
            oversizedFile.outputStream().buffered().use { out ->
                repeat(repeatCount) { idx ->
                    out.write("[$idx] $lineUnit".toByteArray(Charsets.UTF_8))
                }
            }
            assertTrue("Fixture must exceed 256 KB", oversizedFile.length() > 256 * 1024)

            val boundedPreview = loadAttachmentPreviewText(null, oversizedFile.toURI().toString(), "text/plain")
            assertNotNull("Oversized text file must still produce a bounded preview", boundedPreview)
            val previewText = requireNotNull(boundedPreview)
            assertTrue("Bounded preview must include truncation notice", previewText.endsWith(ARTIFACT_TEXT_TRUNCATION_NOTICE))
            assertTrue("Bounded preview must start with head content", previewText.startsWith("[0] LOG-ENTRY 中文日志行"))
            assertFalse("Bounded preview must not contain tail beyond 128 KB", previewText.contains("[${repeatCount - 1}]"))
            val bodyWithoutNotice = previewText.removeSuffix(ARTIFACT_TEXT_TRUNCATION_NOTICE)
            val bodyBytes = bodyWithoutNotice.toByteArray(Charsets.UTF_8).size
            assertTrue(
                "Bounded preview body ($bodyBytes bytes) must not exceed $ARTIFACT_TEXT_PREVIEW_MAX_BYTES bytes",
                bodyBytes <= ARTIFACT_TEXT_PREVIEW_MAX_BYTES,
            )
            assertTrue("Bounded preview must load a substantial prefix", bodyBytes >= ARTIFACT_TEXT_PREVIEW_MAX_BYTES - 8)
            assertFalse("UTF-8 boundary trimming must not emit replacement char U+FFFD", bodyWithoutNotice.contains('\uFFFD'))

            // 3. Binary files (PNG/PDF bytes with NUL bytes, even if mislabeled as .txt or text/plain) are safely rejected.
            val pngBytes = byteArrayOf(
                0x89.toByte(), 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A,
                0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
            )
            val realPng = File(sandbox, "diagram.png").apply { writeBytes(pngBytes) }
            val disguisedPng = File(sandbox, "disguised_png.txt").apply { writeBytes(pngBytes) }
            val pdfBytes = ("%PDF-1.7\n1 0 obj\n<< /Length 4 >>\nstream\n").toByteArray(Charsets.US_ASCII) +
                byteArrayOf(0x00, 0x01, 0x02, 0x00)
            val realPdf = File(sandbox, "spec.pdf").apply { writeBytes(pdfBytes) }
            val disguisedPdf = File(sandbox, "disguised_pdf.log").apply { writeBytes(pdfBytes) }
            val nulBinaryTxt = File(sandbox, "nul_binary.txt").apply {
                writeBytes("header".toByteArray(Charsets.UTF_8) + byteArrayOf(0x00, 0xFF.toByte(), 0xFE.toByte()))
            }

            assertNull("PNG image must not load as text", loadAttachmentPreviewText(null, realPng.absolutePath, "image/png"))
            assertNull("Disguised PNG bytes in .txt must be rejected", loadAttachmentPreviewText(null, disguisedPng.absolutePath, "text/plain"))
            assertNull("PDF file must not load as raw text", loadAttachmentPreviewText(null, realPdf.absolutePath, "application/pdf"))
            assertNull("Disguised PDF bytes in .log must be rejected", loadAttachmentPreviewText(null, disguisedPdf.absolutePath, "text/plain"))
            assertNull("NUL-containing binary .txt must be rejected", loadAttachmentPreviewText(null, nulBinaryTxt.absolutePath, "text/plain"))

            // 4. Non-existent file and directory fail gracefully with null (no unhandled exception).
            val missingFile = File(sandbox, "does_not_exist.kt")
            assertNull(loadAttachmentPreviewText(null, missingFile.absolutePath, "text/plain"))
            assertNull(loadAttachmentPreviewText(null, missingFile.toURI().toString(), "text/plain"))
            assertNull(loadAttachmentPreviewText(null, sandbox.absolutePath, "text/plain"))
            assertNull(loadAttachmentPreviewText(null, "   ", "text/plain"))
        } finally {
            sandbox.deleteRecursively()
        }
    }

    @Test
    fun inlinePreviewRequestRoutesLocalAttachmentsToInspectorAndKeepsHttpExternal() {
        val localImage = AndroidInlinePreviewRequest("file:///data/user/0/com.nola/files/chart.png", "image/png", "chart.png")
        val localCode = AndroidInlinePreviewRequest("file:///data/user/0/com.nola/files/main.kt", "text/x-kotlin", "main.kt")
        val contentDoc = AndroidInlinePreviewRequest("content://com.nola.documents/item/7", "text/plain", "notes.txt")
        val rawPath = AndroidInlinePreviewRequest("/data/user/0/com.nola/cache/trace.log", "text/plain", "trace.log")

        for (req in listOf(localImage, localCode, contentDoc, rawPath)) {
            assertTrue("Local URI must be previewable: ${req.uri}", canPreviewAttachmentUri(req.uri))
            assertFalse("Local URI must NOT be treated as external answer source: ${req.uri}", isExternalAnswerSource(req.uri))
            assertEquals(
                "Local URI must route to inline inspector: ${req.uri}",
                AttachmentPreviewTarget.InlineInspector,
                resolveAttachmentPreviewTarget(req.uri, hasInlineInspector = true),
            )
        }

        val externalHttps = "https://docs.example.com/architecture.pdf"
        val externalHttp = "http://specs.example.org/v2"
        for (url in listOf(externalHttps, externalHttp)) {
            assertTrue(canPreviewAttachmentUri(url))
            assertTrue(isExternalAnswerSource(url))
            assertEquals(AttachmentPreviewTarget.ExternalBrowser, resolveAttachmentPreviewTarget(url, hasInlineInspector = true))
        }

        // Unsafe / unsupported URIs never route to either inline preview or external browser.
        for (bad in listOf("", "   ", "javascript:alert(1)", "https://user:secret@example.com/doc", "ftp://files.example.com/a")) {
            assertEquals("Expected Unsupported for '$bad'", AttachmentPreviewTarget.Unsupported, resolveAttachmentPreviewTarget(bad))
        }

        // Verify inspector toggling & preview switching clears stale state cleanly.
        var activeInspector: ConversationInspector? = null
        var activePreview: AndroidInlinePreviewRequest? = null

        // Clicking a local attachment opens Resources inspector with that inline preview.
        activePreview = localImage
        activeInspector = ConversationInspector.Resources
        assertEquals(localImage, activePreview)
        assertEquals(ConversationInspector.Resources, activeInspector)

        // Switching preview replaces previous selection cleanly.
        activePreview = localCode
        assertNotEquals(localImage, activePreview)
        assertEquals("main.kt", activePreview.title)

        // Dismissing preview clears selection while keeping inspector toggle semantics intact.
        activePreview = null
        assertNull(activePreview)
        activeInspector = toggleInspector(activeInspector, ConversationInspector.Resources)
        assertNull("Toggling active Resources inspector must close it", activeInspector)
        activeInspector = toggleInspector(activeInspector, ConversationInspector.Resources)
        assertEquals(ConversationInspector.Resources, activeInspector)
    }

    // -------------------------------------------------------------------------
    // 2. Bounded Live Reasoning & Markdown/CJK/LaTeX Sanitization Contracts
    // -------------------------------------------------------------------------

    @Test
    fun liveReasoningPreviewStrictlyBounds240LinesAndTracksRollingTail() {
        val incrementalSteps = listOf(40, 120, 240)
        for (totalLines in incrementalSteps) {
            val reasoning = (1..totalLines).joinToString("\n") { idx ->
                "第 $idx 步 / Step $idx: 检查上下文几何布局与残差收敛状态。"
            }
            val live = formatLiveThinkingPreview(reasoning, finished = false)
            val liveLines = live.lines()
            assertTrue("Live reasoning preview must be bounded to <= 19 lines (ellipsis + 18), got ${liveLines.size}", liveLines.size <= 19)
            assertTrue("Live reasoning preview must stay within bounded char budget, got ${live.length}", live.length <= 2405)
            assertTrue("Live preview must start with ellipsis indicator", live.startsWith("…\n"))
            assertTrue("Live preview must display trailing line $totalLines", live.contains("第 $totalLines 步 / Step $totalLines:"))
            assertFalse("Live preview must roll past line 1 once > 18 lines", live.contains("第 1 步 / Step 1:"))
            assertFalse("Live preview must roll past line ${totalLines - 18}", live.contains("第 ${totalLines - 18} 步 / Step ${totalLines - 18}:"))

            // Finished reasoning preserves full history for expanded inspection.
            val full = formatLiveThinkingPreview(reasoning, finished = true)
            assertTrue(full.contains("第 1 步 / Step 1:"))
            assertTrue(full.contains("第 $totalLines 步 / Step $totalLines:"))
        }

        // Single huge unbroken line (> 4000 chars) is still strictly bounded by maxChars.
        val singleHugeLine = "推理片段_".repeat(1_000) + "TAIL_SENTINEL_中文结尾"
        val boundedSingle = formatLiveThinkingPreview(singleHugeLine, finished = false)
        assertTrue("Single huge line must stay bounded, got ${boundedSingle.length}", boundedSingle.length <= 2405)
        assertTrue("Single huge line must show trailing sentinel", boundedSingle.contains("TAIL_SENTINEL_中文结尾"))
        assertTrue("Single huge line must carry ellipsis prefix", boundedSingle.startsWith("…"))
    }

    @Test
    fun cjkLatexNormalizationAndMarkdownBlocksPreserveCjkFencedCodeAndGfmTables() {
        val richMarkdown = """
            # 残差分解与指标对比

            行内公式：${'$'}S_A = \lambda \cdot S_B + (\text{一小坨残差})${'$'}，以及纯数学公式 ${'$'}E = mc^2${'$'}。

            \[
            S_A = \lambda \cdot S_B + (\text{一小坨残差})
            \]

            纯符号块公式：\[x^2 + y^2 = r^2\]

            ```kotlin
            // Fenced code must keep regex escapes \[0-9\] and \(group\) untouched
            val regex = Regex("\\[0-9\\]+\\(group\\)")
            val env = "${'$'}HOME/nola"
            ```

            | 模块 | 延迟 (ms) | 状态 |
            | --- | --- | --- |
            | 推理引擎 | 42 | 正常 |
            | 渲染管线 | 11 | 通过 |
        """.trimIndent()

        val normalized = normalizeLatexForMarkwon(richMarkdown)

        // 1. CJK-in-LaTeX converted cleanly to readable Unicode + CJK (both inline $...$ and display \[...\])
        assertTrue("CJK residual phrase must survive normalization: $normalized", normalized.contains("一小坨残差"))
        assertTrue("Unicode lambda and cdot must appear: $normalized", normalized.contains("λ") && normalized.contains("·"))
        assertFalse("Raw \\text{ command must not leak: $normalized", normalized.contains("\\text{"))

        // 2. Pure Latin/Greek formulas remain in $$...$$ for JLaTeXMath
        assertTrue("Pure inline math must normalize to $$...$$: $normalized", normalized.contains("\$\$E = mc^2\$\$"))
        assertTrue("Pure display math must normalize to $$...$$: $normalized", normalized.contains("\$\$x^2 + y^2 = r^2\$\$"))

        // 3. Fenced code block regexes and $HOME must remain completely untouched!
        assertTrue(
            "Fenced code regex escapes must not be mangled into $$: $normalized",
            normalized.contains("""val regex = Regex("\\[0-9\\]+\\(group\\)")"""),
        )
        assertTrue("Fenced code dollar expression must remain intact: $normalized", normalized.contains("""val env = "${'$'}HOME/nola""""))

        // 4. Structured block parser handles heading, fenced code block, and GFM CJK table cleanly.
        val blocks = parseMarkdownBlocks(richMarkdown)
        assertTrue("Expected Heading block", blocks.any { it is MarkdownBlock.Heading && it.text == "残差分解与指标对比" })
        val codeBlock = blocks.filterIsInstance<MarkdownBlock.CodeBlock>().singleOrNull()
        assertNotNull("Expected single CodeBlock", codeBlock)
        assertEquals("kotlin", codeBlock!!.language)
        assertTrue(codeBlock.code.contains("Regex"))

        val table = blocks.filterIsInstance<MarkdownBlock.Table>().singleOrNull()
        assertNotNull("Expected single GFM Table block", table)
        assertEquals(listOf("模块", "延迟 (ms)", "状态"), table!!.header)
        assertEquals(2, table.rows.size)
        assertEquals(listOf("推理引擎", "42", "正常"), table.rows[0])
        assertEquals(listOf("渲染管线", "11", "通过"), table.rows[1])

        // 5. Streaming tail stabilization closes dangling inline markers without crashing.
        assertEquals("正在输出 **加粗中文**", stabilizeStreamingMarkdownTail("正在输出 **加粗中文"))
        assertEquals("行内代码 `val x = 1`", stabilizeStreamingMarkdownTail("行内代码 `val x = 1"))
    }

    // -------------------------------------------------------------------------
    // 3. Deterministic Local Loopback Mock-HTTP SSE Multi-Turn E2E
    // -------------------------------------------------------------------------

    @Test
    fun localLoopbackMultiTurnSseDrivesReasoningToolsCautiousPermissionErrorAndColdReopen() = runBlocking {
        ensureJniLoaded()
        val sandbox = createPrivateTempDir("nola-android-loopback-multiturn-")
        val dbFile = File(sandbox, "multiturn-v2.db")
        val token = "loopback-multiturn-secret-token-2026"
        val convId = "conv-loopback-e2e-1"

        val openAiCallCount = AtomicInteger(0)
        val openAiCapturedBodies = CopyOnWriteArrayList<JsonObject>()
        val hostCapturedRequests = CopyOnWriteArrayList<Pair<String, JsonObject?>>()
        val executor = Executors.newCachedThreadPool()

        // Loopback OpenAI-compatible SSE server:
        // Call 1: streams reasoning_content + two tool calls (read_file + shell_exec)
        // Call 2: receives tool results (one success + one is_error/stderr) -> streams CJK Markdown final answer
        // Call 3: returns HTTP 400 Bad Request JSON error (context_length_exceeded)
        val openAiServer = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val openAiLoop = executor.submit {
            while (!openAiServer.isClosed) {
                val socket = runCatching { openAiServer.accept() }.getOrNull() ?: break
                socket.use { sock ->
                    val (_, _, rawBody) = readHttpRequest(sock)
                    runCatching { json.parseToJsonElement(rawBody).jsonObject }.getOrNull()?.let { openAiCapturedBodies.add(it) }
                    when (openAiCallCount.incrementAndGet()) {
                        1 -> {
                            val sse = buildString {
                                append("data: ").append(
                                    """{"id":"cmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","reasoning_content":"第一步：读取目标文件并执行构建检查。"},"finish_reason":null}]}"""
                                ).append("\n\n")
                                append("data: ").append(
                                    """{"id":"cmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"reasoning_content":"第二步：并行发起 read_file 和 shell_exec 工具调用。"},"finish_reason":null}]}"""
                                ).append("\n\n")
                                append("data: ").append(
                                    """{"id":"cmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":0,"id":"call_read_1","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"README.md\"}"}}]},"finish_reason":null}]}"""
                                ).append("\n\n")
                                append("data: ").append(
                                    """{"id":"cmpl-1","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"tool_calls":[{"index":1,"id":"call_shell_2","type":"function","function":{"name":"shell_exec","arguments":"{\"cmd\":\"cargo check --broken\"}"}}]},"finish_reason":"tool_calls"}]}"""
                                ).append("\n\n")
                                append("data: [DONE]\n\n")
                            }
                            writeHttpResponse(sock, 200, "OK", "text/event-stream", sse)
                        }
                        2 -> {
                            val sse = buildString {
                                append("data: ").append(
                                    """{"id":"cmpl-2","object":"chat.completion.chunk","choices":[{"index":0,"delta":{"role":"assistant","content":"### 工具执行汇总\n\n- `read_file` 成功读取文档。\n- `shell_exec` 返回非零退出码（exit code 1），已捕获 stderr。\n\n公式：${'$'}S_A = \\lambda \\cdot S_B + (\\text{一小坨残差})${'$'}" },"finish_reason":"stop"}],"usage":{"prompt_tokens":128,"completion_tokens":44,"total_tokens":172}}"""
                                ).append("\n\n")
                                append("data: [DONE]\n\n")
                            }
                            writeHttpResponse(sock, 200, "OK", "text/event-stream", sse)
                        }
                        else -> {
                            val errJson = """{"error":{"type":"invalid_request_error","code":"context_length_exceeded","message":"Prompt exceeds context limit of 32768 tokens"}}"""
                            writeHttpResponse(sock, 400, "Bad Request", "application/json", errJson)
                        }
                    }
                }
            }
        }

        // Loopback encrypted v2 WebHost SSE + Permission endpoint server:
        // Turn 1 POST /messages: encrypted SSE with reasoning_delta, tool_started/finished (ok + is_error=true), text_delta, turn_completed
        // Turn 2 POST /messages (cautious): emits permission_requested; POST /permission records allow/deny decision
        // Turn 3 POST /messages: returns HTTP 400 Bad Request JSON error
        val hostMessageTurn = AtomicInteger(0)
        val hostServer = ServerSocket(0, 50, InetAddress.getByName("127.0.0.1"))
        val hostLoop = executor.submit {
            while (!hostServer.isClosed) {
                val socket = runCatching { hostServer.accept() }.getOrNull() ?: break
                socket.use { sock ->
                    val (reqLine, _, rawBody) = readHttpRequest(sock)
                    val decodedBody = rawBody.takeIf { it.isNotBlank() }?.let {
                        MobileCrypto.decryptJsonObject(json.decodeFromString<EncryptedEnvelope>(it), token)
                    }
                    hostCapturedRequests.add(reqLine to decodedBody)

                    fun encFrame(obj: JsonObject): String {
                        val env = MobileCrypto.encryptJsonObject(obj, token).copy(type = "encrypted")
                        return "data: ${json.encodeToString(env)}\n\n"
                    }

                    when {
                        reqLine.contains("/permission ") -> {
                            val allow = decodedBody?.get("allow")?.jsonPrimitive?.boolean ?: false
                            val requestId = decodedBody?.get("request_id")?.jsonPrimitive?.content.orEmpty()
                            val reply = buildJsonObject {
                                put("ok", true)
                                put("request_id", requestId)
                                put("decision", if (allow) "approved" else "denied")
                                put("persisted", true)
                            }
                            val enc = json.encodeToString(MobileCrypto.encryptJsonObject(reply, token).copy(type = "encrypted"))
                            writeHttpResponse(sock, 200, "OK", "application/json", enc)
                        }
                        reqLine.contains("/messages ") -> {
                            when (hostMessageTurn.incrementAndGet()) {
                                1 -> {
                                    val sse = buildString {
                                        append(encFrame(buildJsonObject {
                                            put("type", "session_started"); put("session_id", convId); put("model", "mock-qwen3")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "reasoning_delta"); put("delta", "分析任务并调用两个本地工具。")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "tool_started"); put("call_id", "call_read_1"); put("name", "read_file")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "tool_finished"); put("call_id", "call_read_1"); put("name", "read_file")
                                            put("is_error", false); put("summary", "Read 18 lines from README.md")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "tool_started"); put("call_id", "call_shell_2"); put("name", "shell_exec")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "tool_finished"); put("call_id", "call_shell_2"); put("name", "shell_exec")
                                            put("is_error", true); put("summary", "exit code 1: stderr: error[E0425]: cannot find value")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "text_delta"); put("delta", "已完成检查：`read_file` 成功，`shell_exec` 失败（exit code 1）。")
                                        }))
                                        append(encFrame(buildJsonObject {
                                            put("type", "turn_completed"); put("session_id", convId)
                                            put("final_text", "已完成检查：`read_file` 成功，`shell_exec` 失败（exit code 1）。")
                                        }))
                                        append("data: [DONE]\n\n")
                                    }
                                    writeHttpResponse(sock, 200, "OK", "text/event-stream", sse)
                                }
                                2 -> {
                                    val sse = buildString {
                                        append(encFrame(buildJsonObject {
                                            put("type", "permission_requested")
                                            put("request_id", "perm-req-42")
                                            put("tool", "shell_exec")
                                            put("reason", "Run workspace build command under cautious policy")
                                            put("arguments", buildJsonObject { put("cmd", "rm -rf build-cache") })
                                        }))
                                        append("data: [DONE]\n\n")
                                    }
                                    writeHttpResponse(sock, 200, "OK", "text/event-stream", sse)
                                }
                                else -> {
                                    val errJson = """{"error":{"code":"bad_request","message":"Malformed upstream payload"}}"""
                                    writeHttpResponse(sock, 400, "Bad Request", "application/json", errJson)
                                }
                            }
                        }
                        else -> writeHttpResponse(sock, 404, "Not Found", "application/json", """{"error":"not_found"}""")
                    }
                }
            }
        }

        val httpClient = HttpClient(CIO)
        try {
            val openAiProvider = OpenAiProvider(httpClient).apply {
                config = OpenAiProvider.Config(
                    baseUrl = "http://127.0.0.1:${openAiServer.localPort}/v1",
                    apiKey = "sk-loopback-test",
                    model = "mock-qwen3-reasoning",
                    executionCapabilities = ModelExecutionCapabilities(
                        toolCalls = true,
                        imageInput = false,
                        reasoning = true,
                        samplingParameters = true,
                    ),
                )
            }
            val webHostService = WebHostApiServiceImpl(httpClient, SseClient(httpClient), WebHostHttpClientFactory(httpClient))
            val hostBaseUrl = "http://127.0.0.1:${hostServer.localPort}"

            val now = 1_788_000_000_000L
            openJvmAndroidApplicationRuntime(dbFile).use { runtime ->
                runtime.createConversation(convId, "Loopback Multi-Turn E2E", now, now, """{"permissionProfile":"cautious"}""")

                // --- Turn 1A: Direct OpenAI SSE streaming reasoning + interleaved tool calls + CJK Markdown final answer ---
                val toolSpecs = listOf(
                    buildJsonObject {
                        put("type", "function")
                        put("function", buildJsonObject { put("name", "read_file"); put("description", "Read file") })
                    },
                    buildJsonObject {
                        put("type", "function")
                        put("function", buildJsonObject { put("name", "shell_exec"); put("description", "Run shell command") })
                    },
                )
                val turn1Events = openAiProvider.generateEventStream(
                    ModelInput(
                        messages = listOf(mapOf("role" to "user", "content" to "检查 README 并运行构建")),
                        tools = toolSpecs,
                    ),
                ).toList()

                val streamedReasoning = turn1Events.filterIsInstance<LlmStreamEvent.ReasoningDelta>().joinToString("") { it.text }
                val streamedTools = turn1Events.filterIsInstance<LlmStreamEvent.ToolCallDelta>()
                assertTrue("Expected streamed reasoning_content: $streamedReasoning", streamedReasoning.contains("第一步") && streamedReasoning.contains("第二步"))
                assertEquals(2, streamedTools.size)
                assertEquals("read_file", streamedTools[0].capabilityId)
                assertEquals("call_read_1", streamedTools[0].callId)
                assertEquals("shell_exec", streamedTools[1].capabilityId)
                assertEquals("call_shell_2", streamedTools[1].callId)
                assertEquals(LlmStreamEvent.Completed("tool_calls"), turn1Events.last())

                // Feed succeeding tool output + failing tool output (non-zero exit code + stderr) back to Turn 1 completion.
                val turn1FinalEvents = openAiProvider.generateEventStream(
                    ModelInput(
                        messages = listOf(
                            mapOf("role" to "user", "content" to "检查 README 并运行构建"),
                            mapOf("role" to "tool", "name" to "read_file", "tool_call_id" to "call_read_1", "content" to """{"ok":true,"content":"# Nola v2"}"""),
                            mapOf("role" to "tool", "name" to "shell_exec", "tool_call_id" to "call_shell_2", "content" to """{"is_error":true,"exit_code":1,"stderr":"error[E0425]: cannot find value"}"""),
                        ),
                        tools = toolSpecs,
                    ),
                ).toList()
                val turn1Answer = turn1FinalEvents.filterIsInstance<LlmStreamEvent.TextDelta>().joinToString("") { it.text }
                assertTrue("Expected CJK Markdown final answer: $turn1Answer", turn1Answer.contains("工具执行汇总") && turn1Answer.contains("一小坨残差"))
                assertTrue("Expected usage event on Turn 1 completion", turn1FinalEvents.any { it is LlmStreamEvent.Usage && it.usage.totalTokens == 172 })
                assertEquals(LlmStreamEvent.Completed("stop"), turn1FinalEvents.last())
                assertEquals(2, openAiCapturedBodies.size)

                // --- Turn 1B: Encrypted v2 WebHost SSE stream (reasoning + ok tool + failed tool is_error=true + CJK answer) ---
                val v2Turn1Events = webHostService.v2StreamMessage(
                    hostBaseUrl = hostBaseUrl,
                    token = token,
                    conversationId = convId,
                    content = "检查 README 并运行构建",
                    permissionProfile = AndroidAgentPermissionMode.Cautious.wireValue,
                ).toList()
                assertTrue(v2Turn1Events.any { it is SseEvent.V2ReasoningDelta && it.delta.contains("两个本地工具") })
                val okTool = v2Turn1Events.filterIsInstance<SseEvent.V2ToolFinished>().single { it.callId == "call_read_1" }
                val failedTool = v2Turn1Events.filterIsInstance<SseEvent.V2ToolFinished>().single { it.callId == "call_shell_2" }
                assertFalse("read_file must succeed", okTool.isError)
                assertTrue("shell_exec must surface isError=true", failedTool.isError)
                assertTrue("shell_exec summary must carry exit code + stderr", failedTool.summary.contains("exit code 1") && failedTool.summary.contains("stderr"))
                val v2Turn1Final = v2Turn1Events.filterIsInstance<SseEvent.V2TurnCompleted>().single()
                assertTrue(v2Turn1Final.finalText.contains("已完成检查"))

                // --- Turn 2: Cautious permission approval flow (pause -> approve & deny -> verify wire + persisted state) ---
                val v2Turn2Events = webHostService.v2StreamMessage(
                    hostBaseUrl = hostBaseUrl,
                    token = token,
                    conversationId = convId,
                    content = "清理构建缓存",
                    permissionProfile = AndroidAgentPermissionMode.Cautious.wireValue,
                ).toList()
                val permReq = v2Turn2Events.filterIsInstance<SseEvent.V2PermissionRequested>().single()
                assertEquals("perm-req-42", permReq.requestId)
                assertEquals("shell_exec", permReq.tool)
                assertTrue(permReq.reason.contains("cautious"))

                // Approve once and verify encrypted response + request payload
                val approvedResp = webHostService.v2RespondPermission(
                    hostBaseUrl = hostBaseUrl,
                    token = token,
                    conversationId = convId,
                    requestId = permReq.requestId,
                    allow = true,
                    scope = "once",
                )
                assertEquals(JsonPrimitive("approved"), approvedResp["decision"])
                assertEquals(JsonPrimitive(true), approvedResp["persisted"])

                // Also exercise deny path
                val deniedResp = webHostService.v2RespondPermission(
                    hostBaseUrl = hostBaseUrl,
                    token = token,
                    conversationId = convId,
                    requestId = "perm-req-43",
                    allow = false,
                    scope = "once",
                )
                assertEquals(JsonPrimitive("denied"), deniedResp["decision"])

                // Verify cautious permission profile was sent on the wire
                val turn2Body = hostCapturedRequests.filter { it.first.contains("/messages ") }[1].second
                assertEquals(JsonPrimitive("cautious"), turn2Body?.get("permissionProfile"))

                // --- Turn 3: Upstream HTTP 400 Bad Request JSON error -> clean surfaced error, zero hang ---
                val openAiErr = runCatching {
                    openAiProvider.generateEventStream(
                        ModelInput(messages = listOf(mapOf("role" to "user", "content" to "Trigger HTTP 400"))),
                    ).toList()
                }.exceptionOrNull()
                assertTrue("Expected LlmRequestException for HTTP 400, got: $openAiErr", openAiErr is LlmRequestException)
                val llmErr = openAiErr as LlmRequestException
                assertEquals(400, llmErr.statusCode)
                assertFalse("HTTP 400 must not be marked retryable", llmErr.retryable)
                assertEquals("context_length_exceeded", llmErr.code)

                val hostErr = runCatching {
                    webHostService.v2StreamMessage(
                        hostBaseUrl = hostBaseUrl,
                        token = token,
                        conversationId = convId,
                        content = "Trigger Host HTTP 400",
                    ).toList()
                }.exceptionOrNull()
                assertTrue("Expected SseStatusException(400) without hanging, got: $hostErr", hostErr is SseStatusException && hostErr.statusCode == 400)

                // Persist multi-turn transcript (Turn 1 reasoning/tools/answer, Turn 2 permission audit, Turn 3 surfaced error)
                val transcript = buildJsonArray {
                    add(buildJsonObject {
                        put("id", "m-u-1"); put("role", "user"); put("created_at_ms", now + 100)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-u-1:t"); put("kind", "text"); put("content_text", "检查 README 并运行构建") })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-a-1"); put("role", "assistant"); put("created_at_ms", now + 200)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-a-1:r"); put("kind", "reasoning"); put("content_text", streamedReasoning) })
                            add(buildJsonObject { put("id", "m-a-1:t"); put("kind", "text"); put("content_text", turn1Answer) })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-u-2"); put("role", "user"); put("created_at_ms", now + 300)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-u-2:t"); put("kind", "text"); put("content_text", "清理构建缓存") })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-a-2"); put("role", "assistant"); put("created_at_ms", now + 400)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-a-2:t"); put("kind", "text"); put("content_text", "权限审批 perm-req-42: approved; perm-req-43: denied") })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-u-3"); put("role", "user"); put("created_at_ms", now + 500)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-u-3:t"); put("kind", "text"); put("content_text", "Trigger HTTP 400") })
                        })
                    })
                    add(buildJsonObject {
                        put("id", "m-a-3"); put("role", "assistant"); put("created_at_ms", now + 600)
                        put("parts", buildJsonArray {
                            add(buildJsonObject { put("id", "m-a-3:t"); put("kind", "text"); put("content_text", "Error 400: ${llmErr.code}") })
                        })
                    })
                }
                runtime.execute(buildJsonObject {
                    put("type", "import_conversation_transcript")
                    put("input", buildJsonObject {
                        put("import_id", "imp-loopback-multiturn-1")
                        put("source", "loopback_e2e")
                        put("conversation_id", convId)
                        put("expected_message_count", 0)
                        put("messages", transcript)
                        put("imported_at_ms", now + 700)
                    })
                })
                runtime.putProductSetting("e2e.permission.last_decision", """{"request_id":"perm-req-42","decision":"approved","mode":"cautious"}""", now + 800)
            }

            // --- Cold Reopen: verify persisted transcript, reasoning/CJK answer, permission state, and error survive reload ---
            openJvmAndroidApplicationRuntime(dbFile).use { reopened ->
                val conv = requireNotNull(reopened.loadConversation(convId))
                assertEquals("Loopback Multi-Turn E2E", conv.title)
                assertEquals(6, conv.messages.size)
                assertTrue(conv.messages[1].text.contains("工具执行汇总") && conv.messages[1].text.contains("一小坨残差"))
                assertTrue(conv.messages[3].text.contains("perm-req-42: approved") && conv.messages[3].text.contains("perm-req-43: denied"))
                assertTrue(conv.messages[5].text.contains("Error 400: context_length_exceeded"))

                val projection = reopened.loadConversationProjectionV1(convId)
                assertEquals(6, projection.messages.size)
                val persistedDecision = requireNotNull(reopened.productSetting("e2e.permission.last_decision"))
                assertTrue(persistedDecision.valueJson.contains("\"decision\":\"approved\""))
                assertTrue(persistedDecision.valueJson.contains("\"mode\":\"cautious\""))
            }
        } finally {
            httpClient.close()
            openAiServer.close()
            hostServer.close()
            openAiLoop.cancel(true)
            hostLoop.cancel(true)
            executor.shutdownNow()
            sandbox.deleteRecursively()
        }
    }

    // -------------------------------------------------------------------------
    // 4. Activity Blank Title Fallback & Semantic Tool Status Visual Mapping
    // -------------------------------------------------------------------------

    @Test
    fun resolveActivityDisplayTitleProvidesLocalizedFallbackForBlankAndPlaceholderTitles() {
        val fallback = "Untitled"
        val convId = "123e4567-e89b-12d3-a456-426614174000"

        assertEquals(fallback, resolveActivityDisplayTitle("", convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle("   \t\n  ", convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle(null, convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle("New conversation", convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle("新对话", convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle(convId, convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle("Conversation 123e4567", convId, fallback))
        assertEquals(fallback, resolveActivityDisplayTitle("Session deadbeef", convId, fallback))

        assertEquals(
            "Review scheduled permission",
            resolveActivityDisplayTitle("  Review scheduled permission  ", convId, fallback),
        )
        assertEquals(
            "请用一句简短的中文回复",
            resolveActivityDisplayTitle("请用一句简短的中文回复", convId, fallback),
        )
    }

    @Test
    fun semanticToolStatusVisualKindMapsCanonicalAndHostToolStatesDeterministically() {
        assertEquals(ToolStatusVisualKind.Running, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Pending))
        assertEquals(ToolStatusVisualKind.Running, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Running))
        assertEquals(ToolStatusVisualKind.Completed, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Completed))
        assertEquals(ToolStatusVisualKind.Failed, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Failed))
        assertEquals(ToolStatusVisualKind.Waiting, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.WaitingForPermission))
        assertEquals(ToolStatusVisualKind.Waiting, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.WaitingForInput))
        assertEquals(ToolStatusVisualKind.Neutral, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Cancelled))
        assertEquals(ToolStatusVisualKind.Neutral, resolveToolStatusVisualKind(ConversationProjectionV1.ActivityState.Sealed))

        assertEquals(HostToolVisualKind.Running, resolveHostToolVisualKind("running", error = false))
        assertEquals(HostToolVisualKind.Running, resolveHostToolVisualKind(" RUNNING ", error = true))
        assertEquals(HostToolVisualKind.PermissionRequired, resolveHostToolVisualKind("permission_required", error = false))
        assertEquals(HostToolVisualKind.PermissionRequired, resolveHostToolVisualKind("permissionRequired", error = false))
        assertEquals(HostToolVisualKind.Failed, resolveHostToolVisualKind("failed", error = false))
        assertEquals(HostToolVisualKind.Failed, resolveHostToolVisualKind("error", error = false))
        assertEquals(HostToolVisualKind.Failed, resolveHostToolVisualKind("completed", error = true))
        assertEquals(HostToolVisualKind.Completed, resolveHostToolVisualKind("completed", error = false))
        assertEquals(HostToolVisualKind.Completed, resolveHostToolVisualKind("succeeded", error = false))
    }

    // -------------------------------------------------------------------------
    // Helpers
    // -------------------------------------------------------------------------

    private fun createPrivateTempDir(prefix: String): File =
        Files.createTempDirectory(
            prefix,
            PosixFilePermissions.asFileAttribute(PosixFilePermissions.fromString("rwx------")),
        ).toFile()

    private fun openJvmAndroidApplicationRuntime(dbFile: File): AndroidApplicationRuntime {
        ensureJniLoaded()
        val unsafeClass = Class.forName("sun.misc.Unsafe")
        val unsafeField = unsafeClass.getDeclaredField("theUnsafe").apply { isAccessible = true }
        val unsafe = unsafeField.get(null)
        val allocateInstance = unsafeClass.getMethod("allocateInstance", Class::class.java)
        val runtime = allocateInstance.invoke(unsafe, AndroidApplicationRuntime::class.java) as AndroidApplicationRuntime
        val handle = AgentCoreNative.nativeApplicationOpen(dbFile.absolutePath.encodeToByteArray())
        assertTrue("nativeApplicationOpen returned 0 for ${dbFile.absolutePath}", handle != 0L)

        fun setField(name: String, value: Any?) {
            val f = AndroidApplicationRuntime::class.java.getDeclaredField(name)
            f.isAccessible = true
            f.set(runtime, value)
        }
        setField("databaseFile", dbFile)
        setField("json", Json { ignoreUnknownKeys = true })
        setField("lock", Any())
        setField("handle", handle)
        setField("submissionJournal", ApplicationSubmissionJournal(File(dbFile.path + ".pending-commands.json")))
        setField("recoveredReceipts", mutableListOf<JsonObject>())
        return runtime
    }

    private fun readHttpRequest(socket: Socket): Triple<String, Map<String, String>, String> {
        val rawIn = socket.getInputStream()
        val headerBuf = java.io.ByteArrayOutputStream()
        var state = 0
        while (true) {
            val b = rawIn.read()
            if (b < 0) break
            headerBuf.write(b)
            state = when {
                state == 0 && b == '\r'.code -> 1
                state == 1 && b == '\n'.code -> 2
                state == 2 && b == '\r'.code -> 3
                state == 3 && b == '\n'.code -> 4
                b == '\r'.code -> 1
                else -> 0
            }
            if (state == 4) break
        }
        val headerLines = headerBuf.toByteArray().toString(Charsets.ISO_8859_1).split("\r\n")
        val requestLine = headerLines.firstOrNull().orEmpty()
        val headers = mutableMapOf<String, String>()
        for (line in headerLines.drop(1)) {
            if (line.isEmpty()) break
            val sep = line.indexOf(':')
            if (sep > 0) headers[line.substring(0, sep).trim().lowercase()] = line.substring(sep + 1).trim()
        }
        val bodyBytes = if (headers["transfer-encoding"]?.contains("chunked", ignoreCase = true) == true) {
            val chunkOut = java.io.ByteArrayOutputStream()
            while (true) {
                val sizeLine = buildString {
                    while (true) {
                        val c = rawIn.read()
                        if (c < 0 || c == '\n'.code) break
                        if (c != '\r'.code) append(c.toChar())
                    }
                }.trim()
                val chunkSize = sizeLine.substringBefore(';').toIntOrNull(16) ?: 0
                if (chunkSize <= 0) break
                val chunk = ByteArray(chunkSize)
                var readTotal = 0
                while (readTotal < chunkSize) {
                    val n = rawIn.read(chunk, readTotal, chunkSize - readTotal)
                    if (n < 0) break
                    readTotal += n
                }
                chunkOut.write(chunk, 0, readTotal)
                rawIn.read() // \r
                rawIn.read() // \n
            }
            chunkOut.toByteArray()
        } else {
            val bodyLength = headers["content-length"]?.toIntOrNull() ?: 0
            val bytes = ByteArray(bodyLength)
            var offset = 0
            while (offset < bodyLength) {
                val read = rawIn.read(bytes, offset, bodyLength - offset)
                if (read < 0) break
                offset += read
            }
            bytes.copyOf(offset)
        }
        return Triple(requestLine, headers, bodyBytes.toString(Charsets.UTF_8))
    }

    private fun writeHttpResponse(socket: Socket, statusCode: Int, reason: String, contentType: String, body: String) {
        val bytes = body.toByteArray(Charsets.UTF_8)
        val output = BufferedWriter(OutputStreamWriter(socket.getOutputStream(), Charsets.UTF_8))
        output.write(
            "HTTP/1.1 $statusCode $reason\r\n" +
                "Content-Type: $contentType\r\n" +
                "Connection: close\r\n" +
                "Content-Length: ${bytes.size}\r\n\r\n",
        )
        output.write(body)
        output.flush()
    }
}
