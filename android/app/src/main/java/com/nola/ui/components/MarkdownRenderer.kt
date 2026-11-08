package com.nola.ui.components

import android.annotation.SuppressLint
import android.content.Context
import android.os.Handler
import android.os.Looper
import android.webkit.JavascriptInterface
import android.webkit.WebSettings
import android.webkit.WebView
import android.webkit.WebViewClient
import android.widget.TextView
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.slideInVertically
import androidx.compose.animation.core.tween
import androidx.compose.animation.core.Animatable
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.text.InlineTextContent
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.drawWithContent
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.BlendMode
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.CompositingStrategy
import androidx.compose.ui.graphics.graphicsLayer
import androidx.compose.ui.graphics.toArgb
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.res.stringResource
import com.nola.R
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.SemanticsPropertyKey
import androidx.compose.ui.semantics.SemanticsPropertyReceiver
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.text
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.SpanStyle
import androidx.compose.ui.text.buildAnnotatedString
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.withStyle
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import io.noties.markwon.Markwon
import io.noties.markwon.AbstractMarkwonPlugin
import io.noties.markwon.SoftBreakAddsNewLinePlugin
import io.noties.markwon.core.MarkwonTheme
import io.noties.markwon.ext.latex.JLatexMathPlugin
import io.noties.markwon.ext.strikethrough.StrikethroughPlugin
import io.noties.markwon.ext.tables.TablePlugin
import io.noties.markwon.ext.tables.TableTheme
import io.noties.markwon.inlineparser.MarkwonInlineParserPlugin
import kotlinx.coroutines.delay
import kotlinx.coroutines.currentCoroutineContext
import kotlinx.coroutines.isActive
import kotlin.math.abs
import kotlin.math.max
import kotlin.math.min
import kotlin.math.roundToInt
import com.nola.agent.trace.streamingMarkdownProjection
import com.nola.agent.trace.StreamingMarkdownProjectionCache

/**
 * Renders markdown text using Compose. Supports:
 * - Headings (# → ######) with sizing/weight
 * - Bold (**text** or __text__), italic (*text* or _text_), strikethrough (~~text~~)
 * - Inline code (`text`)
 * - Fenced code blocks (```...```) with copy button
 * - Unordered lists (- or * items)
 * - Ordered lists (1. items)
 * - Blockquotes (> text)
 * - Horizontal rules (---)
 * - Links [text](url)
 */
@Composable
fun MarkdownRenderer(
    text: String,
    modifier: Modifier = Modifier,
    enableMermaid: Boolean = true,
    revealAppendedText: Boolean = false,
) {
    val segments = remember(text, enableMermaid) {
        if (enableMermaid) splitMermaidSegments(text) else listOf(MarkdownSegment.Markdown(text))
    }
    if (segments.size == 1 && segments.first() is MarkdownSegment.Markdown) {
        MarkdownTextSegments(
            text = text,
            revealAppendedText = revealAppendedText,
            modifier = modifier,
        )
        return
    }

    Column(modifier = modifier.fillMaxWidth()) {
        segments.forEachIndexed { index, segment ->
            when (segment) {
                is MarkdownSegment.Markdown -> {
                    if (segment.text.isNotBlank()) {
                        MarkdownTextSegments(
                            text = segment.text,
                            revealAppendedText = revealAppendedText,
                            modifier = Modifier.fillMaxWidth(),
                        )
                    }
                }
                is MarkdownSegment.Mermaid -> {
                    MermaidDiagramView(source = segment.source, modifier = Modifier.fillMaxWidth())
                }
            }
            if (index < segments.lastIndex) {
                Spacer(Modifier.height(8.dp))
            }
        }
    }
}

@Composable
private fun MarkdownTextSegments(
    text: String,
    modifier: Modifier,
    revealAppendedText: Boolean,
) {
    val segments = remember(text) { streamingMarkdownProjection(text, complete = true).committedBlocks }
    val hasScrollableContent = segments.any {
        val source = it.markdown.trimStart()
        source.startsWith("```") || source.startsWith("~~~") ||
            (source.startsWith('|') && hasEstablishedMarkdownTable(source.lines()))
    }
    if (!hasScrollableContent) {
        RichMarkdownRenderer(text, modifier.clearAndSetSemantics { this.text = AnnotatedString(text) }, revealAppendedText)
        return
    }
    Column(modifier.fillMaxWidth()) {
        segments.forEach { segment ->
            key(segment.id) {
                val structured = remember(segment.markdown) {
                    val source = segment.markdown.trimStart()
                    if (source.startsWith("```") || source.startsWith("~~~") ||
                        (source.startsWith('|') && hasEstablishedMarkdownTable(source.lines()))) {
                        parseMarkdownBlocks(source).singleOrNull()
                    } else null
                }
                when (structured) {
                    is MarkdownBlock.CodeBlock -> MarkdownCodeBlock(structured)
                    is MarkdownBlock.Table -> MarkdownTable(structured)
                    else -> RichMarkdownRenderer(
                        text = segment.markdown,
                        revealAppendedText = revealAppendedText,
                        modifier = Modifier.fillMaxWidth().clearAndSetSemantics {
                            this.text = AnnotatedString(segment.markdown)
                        },
                    )
                }
            }
        }
    }
}

private sealed class MarkdownSegment {
    data class Markdown(val text: String) : MarkdownSegment()
    data class Mermaid(val source: String) : MarkdownSegment()
}

