# AsterDriveMigration 目标架构设计

本文档将现有架构图和文件/对象关系图转写为可实施的文字设计。它描述的是迁移工具的目标架构，同时明确当前代码已经覆盖的部分和后续仍需实现的部分。

配套字段文档：[`cloudreve-to-asterdrive-field-mapping.md`](cloudreve-to-asterdrive-field-mapping.md)。

## 1. 设计目标

AsterDriveMigration 不应成为一组把源表逐行复制到目标表的脚本，而应成为一个可恢复、可校验、可扩展的数据迁移引擎。

核心目标：

| 目标 | 说明 |
|---|---|
| 无直接表复制 | 读取源系统对象，转换为统一领域模型，再通过 AD writer 写入目标 |
| 源系统隔离 | Cloudreve 特有字段、状态和存储规则只存在于 Cloudreve Adapter |
| 目标系统隔离 | AD schema、事务、去重、配额和 ref_count 规则只存在于 AsterDrive Writer |
| 可恢复 | 每个 stage 保存 cursor，进程中断后可以从 checkpoint 继续 |
| 幂等 | 同一个源对象重复处理时，通过 object mapping 找到已有目标对象，不重复创建 |
| 可校验 | 迁移后能够重新读取源和目标，验证数量、关系、大小、哈希、配额和引用计数 |
| 可审计 | 每次运行保留状态、统计、错误、警告和对象映射 |
| 可扩展 | 后续增加 Alist、FileBrowser、Nextcloud 等 source adapter 时不修改 AD writer 核心 |
| 大文件友好 | 文件内容使用流式读写，不将整个对象载入内存 |

## 2. 核心原则

### 2.1 禁止直接复制表

不采用以下模式：

```text
Cloudreve users row -> INSERT INTO AsterDrive users
Cloudreve files row -> INSERT INTO AsterDrive files
```

采用以下模式：

```text
Cloudreve DB/Storage
    -> Cloudreve Adapter
    -> ExternalUser / ExternalFolder / ExternalFile / ExternalBlob
    -> Migration Core
    -> AsterDrive Writer
    -> AsterDrive DB/Storage
```

原因是 Cloudreve 和 AD 的表语义不同。例如 Cloudreve `files` 同时表示文件和目录，而 AD 使用 `files`、`folders`、`file_blobs` 和 `file_versions` 分别建模。

### 2.2 映射驱动，而不是 ID 假设驱动

源 ID 和目标 ID 不能假设相等。所有跨对象关系必须通过持久化 object mapping 解析：

```text
(source_type, source_id) -> (target_type, target_id)
```

典型映射：

| 源对象 | 目标对象 |
|---|---|
| Cloudreve user ID | AD user ID |
| Cloudreve folder/file record ID | AD folder ID 或 file ID |
| Cloudreve entity ID/object key | AD file_blob ID |
| Cloudreve share ID | AD share ID |
| Cloudreve policy ID | AD storage_policy ID |

### 2.3 文件记录与物理对象分离

迁移模型必须区分：

- `ExternalFile`：用户看到的逻辑文件，包含名称、父目录、owner、大小、MIME。
- `ExternalBlob`：存储系统中的物理对象，包含 object key、策略、大小、可选哈希和读取方式。

多个逻辑文件可以复用一个 AD `file_blob`。同一逻辑文件的旧 blob 应写入 `file_versions`，当前 blob 写入 `files.blob_id`。

## 3. 分层架构

## 3.1 Layer 1：Interface Layer

负责用户交互、配置和输出，不承载源系统或 AD 业务规则。

### CLI Commands

目标命令集合：

| 命令 | 作用 |
|---|---|
| `migrate start` | 创建迁移运行，执行预检、规划和迁移 |
| `migrate list` | 列出历史运行及状态 |
| `migrate resume` | 从 checkpoint 恢复中断运行 |
| `migrate verify` | 对已完成或部分完成的运行执行一致性校验 |
| `migrate report` | 导出人类可读或 JSON 报告 |

