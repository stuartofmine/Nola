package com.nola.llm.local

data class DeviceModelCatalogEntry(
    val id: String,
    val displayName: String,
    val publisher: String,
    val quantization: String,
    val downloadUrl: String,
    val sha256: String,
    val byteSize: Long,
    val contextWindow: Int,
    val recommendedRamBytes: Long,
    val recommended: Boolean,
)

object DeviceModelCatalog {
    val entries: List<DeviceModelCatalogEntry> = listOf(
        DeviceModelCatalogEntry(
            id = "qwen3.5-4b-q4-k-m",
            displayName = "Qwen3.5 4B",
            publisher = "Unsloth",
            quantization = "Q4_K_M",
            downloadUrl = "https://huggingface.co/unsloth/Qwen3.5-4B-GGUF/resolve/main/Qwen3.5-4B-Q4_K_M.gguf",
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
            sha256 = "da2572f16c06133561ce56accaa822216f2391ef4d37fba427801cd6736417d4",
            byteSize = 428_970_080,
            contextWindow = 65_536,
            recommendedRamBytes = 2L * 1024 * 1024 * 1024,
            recommended = false,
        ),
    )
}
