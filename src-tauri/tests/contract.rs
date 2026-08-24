//! Договорные тесты слоя команд (T015).
//!
//! Отдельная тестовая цель: договор — это граница, и проверять её надо в отрыве
//! от внутренних тестов слоёв.

#[path = "contract/basics.rs"]
mod basics;

#[path = "contract/contract_sync.rs"]
mod contract_sync;