当前代码提供的是简化命令：

| 当前命令 | 作用 |
|---|---|
| `check` | 连接源/目标数据库，检查 schema 和源数据概况 |
| `migrate` | 在一个目标数据库事务中执行一次性迁移 |

### TUI Progress View（可选）

目标能力：

- 实时迁移进度
- 当前 stage 和状态
- 对象吞吐量、字节吞吐量
- 错误与重试次数
- 预计剩余时间 ETA

TUI 只订阅 engine progress event，不直接访问数据库或实现迁移逻辑。

### Config Loader

配置来源：

- 配置文件，例如 `config.yaml`
- 环境变量
- secret provider 或运行时安全输入
- source 和 target 数据库连接
- source 和 target 存储连接
- 冲突处理、并发、重试和验证选项

密码、数据库凭据、OAuth token、存储 secret 和任务 private state 不得出现在普通日志、迁移报告或 TUI 中。JSON 报告按需求包含重新签发的 direct-link URL；该 URL 本身是公开访问能力，报告文件必须按敏感文件保护。

### Output Renderer

至少支持：

- Human Text：供管理员直接阅读
- JSON：供自动化、CI 或外部控制器消费
- Machine-readable exit code：区分成功、警告完成、校验失败和运行失败

## 3.2 Layer 2：Migration Core

Migration Core 编排完整流程，但不直接理解 Cloudreve 表或 AD Entity。

### Engine

职责：

- 创建或恢复 migration run
- 控制完整生命周期
- 管理取消信号和优雅停止
- 协调并发任务
- 汇总错误和最终状态
- 确保阶段切换符合状态机

建议运行状态：

```text
planned -> running -> verifying -> completed
                    -> completed_with_warnings
                    -> failed
                    -> cancelled
```

### Stage Runner

职责：

- 按依赖顺序执行 stages
- 控制每个 stage 的并行度
- 实现可配置 retry 和 exponential backoff
- 在每批数据完成后保存 cursor
- 支持安全停止和 resume

建议 stage 顺序：

| 顺序 | Stage | 主要输出 |
|---:|---|---|
| 1 | `preflight` | schema、版本、连接、权限和存储能力检查 |
| 2 | `inventory` | 源对象数量、字节数和不兼容能力清单 |
| 3 | `policies` | AD storage policy 及 ID mapping |
| 4 | `policy_groups` | AD policy group、group item 及 mapping |
| 5 | `users` | AD user/profile 及 mapping |
| 6 | `folders` | 目录树及 parent mapping |
| 7 | `blobs` | 对象引用、opaque key 和 blob mapping |
| 8 | `files` | 当前文件和历史版本 |
| 9 | `shares` | 分享和新 token |
| 10 | `metadata_tags` | properties、Cloudreve `tag:*` 到 AD 原生标签及关联 |
| 11 | `direct_links` | 使用 AD secret 重新签发 v2 URL，保存 source link -> URL 映射 |
| 12 | `recalculate` | ref_count、storage_used、配额和统计重算 |
| 13 | `verify` | 数量、关系、哈希、直链签名和可读性校验 |
| 14 | `finalize` | 最终报告和运行状态 |

### Plan Builder

职责：

- 扫描源 inventory
- 识别源版本和 capabilities
- 判断哪些资源可迁移、跳过或必须中止
- 生成 `MigrationPlan`
- 估算对象数、总字节数、阶段成本和风险

计划中应明确：

- 用户、目录、文件、blob、版本、分享数量
- 各存储策略的对象数量和字节数
- 不兼容的存储驱动
- Cloudreve 加密对象数量
- symbolic/placeholder 文件数量
- 重名冲突和孤儿关系数量
- 预计需要的目标存储空间

### Checkpoint Manager

职责：

- 保存和加载 run state
- 保存每个 stage 的 cursor/offset
- 保存 object mapping
- 提供 resume 支持
- 判断重复执行是否安全

