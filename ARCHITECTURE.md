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
│ [ELF 文件: ________] [加载]                                   │
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
├── app.rs                  # 主 App + 布局编排 (控制栏 + DockArea + BottomSheet 模态 + 对话窗锁)
├── model/
│   ├── mod.rs
│   ├── state.rs            # AppSession (连接/采样/BottomSheet/load_error/extend_configs)
│   └── variable_pool.rs    # VariablePool (Vec + HashMap, O(1) 增删查, 仅存extend数据)
├── probe/
│   ├── mod.rs
│   └── session.rs          # ProbeSession (probe-rs 连接/采集/读写)
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
    id: usize, name: String, struct_name: Option<String>,
    type_name: String, basic_type: BasicType,
    address: u64,                              // top-level: DWARF绝对地址; field: DWARF offset
    size: u32, children: Vec<TreeNode>,
}
```

- `address` 的语义：
  - 顶层变量：存储 DWARF 绝对地址（相当于根 base）
  - 结构体成员/嵌套字段：存储 DWARF `data_member_location` 的**原始 offset**
- extend 不存储在 TreeNode 中，改为通过 `DwarfApp` 的遍历方法动态计算
- `compute_extend_name()`: 从根开始逐级拼接变量名得到完整路径，如 `my_struct.status.flags`
- `compute_extend_address()`: 从根开始逐级累加 offset 得到实际绝对地址（类似 address 链式相加）

### ExtendConfig (用户可编辑的 Extend 数据)

```rust
pub struct ExtendConfig {
    pub name: String,          // 初始由 compute_extend_name() 计算，用户不可手动编辑
    pub address: u64,          // 初始由 compute_extend_address() 计算，用户可编辑
    pub ext_type: ExtendType,  // 初始由 basic_type_to_extend() 推导，用户可编辑
    pub size: u32,             // 初始 = node.size，随 ext_type 自动绑定，不可手动编辑
}
```

- 存储在 `AppSession.extend_configs: HashMap<usize, ExtendConfig>`，按 node_id 索引
- `vari_properties_ui()` 通过 `&mut ExtendConfig` 读写
- 首次选择节点时惰性初始化（line 212 in app.rs）
- "添加到 Chart/Table" 时，ExtendConfig 被消耗并存入 `VariablePool.add(config)`

### PooledVariable (池中仅存 extend 数据)

```rust
pub struct PooledVariable {
    pub id: usize,
    pub name: String,          // extend_name
    pub address: u64,          // extend_address
    pub ext_type: ExtendType,  // extend_type
    pub size: u32,             // extend_size
    pub current_value: Vec<u8>,
}
```

- 不再包含 `TreeNode`，只存实际用于采集和显示的数据
- `VariablePool.add(&ExtendConfig)` 创建条目
- 采集时 probe 直接读 `var.address` / `var.size`
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
  ├─ 顶部: ELF 文件路径输入框 + [加载] 按钮 + 错误提示

  ├─ 左面板: vari_tree_ui()
  │   ├─ 搜索栏: 输入变量名 → 模糊匹配 → 高亮 + 自动展开查找路径
  │   ├─ 搜索后自动滚动到第一个结果 (scroll_offset via count_visible_before)
  │   ├─ All/Search 模式切换 (切回 All 时自动全部折叠)
  │   └─ egui_ltreeview::TreeView (NodeBuilder::default_open(false)):
  │       DWARF 编译单元 → 变量 → 结构体成员 (递归, 默认折叠)

  └─ 右面板: vari_properties_ui(config: &mut ExtendConfig) — 三段竖直布局
       ├─ Basic (只读): Name / Address(offset) / Size / Type (DWARF 原始值)
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

### 4. 数据采集

```
控制栏点"开始" → session.running = true

每帧循环 (MemRW3App::ui):
  ├─ self.probe.running = self.session.running       // 同步状态
  ├─ self.probe.acquire(&mut pool, delay_us)         // 采集
  │   └─ 节流: last_read.elapsed() >= delay_us 时才执行
  │   └─ 使用 PooledVariable 的 extend 属性:
  │       addr = var.address
  │       size = var.size
  │       read 按 size 选择: read_word_8/16/32/64 或 block read
  │   └─ var.current_value = 读取的字节 Vec<u8>
  └─ ctx.request_repaint()                            // 持续刷新

Chart 面板 (chart_panel):
  └─ if running: 从 pool.get(legend.variable_id) 读取
  │   └─ decode_value_f64(&current_value, &var.ext_type)
  │       按 ExtendType 解析: u8/u16/u32/u64 → 无符号整型
  │                         i8/i16/i32/i64 → 有符号整型
  │                         Float → f32→f64, Double → f64
  │                         Other → 0.0 (不绘制)
  └─ push_value(time, val) → 图表自动绘制曲线

Table 面板 (table_panel):
  └─ 从 pool.get(entry.variable_id) 读取
      └─ format_value(&current_value, &var.ext_type)
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
  ├─ add(config)  → O(1) push + insert
  ├─ remove(id)   → O(1) swap_remove + 更新被交换项的 index
  ├─ get(id)      → O(1) id_index → Vec[index]
  └─ iter_mut()   → 直接迭代 Vec (采集循环用)

PooledVariable { id, name, address, ext_type, size, current_value: Vec<u8> }
                                                      ↑ 采集的原始字节, 解析由各面板按 ext_type 完成
