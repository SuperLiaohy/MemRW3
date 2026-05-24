# MemRW3 — 工作流程与布局架构

## 项目概述

MemRW3 是一个基于 Rust + egui + probe-rs 的嵌入式内存读写与变量监控工具，是对原 Qt/QML MemRW2 的重构。使用 gimli/object 替代 libdwarf 解析 DWARF 调试信息，使用 probe-rs 替代 libusb 手动协议解析进行 MCU 数据采集，使用 eframe + egui_dock 替代 Qt QML 实现 UI。

## 整体布局

```
┌──────────────────────────────────────────────────────────────┐
│ 控制栏 (Control Bar)                                         │
│ [连接/断开] [开始/暂停] [⚙设置] [延迟] [Reset] [保存] [加载]     Hz: xxx  ● 采集中 │
├──────────────────────────────────────────────────────────────┤ ← 模态阻塞: 不可交互
│ DockArea: [Chart 实时数据 | Table 读写数据]                   │
│ ┌──────────────────────────┬───────────────────────────────┐ │
│ │                          │                               │ │
│ │   Chart 图表区            │   Table 表格区                 │ │
│ │   [坐标轴+曲线+图例]       │   [Name | Value | Write | ✕]  │ │
│ │                          │                               │ │
│ └──────────────────────────┴───────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤ ← BottomSheet 覆盖层 (可交互)
│ [ELF 文件: ________] [浏览] [加载] [追踪]                    │
│ ──────────────────────────────────────────────────────────    │
│ 变量列表 (DWARF Tree)                              [关闭]    │
│ ┌─────────────────────────┬────────────────────────────────┐ │
│ │ Search: [________]       │ 属性                            │ │
│ │ [All] [Search]           │ ── Basic (只读, DWARF原始) ──   │ │
│ │                          │ Name: xxx  Address(offset): xx  │ │
│ │  Tree View (默认折叠)     │ Size: xx    Type: xxx           │ │
│ │   ├─ cu_name             │ ── Extend (可编辑) ──           │ │
│ │   │  ├─ var1             │ Name: [edit]   Address: [hex]   │ │
│ │   │  └─ struct           │ Size: auto    Type: [u32 ▼]     │ │
│ │   │     ├─ member1       │ ── Add ──                       │ │
│ │   │     └─ member2       │ [曲线名/颜色] → [添加到 Chart]  │ │
│ │   └─ cu_name2            │ 或                             │ │
│ │                          │ [显示名] → [添加到 Table]       │ │
│ │                          │ (type=other 时禁止添加)          │ │
│ └─────────────────────────┴────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

## 模块架构

```
src/
├── main.rs                 # 入口: 启动空DwarfApp → eframe
├── types.rs                # 数据类型: TreeNode, BasicType, ExtendType, ExtendConfig, CuInfo, DwarfApp, TypeRef
├── dwarf.rs                # DWARF 解析 (gimli), basic_type 映射, address 存offset
├── app.rs                  # 主 App + 布局编排 + MemRW3App (控制栏 + DockArea + BottomSheet 模态 + 对话窗锁)
├── sync.rs                 # 同步原语: Sync (两阶段握手) - 匹配 MemRW2 的 3-semaphore 模式
├── model/
│   ├── mod.rs
│   ├── state.rs            # AppSession (连接/采样/BottomSheet/load_error/extend_configs)
│   ├── variable_pool.rs    # VariablePool (Vec + HashMap, O(1) 增删查, 仅存extend数据)
│   └── double_buffer.rs    # 无锁双缓冲 (SPSC, [UnsafeCell<Vec<T>>; 2] + AtomicUsize)
├── probe/
│   ├── mod.rs              # ProbeCell (UnsafeCell wrapper, Sync协议保证互斥)
│   └── session.rs          # ProbeSession + AcqSlot (probe-rs 连接/采集/读写)
└── ui/
    ├── mod.rs
    ├── control_bar.rs      # 控制栏 (连接/采集/Probe配置Dialog)
    ├── chart_plugin/
    │   ├── mod.rs
    │   ├── legend.rs       # ChartLegend (曲线名/颜色/可见/缓冲/data_history)
    │   ├── panel.rs        # 图表面板 (坐标轴/曲线/图例覆层/ExtendType 解码 + 自定义颜色选择)
    │   └── line_dialog.rs  # 曲线属性 Dialog (编辑曲线属性 + 显示PooledVariable的Extend属性)
    ├── table_plugin/
    │   ├── mod.rs
    │   ├── panel.rs        # 表格面板 (TableView 读写/ExtendType 格式化)
    │   └── table_dialog.rs # TableEntry + 属性 Dialog (显示PooledVariable的Extend属性)
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
  - 数组元素 `[]`：存储 `elem_size * index`，表示数组内偏移
