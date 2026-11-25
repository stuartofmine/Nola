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
    id?: string;
    quantization?: string;
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

const CHINA_TIMEZONES = new Set([
    'Asia/Shanghai', 'Asia/Chongqing', 'Asia/Harbin', 'Asia/Urumqi',
    'Asia/Hong_Kong', 'Asia/Macau', 'Asia/Taipei', 'Asia/Kashgar',
]);

export function defaultModelSource(locale?: string): ModelSource {
    try {
        const tz = Intl.DateTimeFormat().resolvedOptions().timeZone;
        if (tz && CHINA_TIMEZONES.has(tz)) return 'modelscope';
        const offsetMinutes = new Date().getTimezoneOffset();
        const lang = (locale || (typeof navigator !== 'undefined' ? navigator.language : '')).toLowerCase();
        if (lang.startsWith('zh') && (offsetMinutes === -480 || !tz)) return 'modelscope';
        if (lang === 'zh-cn' || lang === 'zh-hans' || lang.startsWith('zh-hans-')) return 'modelscope';
    } catch { /* Fall back to huggingface when Intl is unavailable. */ }
    return 'huggingface';
}

export function parseDirectModelInput(raw: string): { repository: string; source?: ModelSource } | null {
    const trimmed = raw.trim();
    if (!trimmed) return null;
    try {
        const url = new URL(trimmed);
        const host = url.hostname.toLowerCase();
        const segments = url.pathname.split('/').filter(Boolean);
        if ((host === 'huggingface.co' || host === 'www.huggingface.co' || host === 'hf-mirror.com') && segments.length >= 2) {
            return { repository: `${segments[0]}/${segments[1]}`, source: 'huggingface' };
        }
        if ((host === 'modelscope.cn' || host === 'www.modelscope.cn') && segments[0] === 'models' && segments.length >= 3) {
            return { repository: `${segments[1]}/${segments[2]}`, source: 'modelscope' };
        }
    } catch { /* Not a URL; check owner/repo pattern below. */ }
    if (/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(trimmed)) {
        return { repository: trimmed };
    }
    return null;
}

export function cleanModelName(repository: string): string {
    const tail = repository.split('/').pop() || repository;
    return tail
        .replace(/[-_.]?(?:GGUF|MLX|AWQ|GPTQ|EXL2|FP8|BF16|Q\d[_A-Z0-9]*)$/i, '')
        .replace(/[-_]+/g, ' ')
        .trim() || repository;
}

export function formatUnifiedGb(bytes: number, locale: string): string {
    if (!Number.isFinite(bytes) || bytes < 0) return '';
    const gb = bytes / 1e9;
    if (gb >= 0.1) {
        return `${new Intl.NumberFormat(locale, { maximumFractionDigits: gb >= 10 ? 0 : 1, minimumFractionDigits: gb >= 10 ? 0 : 1 }).format(gb)} GB`;
    }
    return modelByteSize(bytes, locale);
}

export function friendlyRuntimeLabel(backend: string, architecture: string, locale: string): string {
    const b = backend.trim().toLowerCase();
    const isZh = locale.toLowerCase().startsWith('zh');
    if (b === 'vulkan') return isZh ? 'GPU 加速 (Vulkan)' : 'GPU accelerated (Vulkan)';
    if (b === 'cuda') return isZh ? 'GPU 加速 (CUDA)' : 'GPU accelerated (CUDA)';
    if (b === 'metal') return isZh ? 'Apple Silicon (Metal)' : 'Apple Silicon (Metal)';
    if (b === 'rocm' || b === 'hip') return isZh ? 'GPU 加速 (ROCm)' : 'GPU accelerated (ROCm)';
    if (b === 'cpu') return isZh ? '仅 CPU' : 'CPU only';
    return [backend, architecture].filter(Boolean).join(' · ');
}

export function modelFitLevel(candidate: ModelCandidate, hardware?: LocalHardware): 'best' | 'light' | 'large' {
    const budget = hardware?.unifiedMemory
        ? hardware.memoryBytes
        : (hardware?.gpuMemoryBytes ?? hardware?.memoryBytes);
    if (!budget || budget <= 0) return 'best';
    const ratio = candidate.estimatedMemoryBytes / budget;
    if (ratio <= 0.35) return 'light';
    if (ratio <= 0.72) return 'best';
    return 'large';
}

export function formatEta(remainingSeconds: number, locale: string): string {
    if (!Number.isFinite(remainingSeconds) || remainingSeconds <= 0) return '';
    const isZh = locale.toLowerCase().startsWith('zh');
    if (remainingSeconds < 60) {
        const s = Math.max(1, Math.round(remainingSeconds));
        return isZh ? `剩余 ~${s}秒` : `~${s}s left`;
    }
    const mins = Math.floor(remainingSeconds / 60);
    const secs = Math.round(remainingSeconds % 60);
    if (mins < 60) {
        return isZh ? `剩余 ~${mins}分${secs > 0 ? `${secs}秒` : ''}` : `~${mins}m ${secs > 0 ? `${secs}s ` : ''}left`;
    }
    const hrs = Math.floor(mins / 60);
    const remMins = mins % 60;
    return isZh ? `剩余 ~${hrs}小时${remMins > 0 ? `${remMins}分` : ''}` : `~${hrs}h ${remMins}m left`;
}
