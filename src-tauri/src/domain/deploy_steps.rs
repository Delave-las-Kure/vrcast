//! T256, T257 — the steps a deployment is made of, and the order they must go in.
//!
//! **Safety on a repeat comes from the shape of this, not from care in each step** (R-12,
//! FR-124). Every step is a pair: a check that says whether it has already been done, and an
//! application that does it. The check is written to be independent of the application —
//! looking at the server rather than at a record of what we did — and that single rule buys
//! three things at once: a plan that can be shown before anything changes (FR-122), a repeat
//! after a failure that does not redo what is done, and a way to tell whether an already
//! deployed server still matches the reference, using the same checks with nothing applied.
//!
//! Nothing here touches a server. The pairs themselves live in `server::deploy`; what lives
//! here is which steps exist, what each of them will change, the order, and what a run should
//! do given what the checks found.

use serde::{Deserialize, Serialize};

/// The steps, in no particular order — the order is [`ORDER`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum StepId {
    /// Does the domain point here? Changes nothing.
    DnsCheck,
    /// A swap file, if the memory is too small to install packages without one.
    Swap,
    Packages,
    UserDirs,
    Configs,
    Services,
    /// Put our key in and prove it works — with a second connection.
    SshKey,
    /// Turn password logins off.
    SshHardening,
    Firewall,
    /// Carry out the person's choice about IPv6.
    Ipv6,
    Fail2ban,
    UnattendedUpgrades,
    Tuning,
    /// Ask the serving for something over the domain and read the answer.
    Verify,
    /// Write /etc/vrcast/state.json. Last.
    State,
}

/// The order a deployment goes in.
///
/// Three places in it are not preferences (R-12, `data-model.md` section 10):
///
/// 1. `DnsCheck` **first**, before anything is changed. A wrong DNS record found by a check
///    costs nothing; found at `Verify` it costs a half-configured server.
/// 2. `Swap` before `Packages`. Installing is the peak of memory use — that is the whole
///    reason the swap file exists (FR-134), and after the packages it would be a swap file
///    for the next time.
/// 3. `SshKey` before `SshHardening`. The other way round the application turns off the way
///    it got in before it has another, and getting back means the hosting provider's console
///    — which the person may not have to hand.
///
/// `Verify` second to last and `State` last: the state file is a promise that says "all of
/// this was done here", and written any earlier it turns a half-deployed machine into a
/// deployed one, for us and for every later run.
pub const ORDER: [StepId; 15] = [
    StepId::DnsCheck,
    StepId::Swap,
    StepId::Packages,
    StepId::UserDirs,
    StepId::Configs,
    StepId::Services,
    StepId::SshKey,
    StepId::SshHardening,
    StepId::Firewall,
    StepId::Ipv6,
    StepId::Fail2ban,
    StepId::UnattendedUpgrades,
    StepId::Tuning,
    StepId::Verify,
    StepId::State,
];

/// A pair that must not be swapped, and why. Checked rather than remembered: all three were
/// bought, and the third was bought with a server nobody could get back into.
pub const MUST_PRECEDE: [(StepId, StepId); 4] = [
    (StepId::Swap, StepId::Packages),
    // Found by running it (2026-08-27): the directories step gives the log directory to
    // the `caddy` user, and that user does not exist until the Caddy package makes it.
    // The order was already right; the requirement was not written down, so a future
    // rearrangement would have broken it with nothing to say but an empty failure.
    (StepId::Packages, StepId::UserDirs),
    (StepId::SshKey, StepId::SshHardening),
    (StepId::Verify, StepId::State),
];

/// What is wrong with a proposed order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrderProblem {
    /// Something comes before the domain is checked.
    DnsNotFirst { first: StepId },
    /// A pair is the wrong way round.
    OutOfOrder { earlier: StepId, later: StepId },
    /// A step is named twice, or one is missing. Either way the run is not the deployment.
    NotEveryStepOnce,
}

/// Whether an order may be used.
///
/// Exists so the rule is a check rather than a comment above a constant. A constant is edited
/// by whoever is adding a step, and the comment above it is the first thing that stops being
/// read.
pub fn ordering_holds(order: &[StepId]) -> Result<(), OrderProblem> {
    if order.len() != ORDER.len() || ORDER.iter().any(|s| !order.contains(s)) {
        return Err(OrderProblem::NotEveryStepOnce);
    }
    let at = |step: StepId| order.iter().position(|s| *s == step).unwrap_or(usize::MAX);

    match order.first() {
        Some(StepId::DnsCheck) => {}
        Some(first) => return Err(OrderProblem::DnsNotFirst { first: *first }),
        None => return Err(OrderProblem::NotEveryStepOnce),
    }
    for (earlier, later) in MUST_PRECEDE {
        if at(earlier) > at(later) {
            return Err(OrderProblem::OutOfOrder { earlier, later });
        }
    }
    Ok(())
}

/// A failing step that stops the run, as against one that only fails.
///
/// Blocking where going on would build on what did not happen: without packages there is
/// nothing to configure, without configuration nothing to start. Not blocking where the
/// deployment is still usable without it — the tuning makes the serving faster, and a kernel
/// that will not take a key is a reason to say so, not to leave the person without a server.
pub fn blocking(step: StepId) -> bool {
    !matches!(
        step,
        StepId::Tuning | StepId::Fail2ban | StepId::UnattendedUpgrades
    )
}