- extend 不存储在 TreeNode 中，改为通过 `DwarfApp` 的遍历方法动态计算
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

- 存储在 `AppSession.extend_configs: HashMap<usize, ExtendConfig>`，按 node_id 索引
- `vari_properties_ui()` 通过 `&mut ExtendConfig` 读写
- 首次选择节点时惰性初始化
- 数组元素时 index 从 `selected_node.name` 解析同步（含搜索后更新）
- "添加到 Chart/Table" 时，ExtendConfig 被消耗并存入 `VariablePool.add(config)`

### PooledVariable (池中仅存 extend 数据)

```rust
pub struct PooledVariable {
    pub id: usize,
    pub name: String,          // extend_name
    pub address: u64,          // extend_address
    pub ext_type: ExtendType,  // extend_type
    pub size: u32,             // extend_size
    pub incoming: Arc<DoubleBuffer<(f64, [u8; 8])>>,  // 无锁双缓冲, 采集线程push, UI线程drain
}
```

- 不再包含 `TreeNode`，只存实际用于采集和显示的数据
- `VariablePool.add(&ExtendConfig)` 创建条目
- `incoming` 通过 `Arc` 共享: rebuild_slots 时 clone 到 `VarSlotMapping.incoming`, 采集线程无锁写入, UI 线程无锁 drain
- 去重: 添加变量前检查 `Pool.find_by_name_addr(name, address)`, 同 name+address 不重复添加
- Chart/Table 面板直接使用 `var.ext_type` 进行值解码和格式化

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
  ├─ MemRW3App::new(DwarfApp::new(Vec::new())) → 启动空 DwarfApp
  └─ eframe::run_native() → 启动 UI (无预加载数据)

用户操作:
  └─ BottomSheet 顶部 [ELF文件: ________] [加载]
      ├─ fs::read() → 读取 ELF 文件
      ├─ object::File::parse() → 解析 ELF
      ├─ dwarf::load_dwarf() → 加载 DWARF sections
      └─ dwarf::collect_cus() → 遍历 CU, 提取变量树 (TreeNode 递归结构)
          └─ type_name_to_basic_type() 根据 DWARF type name/size 映射 BasicType
```

不再通过命令行参数传入 ELF 路径。App 启动时 DwarfApp 为空，用户通过 BottomSheet 顶部文件选择器加载。

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
Dock Tab 中点击 "📋 打开变量树" → BottomSheet 覆盖显示

BottomSheet (模态覆盖层, 打开时全界面不可交互, 只能点 [关闭] 按钮退出)
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
           ├─ type ≠ other → 显示添加配置 → "添加到 Chart/Table"
           │   ├─ 曲线名(TextEdit) + 颜色(自定义拾色器 + 预设色块) → 添加到 Chart
           │   └─ 显示名(TextEdit) → 添加到 Table
           └─ type = other → 红色提示 "type 为 other，不可添加到 Chart 或 Table"

      添加流程:
        ├─ extend_name 和 extend_address 由 DwarfApp 从 DWARF 树计算得到
        ├─ 用户可在 Extend 段编辑 address/type (size 自动绑定)
        ├─ 编辑结果存入 ExtendConfig (AppSession.extend_configs HashMap)
        ├─ 点 "添加到 Chart/Table" → VariablePool.add(&ExtendConfig)
        ├─ Chart: 曲线名 + 颜色 → 存入 ChartLegend (颜色persist via egui memory)
        └─ Table: 显示名 → 存入 TableEntry
```

