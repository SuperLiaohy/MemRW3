# MemRW3 架构

## 总览

MemRW3 是 Rust 2024 + eframe/egui 桌面程序。Chart、Table 和 Debug 都实现
`MemRWPlugin`，但插件只管理 UI 状态并产生 `PluginAction`；所有硬件访问统一由
`ProbeWorker` 串行执行。

```text
ChartPlugin ── Stream demand ─┐
TablePlugin ── Latest/SVD ────┼─> MemRW3App ── ProbeCommand ──> ProbeWorker
DebugPlugin ── DebugCommand ──┘                                  │
                                                                  ├─ ProbeSession
ChartPlugin <── FrameData/RingBuffer <────────────────────────────┤
TablePlugin <── LatestValue/RegisterData <────────────────────────┤
DebugPlugin <── DebugSnapshot <───────────────────────────────────┘
```

关键约束：

- 一个物理 Probe 只有一个所有者：`ProbeWorker`。
- FullSpeed/Halt 是 Worker 的调度策略，不是线程所有权迁移。
- Chart 的流数据与 Table 的最新值完全分离。
- 静态变量继续由项目原有 DWARF 模型负责；栈帧和局部变量由
  `probe-rs-debug` 负责。
- DebugPlugin 不重复实现 SVD、静态变量、连接、复位或烧录。

## 模块

```text
src/
├── main.rs
├── app.rs                     # UI shell、插件动作和 Worker 事件编排
├── dwarf/
│   ├── extract.rs             # 静态变量 ELF/DWARF 解析
│   └── types.rs               # 静态变量树和 Extend 类型
├── model/
│   ├── debug.rs               # 可跨线程的调试命令和快照 DTO
│   ├── register_io.rs         # SVD 寄存器请求/结果
│   ├── ring_buffer.rs         # Chart 有界无锁流缓冲
│   ├── state.rs               # AppSession
│   └── variable_pool.rs       # 变量池、读取类别和 LatestValue
├── probe/
│   ├── session.rs             # 唯一 probe-rs Session 的硬件原语
│   └── worker.rs              # 命令循环、双模式调度和 DebugEngine
├── svd/
└── ui/
    ├── plugin.rs              # MemRWPlugin、PluginAction、上下文
    ├── dock.rs
    ├── chart_plugin/
    ├── table_plugin/
    ├── debug_plugin/
    └── variable_tree_panel.rs # 共享 ELF 入口及静态变量树
```

## 插件边界

`MemRW3App` 持有：

```rust
plugins: Vec<Box<dyn MemRWPlugin>>
```

默认顺序是 Chart、Table、Debug。插件每帧接收只读上下文；所有副作用通过
`PluginAction` 返回。硬件类动作包括：

```text
WriteVariable
ReadRegisters / WriteRegister
Debug(DebugCommand)
RebuildSlots
```

App 将硬件动作转换成 `ProbeCommand` 放入 Worker 队列，UI 线程不等待 Probe。
Worker 通过 `ProbeEvent` 返回连接、烧录、寄存器和调试结果。

所有插件都通过 `MemRWPlugin::on_enabled_changed` 接收 Dock 启停变化。普通插件暂停时
只停止更新和交互；DebugPlugin 暂停时还会停止 DebugEngine 状态轮询并卸载硬件断点，
但不改变 MCU 当前运行/暂停状态。DebugPlugin 内的“暂停/继续”才对应 Core Halt/Run。

## 程序映像生命周期

`VariableTreePanel` 是 ELF 的共享加载入口，并维护：

```text
loaded_elf_path
program_generation
```

成功加载 ELF 后 generation 递增。App 将路径和 generation 发给 ProbeWorker；
Worker 在线程内部构建 `probe_rs_debug::DebugInfo`。`DebugInfo` 是 `!Send + !Sync`，
所以永远不会穿过线程边界。

同一 ELF 同时服务两个互不替代的模型：

- 原有 `dwarf::extract`：静态地址变量、结构体和数组，用于 Chart/Table。
- `probe-rs-debug`：源码行、源码断点、源码级步进、CFI unwind、栈帧和 locals。

更换 ELF 时，Worker 先卸载旧地址的硬件断点，再用逻辑断点重新解析和安装。
异步结果携带 `program_generation` 和 `revision`，App 不接受旧代结果。

## ProbeWorker

ProbeWorker 是普通阻塞线程，不依赖 async runtime。它持有：

```text
ProbeSession
DebugInfo
TargetState
逻辑/已安装断点表
StackFrame + VariableCache
Acquisition slots
```

循环调度顺序：

