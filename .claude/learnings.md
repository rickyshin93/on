# Project Learnings

## [Tooling] clippy `needless_raw_string_hashes` 会触发现有测试代码
- **日期**: 2026-05-13
- **错误假设**: 现有 `cargo clippy -- -D warnings` 在我的改动外不会有失败。
- **正确做法**: 新版 clippy（≥1.94）启用了 `needless_raw_string_hashes` lint，`src/config.rs` 中多处 `r#"..."#` 测试 YAML 字符串若内部不含 `"`，需要去掉 `#`。但若字符串里包含双引号（如 `echo "starting"`）则必须保留 `r#"..."#` 形式。
- **适用场景**: 升级 Rust 工具链或 CI 环境后 clippy 报新错时，先确认是否预存在；批量 sed 替换 raw string 时要看内容是否有 `"`。

## [TDD 纪律] 不要在 GREEN 阶段提前实现多余分支
- **日期**: 2026-05-13
- **错误假设**: 写 `from_flags` 时一次性把所有 only/short flag 合并逻辑都写完。
- **正确做法**: 严格按 RED→GREEN 一次只写最小代码通过当前测试；用后续 RED 驱动新分支。否则后写的测试虽然通过，但其实不能反映"测试先驱动出代码"的强度。
- **适用场景**: 使用 tdd skill 时，每个 cycle 先把现有实现回退到只够通过已有测试。
