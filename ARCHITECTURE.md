# MemRW3 — 工作流程与布局架构

## 项目概述

MemRW3 是一个基于 Rust + egui + probe-rs 的嵌入式内存读写与变量监控工具，是对原 Qt/QML MemRW2 的重构。使用 gimli/object 替代 libdwarf 解析 DWARF 调试信息，使用 probe-rs 替代 libusb 手动协议解析进行 MCU 数据采集，使用 eframe + egui_dock 替代 Qt QML 实现 UI。

## 整体布局

```
┌──────────────────────────────────────────────────────────────┐
│ 控制栏 (Control Bar)                                         │
│ [连接/断开] [开始/暂停] [⚙设置] [延迟] [Reset]     Hz: xxx  ● 采集中 │
├──────────────────────────────────────────────────────────────┤ ← 模态阻塞: 不可交互
│ DockArea: [Chart 实时数据 | Table 读写数据]                   │
│ ┌──────────────────────────┬───────────────────────────────┐ │
│ │                          │                               │ │
│ │   Chart 图表区            │   Table 表格区                 │ │
│ │   [坐标轴+曲线+图例]       │   [Name | Value | Write | ✕]  │ │
│ │                          │                               │ │
│ └──────────────────────────┴───────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤ ← BottomSheet 覆盖层 (可交互)
│ 变量列表 (DWARF Tree)                              [关闭]    │
│ ┌─────────────────────────┬────────────────────────────────┐ │
│ │ Search: [________]       │ 属性                            │ │
│ │ [All] [Search]           │ ── Basic (只读) ──              │ │
│ │                          │ Name: xxx  Address: 0x...       │ │
│ │  Tree View (默认折叠)     │ Size: xx    Type: xxx           │ │
│ │   ├─ cu_name             │ ── Extend (可编辑) ──           │ │
│ │   │  ├─ var1             │ Name: xxx  Address: [0x...]     │ │
│ │   │  └─ struct           │ Type: [u32 ▼] Size: [4▼]       │ │
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
├── main.rs                 # 入口: 加载 ELF → DWARF 解析 → 启动 eframe
├── types.rs                # 数据类型: TreeNode, BasicType, ExtendType, CuInfo, DwarfApp, TypeRef
├── dwarf.rs                # DWARF 解析 (gimli), basic_type 映射
├── app.rs                  # 主 App + 布局编排 (控制栏 + DockArea + BottomSheet 模态)
├── model/
│   ├── mod.rs
│   ├── state.rs            # AppSession (连接/采样/BottomSheet 状态)
│   └── variable_pool.rs    # VariablePool (Vec + HashMap, O(1) 增删查)
├── probe/
│   ├── mod.rs
│   └── session.rs          # ProbeSession (probe-rs 连接/采集/读写, 使用 extend_* 属性)
└── ui/
    ├── mod.rs
    ├── control_bar.rs      # 控制栏 (连接/采集/Probe配置Dialog)
    ├── chart_plugin/
    │   ├── mod.rs
    │   ├── legend.rs       # ChartLegend (曲线名/颜色/可见/缓冲/data_history)
    │   ├── panel.rs        # 图表面板 (坐标轴/曲线/图例覆层/ExtendType 解码)
    │   └── line_dialog.rs  # 曲线属性 Dialog (编辑曲线属性 + 显示变量 Extend 属性)
    ├── table_plugin/
    │   ├── mod.rs
    │   ├── panel.rs        # 表格面板 (TableView 读写/ExtendType 格式化)
    │   └── table_dialog.rs # TableEntry + 属性 Dialog (显示变量 Extend 属性)
    ├── vari_tree.rs        # DWARF 变量树 (左面板, DefaultOpen(false) 折叠)
    └── vari_properties.rs  # 属性面板 (Basic/Extend/Add 三段竖直布局)
```

## 核心数据类型

### TreeNode
```
TreeNode {
    // ── Basic (DWARF 原始属性, 只读) ──
    id: usize, name: String, struct_name: Option<String>,
    type_name: String, basic_type: BasicType,
    address: u64, size: u32, children: Vec<TreeNode>,

    // ── Extend (用户可编辑, 存入 VariablePool, 驱动采集) ──
    extend_name: Option<String>,          // 默认: struct.name
    extend_address: Option<u64>,          // 默认: address
    extend_type: Option<ExtendType>,      // 默认: basic_type 映射
    extend_size: Option<u32>,             // 默认: size, 随 extend_type 自动绑定
}
```

### BasicType vs ExtendType

| BasicType (DWARF 原始) | ExtendType (用户可选) |
|---|---|
| U8, U16, U32, U64 | U8, U16, U32, U64 |
| I8, I16, I32, I64 | I8, I16, I32, I64 |
| Float, Double | Float, Double |
| Pointer → U64 | — |
| Struct(String) → Other | Other |
| Other(String) → Other | — |

**关键规则**: `extend_type = Other` 的变量**不可添加到 Chart 或 Table**。

### ExtendType ↔ Size 自动绑走