checkpoint 必须在目标记录成功提交后更新，不能先推进 cursor 再写目标数据。

### Object Mapping Registry

职责：

- 根据 source type + ID 查询目标对象
- 创建或更新映射
- 保存源对象哈希、元数据摘要和状态
- 支持 lookup、upsert 和验证
- 为断点续传、去重和冲突处理提供依据

映射不只保存整数 ID。Cloudreve blob 还可能使用 object key 作为稳定源标识。

### Conflict Resolver

需要检测：

- 用户名或邮箱冲突
- 同目录同名文件/目录冲突
- 目标对象已经存在但来源不同
- mapping 指向的目标对象已被删除或修改
- 相同 hash+policy 已有 blob
- 源文件在迁移过程中发生变化

`ImportDecision` 至少支持：

| 决策 | 行为 |
|---|---|
| `fail` | 遇到冲突立即停止，适合严格迁移 |
| `rename` | 生成唯一目标名，例如 `name (1).ext` |
| `skip` | 保留目标数据，记录源对象被跳过 |
| `reuse` | 已验证为同一对象时复用目标记录或 blob |
| `update` | 明确允许时更新已映射的目标对象 |

所有自动决策必须写入报告，不能静默改名或跳过。

### Verification Orchestrator

职责：

- 重新读取源 inventory
- 读取 object mapping
- 读取目标状态
- 执行一致性检查
- 将校验结果交给 Report Builder

### Report Builder

当前实现通过 `--report-path` 为 `check` 和 `migrate` 输出 `schema_version=1` 的 pretty JSON，同时保留终端摘要。报告内容：

- 源和目标对象统计
- 各对象类型迁移数量
- 按对象类型聚合的跳过数量
- 每个跳过对象的 source ID 和原因
- policy、policy group、user、folder、blob、file、share 的 source ID -> target ID
- Cloudreve tag metadata -> AD tag/entity 关联
- Cloudreve direct-link ID -> AD file ID/new URL
- 不兼容能力和人工处理项
- 提交后重新查询 AD 得到的数量校验
- `system.tags` 和 `cloudreve.direct_links` property 校验
- 错误和警告摘要

校验失败时 CLI 必须先写报告和终端摘要，再以非零状态退出。当前报告只覆盖成功提交后的校验；事务提交前发生的致命错误仍直接返回错误，后续可引入 migration run 持久化来保存失败中间态。

## 3.3 Layer 3：Source Adapter Layer

Source Adapter 读取源系统并产出统一领域对象。所有源系统专有逻辑必须留在 adapter 内。

建议 trait：

```rust
trait SourceAdapter {
    async fn capabilities(&self) -> Result<SourceCapabilities>;
    async fn inventory(&self) -> Result<SourceInventory>;
    async fn list_users(&self, cursor: Cursor) -> Result<Page<ExternalUser>>;
    async fn list_folders(&self, cursor: Cursor) -> Result<Page<ExternalFolder>>;
    async fn list_files(&self, cursor: Cursor) -> Result<Page<ExternalFile>>;
    async fn list_shares(&self, cursor: Cursor) -> Result<Page<ExternalShare>>;
    async fn get_blob(&self, blob: &ExternalBlobRef) -> Result<BlobReader>;
}
```

### Cloudreve Adapter

内部组件：

| 组件 | 职责 |
|---|---|
| Cloudreve DB Reader | 读取 users、groups、files、entities、file_entities、shares、metadata、direct_links 和 policies，并只统计 tasks 数量 |
| Cloudreve Storage Resolver | 解析 policy、driver、physical location 和对象是否存在 |
| Cloudreve Model Decoder | 解码 JSON、boolset、版本关系、文件类型和软删除状态 |
| Cloudreve -> Domain Converter | 转成 ExternalUser/Folder/File/Blob/Share，并规范化字段 |

Cloudreve 特有逻辑示例：

