# MemRW3 — 工作流程与布局架构

## 项目概述

MemRW3 是一个基于 Rust + egui + probe-rs 的嵌入式内存读写与变量监控工具，是对原 Qt/QML MemRW2 的重构。使用 gimli/object 替代 libdwarf 解析 DWARF 调试信息，使用 probe-rs 替代 libusb 手动协议解析进行 MCU 数据采集，使用 eframe + egui_dock 替代 Qt QML 实现 UI。

## 整体布局

```
┌──────────────────────────────────────────────────────────────┐
│ 控制栏 (Control Bar)                                         │
│ [连接/断开] [开始/暂停] [⚙设置] [延迟] [Reset]     Hz: xxx  ● 采集中 │
├──────────────────────────────────────────────────────────────┤
│ DockArea: [Chart 实时数据 | Table 读写数据]                   │
│ ┌──────────────────────────┬───────────────────────────────┐ │
│ │                          │                               │ │
│ │   Chart 图表区            │   Table 表格区                 │ │
│ │   [坐标轴+曲线+图例]       │   [Name | Value | Write | ✕]  │ │
│ │                          │                               │ │
│ └──────────────────────────┴───────────────────────────────┘ │
├──────────────────────────────────────────────────────────────┤ ← BottomSheet 覆盖层
│ 变量列表 (DWARF Tree)                              [关闭]    │
│ ┌─────────────────────────┬────────────────────────────────┐ │
│ │ Search: [________]       │ 属性                            │ │
│ │ [All] [Search]           │ Name: xxx  Type: xxx            │ │
│ │                          │ Address: 0x...  Size: xx        │ │
│ │  Tree View               ├────────────────────────────────┤ │
│ │   ├─ cu_name             │ 添加配置                        │ │
│ │   │  ├─ var1             │ [曲线名/颜色] → [添加到 Chart]  │ │
│ │   │  └─ var2             │ 或                             │ │
│ │   └─ cu_name2            │ [显示名] → [添加到 Table]       │ │
│ └─────────────────────────┴────────────────────────────────┘ │
└──────────────────────────────────────────────────────────────┘
```

## 模块架构

```
src/
├── main.rs                 # 入口: 加载 ELF → DWARF 解析 → 启动 eframe
├── types.rs                # 数据类型: TreeNode, CuInfo, DwarfApp, TypeRef
├── dwarf.rs                # DWARF 解析 (gimli)
├── app.rs                  # 主 App + 布局编排 (控制栏 + DockArea + BottomSheet)
├── model/
│   ├── mod.rs
│   ├── state.rs            # AppSession (连接/采样/BottomSheet 状态)
│   └── variable_pool.rs    # VariablePool (Vec + HashMap, O(1) 增删查)
├── probe/
│   ├── mod.rs
│   └── session.rs          # ProbeSession (probe-rs 连接/采集/读写)
└── ui/
    ├── mod.rs
    ├── control_bar.rs      # 控制栏 (连接/采集/Probe配置Dialog)
    ├── chart_plugin/
    │   ├── mod.rs
    │   ├── legend.rs       # ChartLegend (曲线名/颜色/可见/缓冲/data_history)
    │   ├── panel.rs        # 图表面板 (坐标轴/曲线/图例覆层)
    │   └── line_dialog.rs  # 曲线属性 Dialog (编辑/删除/取消)
    ├── table_plugin/
    │   ├── mod.rs
    │   ├── panel.rs        # 表格面板 (TableView 读写)
    │   └── table_dialog.rs # TableEntry + 属性 Dialog
    ├── vari_tree.rs        # DWARF 变量树 (左面板)
    └── vari_properties.rs  # 属性预览 + 添加配置 (右面板)
```

## 完整工作流

### 1. 启动