private fun splitMermaidSegments(markdown: String): List<MarkdownSegment> {
    val lines = markdown.split('\n')
    val segments = mutableListOf<MarkdownSegment>()
    val markdownBuffer = mutableListOf<String>()
    val mermaidBuffer = mutableListOf<String>()
    var inMermaidFence = false
    var fenceMarker = ""
    var fenceOpeningLine = ""

    fun flushMarkdown() {
        val text = markdownBuffer.joinToString("\n")
        if (text.isNotBlank()) segments.add(MarkdownSegment.Markdown(text))
        markdownBuffer.clear()
    }

    fun flushMermaid() {
        segments.add(MarkdownSegment.Mermaid(mermaidBuffer.joinToString("\n").trim('\n')))
        mermaidBuffer.clear()
    }

    for (line in lines) {
        val trimmed = line.trim()
        if (inMermaidFence) {
            if (trimmed.startsWith(fenceMarker)) {
                flushMermaid()
                inMermaidFence = false
                fenceMarker = ""
                fenceOpeningLine = ""
            } else {
                mermaidBuffer.add(line)
            }
            continue
        }

        val marker = when {
            trimmed.startsWith("```") -> "```"
            trimmed.startsWith("~~~") -> "~~~"
            else -> ""
        }
        val language = if (marker.isNotEmpty()) {
            trimmed.removePrefix(marker).trim().split(Regex("\\s+")).firstOrNull().orEmpty()
        } else {
            ""
        }
        if (marker.isNotEmpty() && language.equals("mermaid", ignoreCase = true)) {
            flushMarkdown()
            inMermaidFence = true
            fenceMarker = marker
            fenceOpeningLine = line
            mermaidBuffer.clear()
        } else {
            markdownBuffer.add(line)
        }
    }

    if (inMermaidFence) {
        segments.add(MarkdownSegment.Markdown((listOf(fenceOpeningLine) + mermaidBuffer).joinToString("\n")))
    }
    flushMarkdown()
    return segments.ifEmpty { listOf(MarkdownSegment.Markdown(markdown)) }
}

@SuppressLint("SetJavaScriptEnabled")
@Suppress("DEPRECATION")
@Composable
private fun MermaidDiagramView(
    source: String,
    modifier: Modifier = Modifier
) {
    val density = LocalDensity.current
    val isDarkMode = isSystemInDarkTheme()
    val mermaidTheme = if (isDarkMode) "dark" else "default"
    val textColor = if (isDarkMode) "#F3F4F6" else "#202124"
    val loadKey = "$mermaidTheme\n$source"

    key(loadKey) {
        var diagramHeight by remember { mutableStateOf(220.dp) }
        var lastHeightPx by remember { mutableStateOf(0) }
        val shape = RoundedCornerShape(NolaRadius.md)

        Box(
            modifier = modifier
                .fillMaxWidth()
                .height(diagramHeight)
                .heightIn(min = 220.dp, max = 640.dp)
                .clip(shape)
                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.035f))
                .border(1.dp, MaterialTheme.colorScheme.outline.copy(alpha = 0.22f), shape)
        ) {
            AndroidView(
                modifier = Modifier
                    .fillMaxWidth()
                    .height(diagramHeight),
                factory = { context ->
                    WebView(context).apply {
                        setBackgroundColor(android.graphics.Color.TRANSPARENT)
                        webViewClient = WebViewClient()
                        isVerticalScrollBarEnabled = true
                        isHorizontalScrollBarEnabled = true
                        overScrollMode = WebView.OVER_SCROLL_NEVER
                        settings.javaScriptEnabled = true
                        settings.domStorageEnabled = false
                        settings.cacheMode = WebSettings.LOAD_DEFAULT
                        settings.allowFileAccess = true
                        settings.allowFileAccessFromFileURLs = false
                        settings.allowUniversalAccessFromFileURLs = false
                        settings.loadWithOverviewMode = true
                        settings.useWideViewPort = true
                        addJavascriptInterface(MermaidHeightBridge { heightPx ->
                            val minHeightPx = with(density) { 220.dp.roundToPx() }
                            val maxHeightPx = with(density) { 640.dp.roundToPx() }
                            val boundedPx = heightPx.coerceIn(minHeightPx, maxHeightPx)
                            if (lastHeightPx == 0 || abs(boundedPx - lastHeightPx) >= 12) {
                                lastHeightPx = boundedPx
                                diagramHeight = with(density) { boundedPx.toDp() }.coerceIn(220.dp, 640.dp)
                            }
                        }, "NolaMermaid")
                    }
                },
                update = { webView ->
                    if (webView.tag == loadKey) return@AndroidView
                    webView.tag = loadKey
                    webView.loadDataWithBaseURL(
                        "file:///android_asset/vendor/mermaid/",
                        mermaidHtml(source, mermaidTheme, textColor),
                        "text/html",
                        "UTF-8",
                        null
                    )
                }
            )
        }
    }
}

private class MermaidHeightBridge(
    private val onHeight: (Int) -> Unit
) {
    private val mainHandler = Handler(Looper.getMainLooper())

    @JavascriptInterface
    fun postHeight(height: Int) {
        mainHandler.post {
            onHeight(height)
        }
    }
}

