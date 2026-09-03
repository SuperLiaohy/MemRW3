# MemRW3 — 工作流程与布局架构

## 项目概述

MemRW3 是一个基于 Rust + egui + probe-rs 的嵌入式内存读写与变量监控工具，是对原 Qt/QML MemRW2 的重构。使用 gimli/object 替代 libdwarf 解析 DWARF 调试信息（支持 DWARF 2/3/4/5），使用 probe-rs 替代 libusb 手动协议解析进行 MCU 数据采集，使用 eframe + VS Code 风格左侧插件栏 + pop-out 布局替代 Qt QML 实现 UI；Chart/Table 作为内置 `MemRWPlugin` 默认停靠在主界面，Pop out 后使用 egui multi-viewport 创建原生操作系统窗口。

## 整体布局

```
┌────┬─────────────────────────────────────────────────────────┐
│ 📈 │ 控制栏 (Control Bar)                                    │
│ 📋 │ [连接/断开] [开始/暂停] [⚙设置] [延迟] [Reset] [烧录固件] [保存] [加载] [主题] Hz: xxx │
│    ├─────────────────────────────────────────────────────────┤ ← 模态阻塞: 右侧不可交互
│    │ 当前插件内容区 (默认 Pop in, 可弹出 OS 窗口)             │
│    │ Chart: 坐标轴+曲线+图例 / Table: Name|Value|Write       │
│    │                                                         │
│    ├─────────────────────────────────────────────────────────┤ ← BottomSheet 覆盖层 (可交互)
│    │ [ELF 文件: ________] [浏览] [加载] [追踪]               │
│    │ ─────────────────────────────────────────────────────   │
│    │ 变量列表 (DWARF Tree)                         [关闭]    │
│    │ ┌────────────────────────┬────────────────────────────┐ │
│    │ │ Search: [________]      │ 属性                       │ │
│    │ │ [All] [Search]          │ ── Basic (只读, DWARF原始) ─ │ │
│    │ │                         │ Name: xxx  Address: xx     │ │
│    │ │ Tree View (默认折叠)     │ Size: xx    Type: xxx      │ │
│    │ │  ├─ cu_name             │ ── Extend (可编辑) ──      │ │
│    │ │  │  ├─ var1             │ Name: [edit] Address: [hex]│ │
│    │ │  │  └─ struct           │ Size: auto Type: [u32 ▼]   │ │
│    │ │  │     └─ member        │ [添加到 Chart/Table]       │ │
│    │ │  └─ cu_name2            │ (type=other 时禁止添加)     │ │
│    │ └────────────────────────┴────────────────────────────┘ │
└────┴─────────────────────────────────────────────────────────┘
```

Control Bar 在未连接时保持中性底色，连接后暂停使用 warning 色，采集中使用 success 色；状态同时作用于整条 bar 的底色/描边和开始/暂停按钮，并由当前主题 palette 派生。主题按钮在深色、浅色、跟随系统三态间切换，深浅两套自定义 Style 均预先安装，因此系统主题变化可直接切换完整 palette。状态灯使用固定尺寸绘制图形，运行/暂停切换不会改变工具栏高度。

## 模块架构

```
src/
├── main.rs                 # 入口: 启动空 DwarfState → eframe
├── app.rs                  # 主 App + MemRW3App (控制栏/采集/连接/插件池/配置编排)
├── sync.rs                 # 同步原语: Sync (两阶段握手) - 匹配 MemRW2 的 3-semaphore 模式
├── dwarf/
│   ├── mod.rs              # DWARF 模块入口
│   ├── types.rs            # TreeNode/BasicType/ExtendType/ExtendConfig/CuInfo/DwarfState/TypeRef
│   └── extract.rs          # DWARF 解析 (gimli, 支持 DWARF 2/3/4/5), 跨编译单元类型引用, basic_type 映射
├── model/
│   ├── mod.rs
│   ├── register_io.rs      # 独立 SVD 寄存器批量读请求、结果与写请求
│   ├── state.rs            # AppSession (连接/采样/Probe UI 状态)
│   ├── variable_pool.rs    # VariablePool (Vec + HashMap, O(1) 增删查, 仅存extend数据)
│   └── ring_buffer.rs      # 有界 lock-free 环形队列 (crossbeam ArrayQueue)
├── probe/
│   ├── mod.rs              # ProbeCell (Mutex 保护的 ProbeSession owner)
│   └── session.rs          # ProbeSession + AcqSlot (probe-rs 连接/采集/读写)
├── svd/
│   └── mod.rs              # CMSIS-SVD 解析、数组/继承展开、轻量寄存器树
└── ui/
    ├── mod.rs
    ├── control_bar.rs      # 控制栏 (连接/采集/Probe配置Dialog)
    ├── dock.rs             # VS Code 风格左侧插件栏 + egui multi-viewport 原生窗口 Pop out/in
    ├── plugin.rs           # MemRWPlugin trait + PluginAction/FrameData/插件配置 payload
    ├── theme.rs            # 统一 palette + egui Visuals/WidgetVisuals 配置
    ├── variable_tree_panel.rs # 绑定 viewport 的 DWARF 变量树覆盖组件
    ├── chart_plugin/
    │   ├── mod.rs
    │   ├── legend.rs       # ChartLegend (曲线名/颜色/可见/缓冲/data_history)
    │   ├── fft.rs           # FFT 频谱计算 (自包含: Complex/Radix-2/Hann/Hamming/Blackman/矩形窗, 最大65536点)
    │   ├── panel.rs        # 图表插件实现 (时域+频域, 坐标轴/曲线/图例/光标/ExtendType解码/自定义颜色/Log CSV)
    │   └── line_dialog.rs  # 曲线属性 Dialog (编辑曲线属性 + 显示PooledVariable的Extend属性)
    ├── table_plugin/
    │   ├── mod.rs
    │   ├── panel.rs        # 左侧递归变量树 + 右侧 SVD 的双面板编排
    │   ├── tree.rs         # TableNode/叶子绑定/父子勾选/递归配置
    │   └── svd_panel.rs    # 后台加载与展示 SVD 外设/寄存器/字段
    ├── vari_tree.rs        # DWARF 变量树 (左面板, 搜索自动滚动, DefaultOpen(false) 折叠)
    └── vari_properties.rs  # 属性面板 (Basic/Extend/Add 三段竖直布局, ExtendConfig驱动)
```

## 核心数据类型

### TreeNode (DWARF 原始数据, 不存extend)