```
main.rs
  ├─ 读取命令行参数 <firmware.elf>
  ├─ object::File::parse() → 解析 ELF
  ├─ dwarf::load_dwarf() → 加载 DWARF sections
  ├─ dwarf::collect_cus() → 遍历 CU, 提取变量树 (TreeNode 递归结构)
  ├─ DwarfApp::new(cus) → 初始化 tree_state + 搜索状态
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

BottomSheet (全局窗口级覆盖层, 不挤压 DockArea)
  ├─ 左面板: vari_tree_ui()
  │   ├─ 搜索栏: 输入变量名 → 模糊匹配 → 高亮 + 自动展开
  │   ├─ All/Search 模式切换
  │   └─ egui_ltreeview::TreeView: DWARF 编译单元 → 变量 → 结构体成员 (递归)
  │
  └─ 右面板: vari_properties_ui()
      ├─ 属性: Name / Type / Address / Size
      ├─ 添加配置 (由调用方注入回调):
      │   ├─ Chart Tab 打开时 → 曲线名 + 颜色选择 → "添加到 Chart"
      │   └─ Table Tab 打开时 → 显示名 → "添加到 Table"
      └─ 点击添加 → VariablePool.add(node) → ChartPlugin.add_from_pool() 或 TablePlugin.add_from_pool()
```

### 4. 数据采集

```
控制栏点"开始" → session.running = true

每帧循环 (MemRW3App::ui):
  ├─ self.probe.running = self.session.running       // 同步状态
  ├─ self.probe.acquire(&mut pool, delay_us)         // 采集
  │   └─ 节流: last_read.elapsed() >= delay_us 时才执行
  │   └─ session.core(0).read_word_32(addr)          // 逐一读取池中变量
  │   └─ var.current_value = val.to_le_bytes()       // 更新内存值
  └─ ctx.request_repaint()                            // 持续刷新

Chart 面板 (chart_panel):
  └─ if running: 从 pool.get(legend.variable_id) 读取 → 解析 u32 → push_value(time, val)
  └─ 图表自动绘制曲线 (从 data_history)

Table 面板 (table_panel):
  └─ 从 pool.get(entry.variable_id) 读取 current_value → 显示 hex + 十进制
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
```

### 6. Chart 图表面板特性

| 功能 | 实现 |
|------|------|
| 坐标轴 | Y 轴 7 格 + 数值标签, X 轴 6 格 + 时间标签(s) |
| 网格线 | 自适应深色/浅色 |
| 曲线绘制 | 从 `data_history: VecDeque<(time, value)>` 读取, 折线连接 |
| 图例 (Legend) | 图表右上角浮动: `[色条] 曲线名 = 当前值` |
| 单击图例 | 切换 visible (曲线消失/恢复, 图例变暗) |
| 双击图例 | 弹出居中 Dialog: 编辑曲线名/颜色/缓冲/可见, 删除/确定/取消 |
| 数据源 | `running && !paused` 时从 VariablePool 读取实时值 |
| 空状态 | 居中提示"暂无监控变量" + 打开变量树按钮 |

### 7. Table 读写面板特性

| 功能 | 实现 |
|------|------|
| 表格列 | Name / Value / Write / ✕(删除) |
| Name | `entry.display_name` (可编辑) |
| Value | 从 VariablePool 读取: `0xXXXXXXXX (十进制)` |
| Write | TextEdit 输入 → "写"按钮 → 暂存到 current_value |
| 双击行 | 弹出居中 Dialog: 编辑显示名/删除/确定/取消 |
| 空状态 | 居中提示 + 打开变量树按钮 |

### 8. 控制栏配置 Dialog

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

1. **BottomSheet 为全局覆盖层**: 不在每个 Tab 内分割空间, 而是在 DockArea 上方叠加, 点击 DockArea 自动关闭
2. **VariablePool 用 Vec+HashMap**: 模拟链表 + 哈希对, O(1) 增删查, 比纯 HashMap 更适合频繁迭代的采集场景
3. **Chart 插件与 Table 插件独立**: 各自管理状态 (ChartPluginState / TablePluginState), 通过 VariablePool 共享数据
4. **添加配置回调注入**: `vari_properties_ui()` 通过 `FnOnce` 闭包接收插件定制的添加 UI, 避免 centralized enum dispatch
5. **probe-rs 采集在主线程**: 每帧节流读取, `request_repaint()` 保持 UI 刷新, 无需额外线程
