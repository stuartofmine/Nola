// SPDX-License-Identifier: Apache-2.0
export type LocalEngine = 'llama.cpp' | 'vllm';
export type EngineStatus = {engine:LocalEngine; ready:boolean; canInstall:boolean; guideUrl?:string | null; reason:string};
export function engineGuideUrl(raw?: string | null): string | undefined {
    if (!raw) return;
    try {
        const link = new URL(raw);
        if (link.protocol !== 'https:' || link.port || link.username || link.password) return;
        if (link.hostname === 'brew.sh' || link.hostname === 'docs.vllm.ai' ||
            (link.hostname === 'github.com' && /^\/ggml-org\/llama\.cpp(?:\/|$)/.test(link.pathname))) return link.href;
    } catch { /* No usable official guide was supplied. */ }
}

export type ModelSource = 'huggingface' | 'modelscope';
export type LocalHardware = {
    cpu: string | null; cores: number | null; memoryBytes: number | null; gpu: string | null;
    gpuMemoryBytes: number | null; unifiedMemory: boolean | null; freeDiskBytes: number | null;
};
export type ModelPlan = Record<string, unknown> & { repository: string };
export const targetContextTokens = 128 * 1024;
type ContextLimitReason = 'target_met' | 'native_limit' | 'memory_budget' | 'metadata_unknown';
export type ModelCandidate = {
    quantized?: boolean; targetContextTokens?: number; nativeContextTokens?: number | null;
    contextLimitReason?: ContextLimitReason;
    repository: string; revision: string; downloadBytes: number; variant?: string;
    estimatedMemoryBytes: number; contextTokens: number; plan: ModelPlan;
};

/** Preserve the planner's reason. Older catalogs only reported a context value. */
export function contextLimitation(candidate: ModelCandidate): string | undefined {
    if (candidate.contextTokens >= targetContextTokens) {
        return candidate.contextLimitReason === 'metadata_unknown' ? 'local.contextUnverified' : undefined;
    }
    switch (candidate.contextLimitReason) {
        case 'native_limit':
            return candidate.nativeContextTokens ? 'local.nativeFallback' : 'local.shortFallback';
        case 'memory_budget':
            return 'local.memoryFallback';
        case 'metadata_unknown':
            return 'local.unknownFallback';
        default:
            return 'local.shortFallback';
    }
}

/** Source attribution follows the selected artifact and publisher, including non-GGUF quantizations. */
export function unslothQuantization(candidate: ModelCandidate): 'dynamic' | 'quantized' | undefined {
    const {repository, variant, plan} = candidate;
    if (repository.split('/')[0].toLowerCase() !== 'unsloth' || plan.repository !== repository ||
        !variant || variant !== plan.variant || candidate.quantized === false || plan.quantized === false ||
        !Array.isArray(plan.files) || !plan.files.length) return;
    if (plan.engine === 'llama.cpp') {
        if (!plan.files.some(file => file?.path === variant)) return;
        const filename = variant.split('/').pop() || '';
        if (/\.gguf$/i.test(filename) && /(?:^|[-._])UD-/i.test(filename)) return 'dynamic';
    }
    return candidate.quantized === true && plan.quantized === true ? 'quantized' : undefined;
}

export function repositoryUrl(source: ModelSource, repository: string): string {
    const origin = source === 'modelscope' ? 'https://modelscope.cn/models/' : 'https://huggingface.co/';
    return origin + repository.split('/').map(encodeURIComponent).join('/');
}

export function modelByteSize(bytes: number, locale: string): string {
    const [factor, unit] = bytes >= 1e9 ? [1e9, 'gigabyte'] as const
        : bytes >= 1e6 ? [1e6, 'megabyte'] as const
        : bytes >= 1e3 ? [1e3, 'kilobyte'] as const : [1, 'byte'] as const;
    return new Intl.NumberFormat(locale, { style: 'unit', unit, maximumFractionDigits: 1 }).format(bytes / factor);
}

export function modelMemorySize(bytes: number, locale: string): string {
    const gib = 1024 ** 3;
    const mib = 1024 ** 2;
    const number = new Intl.NumberFormat(locale, { maximumFractionDigits: 1 });
    return bytes >= gib ? number.format(bytes / gib) + ' GiB' : number.format(bytes / mib) + ' MiB';
}

export type ModelDownload = {
    id: string;
    engine: LocalEngine;
    source: ModelSource;
    repository: string;
    plan?: ModelPlan;
    status: 'downloading' | 'cancelling' | 'installed' | 'cancelled' | 'failed';
    downloadedBytes: number;
    totalBytes: number;
    error?: string;
};

export function downloadActive(job?: ModelDownload | null) {
    return job?.status === 'downloading' || job?.status === 'cancelling';
}

export const engineName = (engine: LocalEngine) => engine === 'vllm' ? 'vLLM' : 'llama.cpp';
export const sourceName = (source: ModelSource) => source === 'modelscope' ? 'ModelScope' : 'Hugging Face';
