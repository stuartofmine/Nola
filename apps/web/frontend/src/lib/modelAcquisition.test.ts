// SPDX-License-Identifier: Apache-2.0
import assert from 'node:assert/strict';
import test from 'node:test';
import { modelByteSize, modelMemorySize, downloadActive, downloadSucceeded, cleanModelName, contextLimitation, unslothQuantization, repositoryUrl, engineGuideUrl, type ModelCandidate, type ModelDownload } from './modelAcquisition.ts';

test('compact download estimates preserve the scale of small and large models', () => {
    assert.match(modelByteSize(6_477_163, 'en'), /6\.5 MB/);
    assert.match(modelByteSize(8_400_000_000, 'en'), /8\.4 GB/);
    assert.equal(modelMemorySize(16 * 1024 ** 3, 'en'), '16 GiB');
    assert.equal(downloadActive(null), false);
    const makeJob = (status: ModelDownload['status']): ModelDownload => ({
        id: 'job-1', engine: 'llama.cpp', source: 'huggingface', repository: 'owner/model',
        status, downloadedBytes: 0, totalBytes: 100,
    });
    assert.equal(downloadActive(makeJob('queued')), true);
    assert.equal(downloadActive(makeJob('downloading')), true);
    assert.equal(downloadActive(makeJob('verifying')), true);
    assert.equal(downloadActive(makeJob('cancelling')), true);
    assert.equal(downloadActive(makeJob('installed')), false);
    assert.equal(downloadActive(makeJob('completed')), false);
    assert.equal(downloadSucceeded(makeJob('installed')), true);
    assert.equal(downloadSucceeded(makeJob('completed')), true);
    assert.equal(downloadSucceeded(makeJob('downloading')), false);
    assert.equal(cleanModelName(undefined), '');
    assert.equal(cleanModelName(''), '');
    assert.equal(cleanModelName('unsloth/Qwen3-8B-GGUF'), 'Qwen3 8B');
});

test('context alternatives report planner evidence and an independent target', () => {
    const model: ModelCandidate = {repository:'demo/model',revision:'a',downloadBytes:1,
        estimatedMemoryBytes:1,contextTokens:2048,plan:{repository:'demo/model'}};
    assert.equal(contextLimitation(model), 'local.shortFallback');
    assert.equal(contextLimitation({...model, targetContextTokens:2048, contextLimitReason:'target_met'}), 'local.shortFallback');
    assert.equal(contextLimitation({...model, contextLimitReason:'native_limit'}), 'local.shortFallback');
    assert.equal(contextLimitation({...model, nativeContextTokens:2048, contextLimitReason:'native_limit'}), 'local.nativeFallback');
    assert.equal(contextLimitation({...model, contextLimitReason:'memory_budget'}), 'local.memoryFallback');
    assert.equal(contextLimitation({...model, contextLimitReason:'metadata_unknown'}), 'local.unknownFallback');
    assert.equal(contextLimitation({...model, contextTokens:131072, contextLimitReason:'target_met'}), undefined);
});

test('Unsloth Dynamic attribution requires a selected UD GGUF in the actual publisher namespace', () => {
    const snapshot = (repo: string, variant: string): ModelCandidate => ({
        repository:repo,revision:'a',downloadBytes:100,estimatedMemoryBytes:150,contextTokens:131072,variant,
        plan:{repository:repo,engine:'llama.cpp',variant,files:[{path:variant}]}});
    const model = snapshot('Unsloth/Qwen-GGUF','Qwen-UD-Q4_K_M.gguf');
    assert.equal(unslothQuantization(model), 'dynamic');
    assert.equal(unslothQuantization({...model,quantized:false}), undefined);
    assert.equal(unslothQuantization(snapshot('mirror/unsloth-Qwen','Qwen-UD-Q4_K_M.gguf')), undefined);
    assert.equal(unslothQuantization(snapshot('unsloth/Qwen-GGUF','Qwen-IQ4_XS.gguf')), undefined);
    assert.equal(unslothQuantization({...model,plan:{...model.plan,files:[]}}), undefined);
    assert.equal(unslothQuantization({...model,plan:{...model.plan,engine:'vllm'}}), undefined);
    const standard = snapshot('unsloth/Qwen-GGUF','Qwen-Q4_K_M.gguf');
    assert.equal(unslothQuantization({...standard,quantized:true,plan:{...standard.plan,quantized:true}}), 'quantized');
    assert.equal(repositoryUrl('modelscope',model.repository), 'https://modelscope.cn/models/Unsloth/Qwen-GGUF');
});

test('engine setup links open only the official guide supplied by the Host', () => {
    for (const url of ['https://github.com/ggml-org/llama.cpp/blob/master/docs/build.md',
        'https://docs.vllm.ai/en/latest/getting_started/installation/', 'https://brew.sh/']) {
        assert.equal(engineGuideUrl(url),url);
    }
    for (const url of ['javascript:alert(1)','https://docs.vllm.ai.invalid.example/','https://u:p@brew.sh/',
        'http://brew.sh/','https://github.com/ggml-org/llama.cpp-elsewhere','#',null]) assert.equal(engineGuideUrl(url),undefined);
});
