package com.nola.llm.local

import android.app.ActivityManager
import android.content.Context
import java.util.Locale
import java.util.TimeZone

enum class LocalModelDownloadSource(val label: String) {
    ModelScope("ModelScope"),
    HuggingFace("Hugging Face"),
}

enum class DeviceModelFit {
    Best,
    Light,
    Large,
}

data class DeviceModelCatalogEntry(
    val id: String,
    val displayName: String,
    val publisher: String,
    val quantization: String,
    val downloadUrl: String,
    val modelScopeUrl: String,
    val sha256: String,
    val byteSize: Long,
    val contextWindow: Int,
    val recommendedRamBytes: Long,
    val recommended: Boolean,
) {
    fun downloadUrl(source: LocalModelDownloadSource): String = when (source) {
        LocalModelDownloadSource.ModelScope -> modelScopeUrl
        LocalModelDownloadSource.HuggingFace -> downloadUrl
    }

    fun fitForRam(totalRamBytes: Long): DeviceModelFit {
        if (totalRamBytes <= 0L) return if (recommended) DeviceModelFit.Best else DeviceModelFit.Light
        val ratio = recommendedRamBytes.toDouble() / totalRamBytes.toDouble()
        return when {
            ratio <= 0.45 -> DeviceModelFit.Light
            ratio <= 1.05 -> DeviceModelFit.Best
            else -> DeviceModelFit.Large
        }
    }
}

object DeviceModelCatalog {
    private val chinaTimeZones = setOf(
        "Asia/Shanghai", "Asia/Chongqing", "Asia/Harbin", "Asia/Urumqi",
        "Asia/Hong_Kong", "Asia/Macau", "Asia/Taipei", "Asia/Kashgar",
    )

    fun defaultSource(
        timeZone: TimeZone = TimeZone.getDefault(),
        locale: Locale = Locale.getDefault(),
    ): LocalModelDownloadSource {
        if (timeZone.id in chinaTimeZones) return LocalModelDownloadSource.ModelScope
        val isUtcPlus8 = timeZone.rawOffset == 8 * 60 * 60 * 1000
        val isChinese = locale.language.equals("zh", ignoreCase = true)
        if (isChinese && (isUtcPlus8 || locale.country.equals("CN", ignoreCase = true))) {
            return LocalModelDownloadSource.ModelScope
        }
        return LocalModelDownloadSource.HuggingFace
    }

    fun deviceTotalRamBytes(context: Context): Long {
        val manager = context.getSystemService(Context.ACTIVITY_SERVICE) as? ActivityManager ?: return 0L
        val info = ActivityManager.MemoryInfo()
        manager.getMemoryInfo(info)
        return info.totalMem
    }

    fun formatGb(bytes: Long): String {
        val gb = bytes.toDouble() / 1_000_000_000.0
        return if (gb >= 10.0) String.format(Locale.US, "%.0f GB", gb)
        else String.format(Locale.US, "%.1f GB", gb)
    }

    val entries: List<DeviceModelCatalogEntry> = listOf(
        DeviceModelCatalogEntry(
            id = "qwen3.5-4b-q4-k-m",
            displayName = "Qwen3.5 4B",
            publisher = "Unsloth",
            quantization = "Q4_K_M",
            downloadUrl = "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
            modelScopeUrl = "https://modelscope.cn/models/unsloth/Qwen3.5-4B-GGUF/resolve/master/Qwen3.5-4B-Q4_K_M.gguf",
            sha256 = "00fe7986ff5f6b463e62455821146049db6f9313603938a70800d1fb69ef11a4",
            byteSize = 2_740_937_888,
            contextWindow = 65_536,
            recommendedRamBytes = 8L * 1024 * 1024 * 1024,
            recommended = true,
        ),
        DeviceModelCatalogEntry(
            id = "qwen3-0.6b-q4-0",
            displayName = "Qwen3 0.6B",
            publisher = "ggml.org",
            quantization = "Q4_0",
            downloadUrl = "https://huggingface.co/ggml-org/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_0.gguf",
            modelScopeUrl = "https://modelscope.cn/models/ggml-org/Qwen3-0.6B-GGUF/resolve/master/Qwen3-0.6B-Q4_0.gguf",
            sha256 = "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4",
            byteSize = 428_970_080,
            contextWindow = 65_536,
            recommendedRamBytes = 2L * 1024 * 1024 * 1024,
            recommended = false,
        ),
    )
}