| ExtendType | 默认 Size |
|---|---|
| U8, I8 | 1 |
| U16, I16 | 2 |
| U32, I32, Float | 4 |
| U64, I64, Double | 8 |
| Other | 不自动设置 (保留 basic.size) |

## 完整工作流

### 1. 启动

```
main.rs
  ├─ 读取命令行参数 <firmware.elf>
  ├─ object::File::parse() → 解析 ELF
  ├─ dwarf::load_dwarf() → 加载 DWARF sections
  ├─ dwarf::collect_cus() → 遍历 CU, 提取变量树 (TreeNode 递归结构)
  │   └─ type_name_to_basic_type() 根据 DWARF type name/size 映射 BasicType
  ├─ DwarfApp::new(cus) → 初始化 tree_state + 搜索状态 (默认折叠)
  └─ eframe::run_native() → 启动 UI
```

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

BottomSheet (模态覆盖层, 打开时 DockArea 不可交互, 只能点 [关闭] 按钮退出)
  ├─ 左面板: vari_tree_ui()
  │   ├─ 搜索栏: 输入变量名 → 模糊匹配 → 高亮 + 自动展开查找路径
  │   ├─ All/Search 模式切换 (切回 All 时自动全部折叠)
  │   └─ egui_ltreeview::TreeView (NodeBuilder::default_open(false)):
  │       DWARF 编译单元 → 变量 → 结构体成员 (递归, 默认折叠)
  │
  └─ 右面板: vari_properties_ui() — 三段竖直布局
       ├─ Basic (只读): Name / Address / Size / Type (DWARF 原始值)
       ├─ Extend (可编辑): Name / Address(hex) / Size(随 Type 自动绑定) /
       │   Type(ComboBox: u8~u64, i8~i64, float, double, other)
       └─ Add:
           ├─ type ≠ other → 显示添加配置 → "添加到 Chart/Table"
           └─ type = other → 红色提示 "type 为 other，不可添加到 Chart 或 Table"

      添加流程:
       ├─ 用户编辑 extend_* 属性 → 存入 node (TreeNode)
       ├─ 点 "添加到 Chart/Table" → VariablePool.add(node)
       │   └─ 存入的 TreeView 包含 extend_* 属性
       ├─ Chart: 曲线名 + 颜色 → 存入 ChartLegend (回写用户选择)
       └─ Table: 显示名 → 存入 TableEntry (回写用户选择)
```

### 4. 数据采集

```
控制栏点"开始" → session.running = true

每帧循环 (MemRW3App::ui):
  ├─ self.probe.running = self.session.running       // 同步状态
  ├─ self.probe.acquire(&mut pool, delay_us)         // 采集
  │   └─ 节流: last_read.elapsed() >= delay_us 时才执行
  │   └─ 使用 extend 属性:
  │       addr = extend_address.unwrap_or(address)
  │       size = extend_size.unwrap_or(size)
  │       read 按 size 选择: read_word_8/16/32/64 或 block read
  │   └─ var.current_value = 读取的字节 Vec<u8>
  └─ ctx.request_repaint()                            // 持续刷新

Chart 面板 (chart_panel):
  └─ if running: 从 pool.get(legend.variable_id) 读取
  │   └─ decode_value_f64(&current_value, &extend_type)
  │       按 ExtendType 解析: u8/u16/u32/u64 → 无符号整型
  │                         i8/i16/i32/i64 → 有符号整型
  │                         Float → f32→f64, Double → f64
  │                         Other → 0.0 (不绘制)
  └─ push_value(time, val) → 图表自动绘制曲线

Table 面板 (table_panel):
  └─ 从 pool.get(entry.variable_id) 读取
      └─ format_value(&current_value, &extend_type)
          按 ExtendType 格式化:
          u8/u16/u32/u64 → "0xXXXX (十进制)"
          i8/i16/i32/i64 → "0xXXXX (有符号十进制)"
          Float → "小数", Double → "小数"
          Other → hex dump
```

### 5. VariablePool 数据结构

```
Vec<PooledVariable> + HashMap<usize, usize> (id → index)

操作复杂度:
  ├─ add(node)    → O(1) push + insert
  ├─ remove(id)   → O(1) swap_remove + 更新被交换项的 index
  ├─ get(id)      → O(1) id_index → Vec[index]
  └─ iter_mut()   → 直接迭代 Vec (采集循环用)

PooledVariable { id, tree_node: TreeNode, current_value: Vec<u8> }
                                              ↑ 采集的原始字节, 解析由各面板按 ExtendType 完成