```

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
| 曲线属性 Dialog | 曲线名/颜色/缓冲/可见 + **PooledVariable 的 Extend 属性** (名称/地址/类型/大小) + 删除/确定/取消 |
| 添加配置颜色 | `egui::color_picker::color_edit_button_srgba()` 自定义拾色器 + 预设色块网格, egui memory 持久化 |
| 空状态 | 居中提示"暂无监控变量" + 打开变量树按钮 |

### 7. Table 读写面板特性

| 功能 | 实现 |
|------|------|
| 表格列 | Name / Value / Write / ✕(删除) |
| Name | `entry.display_name` (从添加配置持久化) |
| Value | 按 `var.ext_type` 格式化: u/i → hex+十进制, float/double → 小数, other → hex dump |
| Write | TextEdit 输入 → "写"按钮 → 暂存到 current_value |
| 双击行 | 弹出居中 table_entry_dialog (模态: 全界面拦截对话窗外点击) |
| 变量属性 Dialog | 显示名/当前值 + **PooledVariable 的 Extend 属性** (名称/地址/类型/大小) + 删除/确定/取消 |
| 空状态 | 居中提示 + 打开变量树按钮 |

### 8. 模态 (Modal) 行为

全部模态采用双层拦截机制：`add_enabled_ui` + Z-order click interceptor (`ui.interact`)，确保 egui_dock 的 tab headers 也被阻止。

| 覆盖层 | 阻塞范围 | 拦截方式 | 退出方式 |
|--------|----------|----------|----------|
| BottomSheet | 全界面 (控制栏 + DockArea) | `dock_ui.disable()` + `ui.interact(dock_rect)` + `ctrl_ui.add_enabled_ui(false)` | [关闭] 按钮 |
| 曲线属性 line_dialog | 全界面 (控制栏 + DockArea) | 同上, `dialog_open` 包含 `show_line_dialog` 时触发全锁 | [确定]/[取消]/[删除] |
| 变量属性 table_dialog | 全界面 (控制栏 + DockArea) | 同上, `dialog_open` 包含 `show_entry_dialog` 时触发全锁 | [确定]/[取消]/[删除] |
| 设置 Dialog | 全界面 (控制栏 + DockArea) | 同上, `dialog_open` 包含 `probe.show_settings` 时触发全锁 | [确定]/[取消] |

**`dialog_open`** 在 `app.rs` 顶部计算，聚合三者的开闭状态：

```rust
let dialog_open = self.chart_state.show_line_dialog
    || self.table_state.show_entry_dialog
    || self.probe.show_settings;
```

控制栏锁: `ctrl_ui.add_enabled_ui(!bs_open && !dialog_open, ...)`  
Dock 锁: `dock_ui.disable()` + `ui.interact(dock_rect, ...)` 在 `bs_open || dialog_open` 时触发

**Z-order 拦截原理**: `ui.interact()` 在 DockArea 渲染**之后**、BottomSheet 渲染**之前**执行。egui 的 hit-testing 按插入逆序（后渲染优先）处理，因此：
- interceptor 覆盖 dock 区域（吞掉所有点击）
- BottomSheet 在 interceptor 之后渲染，其自身控件优先于 interceptor

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
   - `compute_extend_name()`: 从根开始逐级拼接，如 `my_struct.field1.subfield`
   - `compute_extend_address()`: 根绝对地址 + 所有路径节点的 offset 累加
   - 两者默认不可编辑

5. **PooledVariable 仅存 extend 数据**
   - 不再包含 `TreeNode` 克隆，只存 `name/address/ext_type/size/current_value`
   - 采集直接读 `var.address`、`var.size`，解码直接用 `var.ext_type`
   - 对话窗显示的是 PooledVariable 的 extend 属性，非 TreeNode 的 basic 属性

6. **BottomSheet 模态覆盖 + 全界面锁**
   - 不在每个 Tab 内分割空间, 而是在 DockArea 上方叠加
   - 打开时通过双层机制锁定全界面: `dock_ui.disable()` + Z-order `ui.interact(dock_rect, ...)` click interceptor
   - 控制栏通过 `add_enabled_ui(!bs_open && !dialog_open)` 锁定
   - 只能通过 [关闭] 按钮退出, 防止误触

7. **对话窗全界面锁**
   - line_dialog、table_dialog、probe_settings 三个对话窗统一通过 `dialog_open` 标志
   - 与 BottomSheet 使用相同的拦截机制，全界面不可交互直至对话窗关闭
   - `egui::Window` 的自身交互不受影响（Window 在更高 Layer，优先于 interceptor）

8. **VariablePool 用 Vec+HashMap**: 模拟链表 + 哈希对, O(1) 增删查, 比纯 HashMap 更适合频繁迭代的采集场景

9. **Chart 插件与 Table 插件独立**: 各自管理状态 (ChartPluginState / TablePluginState), 通过 VariablePool 共享数据

10. **添加配置回调注入**: `vari_properties_ui()` 通过 `FnOnce` 闭包接收插件定制的添加 UI, 避免 centralized enum dispatch; 添加后回写用户选择的曲线名/颜色/显示名

11. **probe-rs 采集在主线程**: 每帧节流读取, `request_repaint()` 保持 UI 刷新, 无需额外线程

12. **Tree View 默认折叠, 搜索自动滚动**: 使用 `NodeBuilder::default_open(false)` 初始化所有树节点为折叠状态; 搜索后通过 `count_visible_before()` 预估结果位置, 使用 `ScrollArea::vertical_scroll_offset()` 自动滚动到第一个匹配项

13. **ELF 文件延迟加载**: 不通过命令行参数加载, App 启动为空, 用户在 BottomSheet 顶部输入路径并点击"加载"触发 `load_elf()`