```
TreeNode {
    // ── Basic (DWARF 原始属性, 只读) ──
    id: usize, parent_id: Option<usize>,
    name: String, struct_name: Option<String>,
    type_name: String, basic_type: BasicType,
    address: u64,                              // top-level: DWARF绝对地址; field: DWARF offset; array elem: size*index
    size: u32, children: Vec<TreeNode>,
}
```

- `parent_id`：指向父节点，用于数组元素的 `parent_array_info()` 向上查找
- `address` 的语义：
  - 顶层变量：存储 DWARF 绝对地址（相当于根 base）
  - 结构体成员/嵌套字段：存储 DWARF `data_member_location` 的**原始 offset**
  - 数组元素 `[]`：初始存储 `0`，通过搜索或属性面板 DragValue 修改为 `elem_size * index`，表示数组内偏移
- 数组元素采用**惰性求值**：默认只创建 `[0]` 占位节点，用户通过搜索（如 `A[2]`）或在属性面板修改 Index 后，`perform_search` 和 `app.rs` 中的后处理逻辑将节点名和地址更新为正确的 `[idx]` 和偏移量
- extend 不存储在 TreeNode 中，改为通过 `DwarfState` 的遍历方法动态计算
- `compute_extend_name()`: 从根开始逐级拼接变量名得到完整路径，如 `my_struct.arr[2]`
- `compute_extend_address()`: 从根开始逐级累加 offset 得到实际绝对地址
- `find_path_to_node()`: 子节点名以 `[` 开头时不加 `.` 号，直接拼接为 `arr[2]` 格式

### ExtendConfig (用户可编辑的 Extend 数据)

```rust
pub struct ExtendConfig {
    pub name: String,               // 初始由 compute_extend_name() 计算，用户不可手动编辑
    pub address: u64,               // 初始由 compute_extend_address() 计算，用户可编辑
    pub ext_type: ExtendType,       // 初始由 basic_type_to_extend() 推导，用户可编辑
    pub size: u32,                  // 初始 = node.size，随 ext_type 自动绑定，不可手动编辑
    pub array_index: Option<u64>,   // 数组元素当前索引 (None=非数组元素)
    pub array_count: Option<u64>,   // 数组元素总长度
}
```

- 存储在 `VariableTreePanel.extend_configs: HashMap<usize, ExtendConfig>`，按 node_id 索引
- `vari_properties_ui()` 通过 `&mut ExtendConfig` 读写
- 首次选择节点时惰性初始化
- `DwarfState.selected_node` 仅保存 node id；属性面板借用节点，避免每帧深克隆子树
- 数组元素时 index 从所选节点名称解析同步（含搜索后更新）
- "添加到 Chart/Table" 时，ExtendConfig 被消耗并存入 `VariablePool.add(config)`

### PooledVariable (池中仅存 extend 数据)

```rust
pub struct PooledVariable {
    pub id: usize,
    pub name: String,          // extend_name
    pub address: u64,          // extend_address
    pub ext_type: ExtendType,  // extend_type
    pub size: u32,             // extend_size
    pub incoming: Arc<RingBuffer<(f64, [u8; 8])>>,  // 无锁环形队列, 采集线程push, UI线程drain
    pub plugins_cnt: usize,    // 总绑定数
    pub active_readers: usize, // 启用读取的绑定数
}
```

- 不再包含 `TreeNode`，只存实际用于采集和显示的数据
- `VariablePool.add(&ExtendConfig)` 创建条目
- `incoming` 通过 `Arc` 共享: rebuild_slots 时 clone 到 `VarSlotMapping.incoming`, 采集线程无锁写入, UI 线程无锁 drain；满载时丢弃最旧样本
- 去重: 添加变量前检查 `Pool.find_compatible(name, address, type, size)`，避免错误共享解码类型
- Chart/Table 面板直接使用 `var.ext_type` 进行值解码和格式化

### MemRWPlugin (动态插件分发)

`MemRW3App` 不再持有固定的 `chart_state` / `table_state` 字段，而是持有:

```rust
plugins: Vec<Box<dyn MemRWPlugin>>
```

内置插件按顺序创建为 Chart、Table。App shell 左侧 Activity Bar 从窗口顶部贯穿到底部，负责按插件 id 切换当前插件；右侧区域承载控制栏和当前插件内容。Dock、BottomSheet、变量添加、删除、写入、Toast、配置保存/加载都通过 trait object 统一分发，不再通过 `DockTab` enum 或 Chart/Table 专用分支判断。

Dock 为每个插件维护独立暂停状态。暂停按钮位于 docked 与 pop-out 标题栏；暂停后跳过该插件的 `update()`，并禁用内容区交互，但保留最后渲染数据、已打开的变量树和其他插件的运行状态。

变量树的遮罩和 Bottom Sheet 使用 `Order::Middle`，高于普通 Panel，但为 `Order::Foreground` 的 Toast 和弹出控件保留顶层，避免提示消息被整屏变量树遮挡。

变量树目标保存请求插件、宿主 viewport 和 pop-out 状态。弹出后由 App 统一定位对应 `MemRWPlugin` 并在固定的独立 viewport 渲染，因此切换或暂停插件不会关闭变量树；原生系统标题栏仅使用 ASCII 标题，避免缺少 CJK 字体的平台显示乱码。新的打开请求在已弹出时仅更新目标插件并保持 Pop out，未弹出时才在请求 viewport 底部显示。Pop in 会聚焦对应插件并恢复为主窗口 Bottom Sheet。

```rust
pub trait MemRWPlugin {
    fn id(&self) -> &'static str;
    fn title(&self) -> &'static str;
    fn render(&mut self, ui: &mut Ui, ctx: PluginRenderContext<'_>) -> Vec<PluginAction>;
    fn update(&mut self, ctx: PluginUpdateContext<'_>) -> Vec<PluginAction>;
    fn reset_data(&mut self);
    fn add_variable_ui(&mut self, ui: &mut Ui, node_id: usize, default_name: &str, candidate: &mut dyn FnMut() -> Result<VariableCandidate, String>, pool: &mut VariablePool) -> Result<bool, String>;
    fn save_config(&self, pool: &VariablePool) -> serde_json::Value;
    fn load_config(&mut self, payload: &serde_json::Value, pool: &mut VariablePool) -> Result<(), String>;
}
```

`PluginAction` 是插件向 App 发起副作用的唯一通道:

```rust
OpenVariableTree { plugin_id, viewport_id }
RemoveVariable { var_id, was_enabled }
SetVariableEnabled { var_id, enabled }
WriteVariable { var_id, value }
ReadRegisters { requests }
WriteRegister { request }
ResetTimer
Toast { level, message }
```