- `files.type=0/1` 的文件/目录拆分
- `file_children` 实际是 parent ID
- `primary_entity` 与 `file_entities` 的当前/历史版本关系
- `entities.type=0/1/2` 的版本、缩略图和 Live Photo 语义
- Cloudreve policy type 到通用 storage capability 的转换
- Cloudreve password、MFA、Passkey 和 share password 格式识别
- 从 `metadata.name=tag:*` 提取标签，在用户个人 scope 内去重，并生成 AD `system.tags` 关联
- 将 Cloudreve `/f/{hashid}/{name}` 语义转换为 AD `/d/v2...` 重新签发映射，而不是复制旧 token
- 只统计 Cloudreve task 数量并写入迁移报告，不产出领域对象或目标写入

### Future Adapters

Alist、FileBrowser、Nextcloud 等 adapter 只需实现 SourceAdapter 并产出相同领域模型，不应修改 AsterDrive Writer。

## 3.4 Layer 4：Domain Model Layer

领域模型必须与 Cloudreve 和 AD 的 ORM Entity 解耦。

### ExternalUser

建议字段：

```text
source_user_id
source_username
source_email
display_name
status
role_hint
storage_used
storage_quota
source_group_id
created_at
updated_at
metadata
```

### ExternalFolder

```text
source_folder_id
source_name
source_parent_id
source_owner_id
source_policy_id
created_at
updated_at
metadata
```

### ExternalFile

```text
source_file_id
source_name
source_folder_id
source_owner_id
source_current_blob
source_historical_blobs
source_size
mime_type
created_at
updated_at
metadata
```

### ExternalBlob

```text
source_blob_id
source_object_key
source_policy_id
source_size
source_hash (optional)
source_thumbnail
created_at
updated_at
```

### ExternalShare

```text
source_share_id
source_owner_id
source_target_type
source_target_id
source_password
view_count
download_count
remaining_downloads
expires_at
metadata
```

### WorkspaceScope

描述目标写入个人空间还是团队空间：

```text
Personal { owner_user_id }
Team { team_id, actor_user_id }
```

Cloudreve group 默认不自动转换成 WorkspaceScope::Team。只有管理员提供明确组织映射时才创建 AD team。

### MigrationPlan

保存：

- source/target 标识
- enabled stages
- storage policy decisions
- conflict policy
- include_deleted
- object-reference and external-transfer decisions
- concurrency/retry 参数
- verification level
- inventory estimate

### ImportDecision

保存 conflict resolver 对某个对象的最终决策，以及决策原因和是否需要人工确认。

## 3.5 Layer 5：Target Layer / AsterDrive Writer

Target Writer 是唯一允许理解 AD schema 和 AD 存储规则的层。

建议 trait：

```rust
trait TargetWriter {
    async fn write_user(&self, user: &ExternalUser) -> Result<TargetRef>;
    async fn write_folder(&self, folder: &ExternalFolder) -> Result<TargetRef>;
    async fn write_blob(&self, blob: &ExternalBlob) -> Result<TargetRef>;
    async fn write_file(&self, file: &ExternalFile) -> Result<TargetRef>;
    async fn write_share(&self, share: &ExternalShare) -> Result<TargetRef>;
    async fn finalize(&self, run: &MigrationRun) -> Result<()>;
}
```

### User Writer

- Create/update user
- 创建 user profile
- 映射 role/status
- 设置临时 Argon2 密码
- 设置 `must_change_password`
- 写入 quota 和 policy group

### Folder Writer

- Create/update folder
- 通过 object mapping 解析 parent
- 保证父目录先于子目录创建
- 处理同目录重名

### Blob Writer

- 根据 hash + policy 查找可复用 blob
- 流式上传对象
- 生成 AD storage path
- 校验 size 和 checksum
- 插入或复用 `file_blobs`

### File Writer

- Create/update `files`
- 写入 metadata/classification
- 指向当前 blob
- 将历史 blob 写入 `file_versions`
- 处理 rename/skip/fail 决策

### Share Writer（可选）