### 4. 数据采集 (多线程架构)

#### 线程模型

```
┌─ 主线程 (UI) ───────────────────────────────────────────────┐
│  egui frame loop:                                            │
│    request_repaint() ← 持续刷新                               │
│    drain DoubleBuffer ← 无锁读取采集数据                        │
│    sync.send_request(|| { probe操作 }) ← 同步时阻塞主线程       │
└──────────────────────────────────────────────────────────────┘
         ↑ ↓ Sync 握手                     ↑ ↓ Arc<DoubleBuffer>
┌─ 采集线程 (acq_thread) ───────────────────────────────────────┐
│  loop:                                                       │
│    sync.try_acquire() ← 非阻塞检查同步请求                     │
│    if running:                                                │
│      acquire_from_slots() → push to DoubleBuffer ← 无锁写入   │
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
- **同步操作时** (连接/断开/复位/写入/更新slots): 主线程通过 `sync.send_request` 暂停采集线程后独占 probe，完成后恢复。闭包运行在**主线程**。

#### 数据流 (无锁路径)

```
[变量树添加变量]
  ├─ Pool.find_by_name_addr(name, addr) → 去重
  │   ├─ 已存在 → 复用 var_id
  │   └─ 新变量 → VariablePool.add(config) → rebuild_slots (sync)
  └─ Plugin.add_legend/entry(var_id) → PooledVariable.plugins_cnt += 1

[点击"开始"]
  ├─ first start 或 after clear → reset_timer() (sync)
  └─ rebuild_slots() → sync → acq_thread.slots + var_mappings

采集线程 (每轮):
  Phase 1: read32 all slots → HashMap<u64, [u8; 4]>
  Phase 2: for mapping in var_mappings:
    assemble value from slots → mapping.incoming.push((ts, val))
  cycle_count.fetch_add(1) → Hz 统计

UI 线程 (每帧开始):
  for var in pool.iter():
    frame_data[var.id] = var.incoming.drain()  ← 每个变量只 drain 一次

  chart_panel(&frame_data):   ← 从 frame_data 读, 不再调用 drain
  table_panel(&frame_data):   ← 同上
```

**插件删除 → 解绑**:

```
remove_legend/entry → removed_var_ids.push(var_id)
ui() drain removed_var_ids:
  PooledVariable.plugins_cnt -= 1
  if plugins_cnt == 0 → pool.remove(var_id) + rebuild_slots (sync)
```

#### ProbeCell (无 Mutex) + Core 缓存

`ProbeSession` 通过 `Arc<ProbeCell>` 共享, `ProbeCell` 是 `UnsafeCell` 包装:

```rust
pub struct ProbeCell(UnsafeCell<ProbeSession>);
// 安全性: Sync 握手协议保证不会并发访问
// - 采集线程: 仅正常运行时访问
// - 主线程: 仅在 send_request 闭包内访问 (采集线程已暂停)
```

**Core 缓存** (性能关键):

`probe_rs::Session::core(0)` 是昂贵的操作 (~200-500µs, 包含 halt 核心、读 CPUID、DAP 寄存器初始化)。Demo 中只调用一次, 我们原先每帧调用一次 → 这是 7K vs 1K Hz 差距的根源。

修复: 首次 `core(0)` 后通过 `unsafe transmute` 缓存为 `Core<'static>`, 后续采集直接复用。

```rust
pub struct ProbeSession {
    cached_core: Option<probe_rs::Core<'static>>,  // 声明先于 session, 先 drop
    session: Option<Session>,
    // ...
}

fn ensure_core(&mut self) -> bool {
    if self.cached_core.is_some() { return true; }
    // 首次: 获取 core, transmute 为 'static (两者同属 self, 同生命周期)
    self.cached_core = Some(unsafe { std::mem::transmute(session.core(0)?) });
}

fn acquire_from_slots(&mut self) {
    self.ensure_core();
    // raw pointer 避免 &mut self 与 &self.slots 的借用冲突
    let core = unsafe { &mut *(self.cached_core.as_mut().unwrap() as *mut _) };
    for slot in &self.slots {
        core.read_word_32(slot.address) → slot.incoming.push((ts, val));
    }
}
```