private fun mermaidHtml(source: String, theme: String, textColor: String): String {
    val escaped = escapeHtml(source)
    val jsSource = jsString(source)
    return """
        <!doctype html>
        <html>
        <head>
        <meta charset="utf-8">
        <meta name="viewport" content="width=device-width, initial-scale=1">
        <style>
        html, body { margin: 0; padding: 0; background: transparent; color: $textColor; font-family: sans-serif; }
        .wrap { box-sizing: border-box; min-width: 100%; padding: 12px; overflow: auto; }
        .mermaid { min-width: max-content; }
        .fallback { white-space: pre; font-family: monospace; font-size: 13px; color: #666; }
        svg { max-width: none; height: auto; }
        </style>
        <script src="mermaid.min.js"></script>
        </head>
        <body>
        <div class="wrap"><div class="mermaid">$escaped</div></div>
        <script>
        function postHeight() {
          const h = Math.max(180, Math.ceil(document.documentElement.scrollHeight || document.body.scrollHeight || 220));
          if (window.NolaMermaid) {
            window.NolaMermaid.postHeight(h);
          }
        }
        function showFallback() {
          document.querySelector('.wrap').innerHTML = '<pre class="fallback"></pre>';
          document.querySelector('.fallback').textContent = $jsSource;
          postHeight();
        }
        if (window.mermaid) {
          try {
            const diagram = document.querySelector('.mermaid');
            if (!diagram) {
              showFallback();
            } else {
              mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: '$theme' });
              Promise.resolve(mermaid.run({ nodes: [diagram] })).then(postHeight).catch(showFallback);
            }
          } catch (error) {
            showFallback();
          }
        } else {
          showFallback();
        }
        window.addEventListener('load', postHeight);
        setTimeout(postHeight, 350);
        </script>
        </body>
        </html>
    """.trimIndent()
}

private fun escapeHtml(value: String): String =
    value.replace("&", "&amp;")
        .replace("<", "&lt;")
        .replace(">", "&gt;")

private fun jsString(value: String): String =
    "\"" + value
        .replace("\\", "\\\\")
        .replace("\"", "\\\"")
        .replace("\n", "\\n")
        .replace("\r", "\\r") + "\""

@Composable
private fun RichMarkdownRenderer(
    text: String,
    modifier: Modifier = Modifier,
    revealAppendedText: Boolean = false,
) {
    val context = LocalContext.current
    val textColor = MaterialTheme.colorScheme.onSurface.toArgb()
    val linkColor = MaterialTheme.colorScheme.primary.toArgb()
    val secondaryTextColor = MaterialTheme.colorScheme.onSurfaceVariant.toArgb()
    val outlineColor = MaterialTheme.colorScheme.outline.copy(alpha = 0.62f).toArgb()
    val codeBackgroundColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.48f).toArgb()
    val tableOddRowColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.12f).toArgb()
    val tableEvenRowColor = MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.04f).toArgb()
    val tableHeaderColor = MaterialTheme.colorScheme.primary.copy(alpha = 0.10f).toArgb()
    val density = LocalDensity.current
    var measuredContentHeightPx by remember { mutableStateOf(0) }
    val normalizedText = remember(text) { normalizeLatexForMarkwon(text) }
    val markwon = remember(
        context,
        textColor,
        linkColor,
        secondaryTextColor,
        outlineColor,
        codeBackgroundColor,
        tableOddRowColor,
        tableEvenRowColor,
        tableHeaderColor
    ) {
        val density = context.resources.displayMetrics.density
        val tableTheme = TableTheme.Builder()
            .tableCellPadding((8f * density).toInt())
            .tableBorderColor(outlineColor)
            .tableBorderWidth(max(1, density.toInt()))
            .tableOddRowBackgroundColor(tableOddRowColor)
            .tableEvenRowBackgroundColor(tableEvenRowColor)
            .tableHeaderRowBackgroundColor(tableHeaderColor)
            .build()
        Markwon.builder(context)
            .usePlugin(SoftBreakAddsNewLinePlugin.create())
            .usePlugin(object : AbstractMarkwonPlugin() {
                override fun configureTheme(builder: MarkwonTheme.Builder) {
                    builder
                        .linkColor(linkColor)
                        .blockQuoteColor(secondaryTextColor)
                        .listItemColor(secondaryTextColor)
                        .codeTextColor(textColor)
                        .codeBlockTextColor(textColor)
                        .codeBackgroundColor(codeBackgroundColor)
                        .codeBlockBackgroundColor(codeBackgroundColor)
                        .headingBreakHeight(0)
                        .headingBreakColor(outlineColor)
                        .headingTextSizeMultipliers(
                            floatArrayOf(1.55f, 1.38f, 1.24f, 1.14f, 1.08f, 1.04f)
                        )
                        .thematicBreakColor(outlineColor)
                }
            })
            .usePlugin(TablePlugin.create(tableTheme))
            .usePlugin(StrikethroughPlugin.create())
            .usePlugin(MarkwonInlineParserPlugin.create())
            .usePlugin(JLatexMathPlugin.create(16f) { builder ->
                builder.inlinesEnabled(true)
            })
            .build()
    }
    val renderKey = remember(
        normalizedText,
        textColor,
        linkColor,
        secondaryTextColor,
        outlineColor,
        codeBackgroundColor,
        tableOddRowColor,
        tableEvenRowColor,
        tableHeaderColor
    ) {
        listOf(
            normalizedText,
            textColor,
            linkColor,
            secondaryTextColor,
            outlineColor,
            codeBackgroundColor,
            tableOddRowColor,
            tableEvenRowColor,
            tableHeaderColor
        )
    }

    val measuredHeightModifier = if (measuredContentHeightPx > 0) {
        Modifier.height(with(density) { measuredContentHeightPx.toDp() })
    } else {
        Modifier
    }
    AndroidView(
        modifier = modifier
            .fillMaxWidth()
            .then(measuredHeightModifier),
        factory = { viewContext ->
            MeasuredMarkdownTextView(viewContext) { contentHeightPx ->
                if (contentHeightPx > 0 && contentHeightPx != measuredContentHeightPx) {
                    measuredContentHeightPx = contentHeightPx
                }
            }.apply {
                layoutParams = android.view.ViewGroup.LayoutParams(
                    android.view.ViewGroup.LayoutParams.MATCH_PARENT,
                    android.view.ViewGroup.LayoutParams.WRAP_CONTENT,
                )
                textSize = 17f
                setTextColor(textColor)
                setLinkTextColor(linkColor)
                includeFontPadding = false
                maxLines = Int.MAX_VALUE
                ellipsize = null
                setHorizontallyScrolling(false)
                isVerticalScrollBarEnabled = false
                setLineSpacing((28f * resources.displayMetrics.scaledDensity - paint.fontSpacing).coerceAtLeast(0f), 1f)
            }
        },
        update = { textView ->
            textView.textSize = 17f
            textView.setLineSpacing((28f * textView.resources.displayMetrics.scaledDensity - textView.paint.fontSpacing).coerceAtLeast(0f), 1f)
            textView.setTextColor(textColor)
            textView.setLinkTextColor(linkColor)
            textView.renderMarkdown(
                markwon = markwon,
                markdown = normalizedText,
                renderKey = renderKey,
                revealAppendedText = revealAppendedText,
            )
        }
    )
}