- 创建或更新分享
- 生成新的 AD token/link code
- 将 Cloudreve 明文分享密码转为 Argon2 hash
- 保留过期时间、访问和下载计数

### Quota Recalculator

- 按用户或 workspace 重新计算 `storage_used`
- 不只信任 Cloudreve 旧统计
- 明确去重 blob 是否按逻辑文件大小还是物理占用计入配额

### RefCount Recalculator

- 根据 `files.blob_id` 和 `file_versions.blob_id` 重算 ref_count
- 找出孤儿 blob
- 找出 ref_count 与实际引用不一致的 blob

### AsterDrive Storage Writer

- Stream/chunk 上传
- 计算 SHA-256
- 写入 AD 规范 storage path
- 支持本地和对象存储
- 完成后重新读取 metadata/size 验证

### AsterDrive DB Writer

- 事务边界
- 批量插入
- index-aware update
- 与 checkpoint 更新保持提交顺序一致

## 3.6 Layer 6：Persistence Layer

### Source System

- Cloudreve Database：用户、目录、文件、分享、设置、自定义字段
- Cloudreve Storage：文件 blob、缩略图、附件和其他物理对象

### Target System

- AsterDrive Database：用户、workspace、目录、文件、blob、分享、配额和 metadata
- AsterDrive Storage：规范化 blob、去重对象和备份/归档对象

### Checkpoint Tables

checkpoint 表保存在 AD 数据库中，以便迁移进程重启后恢复。当前实现先提供阶段级有限恢复。

#### `aster_external_migration_runs`

| 字段 | 当前类型 | 说明 |
|---|---|---|
| `id` | UUID/String PK | Run ID |
| `source_fingerprint` | String | 源数据库/实例指纹，避免连错源 |
| `target_fingerprint` | String | 目标 AD 实例指纹 |
| `plan_fingerprint` | String | local path、flags 和 secret/password 摘要组成的计划指纹 |
| `status` | String | running/failed/completed/validation_failed |
| `last_completed_stage` | String nullable | 最后一个与数据一起原子提交的阶段 |
| `context_json` | JSON/JSONB | 当前内存 ID mapping 和 username mapping 的持久化快照 |
| `report_json` | JSON/JSONB | 已完成阶段的完整结构化报告 |
| `baseline_json` | JSON/JSONB | 新 run 开始前的目标表计数，用于恢复后的最终增量校验 |
| `last_error` | Text nullable | 最近一次阶段失败摘要，不包含 migration secret |
| `created_at` | DateTime | 创建时间 |
| `updated_at` | DateTime | 最后更新时间 |

当前阶段顺序为 policies、policy_groups、users、folders、blobs、files、metadata、shares、direct_links。除 blobs/files 外，每个阶段使用独立目标事务；目标数据和 `last_completed_stage/context_json/report_json` 在同一事务中提交。blobs 按 Cloudreve `entities.id`、files 按 Cloudreve `files.id` keyset 分页；每页独立事务提交目标记录、逐行 mapping、cursor 和报告进度。

当前 source fingerprint 使用数据库 URL 摘要和各源表记录数量，只能防止明显连错源或数量变化，不能检测“数量不变但内容变化”。生产迁移仍应冻结 Cloudreve 写入。

对于 `reuse_source_storage` 的 local policy，当前实现支持用全局 `local_base_path` 或按 Cloudreve source policy ID 覆盖的 local root 配置 AD `base_path`。启用 `verify_local_storage` 后，预检会检查 local root；正式 blob batch 会检查每个复用对象解析后的路径、常规文件状态、打开权限和大小。`dry-run` 会在写入前分页扫描所有兼容 local blob。S3/COS 等远程复用策略仍需要后续的 provider-level `HeadObject`/range-read 验证。

#### `aster_external_migration_stage_cursors`

已用于 blobs 和 files；其他 stage 尚未接入。