- 读错误时 `self.cached_core = None` → 下帧自动重建
- 连接/断开/复位时 `self.cached_core = None` → 强制重建
- `cached_core` 声明先于 `session` (Rust 按声明序 drop) → drop 时 session 仍存活

`AcqSlot` 缓存变量地址/大小/类型, 采集线程无需持有 `VariablePool` 锁:

```rust
pub struct AcqSlot {
    pub address: u64,  // 32-bit 对齐地址, 去重: 多变量可共享同一地址
}
```

`VarSlotMapping` 将一个 `PooledVariable` 映射到其 `AcqSlot` 集合:

```rust
pub struct VarSlotMapping {
    pub slots: Vec<Arc<AcqSlot>>,   // 该变量的全部 32-bit 槽位
    pub size: u32,                   // 变量总大小
    pub byte_offset: usize,          // 变量地址在首槽位中的字节偏移
    pub incoming: Arc<DoubleBuffer<(f64, [u8; 8])>>,
}
```

**rebuild_slots 算法** (每次连接/变量变更时在主线程通过 sync 执行):

```
1. 遍历 VariablePool 中每个 PooledVariable:
   a. slot_addresses(var.address, var.size) → 计算覆盖该地址范围的 32-bit 对齐地址列表
      - u32@0x2000_0000 → [0x2000_0000]
      - u64@0x2000_0000 → [0x2000_0000, 0x2000_0004]
      - u8@0x2000_0001  → [0x2000_0000] (byte_offset=1)
   b. 去重: HashMap<u64, Arc<AcqSlot>>, 同地址共享 Arc
   c. 构建 VarSlotMapping { slots, size, byte_offset, incoming }
2. 将去重后的 Vec<Arc<AcqSlot>> 和 Vec<VarSlotMapping> 写入 ProbeSession (通过 sync)
```

**两阶段采集** (acq_thread):

```
Phase 1: 读取全部去重槽位
  for slot in slots:
    slot_value = core.read_word_32(slot.address)
    → HashMap<u64, [u8; 4]>

Phase 2: 组装变量值 → push DoubleBuffer
  for mapping in var_mappings:
    val = [0u8; 8]
    for (i, slot) in mapping.slots:
      sv = slot_values[slot.address]
      if i == 0: copy sv[mapping.byte_offset..]  → val
      else:      copy sv[..]                      → val
    mapping.incoming.push((ts, val))
```

#### DoubleBuffer (SPSC 无锁双缓冲)

```rust
pub struct DoubleBuffer<T> {
    bufs: [UnsafeCell<Vec<T>>; 2],
    write_idx: AtomicUsize,  // fetch_xor(1) 原子翻转
}
// push() → 采集线程; drain() → UI 线程
// 预分配容量避免频繁分配: with_capacity(2560)
```

#### 延迟控制 + 计时规则

`delay_us: Arc<AtomicU64>` 共享: 主线程 slider 写入, 采集线程读取 → `thread::sleep(delay_us)` 控制采集频率。

- **默认 0** (全速采集, 仅受 probe USB 延迟限制, STM32H7 SWD 10M 可达 ~7KHz)
- 主线程仅 `request_repaint()` 以 vsync 刷新 UI, 不受 delay 影响

**计时规则** (`timer_was_started`):

| 操作 | timer_was_started | 计时行为 |
|------|-------------------|---------|
| "清空" | → `false` | 下次"开始"归零 |
| 首次"开始" | `false` → `true` | `reset_timer()` 归零 |
| 暂停→继续 | `true` | 累积计时 |
| 连接/断开 | 不变 | 不影响 |

### 5. VariablePool 数据结构

```
Vec<PooledVariable> + HashMap<usize, usize> (id → index)

操作复杂度:
  ├─ add(config)  → O(1) push + insert
  ├─ remove(id)   → O(1) swap_remove + 更新被交换项的 index
  ├─ get(id)      → O(1) id_index → Vec[index]
  └─ iter_mut()   → 直接迭代 Vec (采集循环用)

PooledVariable { id, name, address, ext_type, size, incoming: Arc<DoubleBuffer<...>>, plugins_cnt: usize }
                                                          ↑ Arc 共享: 采集线程 push, UI 线程 drain    ↑ 绑定计数: 0 时自动移除
```