1. 每轮优先处理有限数量的控制命令，避免控制请求或采集任一方饥饿。
2. 全局采集已经“开始”且 DebugPlugin 显式启动后，约每 50 ms 查询 `Core::status()`；
   仅连接 Probe 时不启动 DebugEngine，也不安装硬件断点。全局停止会停止 DebugEngine。
3. Running/Sleeping 时根据用户采集意图执行高速采集。
4. Halted 时停止 Stream，并约每秒执行一次 Table Latest 读取。
5. 空闲时使用有超时的通道等待，不进行忙循环。
6. 暂停/空槽或读取失败时继续做链路健康检查；连续失败后断开并发事件。

连接、断开、Reset、烧录、slot 更新、Table/SVD 请求和 Debug 请求都在这个线程
执行。旧的 `ProbeCell + Sync + acq_thread` 已移除。

### 目标状态和采集状态

目标状态独立于采集状态：

```rust
enum TargetState {
    Disconnected,
    Unknown,
    Running,
    Sleeping,
    Halted { reason: String },
    LockedUp,
}
```

`AppSession.acquisition_requested` 表示用户希望采集；`AppSession.running` 表示当前
实际正在产生 Stream 数据。断点命中时 Worker 只将 `running` 置 false，保留
`acquisition_requested`；Continue 后自动恢复。手动停止、断开或烧录会同时取消
采集意图。

## FullSpeed 与 Halt 调度

| 功能 | MCU Running/Sleeping | MCU Halted |
|---|---|---|
| Chart | 高频 Stream | 停止并保留最后画面 |
| Table 左侧 | 同轮读取并更新 Latest | 约 1 Hz Latest |
| Table 写入 | 插入采集轮次之间 | 立即排队执行 |
| SVD | 原频率调度 | 原频率调度 |
| 断点增删 | 短暂 halted access 后恢复 | 直接执行 |
| CPU registers/stack/locals | 不读取 | Halt 快照及按需展开 |
| Flash | Worker 独占 | Worker 独占 |

### Stream 与 Latest

`VariableReadClass`：

```text
Stream  -> Chart
Latest  -> Table
```

`PooledVariable` 分别记录 `stream_readers`、`latest_readers`，同时保留总绑定数和
总活动数用于生命周期管理。

FullSpeed 下，同一物理地址仍只读取一次：Stream 绑定写入 RingBuffer，Latest
绑定写入原子 `LatestValue`。Halt 下只读取含 Latest 绑定的 slot，且绝不向
RingBuffer 写入，因此不会在 Chart、FFT 或 CSV 中制造调试暂停期间的低频数据。

## DebugEngine

底层执行控制直接使用 `probe-rs 0.31`：

- `Core::halt/run/step/status`
- CPU register read
- `available_breakpoint_units`
- `set_hw_breakpoint/clear_hw_breakpoint`
- MemoryInterface 代码和栈读取

高层调试使用 `probe-rs-debug 0.31`：

- `DebugInfo::get_source_location/get_breakpoint_location`
- `DebugRegisters::from_core`
- `DebugInfo::unwind`
- `VariableCache` 的局部变量懒加载

源码步进由 Worker 自己控制，全部只使用 `Core::step()`，不使用临时断点或
`probe-rs-debug::SteppingMode` 的运行等待：

- Step Into 在 DWARF 规范化文件路径或行号首次变化时停止；列号变化不算源码级前进。
- Step Over 在源位置变化且 unwind 调用栈深度不大于起始深度时停止，因此会跨过被调
  函数。
- Step Out 在 unwind 调用栈深度小于起始深度时停止。
- 源码步进期间暂时卸载用户硬件断点，结束后恢复，避免目标指令同时命中用户 comparator
  干扰单步停止原因。

汇编由 Capstone 0.14 完成。支持 Thumb2、A32、A64、RV32 和 RV32C；Xtensa
当前返回不支持而不会导致 Worker panic。Cortex-M 的 Thumb 地址在比较 PC 和
断点时会清除 bit 0。

ELF 加载时同时缓存可执行代码段并生成离线反汇编，因此目标运行或尚未连接时也能
浏览汇编。Halt 快照优先使用匹配 PC 的 ELF 内容，只有 PC 不属于 ELF 代码段时才按
目标内存区域边界读取实时代码，避免不必要的 ARM 总线访问。

`probe-rs-debug 0.31` 会把 C++ 标记为未知语言，但仍能解析变量作用域、类型和
DWARF 位置。Worker 对 `DW_LANG_C_plus_plus*` 变量做第二阶段求值，补充基础整数、
浮点、布尔、字符、枚举、指针和位域值；结构体/数组继续使用其变量树懒加载。