App 仍然是唯一执行硬件读写、变量池解绑、timer reset 和 toast 的编排层。插件只声明意图并管理自身 UI 状态。SVD 寄存器不进入 `VariablePool` 或采集 slots；Table 插件按每个寄存器的 1–30 Hz 周期汇总到期项，App 将整批请求串行交给一个 probe core handle，并把独立结果表回传给插件。

### BasicType vs ExtendType

| BasicType (DWARF 原始) | ExtendType (用户可选) |
|---|---|
| U8, U16, U32, U64 | U8, U16, U32, U64 |
| I8, I16, I32, I64 | I8, I16, I32, I64 |
| Float, Double | Float, Double |
| Pointer → U64 | — |
| Struct(String) → Other | Other |
| Other(String) → Other | — |
| ArrayElem(Box\<BasicType\>, u64) → Other | — |

**关键规则**: `extend_type = Other` 的变量**不可添加到 Chart 或 Table**。

### ExtendType ↔ Size 自动绑定

| ExtendType | 默认 Size |
|---|---|
| U8, I8 | 1 |
| U16, I16 | 2 |
| U32, I32, Float | 4 |
| U64, I64, Double | 8 |
| Other | 不自动设置 |

切换 Type 时 Size 自动更新，用户不可手动编辑 Size。

## 完整工作流

### 1. 启动

```
main.rs
  ├─ MemRW3App::new(DwarfState::new(Vec::new()), egui_ctx) → 启动空 DwarfState
  └─ eframe::run_native() → 启动 UI (无预加载数据)

用户操作:
  └─ BottomSheet 顶部 [ELF文件: ________] [加载]
      ├─ fs::read() → 读取 ELF 文件
      ├─ object::File::parse() → 解析 ELF
      ├─ dwarf::load_dwarf() → 加载 DWARF sections（缺失 section 返回空数据，兼容 DWARF 2/3/4/5）
      └─ dwarf::collect_cus() → 两阶段遍历:
          ├─ Pass 1: 遍历所有 CU，提取 struct/union/class type 定义（type_defs HashMap）
          └─ Pass 2: 遍历所有 CU，提取变量树
              ├─ 对每个 DW_TAG_variable：获取 DW_AT_location（静态地址）
              ├─  获取 DW_AT_type → resolve_type()
              │   ├─ UnitRef（同 CU 内引用）→ resolve_type_impl() 递归解析
              │   └─ DebugInfoRef（跨 CU 引用，DW_FORM_ref_addr）
              │       └─ find_unit_for_debug_info_ref() → 定位目标 CU
              │           → resolve_type() 重新开始递归解析
              └─ type_name_to_basic_type() 根据 type name/size/kind 映射 BasicType
              └─ build_variable_node() / build_field_node() 构建 TreeNode 树
                  ├─ struct/union/class → struct_fields() 递归展开成员
                  └─ 数组类型 → 创建 [0] 占位节点，元素类型写入 ArrayElem
```

不再通过命令行参数传入 ELF 路径。App 启动时 DwarfState 为空，用户通过 BottomSheet 顶部文件选择器加载。

### 2. 连接 MCU

```
控制栏 → "设置" Dialog
  ├─ 选择 MCU 型号 (STM32F407VG / STM32H743ZI / nRF52840_xxAA / ...)
  ├─ 选择协议 (SWD / JTAG)
  ├─ 设置速度 (100-20000 kHz)
  └─ 点"连接" → Session::auto_attach(chip_name, config)

ProbeSession.connect()
  └─ probe_rs::Session::auto_attach(chip_name, SessionConfig { speed, protocol })
      → 自动查找 Probe (CMSIS-DAP/ST-Link/J-Link) 并连接目标
```

### 3. 浏览变量树

```
任一插件点击 "📋 打开变量树" → 动作携带 `plugin_id + viewport_id` → `VariableTreePanel` 仅在触发 viewport 内覆盖显示

BottomSheet (viewport 内模态覆盖层, 只阻止当前窗口交互)
   ├─ 顶部: ELF 文件路径输入框 + [浏览] (rfd 文件选择器, *.elf;*.axf) + [加载] + [追踪] 按钮 + 错误提示

   ├─ 左面板: vari_tree_ui()
   │   ├─ 搜索栏: 输入变量名 → 层级递进匹配 → 高亮 + 自动展开查找路径
   │   │   └─ 搜索规则: "." 分割层级, 非末层精确匹配(忽略大小写), 末层模糊匹配
   │   │   └─ 数组搜索: `A[2]` / `A[0][0]` 自动展开为独立层级, 匹配时校验 index 范围
   │   ├─ 搜索后自动居中滚动到第一个结果 (scroll_target_id + viewport_h 居中计算)
   │   ├─ All/Search 模式切换 (切回 All 时自动全部折叠)
   │   └─ egui_ltreeview::TreeView (NodeBuilder::default_open(false)):
   │       DWARF 编译单元 → 变量 → 结构体成员 → 数组元素 `[]` (递归, 默认折叠)
   │       └─ 多维数组: `float[7][7]` 逐层展开为 A→[i]→[j], 每层独立节点

   └─ 右面板: vari_properties_ui(config: &mut ExtendConfig) — 三段竖直布局
        ├─ Basic (只读): Name / Index(仅数组元素, DragValue可编辑) / Address(offset) / Size / Type
        │   └─ 数组元素: Name=`[index]`, Address=`size*index`, 树节点名同步更新
        ├─ Extend (可编辑): Name(只读label) / Address(hex TextEdit) /
        │   Size(只读label, 随Type自动绑定) / Type(ComboBox: u8~u64, i8~i64, float, double, other)
        └─ Add:
           ├─ Chart: 仅标量；曲线名 + 颜色 → 添加到 Chart
           └─ Table: 标量、结构体或数组；复合节点递归物化全部可读叶子

      添加流程:
        ├─ extend_name 和 extend_address 由 DwarfState 从 DWARF 树计算得到
        ├─ 用户可在 Extend 段编辑 address/type (size 自动绑定)
        ├─ 编辑结果存入 ExtendConfig (VariableTreePanel 内部 HashMap)
        ├─ 插件按钮真正点击后才构建 `VariableCandidate`，避免每帧物化大数组
        ├─ 根据 active plugin id 查找 `Box<dyn MemRWPlugin>`
        └─ 调用 `plugin.add_variable_ui(...)`
            ├─ Chart: 曲线名 + 颜色 → 存入 ChartLegend (颜色persist via egui memory)
            └─ Table: 递归 TableNode → 标量叶子按 name/address/type/size intern 到 VariablePool
```