```

### 6. Chart 图表面板特性

| 功能 | 实现 |
|------|------|
| 坐标轴 | Y 轴 7 格 + 数值标签, X 轴 6 格 + 时间标签(s) |
| 网格线 | 自适应深色/浅色 |
| 曲线绘制 | 从 `data_history: VecDeque<(time, value)>` 读取, 折线连接 |
| 值解码 | 按 `extend_type` 解析: u/i/float/double → f64, Other → 0.0 |
| 图例 (Legend) | 图表右上角浮动: `[色条] 曲线名 = 当前值` |
| 单击图例 | 切换 visible (曲线消失/恢复, 图例变暗) |
| 双击图例 | 弹出居中 Dialog (模态: 面板背景不可交互) |
| 曲线属性 Dialog | 曲线名/颜色/缓冲/可见 + **变量 Extend 属性** (名称/地址/类型/大小) + 删除/确定/取消 |
| 添加配置 | 曲线名 + 颜色选择 → 存入 ChartLegend (颜色持久化) |
| 空状态 | 居中提示"暂无监控变量" + 打开变量树按钮 |

### 7. Table 读写面板特性

| 功能 | 实现 |
|------|------|
| 表格列 | Name / Value / Write / ✕(删除) |
| Name | `entry.display_name` (从添加配置持久化) |
| Value | 按 `extend_type` 格式化: u/i → hex+十进制, float/double → 小数, other → hex dump |
| Write | TextEdit 输入 → "写"按钮 → 暂存到 current_value |
| 双击行 | 弹出居中 Dialog (模态: 面板背景不可交互) |
| 变量属性 Dialog | 显示名/当前值 + **变量 Extend 属性** (名称/地址/类型/大小) + 删除/确定/取消 |
| 空状态 | 居中提示 + 打开变量树按钮 |

### 8. 模态 (Modal) 行为

所有覆盖层均采用模态模式：打开时阻止背景交互，只能通过自身的关闭按钮退出。

| 覆盖层 | 阻塞范围 | 退出方式 |
|--------|----------|----------|
| BottomSheet | DockArea 整体 (`dock_ui.disable()`) | [关闭] 按钮 |
| 曲线属性 Dialog | 图表面板背景 (`add_enabled_ui(false)`) | [确定]/[取消]/[删除] |
| 变量属性 Dialog | 表格面板背景 (`add_enabled_ui(false)`) | [确定]/[取消]/[删除] |
| 设置 Dialog | 通过 `egui::Window` 自然优先捕获输入 | [确定]/[取消] |

### 9. 控制栏配置 Dialog

```
"⚙ 设置" → egui::Window (居中, 可取消)

内容:
  ├─ MCU 型号: ComboBox (9 个常用芯片)
  ├─ 协议: SWD / JTAG
  ├─ 速度: Slider 100-20000 kHz
  ├─ 刷新按钮: Lister::list_all() 扫描已连接 Probe
  ├─ 错误显示: 连接失败时红色文字
  └─ 确定/取消按钮 (设置下次连接生效)
```

## 依赖

```toml
eframe = "0.34"           # GUI 框架
egui_dock = "0.19"        # Dock 面板 (tabbed/horizontal/vertical)
egui_ltreeview = "0.7.0"  # 树形视图 (DWARF 变量树)
probe-rs = "0.31"         # MCU 调试 (CMSIS-DAP/ST-Link/J-Link)
gimli = "0.31"            # DWARF 解析
object = "0.36"           # ELF 解析
anyhow = "1.0"            # 错误处理
```

## 关键设计决策

1. **Extend 属性层与 Basic 属性分离**
   - `TreeNode.basic_type/address/size` 保留 DWARF 原始解析结果 (只读展示)
   - `TreeNode.extend_*` 为用户可编辑属性, 存入 VariablePool, 驱动数据采集的地址/大小/类型解析
   - 首次展示时 extend 从 basic 自动推导初始化

2. **ExtendType 限制类型集合**
   - 仅 11 种可采集类型: u8/u16/u32/u64/i8/i16/i32/i64/float/double/other
   - DWARF 原始类型 Pointer → U64, Struct/Other → Other
   - Other 类型禁止添加到 Chart/Table, 避免类型不匹配的采集错误

3. **extend_size 与 extend_type 自动绑定**
   - 切换 extend_type 时自动更新 extend_size: u8→1, u16→2, u32→4, u64→8
   - Other 类型不自动绑定, 保留 basic.size
   - 用户仍可手动覆盖 extend_size

4. **BottomSheet 模态覆盖**
   - 不在每个 Tab 内分割空间, 而是在 DockArea 上方叠加
   - 打开时 `dock_ui.disable()` 禁用 DockArea 所有交互
   - 只能通过 [关闭] 按钮退出, 防止误触

5. **Dialog 模态**: 曲线/变量属性 Dialog 打开时, 对应面板背景通过 `add_enabled_ui(false)` 禁用, Dialog 窗口本身在顶层保持可交互

6. **VariablePool 用 Vec+HashMap**: 模拟链表 + 哈希对, O(1) 增删查, 比纯 HashMap 更适合频繁迭代的采集场景

7. **Chart 插件与 Table 插件独立**: 各自管理状态 (ChartPluginState / TablePluginState), 通过 VariablePool 共享数据

8. **添加配置回调注入**: `vari_properties_ui()` 通过 `FnOnce` 闭包接收插件定制的添加 UI, 避免 centralized enum dispatch; 添加后回写用户选择的曲线名/颜色/显示名

9. **probe-rs 采集在主线程**: 每帧节流读取, `request_repaint()` 保持 UI 刷新, 无需额外线程

10. **Tree View 默认折叠**: 使用 `NodeBuilder::default_open(false)` 初始化所有树节点为折叠状态, 仅搜索时自动展开匹配路径