/// What a step will change, said in codes rather than sentences.
///
/// This is what a person is shown before they agree (FR-122), and the wordings live in the
/// interface's dictionaries, one per language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Change {
    /// Nothing at all — the step only looks.
    LooksOnly,
    InstallsPackages {
        names: Vec<String>,
    },
    CreatesSwapFile {
        megabytes: u32,
    },
    CreatesSystemUser {
        name: String,
    },
    CreatesDirectory {
        path: String,
    },
    WritesFile {
        path: String,
    },
    EnablesService {
        name: String,
    },
    OpensPorts {
        ports: Vec<String>,
    },
    ClosesEverythingElse,
    AddsSshKey,
    TurnsPasswordLoginOff,
    TurnsIpv6Off,
    SetsKernelSettings,
}

/// Why a step was skipped.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SkipReason {
    /// Not needed on this server — there is already enough memory, or the person chose to
    /// keep IPv6 and there is nothing to turn off.
    NotNeeded,
    /// **Cannot be established in this environment** (T246, measured 2026-08-27).
    ///
    /// The one that must not be quietly folded into "applied". In a container `swapon` is
    /// refused however privileged the container is — and `free` inside it reports the
    /// *host's* swap, so a check that merely looked would pass on a machine that has none.
    /// The keys `net.core.rmem_max`, `wmem_max` and `default_qdisc` are not per-network-
    /// namespace and are invisible inside any container on any host. udev has no real block
    /// device to apply a rule to.
    ///
    /// Without this answer a run in a container reports a fully deployed server that has
    /// neither swap nor tuning — and that report is worse than a failure, because it is
    /// believed.
    NotPossibleHere { detail: String },
}

/// How a step stands.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    NotApplied,
    Applied,
    Failed { detail: String },
    Skipped { why: SkipReason },
}

/// What the check found, before anything is applied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Checked {
    /// Already done. Skipped on a repeat, and this is what makes a repeat safe.
    Applied,
    NotApplied,
    /// Not needed on this server.
    NotNeeded,
    /// Cannot be established here — see [`SkipReason::NotPossibleHere`].
    NotPossibleHere {
        detail: String,
    },
}

/// One step of a plan, as a person is shown it (FR-122, FR-123).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PlannedStep {
    pub id: StepId,
    pub changes: Vec<Change>,
    pub blocking: bool,
    pub status: Status,
}

/// Turn what the checks found into the plan a person agrees to.
///
/// The steps that are already done are shown too, marked as done, rather than left out. A
/// plan that listed only the remaining work would say something different on a repeat than on
/// a first run, and the person would have no way to tell "this was done earlier" from "this
/// will not be done".
/// `ids` is what the plan is about, in whatever order — it comes back in the deployment's
/// own. Handed a subset, the plan speaks only of that subset: a step nobody checked
/// reported as "not applied" is not a gap in the plan, it is the plan making something up,
/// and "is there anything to do" then answers yes for ever.
pub fn plan(
    ids: &[StepId],
    found: &[(StepId, Checked)],
    changes_of: impl Fn(StepId) -> Vec<Change>,
) -> Vec<PlannedStep> {
    ORDER
        .iter()
        .filter(|id| ids.contains(id))
        .map(|id| {
            let status = found
                .iter()
                .find(|(step, _)| step == id)
                .map(|(_, checked)| match checked {
                    Checked::Applied => Status::Applied,
                    Checked::NotApplied => Status::NotApplied,
                    Checked::NotNeeded => Status::Skipped {
                        why: SkipReason::NotNeeded,
                    },
                    Checked::NotPossibleHere { detail } => Status::Skipped {
                        why: SkipReason::NotPossibleHere {
                            detail: detail.clone(),
                        },
                    },
                })
                .unwrap_or(Status::NotApplied);
            PlannedStep {
                id: *id,
                changes: changes_of(*id),
                blocking: blocking(*id),
                status,
            }
        })
        .collect()
}

/// Which steps a run should actually carry out.
///
/// Everything the checks did not find already done, or not needed, or impossible here — in
/// the order of [`ORDER`], whatever order the findings arrived in.
pub fn to_apply(found: &[(StepId, Checked)]) -> Vec<StepId> {
    ORDER
        .iter()
        .copied()
        .filter(|id| {
            !matches!(
                found.iter().find(|(step, _)| step == id).map(|(_, c)| c),
                Some(Checked::Applied)
                    | Some(Checked::NotNeeded)
                    | Some(Checked::NotPossibleHere { .. })
            )
        })
        .collect()
}

/// Whether a failure at this step ends the run.
///
/// Separate from [`blocking`] so the answer reads as a decision rather than as a field: a
/// blocking step that failed stops everything after it, and the report says which one it was
/// (FR-123). Going on would apply steps to a server that is missing what they need, and the
/// failures after it would say nothing about their own causes.
pub fn stops_the_run(step: StepId) -> bool {
    blocking(step)
}