### 4. 数据采集 (多线程架构)

#### 线程模型

```
┌─ 主线程 (UI) ───────────────────────────────────────────────┐
│  egui frame loop:                                            │
│    request_repaint() ← 持续刷新                               │
│    drain_into RingBuffer → 复用 FrameData                      │
│    sync.send_request(|| { probe操作 }) ← 同步时阻塞主线程       │
└──────────────────────────────────────────────────────────────┘
         ↑ ↓ Sync 握手                     ↑ ↓ Arc<RingBuffer>
┌─ 采集线程 (acq_thread) ───────────────────────────────────────┐
│  loop:                                                       │
│    sync.try_acquire() ← 非阻塞检查同步请求                     │
│    if running:                                                │
│      acquire_from_slots() → push to RingBuffer ← 无锁写入     │
│      thread::sleep(delay_us) ← 采集节流                       │
└──────────────────────────────────────────────────────────────┘
```

#### Sync 握手协议 (双 Condvar 设计)

```
主线程 send_request(闭包):                 采集线程 try_acquire():
  1. request_pending = true                 1. if request_pending:
  2. cv_main.wait ← paused=false                cv_mutex.paused = true
                                                cv_main.notify → 唤醒主线程
  3. cv_main返回 (paused=true)                  cv_worker.wait ← done=false
  4. 执行闭包 (独占 probe)                      ↓ 阻塞
  5. done_mutex.done = true                  5. cv_worker返回 (done=true)
     cv_worker.notify → 唤醒采集线程             done = false, 恢复运行
     request_pending = false
```

两个 Condvar 独立: `cv_main` (主线程等) 和 `cv_worker` (采集线程等)，消除共享单 Condvar 的死锁风险。

- **正常运行时**: 采集线程全速采集，主线程无锁 drain 数据渲染。两线程无交互。
- **同步操作时** (连接/断开/复位/写入/更新slots): 主线程通过 `sync.send_request` 暂停采集线程后独占 probe，完成后恢复。烧录由后台任务调用同一握手，在进度 Modal 阻止其他 probe 操作。
- **链路监控**: 采集失败时以及暂停/空槽状态下每 500 ms 读取一次 Core 状态；连续 3 次失败后采集线程释放 Probe、停止 running，通过单容量事件通道通知主线程并主动请求重绘，主线程同步连接状态并显示 Toast。

#### 数据流 (无锁路径)

```
[变量树添加变量]
  ├─ 构建 VariableCandidate (结构/数组保留层级)
  ├─ Pool.find_compatible(name, addr, type, size) → 复用或创建叶子
  └─ plugin bind → plugins_cnt += 1; active_readers += 1

[点击"开始"]
  ├─ first start 或 after clear → reset_timer() (sync)
  └─ rebuild_slots() → sync → acq_thread.slots + var_mappings

采集线程 (每轮):
  Phase 1: read32 all slots → 复用 Vec<[u8; 4]>
  Phase 2: for mapping in var_mappings:
    按 slot_indices 组装值 → mapping.incoming.push((ts, val))
  cycle_count.fetch_add(1) → Hz 统计

UI 线程 (每帧开始):
  for var in pool.iter():
    var.incoming.drain_into(&mut frame_data[var.id])
    // FrameData HashMap 和各 Vec 跨帧复用容量

  for plugin in all_plugins:
    plugin.update(&frame_data) ← 所有插件每帧摄取一次，不依赖当前可见/Pop out 状态
  active/popped plugin.render() ← 只负责 UI
```

**插件删除 → 解绑**:

```
remove_legend/root → 插件为每个叶子返回 RemoveVariable { var_id, was_enabled }
App.handle_plugin_actions:
  PooledVariable.plugins_cnt -= 1
  enabled binding 同时 active_readers -= 1
  if plugins_cnt == 0 → pool.remove(var_id)
  批量动作处理完后只 rebuild_slots 一次
```

#### ProbeCell + 编译器约束的 Core 生命周期

`ProbeSession` 由 `Arc<ProbeCell>` 共享，`ProbeCell` 内部使用 `Mutex<ProbeSession>`。正常采集时 `Sync` 握手令锁保持无竞争；Mutex 同时为意外重叠的控制请求提供内存安全兜底。

```rust
pub struct ProbeCell(Mutex<ProbeSession>);

pub fn with_mut<R>(&self, f: impl FnOnce(&mut ProbeSession) -> R) -> R {
    let mut session = self.0.lock().unwrap_or_else(|e| e.into_inner());
    f(&mut session)
}
```

每次 probe 操作在局部作用域调用 `session.core(0)`，`Core<'_>` 在同一线程、同一借用作用域内析构。不再使用 `transmute` 构造 `'static` Core，也不存在可移动自引用或跨线程析构。

`Sync::send_request` 使用 request mutex 串行化多个请求，并在请求闭包 panic 时先唤醒采集线程再恢复 panic，避免采集线程永久等待。

`AcqSlot` 缓存变量地址/大小/类型, 采集线程无需持有 `VariablePool` 锁:

```rust
pub struct AcqSlot {
    pub address: u64,  // 32-bit 对齐地址, 去重: 多变量可共享同一地址
}
```

`VarSlotMapping` 将一个 `PooledVariable` 映射到其 `AcqSlot` 集合:

```rust
pub struct VarSlotMapping {
    pub slot_indices: Vec<usize>,   // 该变量引用的 slot_values 索引
    pub size: u32,                   // 变量总大小
    pub byte_offset: usize,          // 变量地址在首槽位中的字节偏移
    pub incoming: Arc<RingBuffer<(f64, [u8; 8])>>,
}
```

**rebuild_slots 算法** (每次连接/变量变更时在主线程通过 sync 执行):

```
1. 遍历 VariablePool 中每个 PooledVariable:
   a. slot_addresses(var.address, var.size) → 计算覆盖该地址范围的 32-bit 对齐地址列表
      - u32@0x2000_0000 → [0x2000_0000]
      - u64@0x2000_0000 → [0x2000_0000, 0x2000_0004]
      - u8@0x2000_0001  → [0x2000_0000] (byte_offset=1)
   b. 去重: HashMap<u64, usize>, 地址仅在 rebuild 时映射为连续索引
   c. 构建 VarSlotMapping { slot_indices, size, byte_offset, incoming }
2. 将 Vec<AcqSlot>、Vec<VarSlotMapping> 写入 ProbeSession，并复用同长度 slot_values
```

