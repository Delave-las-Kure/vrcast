//! T281 — last: the file that says this server is ours (FR-127).
//!
//! **Last, and the order is the whole point.** The state file is a promise — it means "all of
//! this was done here" — and written any earlier it turns a half-deployed machine into a
//! deployed one, for us and for every later run. The recognition rule reads it first
//! (`contracts/server-contract.md`), so a file written too soon makes the application stop
//! looking at everything else.

use futures::future::BoxFuture;

use crate::domain::deploy_steps::{Change, Checked, StepId, ORDER};
use crate::domain::server_state::{self, StateFile, APP_EXPECTS};

use super::{Context, DeployError, Result, Step};

pub const PATH: &str = "/etc/vrcast/state.json";

pub fn step<'a>() -> Step<Context<'a>> {
    Step {
        id: StepId::State,
        changes,
        check,
        apply,
    }
}

fn changes(_: &Context<'_>) -> Vec<Change> {
    vec![Change::WritesFile {
        path: String::from(PATH),
    }]
}

/// What goes in the file.
///
/// `steps_applied` lists the whole deployment rather than only what this run did: the file
/// describes the server's state, not the history of how it got there, and a repeat that had
/// nothing left to do would otherwise write an empty list over a full one.
fn body(ctx: &Context<'_>, now: &str) -> String {
    let file = StateFile {
        vrcast_server_version: APP_EXPECTS,
        deployed_at: now.to_owned(),
        deployed_by_app: env!("CARGO_PKG_VERSION").to_owned(),
        steps_applied: ORDER.iter().map(|id| format!("{id:?}")).collect(),
        video_dir: ctx.video_dir.to_owned(),
        domain: ctx.domain.to_owned(),
    };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

fn check<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<Checked>> {
    Box::pin(async move {
        let said = ctx.ran(&format!("cat {PATH} 2>/dev/null || true")).await?;
        let text = said.trim();
        if text.is_empty() {
            return Ok(Checked::NotApplied);
        }
        // The version, not the file's presence. A file left by an older version of the
        // application describes a server that is not the one we deploy, and treating it as
        // done would leave the two disagreeing about where things are.
        Ok(match server_state::parse_state_file(text) {
            Ok(file) if file.vrcast_server_version == APP_EXPECTS => Checked::Applied,
            _ => Checked::NotApplied,
        })
    })
}

fn apply<'x, 'a>(ctx: &'x Context<'a>) -> BoxFuture<'x, Result<()>> {
    Box::pin(async move {
        // The server's own clock. The two machines' clocks differ, sometimes by hours, and a
        // date written from here would say the server was deployed before it existed.
        let now = ctx
            .ran("date -u +%Y-%m-%dT%H:%M:%SZ")
            .await?
            .trim()
            .to_owned();
        if now.is_empty() {
            return Err(DeployError::Step {
                id: StepId::State,
                detail: String::from("the server would not say what time it is"),
                advice: None,
            });
        }
        ctx.put_file(PATH, &body(ctx, &now)).await?;
        Ok(())
    })
}
