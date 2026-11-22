# 桌面本地模型

简体中文 · [English](DESKTOP_LOCAL_MODELS.en.md)

在 macOS 和 Windows 的「模型 → 本地模型」中，Nola 先读取执行推理的电脑配置，再按上次选择的引擎和下载源自动获取推荐。输入名称可以缩小范围，输入完整的 `组织/仓库` 可以预览指定版本。

1. 宿主机读取 CPU、内存、可识别的 GPU 及模型缓存所在卷的剩余空间。Apple 芯片的 CPU 与 GPU 共享一份内存。独立显卡使用可确认的专用显存。
2. 留空模型名时，Nola 优先考虑 ModelFit 为相应及较低内存档位列出的模型，再用其开放数据补充候选。在本机校验适配度后，Nola 到选定的 Hugging Face 或 ModelScope 核对真实仓库。[ModelFit 内存分档推荐](https://modelfit.io/stats/) · [兼容性数据集，CC BY 4.0](https://modelfit.io/data/)。若外部数据不可用或找不到可安装的匹配，Nola 会查询所选模型站的目录，并在结果中标明来源。硬件详情留在本机。
3. 每个方案列出发布者、模型变体、下载体积、预计运行内存和建议上下文长度。估算会给系统、引擎和上下文缓存留出空间。推荐操作只读取目录、配置与分片索引。只有一个结果或用户输入完整仓库名时，也需要手动确认。
4. 用户确认具体方案后，宿主机复核仓库版本、文件、容量和参数。方案有变化时需重新预览。下载完成后，本地模型库保存选定的变体和建议上下文，并优先使用用户已有的显式参数。用户可以随后选择模型用于聊天。

### 首次准备引擎

选中引擎后，Nola 在运行模型的电脑上检测可执行文件。引擎已经就绪时，可以直接选模型。Mac 上如果已有 Homebrew，点击安装按钮可直接安装 llama.cpp；Apple 芯片且系统为 macOS 15 或更新版本时，也可安装 vLLM-Metal。安装成功后立即重新检测。缺少 Homebrew 时，可以打开安装指南，或在 Apple 芯片 Mac 客户端选内置 MLX。手动装好引擎后，回到当前页面重新检测即可。[llama.cpp 的 Homebrew 安装包](https://formulae.brew.sh/formula/llama.cpp) · [vLLM-Metal 安装说明](https://docs.vllm.ai/projects/vllm-metal/en/stable/installation/)

Windows 桌面包随附 llama.cpp。单独运行 Web Host 时使用宿主机上可执行的引擎。远程访问者可以查看状态和准备模型权重；安装系统软件需要在宿主机上操作。下载好的权重会留在模型库，准备好引擎后可以继续使用。

### 推荐窗口与量化选择

推荐目标为单个会话的 **128K（131072 tokens）**，输入、历史和生成内容共用这个窗口。宿主机先核对模型声明的上下文上限，再用架构配置估算 KV 缓存、引擎开销和权重占用。量化仓库缺少架构配置时，可以读取它在同一模型站声明的上游配置。预览保留所用配置的仓库与版本。对采用混合注意力的受支持模型，还计入循环层的固定状态。此处展示容量估算；吞吐量和长文回答质量需要用实际任务衡量。[llmfit 的窗口与内存估算](https://github.com/AlexsJones/llmfit/blob/main/llmfit-core/src/fit.rs) · [llama.cpp 的模型适配工具](https://github.com/ggml-org/llama.cpp/blob/master/tools/fit-params/README.md)

预计能达到 128K 的方案排在前面。同等窗口下，优先推荐所选模型站上由 `unsloth` 发布、且权重确为量化格式的仓库。llama.cpp 在给上下文留出空间后再挑选 GGUF 变体，可用时优先采用对应档位的 Unsloth 动态量化。指定完整仓库名会保留用户选择。模型原生窗口较短、设备内存不足或架构资料不全时，界面分别说明原因并给出较短的建议窗口。用户仍可在模型参数里调整。llama.cpp 启动一个推理槽，让整个窗口供单个会话使用。[Unsloth 模型目录](https://unsloth.ai/docs/get-started/unsloth-model-catalog)

| 引擎 | 获取的文件 | 启动入口 |
| --- | --- | --- |
| llama.cpp | 与设备内存相适配的一种 GGUF 量化及其全部权重分片 | Windows 安装包内的引擎，或本机可执行的 `llama-server` |
| vLLM | safetensors 权重、配置和分词器；Apple 芯片上的 Metal 后端也可选择兼容的 MLX 量化目录，预览时核实量化参数 | 与本机硬件和系统兼容的 `vllm` 可执行文件 |
| MLX（Mac） | MLX 格式的模型目录 | Mac 客户端的 MLX 运行时 |

下载完成后，模型出现在本地库。Mac 可从已下载模型直接发起使用该模型的新对话。Web 和 Windows 会在缺少引擎的模型旁显示“设置引擎”，准备好后可启动服务。选中模型可以调整上下文长度。每个安装项按引擎、来源及仓库区分。模型下载和引擎安装是两个独立状态。如果启动时找不到外部引擎，可将其加入桌面应用的 PATH，或为应用设置 `NOLA_LLAMA_SERVER`、`NOLA_VLLM_BIN` 指向对应的可执行文件。引擎的系统要求见 [vLLM 安装文档](https://docs.vllm.ai/en/stable/getting_started/installation/) 和 [llama.cpp 服务文档](https://github.com/ggml-org/llama.cpp/tree/master/tools/server)。

桌面客户端共用 Rust 下载器。它读取所选来源的仓库文件清单，按版本下载到模型缓存的暂存目录，完成文件校验后再发布快照并更新目录。llama.cpp 从第一个权重分片加载，vLLM 从完整快照目录加载。服务绑定回环地址，在模型目录报告本次启动的模型 ID 后才进入可用状态。