**两阶段采集** (acq_thread):

```
Phase 1: 读取全部去重槽位
  for (index, slot) in slots:
    slot_values[index] = core.read_word_32(slot.address).to_le_bytes()

Phase 2: 组装变量值 → push RingBuffer
  for mapping in var_mappings:
    val = [0u8; 8]
    for (i, slot_index) in mapping.slot_indices:
      sv = slot_values[slot_index]
      if i == 0: copy sv[mapping.byte_offset..]  → val
      else:      copy sv[..]                      → val
    mapping.incoming.push((ts, val))
```

#### RingBuffer (有界 lock-free 环形队列)

```rust
pub struct RingBuffer<T> {
    queue: crossbeam_queue::ArrayQueue<T>,
}
// push() → 采集线程; drain_into() → UI 线程，可同时执行
// 固定容量 2560；满载时覆盖最旧样本
// drain_into() 复用目标 Vec；discard_all() 清空时不分配
```

#### 延迟控制 + 计时规则

`delay_us: Arc<AtomicU64>` 共享: 主线程 slider 写入, 采集线程读取 → `thread::sleep(delay_us)` 控制采集频率。

- **默认 0** (全速采集, 仅受 probe/USB 和每轮 Core 获取开销限制)
- 主线程仅 `request_repaint()` 以 vsync 刷新 UI, 不受 delay 影响

**计时规则** (`timer_was_started`):

| 操作 | timer_was_started | 计时行为 |
|------|-------------------|---------|
| "清空" | running 时保持 `true`，暂停时 `false` | 立即清历史并归零；运行中从新纪元继续 |
| 首次"开始" | `false` → `true` | `reset_timer()` 归零 |
| 暂停→继续 | `true` | 累积计时 |
| 烧录后 | → `false` | 清空插件历史；下次启动先重建 slots、归零，再置 running |
| 断开后 | → `false` | 保留最后显示数据；下次启动先重建 slots、归零，再置 running |
| 连接 | 不变 | 不影响 |

### 5. VariablePool 数据结构

```
Vec<PooledVariable> + HashMap<usize, usize> (id → index)

操作复杂度:
  ├─ add(config)  → O(1) push + insert
  ├─ remove(id)   → O(1) swap_remove + 更新被交换项的 index
  ├─ get(id)      → O(1) id_index → Vec[index]
  └─ iter_mut()   → 直接迭代 Vec (采集循环用)

PooledVariable { id, name, address, ext_type, size, incoming, plugins_cnt, active_readers }
                         ↑ 原始采集共享缓冲          ↑ 总绑定数       ↑ 当前启用读取的绑定数
```

**绑定生命周期**:
- Chart/Table 叶子绑定 → `plugins_cnt += 1`; 默认启用时 `active_readers += 1`
- Table 取消勾选只减少 `active_readers`，保留节点和值；重新勾选再增加
- `rebuild_slots()` 只包含 `active_readers > 0` 的变量，不影响同一变量的其他启用绑定
- 根删除逐叶解绑；`plugins_cnt == 0` 时才从 Pool 删除，整批动作只重建一次 slots
- `find_compatible(name, addr, type, size)` 防止同地址不同解码类型错误复用

### 6. Chart 图表面板特性

| 功能 | 实现 |
|------|------|
| 坐标轴 | Y 轴数值标签, X 轴时间标签(s) |
| 网格线 | 自适应深色/浅色 |
| 曲线绘制 | 历史直接存为 `VecDeque<PlotPoint>`；绘图借用两个物理切片，并用 2 点 bridge 连接环形边界，不复制整段历史 |
| 值解码 | 按 `var.ext_type` 解析: u/i/float/double → f64, Other → 0.0 |
| 图例 (Legend) | 图表右上角浮动: `[色条] 曲线名` |
| 单击图例 | 切换 visible (曲线消失/恢复, 图例变暗) |
| 右键图例 | 弹出居中 line_dialog (模态: 全界面拦截对话窗外点击) |
| 曲线属性 Dialog | 曲线名/颜色(`color_edit_button_srgba`+预设色块)/缓冲/可见 + 变量属性 + 删除/确定/取消 |
| 编辑确认 | 对话框内编本地副本 (state.edit_*), "确定"生效 / "取消"丢弃, 非 running 时缓冲区长度可编辑 |
| 缓冲区 | "确定"时若长度变化 → `data_history = VecDeque::with_capacity(new_size)` 清空重建 |
| X 轴 | 默认 Auto 模式 6s 视窗, 无数据时初始 [0, 6.0] |
| 清空 | 清除 data_history + `clear_all_buffers()` (sync pause → discard all RingBuffers + reset timer) |
| Hz | `acq_cycle_count: Arc<AtomicU64>` 每采集轮询 `fetch_add(1)`, 主线程每秒计算 |
| 计时 | 首次"开始"或"清空"后第一次"开始" → timer 归零; 暂停再继续 → 累积计时 |
| 控制栏 | 显示 `Vari:N Slot:M` (PooledVariable 数 / 去重 AcqSlot 数) + `Hz: xxxx` |
| 添加配置颜色 | `egui::color_picker::color_edit_button_srgba()` 自定义拾色器 + 预设色块网格, egui memory 持久化 |
| 空状态 | 居中提示"暂无监控变量" + 打开变量树按钮 |
| Log CSV | 可选择 CSV 文件, 开始采集时覆盖写入 header+数据行, 暂停时关闭; 使用 FIFO 多路归并直接输出，不复制/排序时间戳; logging 期间禁用添加/删除/改选项 |
| 保存/加载 | JSON 格式保存 Probe/pool/plugin payload/ELF 配置; 加载后自动 trace 更新地址 |
| 游标 (Cursor) | 鼠标悬停时显示竖线 + 浮层: 逐曲线显示时间戳和当前值；浮层使用 Plot 实际 frame 定位并裁剪，不越出图表 |
| FFT 频谱图 | 工具栏 `📊 FFT` 按钮切换; 开启后视图上下分屏: 时域(55%) + 频域(45%) |
| FFT 配置 | 窗函数选择 (Rectangular/Hann/Hamming/Blackman) + 取样点数 (4~65536, 从数据末尾取) |
| FFT 多曲线 | 所有可见曲线各自计算 FFT, 叠加在同一频谱图上, 颜色与图例一致 |
| FFT 游标 | 鼠标悬停频谱图显示竖线 + 浮层: 逐曲线显示 "频率 Hz → 幅值"；浮层限制在 FFT Plot 内 |
| 滚轮缩放 | Both 模式原生双轴缩放; X/Y 模式手动单轴缩放 (锚定视图中心, 缩放因子 1/1.15) |
| 时域缩放模式 | 工具栏 `缩放: X Y Both` 按钮, 独立于 FFT 缩放; 缩放时自动关闭 auto-scroll |
| FFT 缩放模式 | FFT 图表头顶部独立 `缩放: X Y Both` 按钮 |
| 工具栏稳定性 | FFT 开关及 X/Y/Both 使用固定尺寸、始终保留 frame 的按钮，悬浮和选中不会改变工具栏布局 |