| 字段 | 当前类型 | 说明 |
|---|---|---|
| `run_id` | String PK | 所属 run |
| `stage` | String PK | 当前为 `blobs` 或 `files` |
| `cursor_value` | BigInt | 最后提交的 Cloudreve entity ID 或 file ID |
| `processed_count` | BigInt | 已扫描并提交的 blob entity 或 file 数量，包含明确跳过项 |
| `updated_at` | DateTime | checkpoint 时间 |

建议 `(run_id, stage)` 为联合主键或唯一键。

#### `aster_external_migration_object_map`

已用于 blob 和 file 映射；其他对象类型仍使用 run `context_json` 中的 HashMap。

| 字段 | 当前类型 | 说明 |
|---|---|---|
| `run_id` | String PK | 所属 run |
| `object_type` | String PK | 当前为 `blob` 或 `file` |
| `source_id` | BigInt PK | Cloudreve entity ID 或 file ID |
| `target_id` | BigInt | AD file_blob 或 file ID |
| `created_at` | DateTime | 映射创建时间 |
| `state` | String | created/reused/skipped/renamed/failed |
| `created_at` | DateTime | 创建时间 |
| `updated_at` | DateTime | 更新时间 |

建议唯一键：

```text
(run_id, source_type, source_id)
```

## 4. 文件与对象关系

## 4.1 源模型到领域模型

| Cloudreve 对象 | 领域对象 | 关键字段 |
|---|---|---|
| User | ExternalUser | source_user_id、username、email、used_storage |
| Folder/Directory | ExternalFolder | source_folder_id、name、parent_id、owner_id |
| File | ExternalFile | source_file_id、name、folder_id、object/entity、size、MIME |
| Policy | Storage capability/plan decision | policy ID、driver、rules |
| Physical Entity | ExternalBlob | object key、size、policy、storage path |
| Share | ExternalShare | source_share_id、target、password、权限、expires_at |

## 4.2 领域模型到 AD

| 领域对象 | AD 表 | 关键关系 |
|---|---|---|
| ExternalUser | `users` + `user_profiles` | 一个 source user 对应一个 target user |
| ExternalFolder | `folders` | parent_id 通过 mapping 解析 |
| ExternalBlob | `file_blobs` | hash+policy 去重或创建 |
| ExternalFile | `files` | `blob_id` 指向当前 blob |
| Historical ExternalBlob | `file_versions` | `file_id` + `blob_id` + version |
| ExternalShare | `shares` | 指向 file_id 或 folder_id |

## 4.3 关键不变量

| 不变量 | 说明 |
|---|---|
| 文件与 blob 分离 | `files` 只表示逻辑文件，`file_blobs` 表示物理对象 |
| blob 可复用 | 多个 files 可以指向同一 blob |
| 同策略内容去重 | 只有内容哈希可信且 policy 相同才复用 blob |
| parent 先创建 | folder parent mapping 必须在 child 写入前存在 |
| file 依赖 blob | 当前 blob 写入成功后才能插入 files |
| mapping 持久化 | source->target 映射是 resume、验证和幂等的核心 |
| ref_count 可重算 | ref_count 必须能从 files + file_versions 反向计算 |
| storage_used 可重算 | 用户/workspace 使用量必须能从目标文件关系反向校验 |

## 5. 对象存储边界

迁移核心负责数据库转换、对象引用和映射，不负责搬运 blob 字节。兼容存储直接复用 Cloudreve 的相对 object key；目标根目录、bucket 或 endpoint 不同时，由 rsync、rclone、文件系统快照或 provider-side copy 在迁移前完成数据搬运。

外部搬运必须满足以下契约：

1. 保持与 Cloudreve `entity.source` 一致的相对 `storage_path`；
2. 在切换前校验对象数量、总字节和抽样内容哈希；
3. 目标 AsterDrive 进程能使用最终 policy 实际读取对象；
4. 搬运 checkpoint、重试、孤儿清理和带宽控制由所选基础设施工具负责。