private class MeasuredMarkdownTextView(
    context: Context,
    private val onContentHeightChanged: (Int) -> Unit,
) : TextView(context) {
    private var currentRenderKey: Any? = null
    private var reportedContentHeight = 0
    private var acceptedText = ""
    private data class Ink(val start: Int, val end: Int, val time: Long, val position: Float)
    private val ink = mutableListOf<Ink>()
    private val inkFrame = object : Runnable {
        override fun run() {
            val now = android.os.SystemClock.uptimeMillis()
            ink.removeAll { now - it.time >= 520 }
            val spans = text as? android.text.Spannable
            spans?.getSpans(0, spans.length, WaterInk::class.java)?.filter { now - it.ink.time >= 520 }?.forEach { spans.removeSpan(it) }
            invalidate()
            if (ink.isNotEmpty() && isAttachedToWindow) postOnAnimation(this)
        }
    }
    private class WaterInk(val ink: Ink) : android.text.style.CharacterStyle() {
        override fun updateDrawState(paint: android.text.TextPaint) {
            val elapsed = ((android.os.SystemClock.uptimeMillis() - ink.time) / 520f).coerceIn(0f, 1f)
            val local = ((elapsed * 1.34f - ink.position) / .34f).coerceIn(0f, 1f)
            val remaining = (1f - local) * (1f - local) * (1f - local)
            paint.alpha = (paint.alpha * (1f - .92f * remaining)).toInt()
            paint.maskFilter = if (remaining > .02f) android.graphics.BlurMaskFilter(1.4f * remaining, android.graphics.BlurMaskFilter.Blur.NORMAL) else null
        }
    }
    override fun onDetachedFromWindow() {
        removeCallbacks(inkFrame)
        super.onDetachedFromWindow()
    }

    fun renderMarkdown(
        markwon: Markwon,
        markdown: String,
        renderKey: Any,
        revealAppendedText: Boolean,
    ) {
        if (!revealAppendedText) {
            ink.clear(); removeCallbacks(inkFrame)
            val spans = text as? android.text.Spannable
            spans?.getSpans(0, spans.length, WaterInk::class.java)?.forEach { spans.removeSpan(it) }
        }
        if (currentRenderKey == renderKey) { invalidate(); return }
        currentRenderKey = renderKey
        markwon.setMarkdown(this, markdown)
        // Markwon installs link movement even for ordinary prose. Without a
        // link, let the surrounding message receive its long-press gesture.
        if ((text as? android.text.Spanned)?.getSpans(0, text.length,
                android.text.style.ClickableSpan::class.java)?.isEmpty() == true) {
            movementMethod = null
            isClickable = false
            isLongClickable = false
        }
        val next = text.toString()
        if (revealAppendedText && next.startsWith(acceptedText)) {
            val now = android.os.SystemClock.uptimeMillis()
            ink.removeAll { now - it.time >= 520 }
            val start = acceptedText.length
            var index = start
            while (index < next.length) {
                var end = minOf(next.length, index + 3)
                if (end < next.length && Character.isHighSurrogate(next[end - 1])) end++
                ink.add(Ink(index, end, now, (index - start).toFloat() / maxOf(1, next.length - start)))
                index = end
            }
        } else ink.clear()
        acceptedText = next
        if (ink.isNotEmpty()) {
            val spans = android.text.SpannableString(text)
            for (segment in ink) spans.setSpan(WaterInk(segment), segment.start, segment.end, android.text.Spanned.SPAN_EXCLUSIVE_EXCLUSIVE)
            setText(spans, BufferType.SPANNABLE)
            removeCallbacks(inkFrame); postOnAnimation(inkFrame)
        }
        requestLayout()
    }

    override fun onMeasure(widthMeasureSpec: Int, heightMeasureSpec: Int) {
        // AndroidView can first be measured while the TextView is still empty and
        // retain the viewport's finite height after Markwon installs its Spanned
        // content. Measure the document at the exact Compose width but with an
        // unbounded vertical axis so LazyColumn owns scrolling and receives the
        // complete rendered height.
        val contentHeightSpec = MeasureSpec.makeMeasureSpec(0, MeasureSpec.UNSPECIFIED)
        super.onMeasure(widthMeasureSpec, contentHeightSpec)
        val contentHeight = measuredHeight
        if (contentHeight > 0 && contentHeight != reportedContentHeight) {
            reportedContentHeight = contentHeight
            post { onContentHeightChanged(contentHeight) }
        }
    }
}