### 7. Table 读写面板特性

| 功能 | 实现 |
|------|------|
| 布局 | 可调整宽度的左侧变量树 + 右侧 SVD 寄存器浏览器 |
| 结构体 | 递归 `TableNode`，像树一样展开/折叠；只有根节点提供删除按钮 |
| 数组 | 根据 DWARF `[0]` 原型、count 和元素步长物化所有索引；支持多维数组和结构体数组 |
| 勾选采集 | 叶子默认勾选；父节点动态显示全选/部分/未选，点击时递归切换后代 |
| 共享变量 | `plugins_cnt` 保存绑定，`active_readers` 决定是否进入 slots；Table 关闭不会中断 Chart 的读取 |
| Read | 未勾选叶子冻结显示值；勾选叶子使用 `frame_data` 最新值并按 ExtendType 格式化 |
| 刷新频率 | 每个叶子保存 1–60 Hz 的显示刷新率；仅节流当前值格式化，不改变采集 slots |
| Write | 叶子 TextEdit → `validate_write()` → `PluginAction::WriteVariable` |
| 写入流程 | 主循环 drain `pending_writes` → `write_variable(var_id, value)` → `sync.send_request` 暂停采集线程 → `core.write_word_8/16/32/64` → 恢复 |
| 写入校验 | 按 ExtendType 校验: u8(0-255), i8(-128~127), u16, i16, u32, i32, u64, i64, f32, f64; Other 类型禁止写入 |
| SVD | 后台解析 CMSIS-SVD；Enter 提交且只过滤顶层 peripheral，搜索不自动展开，并提供全部折叠 |
| SVD 读写 | 寄存器默认未勾选；可读项独立设置 1–30 Hz，同一调度周期合并为一次 Probe 请求；只读项隐藏写入控件 |
| 配置 | 递归保存树、展开状态、叶子 enabled/refresh_hz、SVD 路径及寄存器 enabled/read_hz；兼容旧版平面 Table payload |

### 7.1 FFT 频谱分析模块 (fft.rs)

**自包含实现，零外部依赖**：

| 组件 | 说明 |
|------|------|
| `Complex` | 自定义复数类型 + Add/Sub/Mul 运算 |
| `fft()` | Radix-2 Cooley-Tukey FFT (原地, 前向), 支持任意 2^k 大小 |
| `FftWindowType` | 枚举: Rectangular / Hann / Hamming / Blackman, 各含 `label()` 和 `ALL` 常量 |
| `window_value()` | 按点计算窗系数，不再分配临时系数向量 |
| `compute_fft()` | 直接遍历 `VecDeque<PlotPoint>`: `(data, sample_count, window_type) → Option<FftResult>` |
| `FftResult` | 输出: `points: Vec<PlotPoint>`, `sample_rate: f64`，绘图直接借用 points |

**计算流程**：
1. 从数据末尾取 `sample_count` 个点 (clamp: `[4, min(total, 65536)]`)
2. 校验有限且严格递增的时间戳，以间隔中位数检测超过 3 倍的断点，只保留最后连续段
3. 将连续段线性重采样到首尾时间之间的均匀网格，由网格步长计算采样率
4. FFT 大小 = `next_power_of_two(take)`（最大 65536），零填充
5. 施加窗函数 → 复数数组 → FFT → 输出含 DC/Nyquist 的单边谱
6. 幅值按窗系数总和补偿；仅普通正频率乘 2，DC 与 Nyquist 保持单倍

**状态字段** (`ChartPluginState`)：
- `fft_sample_count: usize` — 默认 1024
- `fft_window_type: FftWindowType` — 默认 Hann
- `fft_scroll_mode: FftScrollMode` — FFT 图滚轮缩放模式 (默认 Both)
- `fft_plot_bounds: Option<(x_min,x_max,y_min,y_max)>` — 手动缩放 bounds 缓存

### 7.2 滚轮缩放模式 (时域 + 频域)

**`FftScrollMode`** 枚举 (X / Y / Both) 同时用于时域图和 FFT 图：

| 模式 | 时域图 | FFT 图 |
|------|--------|--------|
| **Both** | `allow_scroll(true)`, egui_plot 原生双轴缩放, bounds 自动清除 | 同左 |
| **X** | `allow_scroll(false)`, 手动拦截滚轮仅缩放 X 轴, `plot_bounds()` 同步 | 同左 |
| **Y** | `allow_scroll(false)`, 手动拦截滚轮仅缩放 Y 轴, `plot_bounds()` 同步 | 同左 |

**缩放因子**: 滚轮上 = 1/1.15 (放大), 滚轮下 = 1.15 (缩小), 锚定视图中心。

**时域特有**:
- 缩放时自动关闭 `auto_scroll`
- 双击 / "回到最新" / "清空" → 恢复 auto-scroll + 清除手动 bounds
- Both 模式拖拽后 auto_scroll=false 依赖 egui_plot 原生管理

**状态字段**:
- `td_scroll_mode: FftScrollMode` — 时域图滚轮缩放模式 (默认 Both)
- `td_plot_bounds: Option<(x_min,x_max,y_min,y_max)>` — 时域图手动 bounds 缓存

### 8. 配置保存/加载 + 追踪

**保存** (`save_config`): JSON 格式 (`serde`)，通过 `rfd` 文件对话框保存。包含:
- Probe 配置 (`chip`, `protocol`, `speed`)
- VariablePool (name, address, ext_type, size)
- 插件配置列表 `plugins: [{ plugin_id, payload }]`
  - Chart payload: legends (variable_name+address, curve_name, color, visible, buffer_size)
  - Table payload: entries (variable_name+address, display_name)
- ELF path

**加载** (`load_config`): 解析 JSON → 在临时 `VariablePool` 和默认插件池中重建配置 → 按 `plugin_id` 分发 payload → 插件恢复绑定数和启用读取数。任一插件加载失败时保留当前运行状态并 toast 错误；全部成功后才替换当前 pool/plugins。

