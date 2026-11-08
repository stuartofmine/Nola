// SPDX-License-Identifier: MIT
// Incremental Markdown DOM runtime shared by the CLI Web chat surface.
(function installNolaChatStream(global) {
    'use strict';

    const DEFAULT_SKIP_REVEAL_SELECTOR = [
        'pre', 'code', 'kbd', 'samp', 'script', 'style', 'button',
        '.katex', '.mermaid', '.md-mermaid-block', '.md-mermaid-source',
        '.md-code-block', '.md-table-wrap',
    ].join(', ');

    function takeSemanticPhrase(input, force = false, startOffset = 0) {
        const value = String(input || '');
        const start = Math.max(0, Math.min(value.length, Number(startOffset) || 0));
        if (!value || start >= value.length) return null;
        let offset = start;
        let count = 0;
        let softBoundary = 0;
        while (offset < value.length) {
            const codePoint = value.codePointAt(offset);
            const character = String.fromCodePoint(codePoint);
            offset += character.length;
            count += 1;
            if (character === '\n') {
                return { text: value.slice(start, offset), length: offset - start };
            }
            if (/^[。！？!?；;：:]$/.test(character) && count >= 28) {
                return { text: value.slice(start, offset), length: offset - start };
            }
            if (/^[\s，,、]$/.test(character) && count >= 42) softBoundary = offset;
            if (count >= 96) {
                const end = softBoundary || offset;
                return { text: value.slice(start, end), length: end - start };
            }
        }
        if (!force) return null;
        return { text: value.slice(start), length: value.length - start };
    }

    function prefersReducedMotion() {
        return global.matchMedia?.('(prefers-reduced-motion: reduce)').matches ?? false;
    }

    function isConversationOperationCurrent(
        startedConversationId,
        startedLoadVersion,
        currentConversationId,
        currentLoadVersion,
    ) {
        return startedConversationId != null
            && startedConversationId === currentConversationId
            && startedLoadVersion === currentLoadVersion;
    }

    function canonicalWindowSnapshotMatches(expected, actual) {
        if (!expected || !actual) return false;
        const expectedRevision = Number(expected.projectionRevision);
        const expectedSequence = Number(expected.eventSequence);
        return Number.isSafeInteger(expectedRevision)
            && expectedRevision >= 0
            && Number.isSafeInteger(expectedSequence)
            && expectedSequence >= 0
            && expectedRevision === Number(actual.projectionRevision)
            && expectedSequence === Number(actual.eventSequence);
    }

    function isCompactionSourceRequestCurrent(request, current) {
        if (!request || !current) return false;
        const requestIds = Array.isArray(request.sourceItemIds) ? request.sourceItemIds : [];
        const currentIds = Array.isArray(current.sourceItemIds) ? current.sourceItemIds : [];
        return isConversationOperationCurrent(
            request.conversationId,
            request.loadVersion,
            current.activeConversationId ?? current.conversationId,
            current.loadVersion,
        )
            && request.conversationId === current.conversationId
            && request.messageId === current.messageId
            && request.partId === current.partId
            && Boolean(request.sourceFingerprint)
            && request.sourceFingerprint === current.sourceFingerprint
            && requestIds.length > 0
            && requestIds.length === currentIds.length
            && requestIds.every((itemId, index) => itemId === currentIds[index]);
    }

    function revealTokens(text) {
        const tokens = [];
        let remaining = String(text || '');
        while (remaining) {
            const phrase = takeSemanticPhrase(remaining, true);
            if (!phrase?.text || phrase.length <= 0) break;
            tokens.push(phrase.text);
            remaining = remaining.slice(phrase.length);
        }
        return tokens;
    }

    function delimiterPositions(value, delimiter) {
        const positions = [];
        let cursor = 0;
        while (cursor <= value.length - delimiter.length) {
            const index = value.indexOf(delimiter, cursor);
            if (index < 0) break;
            let escapes = 0;
            for (let offset = index - 1; offset >= 0 && value[offset] === '\\'; offset -= 1) escapes += 1;
            if (escapes % 2 === 0) positions.push(index);
            cursor = index + delimiter.length;
        }
        return positions;
    }

    function unmatchedInlineStart(value) {
        const text = maskFencedMarkdown(String(value || ''));
        const candidates = [];
        for (const delimiter of ['`', '**', '__', '~~']) {
            const positions = delimiterPositions(text, delimiter);
            if (positions.length % 2 === 1) candidates.push(positions[positions.length - 1]);
        }
        const labelStart = text.lastIndexOf('[');
        const labelEnd = text.lastIndexOf(']');
        if (labelStart > labelEnd) candidates.push(labelStart);
        const destinationStart = text.lastIndexOf('](');
        if (destinationStart >= 0 && text.indexOf(')', destinationStart + 2) < 0) {
            candidates.push(text.lastIndexOf('[', destinationStart));
        }
        return candidates.filter(index => index >= 0).sort((a, b) => a - b)[0] ?? -1;
    }

    function maskFencedMarkdown(value) {
        const text = String(value || '');
        const lines = text.match(/[^\n]*(?:\n|$)/g) || [];
        let open = null;
        return lines.map(line => {
            const body = line.endsWith('\n') ? line.slice(0, -1) : line;
            const trimmed = body.replace(/^ {0,3}/, '');
            const match = trimmed.match(/^(`{3,}|~{3,})(.*)$/);
            const fence = match?.[1] || '';
            if (!open) {
                if (fence) open = { character: fence[0], length: fence.length };
            } else if (
                fence &&
                fence[0] === open.character &&
                fence.length >= open.length &&
                String(match?.[2] || '').trim() === ''
            ) {
                open = null;
            }
            if (open || fence) {
                return body.replace(/[^\n]/g, ' ') + (line.endsWith('\n') ? '\n' : '');
            }
            return line;
        }).join('');
    }

    function openFenceAtEnd(value) {
        const lines = String(value || '').match(/[^\n]*(?:\n|$)/g) || [];
        let open = null;
        for (const line of lines) {
            const body = line.endsWith('\n') ? line.slice(0, -1) : line;
            const trimmed = body.replace(/^ {0,3}/, '');
            const match = trimmed.match(/^(`{3,}|~{3,})(.*)$/);
            const fence = match?.[1] || '';
            if (!open) {
                if (fence) open = { character: fence[0], length: fence.length };
            } else if (
                fence &&
                fence[0] === open.character &&
                fence.length >= open.length &&
                String(match?.[2] || '').trim() === ''
            ) {
                open = null;
            }
        }
        return open;
    }

    function incompleteFenceLineStart(value) {
        const text = String(value || '');
        if (!text || text.endsWith('\n')) return -1;
        const lineStart = text.lastIndexOf('\n') + 1;
        const line = text.slice(lineStart);
        const trimmed = line.replace(/^ {0,3}/, '');
        if (/^`{1,2}$/.test(trimmed) || /^~{1,2}$/.test(trimmed)) return lineStart;
        if (
            /^(?:`{3,}|~{3,})[^\n]*$/.test(trimmed) &&
            !openFenceAtEnd(text.slice(0, lineStart))
        ) {
            return lineStart;
        }
        return -1;
    }

    function incompleteTableStart(value) {
        const text = String(value || '');
        const blockStart = text.lastIndexOf('\n\n') + 2;
        const lines = text.slice(blockStart).split('\n');
        const header = lines[0] || '';
        if (!header.includes('|')) return -1;
        const separatorCells = (lines[1] || '')
            .replace(/^\s*\|/, '')
            .replace(/\|\s*$/, '')
            .split('|')
            .map(cell => cell.trim());
        const isSeparator = separatorCells.length >= 2
            && separatorCells.every(cell => /^:?-{3,}:?$/.test(cell));
        return isSeparator ? -1 : blockStart;
    }

    function prepareStreamingMarkdown(value, options = {}) {
        const markdown = String(value || '');
        if (!markdown || options.final === true) return { markdown, pending: '' };
        const boundaries = [
            incompleteFenceLineStart(markdown),
            incompleteTableStart(markdown),
            unmatchedInlineStart(markdown),
        ].filter(index => index >= 0);
        if (!boundaries.length) return { markdown, pending: '' };
        const boundary = Math.min(...boundaries);
        return {
            markdown: markdown.slice(0, boundary),
            pending: markdown.slice(boundary),
        };
    }

    function stableBlockBoundary(value) {
        const text = String(value || '');
        if (!text) return 0;
        const lines = text.match(/[^\n]*(?:\n|$)/g) || [];
        let offset = 0;
        let boundary = 0;
        let openFence = null;
        for (const line of lines) {
            const body = line.endsWith('\n') ? line.slice(0, -1) : line;
            const trimmed = body.replace(/^ {0,3}/, '');
            const fenceMatch = trimmed.match(/^(`{3,}|~{3,})(.*)$/);
            const fence = fenceMatch?.[1] || '';
            if (!openFence && fence) {
                openFence = { character: fence[0], length: fence.length };
            } else if (
                openFence &&
                fence &&
                fence[0] === openFence.character &&
                fence.length >= openFence.length &&
                String(fenceMatch?.[2] || '').trim() === ''
            ) {
                openFence = null;
            }
            offset += line.length;
            if (!openFence && body.trim() === '' && line.endsWith('\n')) boundary = offset;
        }
        return boundary;
    }

    function nodesAreCompatible(current, next) {
        if (!current || !next || current.nodeType !== next.nodeType) return false;
        if (current.nodeType === Node.TEXT_NODE || current.nodeType === Node.COMMENT_NODE) return true;
        if (current.nodeType !== Node.ELEMENT_NODE) return false;
        if (current.tagName !== next.tagName) return false;
        const currentToken = current.getAttribute('data-stream-token');
        const nextToken = next.getAttribute('data-stream-token');
        if (currentToken !== null || nextToken !== null) return currentToken === nextToken;
        return true;
    }

    function syncAttributes(current, next) {
        for (const attr of Array.from(current.attributes)) {
            if (!next.hasAttribute(attr.name)) current.removeAttribute(attr.name);
        }
        for (const attr of Array.from(next.attributes)) {
            if (current.getAttribute(attr.name) !== attr.value) current.setAttribute(attr.name, attr.value);
        }
    }

    function patchNode(current, next) {
        if (!nodesAreCompatible(current, next)) {
            const replacement = next.cloneNode(true);
            current.replaceWith(replacement);
            return replacement;
        }
        if (current.nodeType === Node.TEXT_NODE || current.nodeType === Node.COMMENT_NODE) {
            if (current.nodeValue !== next.nodeValue) current.nodeValue = next.nodeValue;
            return current;
        }
        if (current.nodeType !== Node.ELEMENT_NODE) return current;
        syncAttributes(current, next);
        patchChildren(current, next);
        return current;
    }

    function patchChildren(currentParent, nextParent) {
        let index = 0;
        while (index < nextParent.childNodes.length || index < currentParent.childNodes.length) {
            const current = currentParent.childNodes[index];
            const next = nextParent.childNodes[index];
            if (!next && current) {
                current.remove();
                continue;
            }
            if (next && !current) {
                currentParent.appendChild(next.cloneNode(true));
                index += 1;
                continue;
            }
            patchNode(current, next);
            index += 1;
        }
        return currentParent;
    }

    function patchTailChildren(currentParent, nextParent, stableCount = 0) {
        let currentIndex = Math.max(0, stableCount);
        let nextIndex = 0;
        while (nextIndex < nextParent.childNodes.length || currentIndex < currentParent.childNodes.length) {
            const current = currentParent.childNodes[currentIndex];
            const next = nextParent.childNodes[nextIndex];
            if (!next && current) {
                current.remove();
                continue;
            }
            if (next && !current) {
                currentParent.appendChild(next.cloneNode(true));
                currentIndex += 1;
                nextIndex += 1;
                continue;
            }
            patchNode(current, next);
            currentIndex += 1;
            nextIndex += 1;
        }
        return currentParent;
    }

    function patchHtml(root, html) {
        if (!root) return root;
        const staging = document.createElement('div');
        staging.innerHTML = String(html || '');
        patchChildren(root, staging);
        return root;
    }

    function prependMessageWindow(root, html) {
        if (!root) return [];
        const existingRows = Array.from(root.children).filter(element =>
            element.classList?.contains('chat-msg')
        );
        const staging = document.createElement('template');
        staging.innerHTML = String(html || '');
        const insertedRows = Array.from(staging.content.children).filter(element =>
            element.classList?.contains('chat-msg')
        );
        const offset = insertedRows.length;

        // The paging control belongs before the oldest loaded message and is
        // recreated by the caller after the window metadata is updated.
        Array.from(root.children).forEach(element => {
            if (element.classList?.contains('history-window-control')) element.remove();
        });
        for (const row of existingRows) {
            if (!row.hasAttribute('data-message-index')) continue;
            const currentIndex = Number(row.getAttribute('data-message-index'));
            if (Number.isInteger(currentIndex)) {
                row.setAttribute('data-message-index', String(currentIndex + offset));
            }
        }
        root.insertBefore(staging.content, existingRows[0] || null);
        return insertedRows;
    }

    function decorateRevealTokens(root, stableTokenCount = 0, options = {}) {
        if (!root) return 0;
        const skipSelector = options.skipRevealSelector || DEFAULT_SKIP_REVEAL_SELECTOR;
        const final = options.final === true;
        const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT, {
            acceptNode(node) {
                if (!node.nodeValue || !node.nodeValue.trim()) return NodeFilter.FILTER_REJECT;
                if (node.parentElement?.closest(skipSelector)) return NodeFilter.FILTER_REJECT;
                return NodeFilter.FILTER_ACCEPT;
            },
        });
        const textNodes = [];
        while (walker.nextNode()) textNodes.push(walker.currentNode);

        let tokenIndex = Math.max(0, Number(options.tokenOffset) || 0);
        for (const textNode of textNodes) {
            const tokens = revealTokens(textNode.nodeValue || '');
            if (!tokens.length) continue;
            const fragment = document.createDocumentFragment();
            for (const token of tokens) {
                const span = document.createElement('span');
                span.className = (final || tokenIndex < stableTokenCount)
                    ? 'md-reveal-token is-shown'
                    : 'md-reveal-token';
                span.dataset.streamToken = String(tokenIndex);
                span.textContent = token;
                fragment.appendChild(span);
                tokenIndex += 1;
            }
            textNode.parentNode?.replaceChild(fragment, textNode);
        }
        return tokenIndex;
    }

    function createSmoothMarkdownStreamController(el, options = {}) {
        if (!el) throw new Error('A stream root element is required.');
        if (typeof options.renderMarkdown !== 'function') {
            throw new Error('renderMarkdown(text, state) is required.');
        }

        let source = '';
        let visible = '';
        let queuedUntil = 0;
        let phraseQueue = [];
        let revealTimer = 0;
        let idleTimer = 0;
        let settleTimer = 0;
        let finishDeadlineTimer = 0;
        let destroyed = false;
        let finishing = false;
        let finishingBatchSize = 1;
        let finishResolve = null;
        let committedMarkdown = '';
        let renderedTokenCount = 0;
        let committedSourceLength = 0;
        let committedChildCount = 0;
        let committedTokenCount = 0;
        let revealedPhraseCount = 0;
        let stableBoundaryScannedCharacters = 0;
        let streamPreparationScannedCharacters = 0;
        const reducedMotionOverride = options.reducedMotion;
        const reducedMotionQuery = reducedMotionOverride == null
            ? global.matchMedia?.('(prefers-reduced-motion: reduce)')
            : null;
        let reducedMotion = reducedMotionOverride ?? reducedMotionQuery?.matches ?? false;
        const configuredPhraseDelayMs = options.phraseDelayMs ?? 48;
        const configuredFinishingDelayMs = options.finishingDelayMs ?? 20;
        const configuredIdleBoundaryMs = options.idleBoundaryMs ?? 96;
        const configuredSettleMs = options.settleMs ?? 180;
        const configuredMaximumVisualBacklogMs = options.maximumVisualBacklogMs ?? 320;
        const configuredMaxFinishMs = options.maxFinishMs
            ?? configuredMaximumVisualBacklogMs + configuredSettleMs + 40;
        let phraseDelayMs = reducedMotion ? 0 : configuredPhraseDelayMs;
        let finishingDelayMs = reducedMotion ? 0 : configuredFinishingDelayMs;
        let idleBoundaryMs = reducedMotion ? 0 : configuredIdleBoundaryMs;
        // The final Markdown pass marks every phrase as stable. Keep it behind the
        // reveal duration so the last phrase is not snapped to full opacity.
        let settleMs = reducedMotion ? 0 : configuredSettleMs;
        let maxFinishMs = reducedMotion ? 0 : configuredMaxFinishMs;

        const resolveFinish = () => {
            if (finishDeadlineTimer) global.clearTimeout(finishDeadlineTimer);
            finishDeadlineTimer = 0;
            const resolve = finishResolve;
            finishResolve = null;
            if (resolve) resolve();
        };

        const forceFinish = () => {
            if (destroyed || !finishing || !finishResolve) return;
            if (revealTimer) global.clearTimeout(revealTimer);
            if (settleTimer) global.clearTimeout(settleTimer);
            revealTimer = 0;
            settleTimer = 0;
            phraseQueue = [];
            queuedUntil = source.length;
            visible = source;
            render(true);
            resolveFinish();
        };

        const render = (final = false) => {
            if (destroyed) return;
            // A committed boundary is a blank line outside fences and inline
            // constructs, so streaming completeness can be evaluated from
            // that boundary forward. Re-scanning the immutable prefix on
            // every phrase makes long responses quadratic.
            const uncommittedVisible = visible.slice(committedSourceLength);
            if (!final) streamPreparationScannedCharacters += uncommittedVisible.length;
            const preparedTail = prepareStreamingMarkdown(uncommittedVisible, { final });
            const prepared = {
                markdown: visible.slice(0, committedSourceLength) + preparedTail.markdown,
                pending: preparedTail.pending,
            };
            let nextMarkdown = prepared.markdown;
            if (!final && !nextMarkdown.startsWith(committedMarkdown)) nextMarkdown = committedMarkdown;
            const canSettleWithoutRender = final
                && nextMarkdown === committedMarkdown
                && options.requiresFinalRender?.(visible) === false;
            committedMarkdown = final ? visible : nextMarkdown;
            if (canSettleWithoutRender) {
                el.querySelectorAll?.('.md-reveal-token').forEach(token => token.classList.add('is-shown'));
                el.classList.add('md-stream-shell');
                el.classList.remove('is-streaming');
                committedSourceLength = nextMarkdown.length;
                committedChildCount = el.childNodes.length;
                committedTokenCount = renderedTokenCount;
                options.onRender?.({
                    final: true,
                    source,
                    visible,
                    rendered: nextMarkdown,
                    pending: '',
                    tokenCount: renderedTokenCount,
                    stableBoundaryScannedCharacters,
                    streamPreparationScannedCharacters,
                });
                return;
            }
            if (nextMarkdown.length < committedSourceLength) {
                committedSourceLength = 0;
                committedChildCount = 0;
                committedTokenCount = 0;
                renderedTokenCount = 0;
                el.replaceChildren();
            }
            const uncommittedMarkdown = nextMarkdown.slice(committedSourceLength);
            if (!final) stableBoundaryScannedCharacters += uncommittedMarkdown.length;
            const nextStableBoundary = final
                ? nextMarkdown.length
                : committedSourceLength + stableBlockBoundary(uncommittedMarkdown);
            const activeMarkdown = nextMarkdown.slice(committedSourceLength);
            const staging = document.createElement('div');
            staging.innerHTML = String(options.renderMarkdown(activeMarkdown, {
                final,
                source,
                visible,
                rendered: activeMarkdown,
                pending: prepared.pending,
            }) || '');
            const nextTokenCount = decorateRevealTokens(staging, renderedTokenCount, {
                final,
                skipRevealSelector: options.skipRevealSelector,
                tokenOffset: committedTokenCount,
            });
            el.classList.add('md-stream-shell');
            el.classList.toggle('is-streaming', !final);
            patchTailChildren(el, staging, committedChildCount);
            renderedTokenCount = nextTokenCount;
            if (nextStableBoundary > committedSourceLength) {
                const newlyStableMarkdown = nextMarkdown.slice(committedSourceLength, nextStableBoundary);
                const stableProbe = document.createElement('div');
                stableProbe.innerHTML = String(options.renderMarkdown(newlyStableMarkdown, {
                    final: true,
                    source,
                    visible,
                    rendered: newlyStableMarkdown,
                    pending: '',
                }) || '');
                const newlyStableChildren = stableProbe.childNodes.length;
                const stableNodes = Array.from(el.childNodes).slice(
                    committedChildCount,
                    committedChildCount + newlyStableChildren,
                );
                stableNodes.forEach(node => {
                    if (node.nodeType !== Node.ELEMENT_NODE) return;
                    node.querySelectorAll?.('.md-reveal-token').forEach(token => token.classList.add('is-shown'));
                });
                committedChildCount += newlyStableChildren;
                committedSourceLength = nextStableBoundary;
                committedTokenCount += stableNodes.reduce((count, node) => count + (
                    node.nodeType === Node.ELEMENT_NODE
                        ? node.querySelectorAll?.('.md-reveal-token').length || 0
                        : 0
                ), 0);
            }
            el.dataset.streamPhraseCount = String(revealedPhraseCount);
            options.onRender?.({
                final,
                source,
                visible,
                rendered: nextMarkdown,
                pending: prepared.pending,
                tokenCount: renderedTokenCount,
                stableBoundaryScannedCharacters,
                streamPreparationScannedCharacters,
            });
            options.onLayout?.();
        };

        const finishIfDrained = () => {
            if (!finishing || phraseQueue.length || queuedUntil < source.length || revealTimer || settleTimer) return;
            settleTimer = global.setTimeout(() => {
                settleTimer = 0;
                visible = source;
                render(true);
                resolveFinish();
            }, settleMs);
        };

        const revealNext = () => {
            revealTimer = 0;
            if (destroyed) return;
            // Keep the reveal semantic rather than character based, but bound
            // catch-up latency for long answers. Backlogged phrases rise in a
            // small visual wave instead of forcing one timer per phrase for
            // minutes after the Core run has already completed.
            const normalSteps = Math.max(1, Math.floor(
                configuredMaximumVisualBacklogMs / Math.max(1, phraseDelayMs)
            ));
            const batchSize = reducedMotion
                ? phraseQueue.length
                : !visible
                ? 1
                : finishing
                ? finishingBatchSize
                : Math.max(1, Math.ceil(phraseQueue.length / normalSteps));
            const phrases = phraseQueue.splice(0, batchSize);
            if (phrases.length) {
                visible += phrases.join('');
                revealedPhraseCount += phrases.length;
                render(false);
            }
            if (phraseQueue.length) {
                revealTimer = global.setTimeout(
                    revealNext,
                    finishing ? finishingDelayMs : phraseDelayMs,
                );
            } else {
                finishIfDrained();
            }
        };

        const ensureRevealLoop = () => {
            if (destroyed || revealTimer || !phraseQueue.length) return;
            if (reducedMotion || !visible) revealNext();
            else revealTimer = global.setTimeout(revealNext, finishing ? finishingDelayMs : phraseDelayMs);
        };

        const enqueueAvailable = (force = false, oneOnly = false) => {
            while (queuedUntil < source.length) {
                // Scan the shared source from an offset. Slicing the complete
                // remaining suffix on every phrase makes one large provider
                // delta allocate O(n²) temporary string data.
                const phrase = takeSemanticPhrase(source, force, queuedUntil);
                if (!phrase?.text) break;
                phraseQueue.push(phrase.text);
                queuedUntil += phrase.length;
                if (oneOnly) break;
            }
            ensureRevealLoop();
        };

        const scheduleIdleBoundary = () => {
            if (reducedMotion) {
                enqueueAvailable(true);
                return;
            }
            if (idleTimer) global.clearTimeout(idleTimer);
            idleTimer = global.setTimeout(() => {
                idleTimer = 0;
                enqueueAvailable(true, true);
                if (queuedUntil < source.length && !finishing) scheduleIdleBoundary();
            }, idleBoundaryMs);
        };

        const handleReducedMotionChange = event => {
            const nextReducedMotion = Boolean(event?.matches);
            if (destroyed || nextReducedMotion === reducedMotion) return;
            reducedMotion = nextReducedMotion;
            phraseDelayMs = reducedMotion ? 0 : configuredPhraseDelayMs;
            finishingDelayMs = reducedMotion ? 0 : configuredFinishingDelayMs;
            idleBoundaryMs = reducedMotion ? 0 : configuredIdleBoundaryMs;
            settleMs = reducedMotion ? 0 : configuredSettleMs;
            maxFinishMs = reducedMotion ? 0 : configuredMaxFinishMs;
            if (!reducedMotion) return;

            // An active stream must honor an accessibility change immediately.
            // Cancel animation pacing and reveal the authoritative source rather
            // than leaving a long queued answer draining under the old setting.
            if (revealTimer) global.clearTimeout(revealTimer);
            if (idleTimer) global.clearTimeout(idleTimer);
            if (settleTimer) global.clearTimeout(settleTimer);
            revealTimer = 0;
            idleTimer = 0;
            settleTimer = 0;
            enqueueAvailable(true);
            finishIfDrained();
        };

        if (reducedMotionQuery?.addEventListener) {
            reducedMotionQuery.addEventListener('change', handleReducedMotionChange);
        } else if (reducedMotionQuery?.addListener) {
            reducedMotionQuery.addListener(handleReducedMotionChange);
        }

        const resetTo = (nextText) => {
            source = String(nextText || '');
            visible = '';
            queuedUntil = 0;
            phraseQueue = [];
            renderedTokenCount = 0;
            revealedPhraseCount = 0;
            committedMarkdown = '';
            committedSourceLength = 0;
            committedChildCount = 0;
            committedTokenCount = 0;
            el.replaceChildren();
            if (revealTimer) global.clearTimeout(revealTimer);
            if (idleTimer) global.clearTimeout(idleTimer);
            revealTimer = 0;
            idleTimer = 0;
            render(false);
            enqueueAvailable(false);
            scheduleIdleBoundary();
        };

        return {
            append(delta) {
                if (destroyed || !delta) return;
                source += String(delta);
                enqueueAvailable(false);
                scheduleIdleBoundary();
            },
            replace(nextText) {
                if (destroyed) return;
                const next = String(nextText || '');
                if (!next.startsWith(visible)) {
                    resetTo(next);
                    return;
                }
                source = next;
                queuedUntil = visible.length;
                phraseQueue = [];
                enqueueAvailable(false);
                scheduleIdleBoundary();
                if (!source) render(false);
            },
            finish(finalText) {
                if (destroyed) return Promise.resolve();
                const next = String(finalText ?? source);
                if (!next.startsWith(visible)) resetTo(next);
                else source = next;
                finishing = true;
                if (idleTimer) global.clearTimeout(idleTimer);
                if (revealTimer) global.clearTimeout(revealTimer);
                idleTimer = 0;
                revealTimer = 0;
                enqueueAvailable(true);
                const finishingSteps = Math.max(1, Math.floor(
                    configuredMaximumVisualBacklogMs / Math.max(1, finishingDelayMs)
                ));
                finishingBatchSize = Math.max(1, Math.ceil(phraseQueue.length / finishingSteps));
                return new Promise(resolve => {
                    finishResolve = resolve;
                    if (maxFinishMs > 0) {
                        finishDeadlineTimer = global.setTimeout(forceFinish, maxFinishMs);
                    }
                    finishIfDrained();
                });
            },
            destroy() {
                destroyed = true;
                el.classList.remove('is-streaming');
                if (reducedMotionQuery?.removeEventListener) {
                    reducedMotionQuery.removeEventListener('change', handleReducedMotionChange);
                } else if (reducedMotionQuery?.removeListener) {
                    reducedMotionQuery.removeListener(handleReducedMotionChange);
                }
                if (revealTimer) global.clearTimeout(revealTimer);
                if (idleTimer) global.clearTimeout(idleTimer);
                if (settleTimer) global.clearTimeout(settleTimer);
                if (finishDeadlineTimer) global.clearTimeout(finishDeadlineTimer);
                resolveFinish();
            },
            snapshot() {
                return {
                    source,
                    visible,
                    queuedUntil,
                    queuedPhrases: phraseQueue.length,
                    reducedMotion,
                    renderedTokenCount,
                    stableBoundaryScannedCharacters,
                    streamPreparationScannedCharacters,
                };
            },
        };
    }

    global.NolaChatStream = Object.freeze({
        createSmoothMarkdownStreamController,
        decorateRevealTokens,
        patchChildren,
        patchHtml,
        prependMessageWindow,
        prefersReducedMotion,
        isCompactionSourceRequestCurrent,
        isConversationOperationCurrent,
        canonicalWindowSnapshotMatches,
        prepareStreamingMarkdown,
        stableBlockBoundary,
        takeSemanticPhrase,
    });
})(window);