当前 blob 写入保留 Cloudreve object key，并使用 opaque blob key，避免把非内容摘要伪装成 SHA-256。

## 6. 冲突处理

### 文件和目录重名

| 策略 | 行为 |
|---|---|
| fail | 停止运行，要求管理员清理目标或选择策略 |
| rename | 生成 `name (1).ext`、`name (2).ext` 等唯一名称 |
| skip | 不修改目标，mapping 标记 skipped |

### 用户冲突

需要分别处理：

- username 相同但 email 不同
- email 相同但 username 不同
- 已存在 mapping，但目标用户被修改
- 多个源用户规范化后得到同一 username

自动 rename username 时必须保存原 nick 到 `user_profiles.display_name` 和 mapping metadata。

### Blob 冲突

只有以下条件同时成立才可以自动复用：

```text
可信内容 SHA-256 相同
目标 policy 相同
目标 size 相同
目标对象可读且校验通过
```

Cloudreve entity ID、object key、路径或 `policy+path+size` 摘要都不能当作真实内容哈希。

## 7. 断点续传与幂等

每个 batch 的安全顺序：

```text
1. 读取 source page
2. 查询 object mapping
3. 转换并写入 AD
4. 提交目标事务
5. upsert object mapping
6. 更新 stage cursor
```

如果目标写入、mapping 和 cursor 使用同一个 AD 数据库，建议放入同一事务。对象存储上传无法纳入数据库事务，因此需要：

- 上传使用确定性临时 key 或 upload ID
- DB 提交失败时记录待清理对象
- resume 时检测目标对象是否已存在并校验
- finalize 或后台任务清理孤儿对象

幂等判断不能只依赖“目标表已有同名记录”，必须依赖 object mapping 和对象摘要。

## 8. 验证设计

### 数量校验

- 用户、策略组、目录、文件、分享数量
- blob、历史版本、metadata 数量
- skipped/renamed/reused 数量与计划一致

### 关系校验

- 每个 folder parent 存在
- 每个 file folder/owner/blob 存在
- 每个 file_version file/blob 存在
- 每个 share 的 file/folder 和 owner 存在
- 每个 object mapping 的目标对象存在

### 存储校验

- 对象存在
- metadata size 与 DB size 一致
- 随机抽样完整读取
- 支持 range read 的策略执行范围读取
- 外部搬运场景抽样校验 SHA-256

### 可重算校验

- `file_blobs.ref_count` 与实际引用数一致
- `users.storage_used` 与 AD 配额算法计算结果一致
- 没有未映射的目标对象或孤儿 blob

### 迁移期间源变化检测

迁移前后重新读取 source inventory 或关键对象摘要。如果源仍在写入：

- 标记 run 为 verification failed 或 source_changed
- 不宣称迁移完成
- 要求冻结 Cloudreve 写入后执行最终同步

## 9. 当前实现与目标架构差距