当前配置格式以 `plugins` 字段为准，不做旧版 `chart_legends` / `table_entries` 字段迁移。

加载成功后自动 `trace_variables()`:
1. `load_elf()` 重新解析 DWARF
2. 遍历 PooledVariable, 按 ExtendName 逐层精确匹配新 DWARF 树 (`trace_exact`)
3. 唯一匹配 → 更新 address/ext_type/size; 失败 → toast 红色 15s 可关闭
4. `rebuild_slots()` 同步

**按钮可用性**:
- 加载: `add_enabled_ui(!running)`
- 保存: 始终可用
- Reset: `add_enabled_ui(connected)`
- 烧录固件: 连接后可用；选择文件并二次确认，任务期间自动暂停采集并显示不可关闭的进度 Modal
- ⚙设置: `add_enabled_ui(!connected)`

### 9. 模态 (Modal) 行为 + Toast 通知

属性/设置对话框使用 `egui::Modal` 实现穿透防护；变量树组件在当前 viewport 使用手写 `egui::Area` 遮罩 + 底部锚定面板。

| 覆盖层 | 实现 | 退出方式 |
|--------|------|----------|
| 变量树 BottomSheet | viewport 专属 Area ID + 当前 viewport 矩形 | 点击遮罩 / [关闭] / 关闭所属 pop-out |
| 曲线属性 line_dialog | `Modal::new("line_dialog_modal").show(ctx)` | [确定]/[取消]/[删除] |
| 设置 Dialog | `Modal::new("probe_settings_modal").show(ctx)` | [确定]/[取消] |
| 固件烧录 | `Modal::new("firmware_flash_modal").show(ctx)` | 烧录线程完成后自动关闭 |

**Toast 通知** (`egui-notify 0.22`): 右下角 (Anchor::BottomRight), 写入成功=绿色2s可关闭, 失败=红色3s可关闭, 追踪失败=红色15s可关闭。`self.toasts.show(ctx)` 每帧在 ui() 末尾调用。

### 10. 控制栏配置 Dialog

```
"⚙ 设置" → egui::Window (居中, 可取消)

内容:
  ├─ MCU 型号: [搜索过滤...] ← 搜索框
  │   └─ 固定 200px 列表 (ScrollArea), 支持实时过滤, selectable_label 选中
  │   └─ 来源: Registry::from_builtin_families() (启动时缓存到 AppSession.all_chips)
  ├─ 协议: SWD / JTAG
  ├─ 速度: Slider 100-20000 kHz (默认 10000 = 10MHz)
  ├─ Probe 设备: ComboBox (Lister::list_all() 扫描)
  └─ [确定] [取消] (水平布局)
      └─ 编辑本地副本 (edit_chip/protocol/speed), 确定生效 / 取消丢弃
```

**Toast 通知** (`egui-notify 0.22`): 右下角 (Anchor::BottomRight), 写入成功=绿色2s, 失败=红色3s

## 依赖

```toml
eframe = "0.34"           # GUI 框架
egui_ltreeview = "0.7.0"  # 树形视图 (DWARF 变量树)
probe-rs = "0.31"         # MCU 调试 (CMSIS-DAP/ST-Link/J-Link)
crossbeam-queue = "0.3"   # 有界 lock-free 采集环形队列
gimli = "0.31"            # DWARF 解析
object = "0.36"           # ELF 解析
anyhow = "1.0"            # 错误处理
egui_plot = "0.35"        # 图表绘制
egui-notify = "0.22"      # Toast 通知
rfd = "0.15"              # 系统文件对话框
serde = "1"               # 序列化
serde_json = "1"          # JSON
svd-parser = "0.14.10"    # CMSIS-SVD 解析与数组/继承展开
```

## 关键设计决策

1. **TreeNode 不存 Extend，改为动态计算 + ExtendConfig 覆盖**
   - `TreeNode` 仅存 DWARF 原始数据（address 存 offset 而非绝对地址，用于树遍历计算）
   - 首次预览时 extend 值由 `compute_extend_name/address()` 和 `basic_type_to_extend()` 从 basic 计算
   - 用户编辑存入 `VariableTreePanel.extend_configs` (`HashMap<usize, ExtendConfig>`)
   - 添加时 ExtendConfig 被消耗到 `PooledVariable`

1.5 **DWARF 跨 CU 类型引用解析**
   - `DW_FORM_ref_addr`（gimli: `AttributeValue::DebugInfoRef`）指向任意 `.debug_info` section 偏移
   - `find_unit_for_debug_info_ref()` 扫描所有 CU 定位目标，`UnitOffset` = `section_offset - cu_header_offset`（gimli 内部自动减去 `header_size`）
   - `resolve_type_impl` 的 6 个类型跟随 tag（typedef/pointer/const/volatile/reference/rvalue-ref/array）通过 `follow_type_attr_or_resolve` 统一处理跨 CU ref，自动重启 `resolve_type` 链
   - 解析失败的引用容错降级为 `TypeKind::Other`，不阻断解析流程

2. **ExtendType 限制类型集合**
   - 仅 11 种可采集类型: u8/u16/u32/u64/i8/i16/i32/i64/float/double/other
   - DWARF 原始类型 Pointer → U64, Struct/Other → Other
   - Other 类型禁止添加到 Chart/Table, 避免类型不匹配的采集错误

3. **extend_size 自动绑定，用户不可编辑**
   - 切换 extend_type 时自动更新 size: u8→1, u16→2, u32→4, u64→8
   - Other 类型不自动绑定
   - Size 字段在 Extend 段仅显示，不是 TextEdit

4. **extend_name 由顶级变量拼接，extend_address 链式相加**
    - `compute_extend_name()`: 从根开始逐级拼接，如 `my_struct.arr[2]`
    - `compute_extend_address()`: 根绝对地址 + 所有路径节点的 offset 累加
    - `find_path_to_node()`: 子节点名以 `[` 开头时不加 `.` 号，如 `arr[2]` 而非 `arr.[2]`
    - 两者默认不可编辑

5. **PooledVariable 仅存 extend 数据**
   - 不再包含 `TreeNode` 克隆，只存 `name/address/ext_type/size/current_value`
   - 采集直接读 `var.address`、`var.size`，解码直接用 `var.ext_type`
   - 对话窗显示的是 PooledVariable 的 extend 属性，非 TreeNode 的 basic 属性

6. **变量树 viewport 路由**: `OpenVariableTree` 携带触发 viewport，覆盖层由 docked/pop-out 各自渲染