@Composable
private fun PlainMarkdownRenderer(
    text: String,
    modifier: Modifier = Modifier
) {
    val blocks = remember(text) { parseMarkdownBlocks(text) }

    Column(modifier = modifier.fillMaxWidth()) {
        blocks.forEachIndexed { index, block ->
            when (block) {
                is MarkdownBlock.Heading -> MarkdownHeading(block)
                is MarkdownBlock.Paragraph -> MarkdownParagraph(block)
                is MarkdownBlock.CodeBlock -> MarkdownCodeBlock(block)
                is MarkdownBlock.UnorderedList -> MarkdownList(block)
                is MarkdownBlock.OrderedList -> MarkdownOrderedList(block)
                is MarkdownBlock.Blockquote -> MarkdownBlockquote(block)
                is MarkdownBlock.HorizontalRule -> MarkdownHR()
                is MarkdownBlock.Table -> MarkdownTable(block)
            }
            if (index < blocks.size - 1) {
                Spacer(Modifier.height(4.dp))
            }
        }
    }
}

private fun normalizeLatexForMarkwon(markdown: String): String {
    val source = markdown.replace("\\[", "$$").replace("\\]", "$$")
    val builder = StringBuilder(source.length)
    var i = 0
    var inFence = false
    while (i < source.length) {
        if (source.startsWith("```", i) || source.startsWith("~~~", i)) {
            inFence = !inFence
            val marker = source.substring(i, i + 3)
            builder.append(marker)
            i += 3
            continue
        }
        if (!inFence && source[i] == '$' && !source.startsWith("$$", i)) {
            val close = findClosingSingleDollar(source, i + 1)
            if (close > i + 1) {
                builder.append("$$")
                builder.append(source, i + 1, close)
                builder.append("$$")
                i = close + 1
                continue
            }
        }
        builder.append(source[i])
        i++
    }
    return builder.toString()
}

private fun findClosingSingleDollar(source: String, start: Int): Int {
    var i = start
    while (i < source.length) {
        if (source[i] == '\n') return -1
        if (source[i] == '$' && (i == 0 || source[i - 1] != '\\') && !source.startsWith("$$", i)) {
            return i
        }
        i++
    }
    return -1
}

val StreamingMarkdownPresentedTextKey =
    SemanticsPropertyKey<String>("StreamingMarkdownPresentedText")
var SemanticsPropertyReceiver.streamingMarkdownPresentedText by
    StreamingMarkdownPresentedTextKey

@Composable
fun SmoothStreamingMarkdownRenderer(
    text: String,
    modifier: Modifier = Modifier,
    isStreaming: Boolean = true,
    revealOnFirstComposition: Boolean = false,
) {
    val reduceMotion = rememberReduceMotionEnabled()

    val presentationInProgress = isStreaming
    val projectionCache = remember { StreamingMarkdownProjectionCache() }
    val projection = remember(text, presentationInProgress) {
        projectionCache.project(text, complete = !presentationInProgress)
    }
    // Ordinary prose must remain visible while its Markdown block is mutable.
    // Filtering it here made long answers arrive in abrupt paragraph-sized jumps.
    val liveTail = projection.mutableTail
    val renderBlocks = projection.committedBlocks + listOfNotNull(liveTail)
    Column(
        modifier = modifier
            .fillMaxWidth()
            .semantics { streamingMarkdownPresentedText = text },
    ) {
        renderBlocks.forEach { block ->
            key(block.id) {
                StreamingMarkdownBlockRenderer(
                    markdown = if (block.committed) {
                        block.markdown
                    } else {
                        stabilizeStreamingMarkdownTail(block.markdown)
                    },
                    // Final Markdown normalization may turn the mutable tail into a
                    // committed block. Never replay its entrance animation after the
                    // user has already watched that text arrive.
                    reduceMotion = reduceMotion || (!presentationInProgress && !revealOnFirstComposition),
                    enableMermaid = block.committed && !presentationInProgress,
                )
            }
        }
    }
}

@Composable
private fun StreamingMarkdownBlockRenderer(
    markdown: String,
    reduceMotion: Boolean,
    enableMermaid: Boolean,
) {
    // Native ink adapter only: accepted text and layout are never paced by animation.
    MarkdownRenderer(text = markdown, modifier = Modifier.fillMaxWidth(), enableMermaid = enableMermaid,
        revealAppendedText = !reduceMotion)

}

internal fun stabilizeStreamingMarkdownTail(source: String): String {
    if (source.isEmpty()) return source
    val inlineSource = markdownOutsideFences(source)
    val suffix = buildString {
        if (inlineSource.count { it == '`' }.isOdd()) append('`')
        if (countOccurrences(inlineSource, "**").isOdd()) append("**")
        if (countOccurrences(inlineSource, "__").isOdd()) append("__")
        if (countOccurrences(inlineSource, "~~").isOdd()) append("~~")
    }
    return source + suffix
}

private fun markdownOutsideFences(source: String): String = buildString {
    var openFence: Char? = null
    var openFenceCount = 0
    source.lines().forEach { line ->
        val trimmed = line.trimStart()
        val markerCharacter = trimmed.firstOrNull()
            ?.takeIf { it == '`' || it == '~' }
        val markerCount = markerCharacter
            ?.let { character -> trimmed.takeWhile { it == character }.length }
            ?: 0
        val isFenceLine = markerCharacter != null && markerCount >= 3

        if (openFence == null) {
            if (isFenceLine) {
                openFence = markerCharacter
                openFenceCount = markerCount
            } else {
                appendLine(line)
            }
        } else if (
            markerCharacter == openFence &&
            markerCount >= openFenceCount &&
            trimmed.drop(markerCount).isBlank()
        ) {
            openFence = null
            openFenceCount = 0
        }
    }
}

