# Cloudreve v4 到 AsterDrive 字段映射与差异

本文档基于以下代码快照整理：

- Cloudreve v4：[`cloudreve/cloudreve`](https://github.com/cloudreve/cloudreve) ，Ent 实际表定义位于 `ent/migrate/schema.go`
- AsterDrive：[`AsterCommunity/AsterDrive`](https://github.com/AsterCommunity/AsterDrive)，SeaORM Entity 位于 `src/entities/`
- 迁移工具：当前仓库 `crates/cloudreve_schema`，以及 AsterDrive Git 仓库提供的 `aster_drive_model` 和 `aster_drive_migration`

本文档的目标不是假设所有数据都应原样复制，而是明确：

1. 两边分别有什么表和字段。
2. 哪些字段可以直接映射。
3. 哪些字段需要转换、拆分或生成。
4. 哪些数据无法安全迁移，需要重新配置或由用户决策。

## 1. 总体结构差异

| 主题 | Cloudreve v4 | AsterDrive | 迁移影响 |
|---|---|---|---|
| 表数量 | 17 张业务表 | 45 张业务表 | AD 对认证、MFA、团队、任务、远端节点和存储凭据拆分更细 |
| 文件树 | `files` 同时保存文件和目录，通过 `type` 区分 | `files` 与 `folders` 分表 | 必须拆分 Cloudreve `files` |
| 物理对象 | `entities` 保存物理对象，`file_entities` 关联逻辑文件 | `file_blobs` 保存物理对象，`file_versions` 保存历史版本 | 需要重建当前 blob 和历史版本关系 |
| 用户组 | `groups` 同时承载权限、配额和默认存储策略 | `storage_policy_groups` 只负责存储路由；团队使用 `teams` | Cloudreve 用户组不应直接迁移为 AD 团队 |
| 用户密码 | Cloudreve SHA256/SHA1/旧 MD5 加盐格式 | AD Argon2 PHC 字符串 | 不能直接复用，必须设置临时密码或走重置流程 |
| 分享标识 | 分享 URL token 由 ID 和 Cloudreve HashID 密钥计算 | `shares.token` 为独立持久化 token | 必须生成新 token，旧分享 URL 不能保持不变 |
| 存储策略 | 多个 Cloudreve 专用驱动，配置集中在一张表 | 驱动类型更少，OAuth 凭据和应用配置拆表 | 仅部分驱动可直接复用 |
| 系统配置 | `settings(name, value)` 键值表 | `system_config` 包含类型、命名空间、可见性和敏感标记 | 不能盲目全量复制，需要配置键映射表 |
| 软删除 | 大部分 Cloudreve 表有 `deleted_at`，但当前 v4 `files` 没有 | AD 仅部分资源有 `deleted_at` | 需要决定是否包含已删除数据 |
| 认证数据 | OAuth Client/Grant、Passkey、TOTP secret 混合保存 | 外部认证、Passkey、MFA 流程分别建模 | 凭据结构和加密方式不兼容，不应直接复制 |

## 2. 迁移状态说明

| 标记 | 含义 |
|---|---|
| 直接 | 类型和语义基本一致，只需替换外键 ID |
| 转换 | 可以迁移，但需要格式、枚举、类型或语义转换 |
| 生成 | AD 必填，但 Cloudreve 没有，需要迁移工具生成 |
| 拆分 | 一个 Cloudreve 记录需要写入多张 AD 表 |
| 合并 | 多张 Cloudreve 表共同生成一张 AD 表或一组记录 |
| 跳过 | 运行时或临时数据，不建议迁移 |
| 不兼容 | 凭据、加密或协议语义不同，不能安全直接迁移 |
| 决策 | 没有唯一正确映射，需要迁移前由管理员确定 |

## 3. 用户组映射

Cloudreve `groups` 更接近“用户套餐/权限组”，而不是 AD 的协作团队。当前建议映射为：

- `groups` -> `storage_policy_groups`
- `groups.storage_policy_id` -> `storage_policy_group_items.policy_id`
- `users.group_users` -> `users.policy_group_id`
- `groups.permissions` 中的管理员位 -> `users.role = admin`

### `groups` -> `storage_policy_groups`

| Cloudreve 字段 | AD 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `storage_policy_groups.id` | 转换 | 保存旧 ID -> 新 ID 映射，不建议强行复用 ID |
| `name` | `name` | 直接 | 注意目标库可能已有同名组 |
| `created_at` | `created_at` | 直接 | 保留原时间 |
| `updated_at` | `updated_at` | 直接 | 保留原时间 |
| `deleted_at` | 无直接字段 | 转换 | 默认跳过已删除组；若包含则建议 `is_enabled = false` |
| `max_storage` | `users.storage_quota` | 转换 | AD 策略组自身没有配额，需要下放到组内每个用户 |
| `speed_limit` | 无直接字段 | 决策 | AD 当前用户/组模型没有等价限速列，可写入用户 `config` 或忽略 |
| `permissions` | `users.role`，部分可能进入 `config` | 转换 | 管理员位映射为 `admin`；其他 Cloudreve 权限没有一一对应字段 |
| `settings` | 无直接字段 | 决策 | 压缩、离线下载、回收站配置可选择写入 AD 配置扩展，不宜直接复制 |
| `storage_policy_id` | `storage_policy_group_items.policy_id` | 拆分 | 先迁移存储策略，再创建 group item |
| 无 | `description` | 生成 | 可写 `Migrated from Cloudreve group <id>` |
| 无 | `is_enabled` | 生成 | 活跃组为 `true`，已删除组为 `false` |
| 无 | `is_default` | 决策 | 选择 Cloudreve 默认用户组对应的 AD 默认策略组 |

### `groups.storage_policy_id` -> `storage_policy_group_items`

| AD 字段 | 来源 | 规则 |
|---|---|---|
| `id` | 生成 | AD 自增 ID |
| `group_id` | Cloudreve group ID 映射 | 指向新 `storage_policy_groups.id` |
| `policy_id` | `groups.storage_policy_id` 映射 | 指向新 `storage_policies.id` |
| `priority` | 生成 | 单策略组可设为 `0` |
| `min_file_size` | 生成 | `0` |
| `max_file_size` | 生成 | `0` 表示不额外限制 |
| `created_at` | `groups.created_at` | 保留原时间 |

## 4. 用户与资料映射

Cloudreve 一条 `users` 记录需要拆成 AD 的 `users` 和 `user_profiles`。

### `users` -> AD `users`

| Cloudreve 字段 | AD 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存用户 ID 映射，所有 owner/user 外键都要替换 |
| `email` | `email` | 直接 | AD 同样要求唯一；迁移前检查重复和空值 |
| `nick` | `username` | 转换 | AD `username` 唯一且最长 64；重名时追加 Cloudreve ID |
| `nick` | `user_profiles.display_name` | 拆分 | 原昵称同时作为展示名 |
| `password` | `password_hash` | 不兼容 | Cloudreve SHA/MD5 格式不能被 AD Argon2 验证 |
| `status=active` | `status=active` | 转换 | 直接映射 |
| `status=inactive/manual_banned/sys_banned` | `status=disabled` | 转换 | AD 当前只有 `active`/`disabled` |
| `storage` | `storage_used` | 直接 | 保留已用空间统计，迁移后应重新校验 blob 汇总 |
| `group_users -> groups.max_storage` | `storage_quota` | 转换 | 配额来自用户所属 Cloudreve 组 |
| `group_users` | `policy_group_id` | 转换 | 使用 group ID 映射 |
| `settings` | `config` | 转换 | 可保存原 JSON，但 AD 不会自动理解 Cloudreve 键 |
| `two_factor_secret` | 无安全直接映射 | 不兼容 | AD 需要加密后的 `mfa_factors.secret_ciphertext`，必须重新绑定 MFA |
| `avatar` | `user_profiles.avatar_source/avatar_key` | 转换 | 根据值判断 `none`、`gravatar` 或 `upload` |
| `created_at` | `created_at` | 直接 | 保留原时间 |
| `updated_at` | `updated_at` | 直接 | 保留原时间 |
| `deleted_at` | `status=disabled` 或跳过 | 决策 | 默认跳过软删除用户；包含时应禁用 |
| `groups.permissions` 管理员位 | `role` | 转换 | 管理员组用户 -> `admin`，其他 -> `user` |
| 无 | `session_version` | 生成 | 建议初始化为 `1` |
| 无 | `email_verified_at` | 生成/决策 | Cloudreve 没有等价字段；可对活跃用户设为 `created_at`，或要求重新验证 |
| 无 | `pending_email` | 生成 | `NULL` |
| 无 | `must_change_password` | 生成 | 设置为 `true` |

### AD `user_profiles` 字段

| AD 字段 | Cloudreve 来源 | 规则 |
|---|---|---|
| `user_id` | 新 AD 用户 ID | 与 `users.id` 一对一 |
| `display_name` | `users.nick` | 保留原昵称 |
| `wopi_user_info` | 无 | `NULL` |
| `avatar_source` | `users.avatar` | `none` / `gravatar` / `upload` |
| `avatar_key` | `users.avatar` | 非空时保存原值；之后可能需要单独搬迁头像文件 |
| `avatar_version` | 无 | `0` |
| `created_at` | `users.created_at` | 保留 |
| `updated_at` | `users.updated_at` | 保留 |

## 5. 存储策略映射

### 驱动类型

| Cloudreve `type` | AD `driver_type` | 状态 | 说明 |
|---|---|---|---|
| `local` | `local` | 条件迁移 | 相对 source 拒绝 `..` 后保留；绝对 source 必须位于配置 root 内并转换为相对 `storage_path` |
| `s3` | `s3` | 条件迁移 | endpoint、bucket、key、path-style 和显式 region 可复用 |
| `oss` | 无 | 不兼容 | Cloudreve 使用阿里 OSS 原生 SDK、endpoint/CNAME 语义和签名；等待 AD #450 |
| `ks3` | `s3` | 条件迁移 | 接口接近 S3，但必须保留 region 并完成目标运行时验证 |
| `obs` | 无 | 不兼容 | Cloudreve 明确使用华为 OBS `SignatureObs`；等待 AD #451 |
| `cos` | `tencent_cos` | 条件迁移 | 仅接受 `https://<bucket>.cos.<region>.myqcloud.com` 且 endpoint bucket 与 `bucket_name` 一致 |
| `qiniu` | 无 | 不兼容 | AD 当前没有七牛驱动，也不能保证 S3 兼容 |
| `upyun` | 无 | 不兼容 | AD 当前没有又拍云驱动 |
| `onedrive` | `onedrive` | 条件迁移 | 保留 Graph cloud、drive 定位、对象路径和传输拓扑；OAuth token 与应用 secret 不迁移，切换前必须在 AD 中重新授权 |
| `remote` | `remote` 但节点协议不同 | 不兼容 | Cloudreve Slave 与 AD follower 协议完全不同 |

启用了 Cloudreve `settings.encryption=true` 的策略不能只迁移数据库元数据。Cloudreve 对象内容仍是 Cloudreve 加密格式，AD 无法直接读取，必须先通过 Cloudreve 解密导出再上传到 AD。

### `storage_policies` 字段

| Cloudreve 字段 | AD 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存策略 ID 映射 |
| `name` | `name` | 直接 | 保留名称 |
| `type` | `driver_type` | 转换 | 按上表映射 |
| `server` | `endpoint` / `options.onedrive_cloud` | 转换 | 对象存储复制 endpoint；OneDrive 仅接受 Global/China 标准 Graph host，并转换为 cloud 枚举，目标 endpoint 留空 |
| `bucket_name` | `bucket` | 转换 | 对象存储 `NULL` 转为空字符串；Cloudreve OneDrive 中该字段是 Client ID，目标 policy bucket 留空 |
| `access_key` | `access_key` | 直接/敏感 | 对象存储复制；Cloudreve OneDrive 中该字段是 refresh token，不写入目标 policy 或 report |
| `secret_key` | `secret_key` | 直接/敏感 | 对象存储复制；Cloudreve OneDrive Client Secret 不写入目标 policy 或 report |
| `max_size` | `max_file_size` | 转换 | Cloudreve `NULL` -> AD `0`，两边都表示无限制时才成立 |
| `settings.file_type` | `allowed_types` | 转换 | 保存为 JSON 数组；还需处理 deny-list 语义 |
| `settings.chunk_size` | `chunk_size` | 转换 | 保留正数字节数；OneDrive 的 `0` 按 Cloudreve 运行时缺省值转换为 50 MiB，其他驱动保留 `0` 的单次上传语义 |
| `settings.s3_path_style` | `options.s3_path_style` | 转换 | 保留布尔值 |
| `settings.region` / `settings.s3_region` | `options.s3_region` | 转换 | S3/KS3 保留修剪后的显式 region；缺省时不写字段，由 AD 使用 `auto`；非法 region 会跳过策略并报告 |
| `settings.relay` | `options.object_storage_upload_strategy` | 转换 | `true` -> `relay_stream`；`false` 或缺省 -> `presigned` |
| `settings.internal_proxy` | `options.object_storage_download_strategy` | 转换 | `true` -> `relay_stream`；`false` 或缺省 -> `presigned`；`custom_proxy/proxy_server` 的自定义代理域名不在此字段中表达 |
| OneDrive `settings.relay` | `options.provider_resumable_upload_strategy` | 转换 | `true` -> `server_relay`；`false` 或缺省 -> `frontend_direct` |
| OneDrive `settings.internal_proxy` | `options.provider_download_strategy` | 转换 | `true` -> `server_relay`；`false` 或缺省 -> `frontend_direct` |
| OneDrive `settings.od_driver` | `options.onedrive_*` | 转换 | 支持空值/`me/drive`、`drives/{id}`、`sites/{id}/drive`、`sites/{id}/drives/{drive}`、`groups/{id}/drive`、`groups/{id}/drives/{drive}` |
| `is_private` | 无单独字段 | 合并 | AD 下载策略由 `options` 控制，不应机械复制 |
| `dir_name_rule` | 无直接字段 | 决策 | 不迁移生成规则；已有对象使用规格化后的 `entity.source` |
| `file_name_rule` | 无直接字段 | 决策 | 同上 |
| `settings` 其他键 | `options.cloudreve_source` 或忽略 | 决策 | 可存档原 JSON，但 AD 不会自动使用未知键 |
| `node_id` | `remote_node_id` | 不兼容 | Cloudreve node 不能直接变成 AD managed follower |
| `created_at` | `created_at` | 直接 | 保留 |
| `updated_at` | `updated_at` | 直接 | 保留 |
| `deleted_at` | 无 | 转换 | 默认跳过已删除策略 |
| 无 | `base_path` | 生成/配置 | 本地策略必须由管理员提供 Cloudreve 数据根目录 |
| 无 | `remote_storage_target_key` | 生成 | 普通策略为 `NULL` |
| 无 | `is_default` | 决策 | 选择一个迁移策略作为默认策略 |

## 6. 文件树拆分

Cloudreve `files.type`：

| 值 | 含义 | AD 目标 |
|---|---|---|
| `0` | 文件 | `files` |
| `1` | 目录 | `folders` |

### Cloudreve 目录记录 -> AD `folders`

| Cloudreve `files` 字段 | AD `folders` 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存 folder ID 映射 |
| `type=1` | 决定目标表 | 拆分 | 不写入 AD `files` |
| `name` | `name` | 直接 | 保留 |
| `file_children` | `parent_id` | 转换 | Cloudreve 字段名实际表示父目录 ID |
| `owner_id` | `owner_user_id` | 转换 | 使用用户 ID 映射 |
| `owner_id` | `created_by_user_id` | 生成/转换 | Cloudreve 没有独立创建者时使用 owner |
| `owner_id -> users.nick` | `created_by_username` | 生成 | 保存迁移后的 username |
| `storage_policy_files` | `policy_id` | 转换 | 使用策略 ID 映射 |
| `created_at` | `created_at` | 直接 | 保留 |
| `updated_at` | `updated_at` | 直接 | 保留 |
| `props` | 无直接字段 | 决策 | 可按需要拆到 `entity_properties` |
| `size` | 无 | 忽略 | 目录大小不写入 AD folder |
| `primary_entity` | 无 | 忽略 | 目录不应有主实体 |
| `is_symbolic` | 无 | 不兼容 | Cloudreve 占位/符号对象需要单独处理 |
| 无 | `team_id` | 生成 | 个人空间迁移为 `NULL`；若要转团队需另行规划 |
| 无 | `deleted_at` | 生成/决策 | 当前 Cloudreve v4 `files` 无此字段，通常为 `NULL` |
| 无 | `is_locked` | 生成 | `false` |

### Cloudreve 文件记录 -> AD `files`

| Cloudreve `files` 字段 | AD `files` 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存 file ID 映射 |
| `type=0` | 决定目标表 | 拆分 | 不写入 AD `folders` |
| `name` | `name` | 直接 | 保留 |
| `file_children` | `folder_id` | 转换 | 使用 folder ID 映射 |
| `owner_id` | `owner_user_id` | 转换 | 使用用户 ID 映射 |
| `owner_id` | `created_by_user_id` | 生成/转换 | 没有独立创建者时使用 owner |
| `owner_id -> username` | `created_by_username` | 生成 | 保存迁移后的 AD username |
| `primary_entity` | `blob_id` | 转换 | 先将主 entity 迁移为 `file_blobs` |
| `size` | 无直接写入 | 校验 | 仅校验源值非负；目标 `files.size` 使用当前 `primary_entity` 对应的 `entities.size` |
| `created_at` | `created_at` | 直接 | 保留 |
| `updated_at` | `updated_at` | 直接 | 保留 |
| `props` | 无直接字段 | 决策 | 可存入 `entity_properties` |
| `storage_policy_files` | 无直接字段 | 间接 | 文件实际策略由 `blob_id -> file_blobs.policy_id` 决定 |
| `is_symbolic=true` | 无直接模型 | 不兼容 | 默认跳过；若是占位文件需先在 Cloudreve 物化 |
| 无 | `mime_type` | 生成 | 根据文件名扩展名推断，建议迁移后重新扫描 |
| 无 | `extension` | 生成 | 小写末级扩展名 |
| 无 | `compound_extension` | 生成 | 如 `tar.gz` |
| 无 | `file_category` | 生成 | image/video/audio/document 等分类 |
| 无 | `team_id` | 生成 | 个人空间为 `NULL` |
| 无 | `deleted_at` | 生成 | 当前 Cloudreve v4 文件表没有该字段，通常为 `NULL` |
| 无 | `is_locked` | 生成 | `false`；Cloudreve 运行时锁不迁移 |

## 7. 物理对象、当前版本与历史版本

Cloudreve 使用：

- `entities`：物理对象记录
- `file_entities`：逻辑文件与物理对象多对多关系
- `files.primary_entity`：当前使用的物理对象
- `entities.type=0`：文件版本
- `entities.type=1`：缩略图
- `entities.type=2`：Live Photo 关联对象

AD 使用：

- `file_blobs`：物理对象
- `files.blob_id`：当前 blob
- `file_versions`：历史 blob
- `file_blobs.thumbnail_path`：缩略图路径

### `entities` -> `file_blobs`

| Cloudreve 字段 | AD 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存 entity ID -> blob ID 映射 |
| `type=0` | 创建 `file_blobs` | 转换 | 普通版本实体 |
| `type=1` | 不迁移 | 重建 | Cloudreve 缩略图没有 AD 的 processor/version 缓存契约；三个缩略图缓存字段均写 `NULL`，由 AD 重建 |
| `type=2` | 无直接字段 | 决策 | 可作为额外 blob + property，当前 AD 无 Live Photo 专用关系 |
| `source` | `storage_path` | 直接/关键 | 复用旧对象时必须原样保留 |
| `size` | `size` | 直接 | 保留 |
| `storage_policy_entities` | `policy_id` | 转换 | 使用策略 ID 映射 |
| `reference_count` | `ref_count` | 校验后转换 | 建议根据 `file_entities` 重新计算，不完全信任旧计数 |
| `created_at` | `created_at` | 直接 | 保留 |
| `updated_at` | `updated_at` | 直接 | 保留 |
| `deleted_at` | 无直接字段 | 决策 | 默认跳过已删除实体 |
| `upload_session_id` | 无 | 跳过 | 历史上传会话不应迁移 |
| `recycle_options` | 无直接字段 | 跳过/决策 | `encrypt_metadata` 非空表示对象由 Cloudreve 加密：跳过该 blob；若它是文件当前版本则同时跳过文件，并在迁移报告记录类型、源 ID 和固定原因，不输出 key/IV。其他回收属性不影响迁移 |
| `created_by` | 无 blob 创建者字段 | 间接 | 文件记录仍保存 owner/creator |
| 无 | `hash` | 生成 | Cloudreve 没有可靠内容 SHA256；应生成非 SHA256 形态的 opaque key，例如 `cloudreve-<entity-id>` |
| 无 | `thumbnail_processor` | 生成 | `NULL`，等待 AD 重新处理 |
| 无 | `thumbnail_version` | 生成 | `NULL` |

不能把 `SHA256(policy_id + source + size)` 伪装成内容哈希。AD 会将 64 位十六进制值视为真实内容 SHA256，在校验或存储迁移时会产生错误语义。

### `file_entities` 和 `primary_entity` -> `file_versions`

| Cloudreve 来源 | AD 字段 | 规则 |
|---|---|---|
| `file_entities.file_id` | `file_versions.file_id` | 使用逻辑文件 ID 映射 |
| `file_entities.entity_id` | `file_versions.blob_id` | 使用 entity -> blob 映射 |
| `files.primary_entity` | `files.blob_id` | 当前版本，不写入历史版本表 |
| 非 primary 的 `entity.type=0` | 一条 `file_versions` | 按创建时间升序编号 |
| `entities.size` | `file_versions.size` | 保留 |
| `entities.created_at` | `file_versions.created_at` | 保留 |
| 无明确版本号 | `file_versions.version` | 按历史 entity 创建时间生成 `1..N` |

普通文件缺少 `primary_entity`，或该 entity 因软删除/策略不兼容而未迁移时，整条文件记录跳过并报告；不会按创建时间把某个历史 entity 猜成当前版本。

## 8. 元数据映射

### `metadata` -> `entity_properties`

| Cloudreve 字段 | AD 字段 | 状态 | 规则 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 无需保留原 ID |
| `file_id` | `entity_id` | 转换 | 根据 Cloudreve `files.type` 使用 AD file 或 folder ID 映射 |
| `files.type` | `entity_type` | 转换 | `type=0 -> file`，`type=1 -> folder` |
| `is_public=true` | `namespace` | 转换 | `cloudreve.public` |
| `is_public=false` | `namespace` | 转换 | `cloudreve.private` |
| `name` | `name` | 直接 | 保留 |
| `value` | `value` | 直接 | 转为 `Some(value)` |
| `created_at` | 无 | 差异 | AD `entity_properties` 当前没有时间字段 |
| `updated_at` | 无 | 差异 | 无法保留 |
| `deleted_at` | 无 | 转换 | 默认跳过已删除元数据 |

### Cloudreve v4 标签 -> AD `tags` + `entity_properties`

Cloudreve v4 确实有标签功能，但没有独立 `tags` 表。标签保存在 `metadata.name = tag:<标签名>` 中，`metadata.value` 保存可选颜色。AD 使用 `tags` 保存标签定义，并使用 `entity_properties(namespace=system.tags, name=<AD tag_id>)` 关联文件或目录。

| Cloudreve 来源 | AD 目标 | 迁移规则 |
|---|---|---|
| `metadata.name = tag:<name>` | `tags.name` | 去掉 `tag:`；去除首尾空白；最长保留 64 个字符 |
| 标签名小写值 | `tags.normalized_name` | 按 AD 当前规则 `trim + lowercase`，并按用户个人空间去重 |
| `metadata.value` | `tags.color` | `#RRGGBB` 直接规范化；`#RGB` 展开；空值或非法值使用 `#3b82f6` |
| `files.owner_id` | `tags.owner_user_id` | 使用用户 ID 映射，`scope_type=personal`，`team_id=NULL` |
| `metadata.created_at/updated_at` | `tags.created_at/updated_at` | 创建标签定义时保留首条标签元数据时间 |
| 目标 file/folder ID | `entity_properties.entity_id` | 根据源 `files.type` 关联文件或目录 |
| 无 | `entity_properties.namespace` | 固定为 AD 原生标签命名空间 `system.tags` |
| 新 AD tag ID | `entity_properties.name` | 十进制字符串；`value=NULL` |

同一用户下大小写不同但规范化名称相同的 Cloudreve 标签会合并为一个 AD 标签定义。Cloudreve v3 曾有独立标签模型，但 Cloudreve 官方 v3 -> v4 迁移阶段没有对应标签迁移步骤；若数据仍停留在 v3 表中，需要直接读取 v3 数据库，不能从 v4 `metadata` 反推出已丢失记录。

Cloudreve `files.props` 是另一组 JSON 属性，不在 `metadata` 表中。是否拆成多个 `entity_properties` 需要先定义允许迁移的键，避免将 Cloudreve 内部状态直接暴露给 AD。

## 9. 分享映射

### `shares` -> AD `shares`

| Cloudreve 字段 | AD 字段 | 状态 | 规则或差异 |
|---|---|---|---|
| `id` | 新生成 `id` | 转换 | 保存旧 ID仅用于报告 |
| 无持久化 token | `token` | 生成 | Writer 使用随机 UUID simple token 生成新的 32 字符 capability；旧 URL 会失效，不从可预测的源 ID 派生 |
| `user_shares` | `user_id` | 转换 | 使用用户 ID 映射 |
| `file_shares` 指向文件 | `file_id` | 转换 | 根据源 `files.type=0` 判断 |
| `file_shares` 指向目录 | `folder_id` | 转换 | 根据源 `files.type=1` 判断 |
| `password` | `password` | 转换 | Cloudreve 保存明文分享密码；AD 必须保存 Argon2 hash |
| `expires` | `expires_at` | 直接 | 保留 |
| `downloads` | `download_count` | 直接 | 保留已下载次数 |
| `views` | `view_count` | 直接 | 保留浏览次数 |
| `remain_downloads` | `max_downloads` | 转换 | 有剩余次数时：`downloads + remain_downloads`；`NULL` 通常表示无限制，映射为 `0` |
| `props` | 无直接字段 | 决策 | Cloudreve 分享展示/权限属性需逐键分析 |
| `created_at` | `created_at` | 直接 | 保留 |
| `updated_at` | `updated_at` | 直接 | 保留 |
| `deleted_at` | 无 | 转换 | 默认跳过已删除分享 |
| 无 | `team_id` | 生成 | 个人分享为 `NULL` |

Cloudreve 允许同一 owner 对同一资源创建多条分享；AD 的表结构也能保存这些历史链接，但当前创建服务只允许同一资源存在一条未过期且未耗尽的活跃分享。迁移保留每一条未删除的 Cloudreve share，并在发现重复活跃 owner/target 组合时给出兼容性警告，不擅自合并或丢弃 capability。

### `direct_links` -> AD v2 直链 + 迁移映射属性

Cloudreve 直链并非“完全不能迁移”，但不能原样复制。Cloudreve 表只保存 `direct_link.id/name/downloads/speed/file_id`，旧 URL `/f/{hashid}/{name}` 中的 HashID 是运行时由 `[direct_link.id, SourceLinkID]` 和 Cloudreve salt 计算的。AD 没有 `direct_links` 表，而是使用 `auth.direct_link_secret` 对目标 file ID 和个人/团队 scope 做 HMAC-SHA256，生成 `/d/v2.{base62(file_id)}.{base64url_hmac}/{filename}`。

| Cloudreve 字段/语义 | AD 迁移结果 | 差异 |
|---|---|---|
| `file_id` | 映射后的 AD file ID | 文件必须已成功迁移 |
| `id` | `entity_properties.name` | 保存源 direct-link ID，便于查回映射 |
| `name` | 映射 JSON 的 `source_name` | AD URL 必须使用目标文件真实名称，不能继续使用任意旧 link name |
| `downloads` | 映射 JSON 的 `source_downloads` | AD 无持久化直链行，不能继续累计每条旧链接的计数 |
| `speed` | 映射 JSON 的 `source_speed_limit` | AD 没有每条直链限速字段，仅作历史存档 |
| Cloudreve HashID URL | 新 AD v2 URL | 通过 `--direct-link-secret` / `ASTER_DIRECT_LINK_SECRET` 重新签发 |
| 无 | `entity_properties.namespace` | `cloudreve.direct_links` |

多个指向同一文件的 Cloudreve 直链会得到同一个确定性 AD token，但每个源 `direct_link.id` 都保留一条映射属性。旧 `/f/...` URL、逐条撤销语义、逐条下载计数和限速无法自动延续；若必须保留旧 URL，需要额外实现兼容重定向路由和持久化映射表。

即使使用 `--include-deleted`，已软删除的 Cloudreve direct-link 也不会重新签发，以免把已经撤销的公开入口重新激活。

## 10. 不能原样迁移的 Cloudreve 表

| Cloudreve 表 | 最接近的 AD 表 | 建议 | 原因 |
|---|---|---|---|
| `dav_accounts` | `webdav_accounts` | 不直接迁移 | URI/root 语义不同；Cloudreve 密码不是 AD Argon2 hash；options 位集需重建 |
| `direct_links` | AD stateless direct link + `entity_properties` | 重新签发 | 不能复用旧 HashID URL；提供 AD direct-link secret 后生成新 v2 URL |
| `fs_events` | 无持久化对应表 | 跳过 | 事件订阅属于运行时状态，subscriber UUID 对 AD 无意义 |
| `nodes` | `managed_followers` / `remote_storage_targets` | 不直接迁移 | Cloudreve master/slave 协议与 AD primary/follower 协议不同，密钥也不能复用 |
| `oauth_clients` | `external_auth_providers` 并不等价 | 跳过或导出报告 | Cloudreve 表是“第三方应用访问 Cloudreve”，AD external provider 是“用户通过外部 IdP 登录 AD” |
| `oauth_grants` | 无直接等价 | 跳过 | grant、scope 和 client 都是 Cloudreve 安全域数据 |
| `passkeys` | `passkeys` | 要求重新注册 | credential JSON、user_handle、签名计数和序列化格式不同，直接复制可能导致认证失败或安全风险 |
| `settings` | `system_config` | 只迁移白名单键 | AD 需要 value type、namespace、category、visibility、sensitive 等元数据 |
| `tasks` | 无目标写入 | 跳过并统计 | 运行时状态不具备跨系统迁移意义 |

## 11. AD 新增但 Cloudreve 没有的数据域

以下 AD 表没有可直接对应的 Cloudreve 数据。迁移时通常保持为空，由 AD 运行时重新生成或由管理员配置。

| AD 数据域 | AD 表 | 迁移策略 |
|---|---|---|
| 登录会话 | `auth_sessions` | 不迁移，所有用户重新登录 |
| 审计 | `audit_logs` | 可选生成一条“从 Cloudreve 迁移”审计记录，不伪造历史审计 |
| 后台任务 | `background_tasks`、`storage_migration_checkpoints` | 保持为空，由 AD 运行时按自身业务生成 |
| 邮件 | `mail_outbox` | 不迁移 |
| 邮箱验证 | `contact_verification_tokens`、`external_auth_email_verification_flows` | 不迁移临时 token |
| 外部登录 | `external_auth_providers`、`external_auth_identities`、`external_auth_login_flows` | 由管理员重新配置 IdP，用户重新绑定 |
| MFA | `mfa_factors`、`mfa_recovery_codes`、`mfa_login_flows`、`mfa_email_codes`、`mfa_totp_setup_flows` | 不复制 Cloudreve secret，要求重新绑定 |
| 团队空间 | `teams`、`team_members` | Cloudreve group 不等于 team；需要单独的组织映射方案 |
| 标签 | `tags` + `entity_properties(system.tags)` | 从 Cloudreve v4 `metadata.name=tag:*` 创建标签定义和文件/目录关联 |
| 锁 | `resource_locks` | 不迁移运行时锁 |
| WOPI | `wopi_sessions` | 不迁移会话 |
| 上传 | `upload_sessions`、`upload_session_parts` | 不迁移未完成上传 |
| 用户邀请 | `user_invitations` | 不迁移 |
| 远端节点 | `managed_followers`、`follower_enrollment_sessions`、`master_bindings`、`remote_storage_targets` | 重新注册 AD follower |
| 存储 OAuth | `storage_policy_credentials`、`storage_policy_authorization_flows`、`storage_connector_application_configs` | OneDrive policy 和对象路径会迁移；Cloudreve token/secret 不进入 AD 加密凭据表，必须在 AD 中配置应用并重新授权 |
| 媒体解析 | `blob_media_metadata` | 可在迁移后由 AD 重新扫描生成 |

## 12. 完整 Cloudreve 表与字段清单

| 表 | 字段 |
|---|---|
| `dav_accounts` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `uri`, `password`, `options`, `props`, `owner_id` |
| `direct_links` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `downloads`, `speed`, `file_id` |
| `entities` | `id`, `created_at`, `updated_at`, `deleted_at`, `type`, `source`, `size`, `reference_count`, `upload_session_id`, `recycle_options`, `storage_policy_entities`, `created_by` |
| `file_entities` | `file_id`, `entity_id` |
| `files` | `id`, `created_at`, `updated_at`, `type`, `name`, `size`, `primary_entity`, `is_symbolic`, `props`, `file_children`, `storage_policy_files`, `owner_id` |
| `fs_events` | `id`, `created_at`, `updated_at`, `deleted_at`, `event`, `subscriber`, `user_fsevent` |
| `groups` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `max_storage`, `speed_limit`, `permissions`, `settings`, `storage_policy_id` |
| `metadata` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `value`, `is_public`, `file_id` |
| `nodes` | `id`, `created_at`, `updated_at`, `deleted_at`, `status`, `name`, `type`, `server`, `slave_key`, `capabilities`, `settings`, `weight` |
| `oauth_clients` | `id`, `created_at`, `updated_at`, `deleted_at`, `guid`, `secret`, `name`, `homepage_url`, `redirect_uris`, `scopes`, `props`, `is_enabled` |
| `oauth_grants` | `id`, `created_at`, `updated_at`, `deleted_at`, `scopes`, `last_used_at`, `client_id`, `user_id` |
| `passkeys` | `id`, `created_at`, `updated_at`, `deleted_at`, `credential_id`, `name`, `credential`, `used_at`, `user_id` |
| `settings` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `value` |
| `shares` | `id`, `created_at`, `updated_at`, `deleted_at`, `password`, `views`, `downloads`, `expires`, `remain_downloads`, `file_shares`, `user_shares`, `props` |
| `storage_policies` | `id`, `created_at`, `updated_at`, `deleted_at`, `name`, `type`, `server`, `bucket_name`, `is_private`, `access_key`, `secret_key`, `max_size`, `dir_name_rule`, `file_name_rule`, `settings`, `node_id` |
| `tasks` | `id`, `created_at`, `updated_at`, `deleted_at`, `type`, `status`, `public_state`, `private_state`, `correlation_id`, `user_tasks` |
| `users` | `id`, `created_at`, `updated_at`, `deleted_at`, `email`, `nick`, `password`, `status`, `storage`, `two_factor_secret`, `avatar`, `settings`, `group_users` |

## 13. 完整 AsterDrive 表与字段清单

| 表 | 字段 |
|---|---|
| `audit_logs` | `id`, `user_id`, `action`, `entity_type`, `entity_id`, `entity_name`, `details`, `ip_address`, `user_agent`, `created_at` |
| `auth_sessions` | `id`, `user_id`, `current_refresh_jti`, `previous_refresh_jti`, `refresh_expires_at`, `ip_address`, `user_agent`, `created_at`, `last_seen_at`, `revoked_at` |
| `background_tasks` | `id`, `kind`, `status`, `creator_user_id`, `team_id`, `share_id`, `display_name`, `payload_json`, `result_json`, `steps_json`, `progress_current`, `progress_total`, `status_text`, `attempt_count`, `max_attempts`, `next_run_at`, `processing_token`, `processing_started_at`, `last_heartbeat_at`, `lease_expires_at`, `started_at`, `finished_at`, `last_error`, `failure_can_retry`, `expires_at`, `created_at`, `updated_at`, `runtime_json` |
| `blob_media_metadata` | `id`, `blob_id`, `blob_hash`, `kind`, `status`, `metadata_json`, `error_message`, `parser`, `parser_version`, `created_at`, `updated_at` |
| `contact_verification_tokens` | `id`, `user_id`, `channel`, `purpose`, `target`, `token_hash`, `expires_at`, `consumed_at`, `created_at` |
| `entity_properties` | `id`, `entity_type`, `entity_id`, `namespace`, `name`, `value` |
| `external_auth_email_verification_flows` | `id`, `provider_id`, `identity_namespace`, `subject`, `target_email`, `display_name_snapshot`, `preferred_username_snapshot`, `return_path`, `flow_token_hash`, `verification_token_hash`, `email_requested_at`, `created_at`, `expires_at`, `consumed_at` |
| `external_auth_identities` | `id`, `user_id`, `provider_id`, `identity_namespace`, `subject`, `email_snapshot`, `display_name_snapshot`, `created_at`, `updated_at`, `last_login_at` |
| `external_auth_login_flows` | `id`, `provider_id`, `state_hash`, `nonce`, `pkce_verifier`, `redirect_uri`, `return_path`, `created_at`, `expires_at`, `consumed_at` |
| `external_auth_providers` | `id`, `key`, `display_name`, `icon_url`, `provider_kind`, `protocol`, `issuer_url`, `authorization_url`, `token_url`, `userinfo_url`, `client_id`, `client_secret`, `scopes`, `enabled`, `auto_provision_enabled`, `auto_link_verified_email_enabled`, `require_email_verified`, `subject_claim`, `username_claim`, `display_name_claim`, `email_claim`, `email_verified_claim`, `groups_claim`, `avatar_url_claim`, `allowed_domains`, `created_at`, `updated_at`, `options` |
| `file_blobs` | `id`, `hash`, `size`, `policy_id`, `storage_path`, `thumbnail_path`, `thumbnail_processor`, `thumbnail_version`, `ref_count`, `created_at`, `updated_at` |
| `file_versions` | `id`, `file_id`, `blob_id`, `version`, `size`, `created_at` |
| `files` | `id`, `name`, `folder_id`, `team_id`, `blob_id`, `size`, `owner_user_id`, `created_by_user_id`, `created_by_username`, `mime_type`, `created_at`, `updated_at`, `deleted_at`, `is_locked`, `extension`, `compound_extension`, `file_category` |
| `folders` | `id`, `name`, `parent_id`, `team_id`, `owner_user_id`, `created_by_user_id`, `created_by_username`, `policy_id`, `created_at`, `updated_at`, `deleted_at`, `is_locked` |
| `follower_enrollment_sessions` | `id`, `managed_follower_id`, `token_hash`, `ack_token_hash`, `expires_at`, `redeemed_at`, `acked_at`, `invalidated_at`, `created_at` |
| `mail_outbox` | `id`, `template_code`, `to_address`, `to_name`, `payload_json`, `status`, `attempt_count`, `next_attempt_at`, `processing_started_at`, `sent_at`, `last_error`, `created_at`, `updated_at` |
| `managed_followers` | `id`, `name`, `base_url`, `access_key`, `secret_key`, `is_enabled`, `last_capabilities`, `last_error`, `last_checked_at`, `created_at`, `updated_at`, `transport_mode`, `tunnel_last_error`, `tunnel_last_seen_at` |
| `master_bindings` | `id`, `name`, `master_url`, `access_key`, `secret_key`, `storage_namespace`, `is_enabled`, `created_at`, `updated_at` |
| `mfa_email_codes` | `id`, `flow_id`, `user_id`, `code_hash`, `expires_at`, `consumed_at`, `created_at` |
| `mfa_factors` | `id`, `user_id`, `method`, `name`, `secret_ciphertext`, `secret_version`, `enabled_at`, `last_used_at`, `created_at`, `updated_at` |
| `mfa_login_flows` | `id`, `flow_token_hash`, `user_id`, `user_session_version`, `first_factor`, `return_path`, `ip_address`, `user_agent`, `attempt_count`, `expires_at`, `consumed_at`, `created_at` |
| `mfa_recovery_codes` | `id`, `user_id`, `code_hash`, `used_at`, `created_at` |
| `mfa_totp_setup_flows` | `id`, `flow_token_hash`, `user_id`, `secret_ciphertext`, `secret_version`, `expires_at`, `consumed_at`, `created_at` |
| `passkeys` | `id`, `user_id`, `credential_id`, `user_handle`, `credential`, `name`, `transports`, `backup_eligible`, `backed_up`, `sign_count`, `created_at`, `updated_at`, `last_used_at` |
| `remote_storage_targets` | `id`, `master_binding_id`, `target_key`, `name`, `driver_type`, `endpoint`, `bucket`, `access_key`, `secret_key`, `base_path`, `is_default`, `desired_revision`, `applied_revision`, `last_error`, `created_at`, `updated_at` |
| `resource_locks` | `id`, `token`, `entity_type`, `entity_id`, `path`, `owner_id`, `owner_info`, `timeout_at`, `shared`, `deep`, `created_at` |
| `shares` | `id`, `token`, `user_id`, `team_id`, `file_id`, `folder_id`, `password`, `expires_at`, `max_downloads`, `download_count`, `view_count`, `created_at`, `updated_at` |
| `storage_connector_application_configs` | `id`, `policy_id`, `provider`, `tenant_id`, `scopes`, `client_id`, `client_secret_ciphertext`, `metadata`, `created_at`, `updated_at` |
| `storage_migration_checkpoints` | `task_id`, `source_policy_id`, `target_policy_id`, `plan_hash`, `stage`, `last_processed_blob_id`, `scanned_blobs`, `migrated_blobs`, `merged_blobs`, `skipped_blobs`, `failed_blobs`, `migrated_bytes`, `last_error`, `created_at`, `updated_at`, `renamed_opaque_blobs` |
| `storage_policies` | `id`, `name`, `driver_type`, `endpoint`, `bucket`, `access_key`, `secret_key`, `base_path`, `remote_node_id`, `max_file_size`, `allowed_types`, `options`, `is_default`, `chunk_size`, `created_at`, `updated_at`, `remote_storage_target_key` |
| `storage_policy_authorization_flows` | `id`, `provider`, `policy_id`, `created_by_user_id`, `state_hash`, `pkce_verifier`, `redirect_uri`, `scopes`, `context`, `status`, `created_at`, `expires_at`, `consumed_at` |
| `storage_policy_credentials` | `id`, `policy_id`, `provider`, `credential_kind`, `account_label`, `subject`, `tenant_id`, `scopes`, `access_token_ciphertext`, `refresh_token_ciphertext`, `metadata`, `status`, `status_reason`, `expires_at`, `authorized_at`, `last_refreshed_at`, `last_validated_at`, `created_at`, `updated_at` |
| `storage_policy_group_items` | `id`, `group_id`, `policy_id`, `priority`, `min_file_size`, `max_file_size`, `created_at` |
| `storage_policy_groups` | `id`, `name`, `description`, `is_enabled`, `is_default`, `created_at`, `updated_at` |
| `system_config` | `id`, `key`, `value`, `value_type`, `requires_restart`, `is_sensitive`, `source`, `namespace`, `category`, `description`, `updated_at`, `updated_by`, `visibility` |
| `tags` | `id`, `scope_type`, `owner_user_id`, `team_id`, `name`, `normalized_name`, `color`, `sort_order`, `created_at`, `updated_at` |
| `team_members` | `id`, `team_id`, `user_id`, `role`, `created_at`, `updated_at` |
| `teams` | `id`, `name`, `description`, `created_by`, `storage_used`, `storage_quota`, `policy_group_id`, `created_at`, `updated_at`, `archived_at` |
| `upload_session_parts` | `id`, `upload_id`, `part_number`, `etag`, `size`, `created_at`, `updated_at` |
| `upload_sessions` | `id`, `user_id`, `team_id`, `filename`, `total_size`, `chunk_size`, `total_chunks`, `received_count`, `folder_id`, `policy_id`, `status`, `object_temp_key`, `object_multipart_id`, `file_id`, `created_at`, `expires_at`, `updated_at`, `frontend_client_id` |
| `user_invitations` | `id`, `email`, `token_hash`, `status`, `invited_by`, `accepted_user_id`, `expires_at`, `created_at`, `updated_at`, `accepted_at`, `revoked_at` |
| `user_profiles` | `user_id`, `display_name`, `wopi_user_info`, `avatar_source`, `avatar_key`, `avatar_version`, `created_at`, `updated_at` |
| `users` | `id`, `username`, `email`, `password_hash`, `role`, `status`, `session_version`, `email_verified_at`, `pending_email`, `storage_used`, `storage_quota`, `policy_group_id`, `created_at`, `updated_at`, `config`, `must_change_password` |
| `webdav_accounts` | `id`, `user_id`, `username`, `password_hash`, `root_folder_id`, `is_active`, `created_at`, `updated_at`, `team_id` |
| `wopi_sessions` | `id`, `token_hash`, `actor_user_id`, `session_version`, `team_id`, `file_id`, `app_key`, `expires_at`, `created_at` |

## 14. 建议在执行迁移前确认的决策

| 决策项 | 可选方案 | 当前迁移实现默认值 |
|---|---|---|
| 已删除数据 | 跳过 / 迁移并标记删除或禁用 | 默认跳过；可显式包含部分软删除表 |
| 用户密码 | 统一临时密码 / 每人随机密码并导出 / 邮件重置 | 统一临时 Argon2 密码，`must_change_password=true` |
| 邮箱验证 | 视为已验证 / 全部重新验证 | 活跃用户按已验证处理 |
| Cloudreve group | AD 策略组 / AD team / 两者都建 | 映射为存储策略组，不自动创建 team |
| 本地存储根目录 | 复用 Cloudreve 工作目录 / 搬迁文件到 AD 新目录 | 通过 `--local-base-path` 指定并复用旧对象路径 |
| 多本地策略根目录 | 对所有策略使用同一根目录 / 按 source policy 分别指定 | `--local-policy-root <source_policy_id>=<path>` 覆盖指定策略，未指定策略回退 `--local-base-path` |
| 本地对象预检 | 仅检查根目录 / 逐对象路径、权限和大小检查 | `--verify-local-storage`；配合 `--dry-run` 可在写入前分页扫描全部可复用 local blob |
| 对象存储 | 复用原 bucket / 复制到新 bucket | 复用原 endpoint、bucket 和 object key |
| 历史版本 | 全部迁移 / 只迁移当前版本 | 当前实现迁移全部 `type=0` entity |
| 分享 URL | 生成新 token / 尝试兼容旧 HashID | 生成新 token，旧 URL 失效 |
| Direct link | 重新签发 AD v2 URL / 跳过 / 建旧 URL 兼容层 | 提供 `--direct-link-secret` 时重新签发并存档 source ID -> URL 映射；旧 `/f/...` 失效 |
| Cloudreve metadata | 全量存档 / 白名单迁移 | `metadata` 表全量迁移到带命名空间的 property；`files.props` 暂不迁移 |
| Cloudreve 标签 | 从 `tag:*` 生成 AD 原生标签 / 仅按普通 property 存档 | 当前生成 `tags` 定义和 `system.tags` 关联 |
| Cloudreve 任务 | 跳过 / 历史归档 / 尝试恢复执行 | 当前跳过，仅在报告中统计数量 |
| 不兼容策略 | 中止 / 跳过策略和依赖文件 / 解密导出后再迁移 | 默认中止；显式参数可跳过 |
| Passkey/MFA/WebDAV | 尝试转换 / 要求重新绑定 | 要求重新绑定 |
| 系统设置 | 全量复制 / 白名单映射 / 不迁移 | 当前不迁移，建议单独制作配置键白名单 |

## 15. 建议的验证顺序

| 阶段 | 检查内容 |
|---|---|
| 迁移前预检 | 表是否存在、目标核心表是否为空、源外键是否完整、是否存在不兼容存储策略 |
| 用户迁移后 | 用户数、管理员数、禁用用户数、邮箱唯一性、用户名冲突、配额合计 |
| 文件树迁移后 | 根目录数、目录数、文件数、孤儿 parent、孤儿 owner、同目录重名 |
| blob 迁移后 | entity 数、blob 数、当前 blob 关联、历史版本数、ref_count 重算结果 |
| 存储验证 | 随机抽样每种策略执行 metadata、读取、范围读取，校验文件大小 |
| 分享验证 | 文件分享、目录分享、密码分享、过期分享、下载次数限制 |
| 标签验证 | 每个 `tag:*` 元数据有对应 AD tag；`system.tags` 关联指向正确 file/folder 和 scope |
| 直链验证 | 使用与 AD 相同的 `auth.direct_link_secret` 请求新 `/d/v2...` URL；核对映射属性数量 |
| AD 启动后 | 重新生成媒体元数据、缩略图和搜索索引；检查 storage_used 汇总 |
| 切换前 | 冻结 Cloudreve 写入，再跑最终迁移或增量校验，备份目标库 |

## 16. JSON 报告字段

通过 `--report-path <path>` 可为 `check` 或 `migrate` 输出 `schema_version=1` 的 JSON 报告。

| 报告字段 | 内容 |
|---|---|
| `source_*` | Cloudreve 各类源对象数量 |
| `migrated_*` | 本次写入 AD 的各类对象数量 |
| `skipped_by_type` | 按 `file`、`blob`、`share`、`direct_link` 等类型聚合的跳过数量 |
| `skipped_objects` | 每条跳过记录的对象类型、Cloudreve source ID 和明确原因 |
| `mappings` | policy、policy group、user、folder、blob、file、share 的排序 source ID -> target ID |
| `tag_assignments` | Cloudreve metadata ID、源 file/folder ID、AD entity ID、AD tag ID 和标签名 |
| `direct_links` | Cloudreve direct-link/file ID、AD file ID、新 URL、原名称、下载次数和限速 |
| `validation` | 是否执行/通过，以及每项检查的 expected、actual 和失败信息 |
| `run_id` | checkpoint run ID；未指定时由迁移工具生成 UUID |
| `resumed` | 本次执行是否从已有 checkpoint 恢复 |
| `completed_stages` | 已原子提交完成的迁移阶段列表 |

当前提交后校验覆盖核心表增量数量、`system.tags` 关联是否存在、`cloudreve.direct_links` 属性中的 URL 是否与报告一致。报告不会保存数据库密码或存储密钥，但会包含新 direct-link URL，因此必须限制报告文件访问权限。

## 17. 有限断点续传语义

迁移工具会在 AD 数据库自动创建 `aster_external_migration_runs`。首次迁移建议显式指定 `--run-id`；失败后使用相同参数和 `--resume --run-id <id>`。

| 行为 | 当前实现 |
|---|---|
| checkpoint 粒度 | blobs/files 为 batch 级；其他 stage 为 stage 级 |
| 原子性 | blob/file page 的目标记录、object mapping、cursor 和 report 同事务；其他 stage 的目标记录、context、report 和 last completed stage 同事务 |
| 已完成 stage | resume 时跳过，不重复插入 |
| 失败 stage | 整个 stage 回滚，resume 时从该 stage 开头重新执行 |
| ID mapping | blob/file 映射逐行保存在 `aster_external_migration_object_map`；其他映射仍在 `context_json` |
| 初始目标计数 | 保存在 `baseline_json`，恢复完成后仍可执行正确的增量数量校验 |
| 源校验 | URL 摘要 + 源表数量指纹 |
| 计划校验 | local path、迁移 flags、临时密码摘要和 direct-link secret 摘要 |
| stage 内分页 cursor | blobs 使用 `entities.id`、files 使用 `files.id` keyset cursor；其他 stage 尚未实现 |
| 对象上传断点 | 尚未实现 |

由于源指纹不能检测数量不变的内容修改，断点恢复期间仍必须冻结 Cloudreve 写入。blobs 可从最后提交的 entity ID 继续，files 可从最后提交的 file ID 继续，批大小由 `--blob-batch-size` 和 `--file-batch-size` 控制，默认均为 500；folders、metadata、shares 和 direct links 等其他 stage 仍会从 stage 起点重跑。对象字节复制/上传仍未实现，因此也没有对象存储分片上传续传。