### Halt 快照

Worker 仅在 `Running -> Halted`、Step 完成、切换栈帧或用户显式刷新时构建快照：

```text
DebugSnapshot
├── target_state / halt reason / PC
├── CPU registers
├── PC 附近汇编 + 源码定位
├── call frames
├── selected-frame locals
├── SP 附近原始栈内存
├── logical/resolved breakpoints
└── revision / program_generation / stop_id
```

局部变量不会进入 `VariablePool`。它们可能位于寄存器、CFA 相对内存、常量或
碎片位置，仅对当前 `stop_id` 有效。复合变量首次只展开有限深度，用户展开节点
时再发送 `ExpandVariable`。

可写局部变量只限 `VariableLocation::Address` 的标量。UI 发送的新值同时携带
`stop_id`、frame index 和 variable reference；Worker 拒绝过期请求，并通过
`Variable::update_value` 编码写入后刷新变量缓存。probe-rs-debug 将 C++ 标记为未知语言，
因此 Worker 写入 C++ 基础类型时临时按 C ABI 解析，完成后恢复原 DWARF 语言和类型元数据。

Debug 工作区底部拆分为局部变量和 CPU 寄存器两个可调整面板；右侧检查器只保留调用栈
和栈内存。中央编辑区支持源码、汇编以及二者并排模式。源码行内汇编通过现有
`Disassemble(Source)` 命令按需加载，UI 按源码路径+行号缓存结果，不扩大全局快照。

### 断点

配置保存 Debug 启动模式和逻辑断点：源码路径+行/列，或指令地址。连接和 ELF 更新时
只解析逻辑断点；DebugPlugin 启动后，以及已启动状态下的 Reset/烧录后才重新安装，
不保存 comparator 编号。

第一版只提供可移植的硬件指令断点。界面显示硬件容量、verified/unverified 和
具体错误。硬件槽用尽时不会伪装成安装成功。

“运行到光标”复用同一源码/地址解析器。若目标地址没有用户硬件断点，Worker
临时安装一个断点并运行；目标在该地址或其他原因暂停时立即清除临时断点，不将其
写入配置。

源码断点先按 DWARF 完整路径解析；若路径来自另一台构建机，则按规范化最长后缀
在工程源码清单中匹配。请求行不是可暂停行时，使用预建的 DWARF `is_stmt` 行索引
向后查找最多 32 行，并在 UI 中同时显示请求位置、实际解析位置和硬件地址。

## 静态变量与 SVD

原有职责保持不变：

- `dwarf::extract` 只接受可化简为绝对地址的静态变量位置。
- `VariablePool` 存储稳定地址和最多 8 字节的采样表示。
- Chart 负责流式历史、FFT 和 CSV。
- Table 负责静态变量树、写入和 CMSIS-SVD 外设寄存器。
- DebugPlugin 只显示 CPU core registers 和 frame locals，不再提供 globals/SVD。

## 配置和失效规则

配置文件包含 Probe 设置、ELF 路径、VariablePool 和每个插件 payload。Debug
payload 保存逻辑断点。加载配置前必须停止采集并断开目标。

以下事件会令 Halt 快照失效：Continue、停止 DebugPlugin、断开、Reset、烧录、更换
ELF。重新连接不会自动启动调试；用户选择 Attach 或 Reset 并启动后，Worker 才查询
硬件断点容量并安装逻辑断点。异步 UI 使用 request/revision/generation 防止过期结果
覆盖新状态。

## 当前限制

- 当前使用 core 0；模型保留了向多核扩展的空间，但尚未提供核心选择器。
- 硬件断点数量取决于目标 MCU；没有通用软件断点和数据 watchpoint。
- 优化构建中的变量可能被优化掉，源码级步进可能跨行。
- Cortex-M 源码级 Step Into/Over/Out 会在命令期间临时设置并回读验证 PRIMASK，防止
  普通可屏蔽中断把步进带入 ISR；NMI、Fault 和非 Cortex-M 架构仍可能影响落点。
- DebugPlugin 从 DWARF line program 构建工程源码树；构建机路径不可用时，可将
  DWARF 公共根目录映射到本机源码根目录。
- 没有物理 Probe 时只能执行纯逻辑和 UI 测试，硬件功能需在目标板上验证。

## 开发进度记录

按日期维护的 DebugPlugin 已完成项、剩余问题和后续计划见
[`DEBUG_PLUGIN_PROGRESS.md`](DEBUG_PLUGIN_PROGRESS.md)。后续继续开发前应先检查该记录和
当前工作树，避免重复实现。