7. **Modal 统一管理**: line_dialog / probe_settings / firmware_flash 均使用 `egui::Modal::new().show(ctx)` 实现穿透防护，无需手动拦截

8. **VariablePool 用 Vec+HashMap**: 模拟链表 + 哈希对, O(1) 增删查, 比纯 HashMap 更适合频繁迭代的采集场景

9. **Chart 插件与 Table 插件实现同一个动态 trait**: `ChartPluginState` 与 `TablePluginState` 都实现 `MemRWPlugin`; App/Dock/BottomSheet 只通过 `Box<dyn MemRWPlugin>` 分发, 不使用 Chart/Table enum 做中心化分支

10. **添加配置由目标插件提供**: `vari_properties_ui()` 仍通过 `FnOnce` 闭包承载 Add 区域, 但闭包内部按 active `plugin_id` 调用 `plugin.add_variable_ui(...)`; 添加后插件自行保存曲线名/颜色/显示名

11. **多线程采集架构 (参考 MemRW2)**:
    - `acq_thread`: 独立采集线程, 非阻塞 `try_acquire` 检查同步请求, 正常运行时全速采集
    - `Sync`: **双 Condvar** 握手 (`cv_main` + `cv_worker`), 消除共享单 Condvar 死锁
    - `ProbeCell`: `Mutex<ProbeSession>` 提供安全所有权；Sync 令正常采集路径保持无竞争
    - **Core 生命周期**: 每次操作局部获取并析构 `Core<'_>`，无 `transmute`、自引用或跨线程 drop
    - `AcqSlot`: 纯 32-bit 地址标记；`VarSlotMapping` 保存连续 slot 索引，不在采集热路径中使用 HashMap/Arc 查找
    - **两阶段采集**: Phase1 read32 到跨轮复用的 `slot_values: Vec<[u8; 4]>` → Phase2 按索引和 byte_offset 组装变量值
    - `RingBuffer`: 基于 `crossbeam_queue::ArrayQueue` 的有界 lock-free 环形队列，容量 2560，满载时丢弃最旧样本
    - `delay_us: Arc<AtomicU64>`: 默认 0 (全速), 采集线程 sleep 节流, 主线程独立 vsync 刷新
    - **FrameData 预 drain**: UI 每帧用 `drain_into` 消费到跨帧复用的 HashMap/Vec，再向所有插件调用 `update`；隐藏 Chart 也持续记录
    - **Plot/FFT**: 时域 Plot 直接借用 VecDeque 切片；FFT 直接遍历历史并输出 `Vec<PlotPoint>` 供绘图借用，应用层不再复制整段点集
    - **PluginAction**: 插件返回带 viewport 的 OpenVariableTree、RemoveVariable、SetVariableEnabled、WriteVariable 等意图
    - **绑定/读取分离**: `plugins_cnt` 管生命周期，`active_readers` 管是否生成采集 slots；Table 父节点批量操作后只 rebuild 一次
    - **Hz**: `acq_cycle_count: Arc<AtomicU64>` 采集线程每轮 +1, 主线程每秒计算采集轮询频率
    - **计时**: 首次/清空/烧录后的启动先重建、清历史和归零，最后置 running；暂停继续则保持时间轴

12. **Tree View 默认折叠, 搜索居中滚动**: 使用 `NodeBuilder::default_open(false)` 初始化所有树节点为折叠状态; 搜索后通过 `count_nodes_before()` (基于 `tree_state` 展开状态) 计算可见节点数, 使用 `viewport_h` 居中偏移公式 `ScrollArea::vertical_scroll_offset()` 居中显示。点击节点仅选中不滚动。

13. **ELF 文件延迟加载**: 不通过命令行参数加载, App 启动为空, 用户在 BottomSheet 顶部输入路径并点击"加载"触发 `load_elf()`

14. **数组支持 (ArrayElem)**:
     - DWARF 数组类型逐层构建嵌套 `TypeRef` 链: `float[7][7]` → TypeRef(name="float[7][7]") → TypeRef(name="float[7]") → TypeRef(name="float")
     - 树中每层为独立 `[]` 节点，`parent_id` 指向数组节点，`name` 默认 `[0]`（惰性求值：仅创建占位节点）
     - `BasicType::ArrayElem(Box<BasicType>, u64)` 存元素类型和数组长度
     - `basic_type_to_extend(ArrayElem)` → `ExtendType::Other`（不递归穿透多层）
     - Basic 栏显示 `[index]` + 可编辑 DragValue；Extend name/address 由 `compute_extend_name/address` 自然拼接 `A[2]`
     - **数组索引更新机制**：
       - 用户搜索 `A[2]`：`perform_search` → 后处理遍历祖先数组节点，更新占位节点名和地址
       - 用户 DragValue 修改 Index：`vari_properties_ui` → `app.rs` 帧末尾重新同步树节点
       - 初始化时从 `node.name` 解析 index 到 `config.array_index`（仅当值不同时更新，避免覆盖 DragValue）

15. **DWARF 版本兼容（2/3/4/5）**:
     - gimli 的 `parse_unit_header` 已原生支持 DWARF 2-5，CU 头格式差异已内部处理
     - **DW_FORM_ref_addr（跨 CU 类型引用）**：gimli 映射为 `AttributeValue::DebugInfoRef`
       - `find_unit_for_debug_info_ref()` 扫描所有 CU 定位目标，计算 `UnitOffset`（相对于 CU 头起始）
       - `resolve_type_impl` 内部 6 个 tag case 通过 `follow_type_attr_or_resolve` 统一处理
       - 跨 CU ref 重新启动 `resolve_type` 链，确保类型名、size、嵌套结构正确解析
     - **.debug_addr 缺失**（DWARF 3 无此 section）：`location_address` 的 `AddressIndex` 分支使用 `.ok()`
     - **属性读取容错**：`resolve_type_impl` 的 catch-all `_ =>` 分支使用 `.ok().flatten()` 避免截断 DIE 导致崩溃

16. **搜索层级递进匹配**:
    - `.` 分割层级，非末层用 `eq_ignore_ascii_case` 精确匹配，末层用 `contains` 模糊匹配
    - 匹配失败时**不再跳过当前层级深入搜索**，直接终止该分支
    - `[idx]` 记号自动展开为独立层级，匹配时校验 index 是否在 `[0, count)` 范围内
    - 搜索成功后更新树节点 name/address 并同步 `config.array_index`

17. **BottomSheet**: 独立 `VariableTreePanel` 在触发 viewport 内渲染，支持拖拽高度、点击遮罩或 [关闭] 退出