private fun hasEstablishedMarkdownTable(lines: List<String>): Boolean =
    lines.indices.any { index ->
        index > 0 && lines[index - 1].contains('|') && isMarkdownTableDivider(lines[index])
    }

private fun isMarkdownTableDivider(line: String): Boolean {
    val cells = line.trim().trim('|').split('|')
    return cells.isNotEmpty() && cells.all { cell ->
        val core = cell.trim().trim(':')
        core.length >= 3 && core.all { it == '-' }
    }
}

private fun countOccurrences(source: String, marker: String): Int {
    var count = 0
    var index = 0
    while (index <= source.length - marker.length) {
        if (source.startsWith(marker, index)) {
            count += 1
            index += marker.length
        } else {
            index += 1
        }
    }
    return count
}

private fun Int.isOdd(): Boolean = this % 2 != 0

// ---- Block types ----

sealed class MarkdownBlock {
    data class Heading(val level: Int, val text: String) : MarkdownBlock()
    data class Paragraph(val inlines: List<MarkdownInline>) : MarkdownBlock()
    data class CodeBlock(val language: String?, val code: String) : MarkdownBlock()
    data class UnorderedList(val items: List<List<MarkdownInline>>) : MarkdownBlock()
    data class OrderedList(val items: List<List<MarkdownInline>>, val start: Int = 1) : MarkdownBlock()
    data class Blockquote(val text: String) : MarkdownBlock()
    data object HorizontalRule : MarkdownBlock()
    data class Table(val header: List<String>, val rows: List<List<String>>) : MarkdownBlock()
}

sealed class MarkdownInline {
    data class Text(val content: String) : MarkdownInline()
    data class Bold(val content: String) : MarkdownInline()
    data class Italic(val content: String) : MarkdownInline()
    data class Strikethrough(val content: String) : MarkdownInline()
    data class Code(val content: String) : MarkdownInline()
    data class Link(val text: String, val url: String) : MarkdownInline()
    data object SoftBreak : MarkdownInline()
}

// ---- Parser ----

fun parseMarkdownBlocks(text: String): List<MarkdownBlock> {
    val lines = text.lines()
    val blocks = mutableListOf<MarkdownBlock>()
    var i = 0

    while (i < lines.size) {
        val line = lines[i].trimEnd()

        if (line.isBlank()) { i++; continue }

        // Fenced code block
        if (line.startsWith("```") || line.startsWith("~~~")) {
            val fence = line.takeWhile { it == line.first() }
            val lang = line.removePrefix(fence).trim()
            val codeLines = mutableListOf<String>()
            i++
            while (i < lines.size && !lines[i].trimStart().startsWith(fence)) {
                codeLines.add(lines[i])
                i++
            }
            blocks.add(MarkdownBlock.CodeBlock(lang.ifEmpty { null }, codeLines.joinToString("\n")))
            i++ // skip closing ```
            continue
        }

        // Heading
        val headingMatch = Regex("^(#{1,6})\\s+(.+)").find(line)
        if (headingMatch != null) {
            blocks.add(MarkdownBlock.Heading(
                level = headingMatch.groupValues[1].length,
                text = headingMatch.groupValues[2]
            ))
            i++; continue
        }

        // Horizontal rule
        if (line.matches(Regex("^[-*_]{3,}\\s*$"))) {
            blocks.add(MarkdownBlock.HorizontalRule)
            i++; continue
        }

        // Blockquote
        if (line.startsWith("> ")) {
            blocks.add(MarkdownBlock.Blockquote(line.removePrefix("> ")))
            i++; continue
        }

        // Table
        if (line.startsWith("|") && line.endsWith("|")) {
            val header = line.split("|").drop(1).dropLast(1).map { it.trim() }
            i++
            // Skip separator line
            if (i < lines.size && lines[i].contains("---")) i++
            val rows = mutableListOf<List<String>>()
            while (i < lines.size && lines[i].trimStart().startsWith("|")) {
                rows.add(lines[i].trim().removePrefix("|").removeSuffix("|").split("|").map { it.trim() })
                i++
            }
            blocks.add(MarkdownBlock.Table(header, rows))
            continue
        }

        // Unordered list
        if (line.matches(Regex("^\\s*[-*]\\s+.+"))) {
            val items = mutableListOf<List<MarkdownInline>>()
            while (i < lines.size && lines[i].matches(Regex("^\\s*[-*]\\s+.+"))) {
                items.add(parseInline(lines[i].replaceFirst(Regex("^\\s*[-*]\\s+"), "")))
                i++
            }
            blocks.add(MarkdownBlock.UnorderedList(items))
            continue
        }

        // Ordered list
        if (line.matches(Regex("^\\s*\\d+\\.\\s+.+"))) {
            val items = mutableListOf<List<MarkdownInline>>()
            while (i < lines.size && lines[i].matches(Regex("^\\s*\\d+\\.\\s+.+"))) {
                items.add(parseInline(lines[i].replaceFirst(Regex("^\\s*\\d+\\.\\s+"), "")))
                i++
            }
            blocks.add(MarkdownBlock.OrderedList(items))
            continue
        }

        // Paragraph (accumulate until blank line or block element)
        val paraLines = mutableListOf<String>()
        while (i < lines.size &&
               lines[i].isNotBlank() &&
               !lines[i].startsWith("#") &&
               !lines[i].startsWith("```") &&
               !lines[i].startsWith("> ") &&
               !lines[i].startsWith("|") &&
               !lines[i].matches(Regex("^\\s*[-*]\\s+.+")) &&
               !lines[i].matches(Regex("^\\s*\\d+\\.\\s+.+"))
        ) {
            paraLines.add(lines[i].trimEnd())
            i++
        }
        if (paraLines.isNotEmpty()) {
            blocks.add(MarkdownBlock.Paragraph(parseInline(paraLines.joinToString("\n"))))
        } else {
            // A partial block marker is still visible text, not a completed block.
            blocks.add(MarkdownBlock.Paragraph(parseInline(lines[i])))
            i++
        }
    }

    return blocks
}