**plugins_cnt 生命周期**:
- `add_legend/entry` → `plugins_cnt += 1`
- `remove_legend/entry` → `plugins_cnt -= 1`; 若为 0 → `pool.remove(id)` + `rebuild_slots`
- `find_by_name_addr(name, addr)` 添加前去重, 已存在则复用 id

### 6. Chart 图表面板特性

| 功能 | 实现 |
|------|------|
| 坐标轴 | Y 轴 7 格 + 数值标签, X 轴 6 格 + 时间标签(s) |
| 网格线 | 自适应深色/浅色 |
| 曲线绘制 | 从 `data_history: VecDeque<(time, value)>` 读取, 折线连接 |
| 值解码 | 按 `var.ext_type` 解析: u/i/float/double → f64, Other → 0.0 |
| 图例 (Legend) | 图表右上角浮动: `[色条] 曲线名 = 当前值` |
| 单击图例 | 切换 visible (曲线消失/恢复, 图例变暗) |
| 双击图例 | 弹出居中 line_dialog (模态: 全界面拦截对话窗外点击) |
| 曲线属性 Dialog | 曲线名/颜色(`color_edit_button_srgba`+预设色块)/缓冲/可见 + 变量属性 + 删除/确定/取消 |
| 编辑确认 | 对话框内编本地副本 (state.edit_*), "确定"生效 / "取消"丢弃, 非 running 时缓冲区长度可编辑 |
| 缓冲区 | "确定"时若长度变化 → `data_history = VecDeque::with_capacity(new_size)` 清空重建 |
| X 轴 | 默认 Auto 模式 6s 视窗, 无数据时初始 [0, 6.0] |
| 清空 | 清除 data_history + `clear_all_buffers()` (sync pause → drain all DoubleBuffers + reset timer) |
| Hz | `acq_cycle_count: Arc<AtomicU64>` 每采集轮询 `fetch_add(1)`, 主线程每秒计算 |
| 计时 | 首次"开始"或"清空"后第一次"开始" → timer 归零; 暂停再继续 → 累积计时 |
| 控制栏 | 显示 `Vari:N Slot:M` (PooledVariable 数 / 去重 AcqSlot 数) + `Hz: xxxx` |
| 添加配置颜色 | `egui::color_picker::color_edit_button_srgba()` 自定义拾色器 + 预设色块网格, egui memory 持久化 |
| 空状态 | 居中提示"暂无监控变量" + 打开变量树按钮 |
| Log CSV | 可选择 CSV 文件, 开始采集时覆盖写入 header+数据行, 暂停时关闭; toast 提醒开始/停止; logging 期间禁用添加/删除/改选项 |
| 保存/加载 | JSON 格式保存 Probe/chart/table/pool/ELF 配置; 加载后自动 trace 更新地址 |

### 7. Table 读写面板特性

| 功能 | 实现 |
|------|------|
| 表格列 | Name / Read / Write (三列) |
| Name | `entry.display_name`, 双击打开属性 Dialog (含删除) |
| Read | `frame_data` 最新值 或 `DoubleBuffer.latest()`, 按 `var.ext_type` 格式化: u/i → hex+十进制, float/double → 小数 |
| Write | TextEdit 输入 → `validate_write()` 校验类型范围 → 点"写" → `pending_writes.push((var_id, value))` |
| 写入流程 | 主循环 drain `pending_writes` → `write_variable(var_id, value)` → `sync.send_request` 暂停采集线程 → `core.write_word_8/16/32/64` → 恢复 |
| 写入校验 | 按 ExtendType 校验: u8(0-255), i8(-128~127), u16, i16, u32, i32, u64, i64, f32, f64; Other 类型禁止写入 |
| 通知 | `egui-notify` toast: 成功=绿色2s, 失败=红色3s, 校验错误=红色3s |
| 空状态 | 居中提示 + 打开变量树按钮 |

### 8. 配置保存/加载 + 追踪

