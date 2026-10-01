//! Contract tests for the command layer (T015).
//!
//! A test target of its own: the contract is a boundary, and it has to be checked apart
//! from the layers' internal tests.

#[path = "contract/support.rs"]
mod support;

#[path = "contract/basics.rs"]
mod basics;

#[path = "contract/contract_sync.rs"]
mod contract_sync;

/// T337 — состав серверной части версии 1 сверяется с кодом, а не обещается.
/// T356–T358 — удаление всего, что приложение о человеке держит (FR-114).
#[path = "contract/forget.rs"]
mod forget;

/// T643 — «Забыть всё» не идёт при живых задачах, и задачи не стартуют, пока оно идёт.
#[path = "contract/forget_tasks.rs"]
mod forget_tasks;

#[path = "contract/server_inventory.rs"]
mod server_inventory;

#[path = "contract/library.rs"]
mod library;

/// T651 — a library refresh is joined, kept and said only when it changed something.
#[path = "contract/library_refresh.rs"]
mod library_refresh;

#[path = "contract/deploy.rs"]
mod deploy;

/// T647 — a deployment's stop goes to the server the run started on, with the run's own way in.
#[path = "contract/deploy_stop_target.rs"]
mod deploy_stop_target;

#[path = "contract/ladder.rs"]
mod ladder;

#[path = "contract/limits.rs"]
mod limits;

#[path = "contract/quality.rs"]
mod quality;

#[path = "contract/responsiveness.rs"]
mod responsiveness;

#[path = "contract/secrets_never_returned.rs"]
mod secrets_never_returned;

#[path = "contract/servers.rs"]
mod servers;

#[path = "contract/upload.rs"]
mod upload;

/// T653: dropping a raised upload tidies up after it.
#[path = "contract/upload_leftover.rs"]
mod upload_leftover;

#[path = "contract/convert.rs"]
mod convert;

#[path = "contract/viewers.rs"]
mod viewers;

#[path = "contract/supplied.rs"]
mod supplied;

/// T672 — the commands of a video in work: each file on its own, the plan, the states, a
/// problem with its actions, the event, and a restart.
#[path = "contract/video.rs"]
mod video;