private fun takeCodePointSlice(source: String, startIndex: Int, maxCodePoints: Int): String {
    if (startIndex >= source.length || maxCodePoints <= 0) return ""
    val available = source.codePointCount(startIndex, source.length)
    val codePoints = min(maxCodePoints, available)
    val endIndex = source.offsetByCodePoints(startIndex, codePoints)
    return source.substring(startIndex, endIndex)
}

fun parseInline(text: String): List<MarkdownInline> {
    return buildList {
        text.split('\n').forEachIndexed { index, line ->
            addAll(parseInlineLine(line))
            if (index < text.count { it == '\n' }) add(MarkdownInline.SoftBreak)
        }
    }
}

private fun parseInlineLine(text: String): List<MarkdownInline> {
    val result = mutableListOf<MarkdownInline>()
    var remaining = text

    while (remaining.isNotEmpty()) {
        // Inline code: `text`
        val codeMatch = Regex("`([^`]+)`").find(remaining)
        // Bold: **text** or __text__
        val boldMatch = Regex("\\*\\*(.+?)\\*\\*|__(.+?)__").find(remaining)
        // Italic: *text* or _text_ (but not **)
        val italicMatch = Regex("(?<!\\*)\\*(?!\\*)(.+?)(?<!\\*)\\*(?!\\*)|(?<!_)_(?!_)(.+?)(?<!_)_(?!_)").find(remaining)
        // Strikethrough: ~~text~~
        val strikethroughMatch = Regex("~~(.+?)~~").find(remaining)
        // Link: [text](url)
        val linkMatch = Regex("\\[(.+?)\\]\\((.+?)\\)").find(remaining)

        val matches = listOfNotNull(
            codeMatch?.let { it.range.first to it },
            boldMatch?.let { it.range.first to it },
            italicMatch?.let { it.range.first to it },
            strikethroughMatch?.let { it.range.first to it },
            linkMatch?.let { it.range.first to it }
        ).sortedBy { it.first }

        if (matches.isEmpty()) {
            result.add(MarkdownInline.Text(remaining))
            break
        }

        val (pos, match) = matches.first()
        if (pos > 0) {
            result.add(MarkdownInline.Text(remaining.substring(0, pos)))
        }

        when {
            match.value.startsWith("`") -> {
                result.add(MarkdownInline.Code(match.groupValues[1]))
            }
            match.value.startsWith("**") || match.value.startsWith("__") -> {
                result.add(MarkdownInline.Bold(match.groupValues.getOrElse(1) { match.groupValues[2] }))
            }
            match.value.startsWith("~") -> {
                result.add(MarkdownInline.Strikethrough(match.groupValues[1]))
            }
            match.value.startsWith("[") -> {
                result.add(MarkdownInline.Link(match.groupValues[1], match.groupValues[2]))
            }
            else -> {
                result.add(MarkdownInline.Italic(match.groupValues.getOrElse(1) { match.groupValues[2] }))
            }
        }

        remaining = remaining.substring(match.range.last + 1)
    }

    if (result.isEmpty()) {
        result.add(MarkdownInline.Text(text))
    }
    return result
}

// ---- Renderers ----

@Composable
fun MarkdownHeading(block: MarkdownBlock.Heading) {
    val style = when (block.level) {
        1 -> MaterialTheme.typography.headlineSmall.copy(fontWeight = FontWeight.Bold)
        2 -> MaterialTheme.typography.titleLarge.copy(fontWeight = FontWeight.Bold)
        3 -> MaterialTheme.typography.titleMedium.copy(fontWeight = FontWeight.SemiBold)
        4 -> MaterialTheme.typography.bodyLarge.copy(fontWeight = FontWeight.SemiBold)
        5 -> MaterialTheme.typography.bodyMedium.copy(fontWeight = FontWeight.SemiBold)
        else -> MaterialTheme.typography.bodySmall.copy(fontWeight = FontWeight.SemiBold)
    }
    Text(block.text, style = style)
}

@Composable
fun MarkdownParagraph(block: MarkdownBlock.Paragraph) {
    val annotatedText = buildInlineAnnotatedString(block.inlines)
    val uriHandler = androidx.compose.ui.platform.LocalUriHandler.current
    androidx.compose.foundation.text.ClickableText(
        text = annotatedText,
        style = MaterialTheme.typography.bodyMedium,
        onClick = { offset ->
            annotatedText.getStringAnnotations(tag = "URL", start = offset, end = offset)
                .firstOrNull()
                ?.let { annotation -> runCatching { uriHandler.openUri(annotation.item) } }
        }
    )
}

