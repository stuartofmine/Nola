# Desktop local model acquisition

[简体中文](DESKTOP_LOCAL_MODELS.md) · English

Run hardware detection on the inference machine. Inspect CPU, usable system memory, available disk space and the GPU memory the Host can identify. Apple silicon uses a shared memory pool. The engine, model files and running inference service have independent status.

1. Choose llama.cpp, vLLM or the supported Mac MLX path, then choose Hugging Face or ModelScope. A model name narrows the search. A full repository ID preserves that choice.
2. The planner uses ModelFit recommendations where available, looks up the actual repository and checks its files and architecture. Include weights, engine overhead and context cache in memory estimates. Rank plans that fit a 131,072-token session first. At comparable capacity, prefer a verified quantized variant published by Unsloth. Network or metadata limits must be visible in the result.
3. Present publisher, revision, variant, download size, estimated memory and suggested context. Confirmation binds the selected plan. Recheck the repository revision, file set and available space before installation. A changed plan goes back to preview.
4. Stage the files, verify the snapshot and register it in the local library. A missing engine can be prepared independently of a completed download. The service becomes ready after its model catalog reports the expected ID.

| Engine | Weights and launch |
| --- | --- |
| llama.cpp | Select one fitting GGUF variant with all its shards. Start llama-server on loopback. The Windows distribution includes its engine. macOS can use the detected Homebrew installation. |
| vLLM | Use a compatible checkpoint and tokenizer, or a supported quantized MLX checkpoint on the Metal path. Engine installation depends on the executing OS and hardware. |
| MLX | Use an MLX-format model directory with the native Mac client’s MLX runtime. |

The estimated window includes input, history and generation. It is not a measurement of throughput or answer quality. Validate loading and inference separately on the intended device. [Planner](../../../../apps/cli/src/model_acquisition/planning.rs) · [Engine setup](../../../../apps/cli/src/model_acquisition/engine_setup.rs) · [Storage](../../../../apps/cli/src/model_acquisition/storage.rs)