**保存** (`save_config`): JSON 格式 (`serde`)，通过 `rfd` 文件对话框保存。包含:
- Probe 配置 (`chip`, `protocol`, `speed`)
- VariablePool (name, address, ext_type, size)
- Chart legends (variable_name+address, curve_name, color, visible, buffer_size)
- Table entries (variable_name+address, display_name)
- ELF path

**加载** (`load_config`): 解析 JSON → 清空现有 pool/plugins → 重建 VariablePool → 按 `(name, address)` 精确匹配 chart/table 子项 → `plugins_cnt += 1`。任一匹配失败 → 全部清空 + toast 错误。

加载成功后自动 `trace_variables()`:
1. `load_elf()` 重新解析 DWARF
2. 遍历 PooledVariable, 按 ExtendName 逐层精确匹配新 DWARF 树 (`trace_exact`)
3. 唯一匹配 → 更新 address/ext_type/size; 失败 → toast 红色 15s 可关闭
4. `rebuild_slots()` 同步

**按钮可用性**:
- 加载: `add_enabled_ui(!running)`
- 保存: 始终可用
- Reset: `add_enabled_ui(connected)`
- ⚙设置: `add_enabled_ui(!connected)`

### 9. 模态 (Modal) 行为 + Toast 通知

全部模态使用 `egui::Modal` 实现穿透防护。

| 覆盖层 | 实现 | 退出方式 |
|--------|------|----------|
| 曲线属性 line_dialog | `Modal::new("line_dialog_modal").show(ctx)` | [确定]/[取消]/[删除] |
| 变量属性 table_dialog | `Modal::new("table_entry_modal").show(ctx)` | [确定]/[取消]/[删除] |
| 设置 Dialog | `Modal::new("probe_settings_modal").show(ctx)` | [确定]/[取消] |

**Toast 通知** (`egui-notify 0.22`): 右下角 (Anchor::BottomRight), 写入成功=绿色2s可关闭, 失败=红色3s可关闭, 追踪失败=红色15s可关闭。`self.toasts.show(ctx)` 每帧在 ui() 末尾调用。

### 9. 控制栏配置 Dialog

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
egui_dock = "0.19"        # Dock 面板 (tabbed/horizontal/vertical)
egui_ltreeview = "0.7.0"  # 树形视图 (DWARF 变量树)
probe-rs = "0.31"         # MCU 调试 (CMSIS-DAP/ST-Link/J-Link)
gimli = "0.31"            # DWARF 解析
object = "0.36"           # ELF 解析
anyhow = "1.0"            # 错误处理
egui_plot = "0.35"        # 图表绘制
egui-notify = "0.22"      # Toast 通知
rfd = "0.15"              # 系统文件对话框
serde = "1"               # 序列化
serde_json = "1"          # JSON
```

## 关键设计决策

1. **TreeNode 不存 Extend，改为动态计算 + ExtendConfig 覆盖**
   - `TreeNode` 仅存 DWARF 原始数据（address 存 offset 而非绝对地址，用于树遍历计算）
   - 首次预览时 extend 值由 `compute_extend_name/address()` 和 `basic_type_to_extend()` 从 basic 计算
   - 用户编辑存入 `AppSession.extend_configs` (`HashMap<usize, ExtendConfig>`)
   - 添加时 ExtendConfig 被消耗到 `PooledVariable`

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

6. **BottomSheet 手动模拟 Modal 覆盖**: 用 `egui::Area` 来实现

7. **Modal 统一管理**: line_dialog / table_dialog / probe_settings 均使用 `egui::Modal::new().show(ctx)` 实现穿透防护，无需手动拦截

8. **VariablePool 用 Vec+HashMap**: 模拟链表 + 哈希对, O(1) 增删查, 比纯 HashMap 更适合频繁迭代的采集场景

9. **Chart 插件与 Table 插件独立**: 各自管理状态 (ChartPluginState / TablePluginState), 通过 VariablePool 共享数据

10. **添加配置回调注入**: `vari_properties_ui()` 通过 `FnOnce` 闭包接收插件定制的添加 UI, 避免 centralized enum dispatch; 添加后回写用户选择的曲线名/颜色/显示名

11. **多线程采集架构 (参考 MemRW2)**:
    - `acq_thread`: 独立采集线程, 非阻塞 `try_acquire` 检查同步请求, 正常运行时全速采集
    - `Sync`: **双 Condvar** 握手 (`cv_main` + `cv_worker`), 消除共享单 Condvar 死锁
    - `ProbeCell` (UnsafeCell): 无 Mutex 开销, Sync 协议保证互斥
    - **Core 缓存**: `session.core(0)` 首次调用后通过 `unsafe transmute` 缓存为 `Core<'static>`, 避免每帧重复初始化 (性能关键: 200-500µs → ~0µs)
    - `AcqSlot`: 纯 32-bit 地址标记, 去重: 多变量共享同地址; `VarSlotMapping` 将变量映射到其槽位集合
    - **两阶段采集**: Phase1 read32 全部去重槽位 → Phase2 按 byte_offset 组装变量值
    - `DoubleBuffer`: SPSC 无锁双缓冲, `fetch_xor` 原子翻转, 预分配容量 2560
    - `delay_us: Arc<AtomicU64>`: 默认 0 (全速), 采集线程 sleep 节流, 主线程独立 vsync 刷新
    - **FrameData 预 drain**: UI 每帧开始时统一 drain 所有 DoubleBuffer 到 HashMap, plugin 只读不 drain — 避免同一变量被多处引用时多次切换缓冲区
    - **plugins_cnt**: 变量被 plugin 绑定时 +1, 解绑时 -1; 归零自动从 Pool 移除 + rebuild_slots
    - **Hz**: `acq_cycle_count: Arc<AtomicU64>` 采集线程每轮 +1, 主线程每秒计算采集轮询频率
    - **计时**: `timer_was_started` 追踪, 首次"开始"和清空后第一次"开始"归零, 暂停再继续累积