@Composable
fun MarkdownCodeBlock(block: MarkdownBlock.CodeBlock) {
    val clipboard = LocalClipboardManager.current
    var copied by remember { mutableStateOf(false) }

    Box(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f))
    ) {
        Column {
            // Header bar
            Row(
                modifier = Modifier
                    .fillMaxWidth()
                    .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.7f))
                    .padding(horizontal = 8.dp, vertical = 4.dp),
                verticalAlignment = Alignment.CenterVertically
            ) {
                Text(
                    block.language ?: "code",
                    style = MaterialTheme.typography.labelSmall,
                    color = MaterialTheme.colorScheme.onSurfaceVariant,
                    modifier = Modifier.weight(1f)
                )
                IconButton(onClick = {
                    clipboard.setText(AnnotatedString(block.code))
                    copied = true
                }, modifier = Modifier.size(48.dp)) {
                    Icon(painterResource(R.drawable.nola_copy), stringResource(R.string.chat_message_action_copy), Modifier.size(18.dp))
                }
            }
            // Code content
            Text(
                text = block.code,
                modifier = Modifier
                    .horizontalScroll(rememberScrollState())
                    .padding(12.dp),
                style = MaterialTheme.typography.bodySmall.copy(
                    fontFamily = FontFamily.Monospace,
                    fontSize = 13.sp
                ),
                color = MaterialTheme.colorScheme.onSurface
            )
        }
    }
}

@Composable
fun MarkdownList(block: MarkdownBlock.UnorderedList) {
    Column {
        block.items.forEachIndexed { i, item ->
            Row(modifier = Modifier.padding(start = 8.dp)) {
                Text("•", style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.width(8.dp))
                Text(
                    buildInlineAnnotatedString(item),
                    style = MaterialTheme.typography.bodyMedium
                )
            }
            if (i < block.items.size - 1) Spacer(Modifier.height(2.dp))
        }
    }
}

@Composable
fun MarkdownOrderedList(block: MarkdownBlock.OrderedList) {
    Column {
        block.items.forEachIndexed { i, item ->
            Row(modifier = Modifier.padding(start = 8.dp)) {
                Text("${i + block.start}.", style = MaterialTheme.typography.bodyMedium)
                Spacer(Modifier.width(8.dp))
                Text(
                    buildInlineAnnotatedString(item),
                    style = MaterialTheme.typography.bodyMedium
                )
            }
            if (i < block.items.size - 1) Spacer(Modifier.height(2.dp))
        }
    }
}

@Composable
fun MarkdownBlockquote(block: MarkdownBlock.Blockquote) {
    Row(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(4.dp))
            .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.3f))
    ) {
        Box(
            modifier = Modifier
                .width(3.dp)
                .height(40.dp)
                .background(MaterialTheme.colorScheme.primary.copy(alpha = 0.5f))
        )
        Text(
            block.text,
            modifier = Modifier.padding(8.dp),
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant
        )
    }
}

@Composable
fun MarkdownHR() {
    androidx.compose.material3.HorizontalDivider(
        modifier = Modifier.padding(vertical = 8.dp),
        thickness = 1.dp,
        color = MaterialTheme.colorScheme.outline.copy(alpha = 0.3f)
    )
}

@Composable
fun MarkdownTable(block: MarkdownBlock.Table) {
    val scrollState = rememberScrollState()

    Column(
        modifier = Modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(8.dp))
            .horizontalScroll(scrollState)
    ) {
        // Header
        Row(
            modifier = Modifier
                .background(MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.5f))
                .padding(vertical = 8.dp)
        ) {
            block.header.forEach { cell ->
                Text(
                    cell,
                    modifier = Modifier.width(120.dp).padding(horizontal = 8.dp),
                    style = MaterialTheme.typography.labelMedium,
                    fontWeight = FontWeight.SemiBold
                )
            }
        }
        // Rows
        block.rows.forEachIndexed { i, row ->
            Row(
                modifier = Modifier
                    .background(
                        if (i % 2 == 0) Color.Transparent
                        else MaterialTheme.colorScheme.surfaceVariant.copy(alpha = 0.2f)
                    )
                    .padding(vertical = 6.dp)
            ) {
                row.forEach { cell ->
                    Text(
                        cell,
                        modifier = Modifier.width(120.dp).padding(horizontal = 8.dp),
                        style = MaterialTheme.typography.bodySmall
                    )
                }
            }
        }
    }
}

// ---- Inline annotated string builder ----

@Composable
fun buildInlineAnnotatedString(inlines: List<MarkdownInline>): AnnotatedString = buildAnnotatedString {
    inlines.forEach { inline ->
        when (inline) {
            is MarkdownInline.Text -> append(inline.content)
            is MarkdownInline.Bold -> withStyle(SpanStyle(fontWeight = FontWeight.Bold)) { append(inline.content) }
            is MarkdownInline.Italic -> withStyle(SpanStyle(fontStyle = FontStyle.Italic)) { append(inline.content) }
            is MarkdownInline.Strikethrough -> withStyle(SpanStyle(textDecoration = TextDecoration.LineThrough)) { append(inline.content) }
            is MarkdownInline.Code -> withStyle(SpanStyle(
                fontFamily = FontFamily.Monospace,
                fontSize = 13.sp,
                background = Color(0x20000000)
            )) { append(inline.content) }
            is MarkdownInline.Link -> {
                pushStringAnnotation(tag = "URL", annotation = inline.url)
                withStyle(SpanStyle(
                    color = MaterialTheme.colorScheme.primary,
                    textDecoration = TextDecoration.Underline
                )) { append(inline.text) }
                pop()
            }
            is MarkdownInline.SoftBreak -> append("\n")
        }
    }
}