| 能力 | 当前状态 | 目标状态 |
|---|---|---|
| Cloudreve/AD schema 预检 | 已实现 | 扩展版本、权限、capability 和存储连通性检查 |
| 用户、策略组、目录、文件、blob、版本、分享、metadata | policy/group/user/folder 已通过独立 adapter/domain/writer crates；其余已实现基础迁移 | 继续迁移其余对象转换并分页 stage |
| Cloudreve v4 标签 | 已从 `tag:*` metadata 生成 AD `tags` 和 `system.tags` 文件/目录关联 | 增加跨批次持久化去重和冲突报告 |
| Direct links | 提供 AD `direct_link_secret` 时生成 v2 URL，并以 `cloudreve.direct_links` property 保存源 ID 映射 | 增加独立 JSON/CSV 报告和可选旧 URL 重定向层 |
| Cloudreve 任务 | 仅统计数量并跳过，不写入 `background_tasks` | 保持为源 inventory 信息，不进入领域模型或 Writer |
| 目标事务 | blobs/files 已按 page 提交；其他 stage 独立事务 | folders、metadata、shares 等 stage 继续分页，增加对象上传事务补偿 |
| ID mapping | blob/file 使用独立 object mapping 表；其他映射仍在 `context_json` | 将 folders、shares 等映射也迁入独立表 |
| 断点续传 | blobs/files 已支持 source ID cursor；其他 stage 为阶段级恢复 | folders 等 cursor、对象存储上传恢复和并行 worker checkpoint |
| 幂等重复运行 | 同一 run 会跳过已完成 stage；失败 stage 因事务回滚可安全重跑 | 不依赖 run checkpoint 的对象级 upsert/reuse 和完整重复运行 |
| 冲突策略 | 仅 username 自动后缀，其他冲突依赖 DB 报错 | fail/rename/skip/reuse/update 可配置 |
| 对象字节搬运 | 迁移器不搬运；复用兼容引用，跨根目录/bucket 由外部工具完成 | 补充外部搬运清单、校验记录与运行手册 |
| ref_count 重算 | 最终事务按 `files + file_versions` 统一重算并回写，再复核无漂移 | 大型库的数据库端聚合优化 |
| storage_used 重算 | 最终事务按 AD 当前文件 + 历史版本、个人/团队 scope 统一重算并回写，再复核无漂移 | 大型库的数据库端聚合优化 |
| 验证编排 | 提交后重查核心表数量、标签绑定、直链 property、folder/blob/file/version/share 关系和目录环；`--verify-local-storage` 打开并读取本地 AD 对象；有 SQLite 端到端测试 | AD HTTP 下载/分享/直链验证与远端对象存储可读性验证 |
| 报告 | 已支持终端摘要和 `--report-path` JSON，包含分类跳过原因、核心 ID 映射、标签/直链记录和校验结果 | 增加失败中间态、stage 进度、吞吐量、字节数和独立 CSV 导出 |
| TUI | 未接入新迁移核心 | 可选进度视图 |
| 多源系统插件 | 未实现 | SourceAdapter 插件化 |

## 10. 推荐实施阶段

### Phase 1：稳定当前一次性迁移

- 增加真实 Cloudreve 样本库测试
- 覆盖 SQLite、MySQL、PostgreSQL
- 增加 orphan、cycle、重名和不兼容策略预检
- 增加迁移后 ref_count/storage_used 验证

### Phase 2：领域模型和 Adapter/Writer 重构

- 已建立 `aster_drive_migration_core`、`cloudreve_adapter`、`aster_drive_writer` 独立 crates
- 继续引入 File/Blob/Share 等 source-neutral domain values
- 将剩余 Cloudreve ORM 转换移动到 Cloudreve Adapter
- 将剩余 AD ActiveModel 写入移动到 AsterDrive Writer

### Phase 3：持久化运行状态

- 新增 migration runs、stage cursors 和 object map 表
- 分页 stage runner
- resume 和幂等 upsert
- `migrate list/resume/report`

### Phase 4：对象存储交接

- 输出需要外部搬运的 policy 和相对路径清单
- 记录 rsync/rclone/快照/provider copy 的完成证据
- 在 AsterDrive 运行环境执行读取抽检

### Phase 5：验证和运维体验

- Verification Orchestrator
- Human/JSON report
- TUI progress
- 吞吐量、ETA、失败重试和人工冲突处理

## 11. 最终完成标准

一次迁移只有同时满足以下条件，才能标记为 completed：

1. 所有必需 stages 完成并持久化 cursor。
2. 所有迁移对象都有 created/reused/skipped/renamed 等明确 mapping 状态。
3. 目标关系完整，没有孤儿 owner、parent、blob、version 或 share。
4. 外部对象搬运完成，且目标运行环境的大小与抽样 SHA-256 校验通过。
5. ref_count 和 storage_used 重算通过。
6. 源在最终校验窗口内没有发生未处理变化。
7. 所有 skipped 和 warning 都出现在最终报告中。
8. 不存在包含明文密码、token、secret 或 access key 的日志和报告。