12. **Tree View 默认折叠, 搜索居中滚动 + 点击滚动**: 使用 `NodeBuilder::default_open(false)` 初始化所有树节点为折叠状态; 搜索后通过 `count_nodes_before()` (基于 `tree_state` 展开状态) 计算可见节点数, 使用 `viewport_h` 居中偏移公式 `ScrollArea::vertical_scroll_offset()` 居中显示; 点击节点也设 `scroll_target_id` 实现滚到居中。

13. **ELF 文件延迟加载**: 不通过命令行参数加载, App 启动为空, 用户在 BottomSheet 顶部输入路径并点击"加载"触发 `load_elf()`

14. **数组支持 (ArrayElem)**:
    - DWARF 数组类型逐层构建嵌套 `TypeRef` 链: `float[7][7]` → TypeRef(name="float[7][7]") → TypeRef(name="float[7]") → TypeRef(name="float")
    - 树中每层为独立 `[]` 节点，`parent_id` 指向数组节点，`name` 默认 `[0]`
    - `BasicType::ArrayElem(Box<BasicType>, u64)` 存元素类型和数组长度
    - `basic_type_to_extend(ArrayElem)` → `ExtendType::Other`（不递归穿透多层）
    - Basic 栏显示 `[index]` + 可编辑 DragValue；Extend name/address 由 `compute_extend_name/address` 自然拼接 `A[2]`

15. **搜索层级递进匹配**:
    - `.` 分割层级，非末层用 `eq_ignore_ascii_case` 精确匹配，末层用 `contains` 模糊匹配
    - 匹配失败时**不再跳过当前层级深入搜索**，直接终止该分支
    - `[idx]` 记号自动展开为独立层级，匹配时校验 index 是否在 `[0, count)` 范围内
    - 搜索成功后更新树节点 name/address 并同步 `config.array_index`

16. **BottomSheet 拖拽**: 改用初始点 + 屏幕坐标相对位移替代 `drag_delta()` 逐帧累加，消除坐标漂移。写入前 `clamp(min_h, max_h)` 确保无越界帧。

17. **搜索居中滚动**: `scroll_target_id` 驱动，`count_nodes_before()` 基于 `tree_state` 计算可见节点数，`viewport_h` 居中偏移公式 `(count*24 - viewport_h/2 + 12).max(0)` 统一搜索和点击滚动。
