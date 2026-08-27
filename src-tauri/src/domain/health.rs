//! T303 — reading a server snapshot as "fine / worth a look / trouble" (FR-070).
//!
//! **The recorded trap first, because it is the whole reason this is a module and not five
//! comparisons at the call site.** `systemctl is-active ufw` answers `inactive` on a healthy
//! machine: the firewall is a oneshot unit, it applies the rules and exits, and having exited
//! it is not "active". A judgement that does not know this calls a working firewall a problem
//! every single time — and a panel that cries wolf on every snapshot is a panel nobody reads,
//! which costs more than having no panel at all. The firewall is judged by `ufw status`, which
//! is the thing that actually knows.
//!
//! The same shape of mistake is guarded against twice more below: a container cannot show the
//! kernel settings at all (T246), and a serving cache that is small is only worth mentioning
//! **while somebody is watching** — on an idle server it means nothing was asked for lately,
//! which is not news.
//!
//! What is carried over from the diagnosis skill unchanged (principle VI) is marked at each
//! constant. What is a choice is marked too, and marked plainly.

use serde::{Deserialize, Serialize};

use super::wording::{Detail, DetailCode};

/// How one reading is doing.
///
/// Four answers, not three. `Unknown` is separate from `Fine` on purpose and for the same
/// reason `Checked::NotPossibleHere` exists in the deployment: "we could not find out" shown
/// as "fine" is the failure that hides every other failure behind it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Rating {
    /// Nothing to do.
    Fine,
    /// Not broken, but it is what a person should look at first when something is off.
    Watch,
    /// Broken, and viewers are affected now or will be.
    Trouble,
    /// Could not be established here. Never dressed up as either of the above.
    Unknown,
}

/// Which reading a judgement is about.
///
/// A code, so the interface names it in its own language and puts them in its own order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reading {
    /// The serving itself — is Caddy running.
    Serving,
    /// Does the domain answer a real range request over HTTPS.
    Delivery,
    Firewall,
    /// Which ports are open to the outside.
    OpenPorts,
    Memory,
    /// The page cache the serving reads out of.
    ServingCache,
    Swap,
    DiskSpace,
    /// BBR, the queueing discipline, slow start after idle.
    Network,
    /// The disk's readahead.
    Readahead,
    /// Does the serving come back on its own after a crash.
    AutoRestart,
}

/// One reading, judged.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rated {
    pub about: Reading,
    pub rating: Rating,
    /// What to say, with the numbers it rests on. Never a ready-made sentence (FR-105).
    pub say: Detail,
}

/// A service as the machine reports it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Service {
    pub name: String,
    /// What `systemctl is-active` said: `active`, `inactive`, `failed`, `unknown`.
    pub state: String,
}

/// Memory, in megabytes, as `free -m` reports it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Memory {
    pub total_mb: u32,
    pub used_mb: u32,
    /// The serving cache. This is the field the whole reading is about.
    pub buff_cache_mb: u32,
    pub swap_total_mb: u32,
    pub swap_used_mb: u32,
}

/// The disk the videos live on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Disk {
    pub used_mb: u64,
    pub free_mb: u64,
}

/// The settings that were measured into the deployment (`server::deploy::tuning`).
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Tuning {
    /// `net.ipv4.tcp_congestion_control`.
    pub congestion: Option<String>,
    /// The queueing discipline on the interface itself, not the default. The two differ on a
    /// running machine, and it is the interface's that is serving right now.
    pub qdisc: Option<String>,
    pub slow_start_after_idle: Option<bool>,
    pub readahead_kb: Option<u32>,
    /// `systemctl show caddy -p Restart --value`.
    pub restart: Option<String>,
}

/// What the machine answers a range request over HTTPS with.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Delivery {
    /// It served. `status` is what it answered — 206 for a range, which is what is asked for.
    Answered { status: u16 },
    /// It did not answer at all.
    Silent,
    /// There was nothing to ask for: no video on the server yet. Not a fault.
    NothingToServe,
}

/// Everything read off the machine in one go.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Snapshot {
    pub services: Vec<Service>,
    /// The first line of `ufw status`, which is the one that knows. See the module note.
    pub firewall_status: Option<String>,
    pub memory: Memory,
    pub disk: Disk,
    pub tuning: Tuning,
    /// Addresses the machine listens on that are not loopback.
    pub open_ports: Vec<String>,
    pub delivery: Delivery,
    /// How many viewers are being served at the moment of the snapshot.
    ///
    /// Not decoration: it decides whether a small serving cache is worth saying anything about
    /// at all.
    pub watching_now: u32,
    /// A container cannot be asked about kernel settings or a disk (T246), and saying so is
    /// the honest answer, not "not applied".
    pub container: bool,
}

/// The name the serving runs under.
pub const SERVING_SERVICE: &str = "caddy";

/// What the congestion control has to be. Measured: cubic reads random Wi-Fi loss as
/// congestion and collapses the rate (`server::deploy::tuning`).
pub const WANTED_CONGESTION: &str = "bbr";

/// What the queueing discipline has to be. Measured, same place.
pub const WANTED_QDISC: &str = "fq";

/// What the disk's readahead has to be, in kilobytes. Measured: 128 KB gave 17 MB/s against
/// virtio's latency, 8 MB gives 40–60 MB/s on sequential serving.
pub const WANTED_READAHEAD_KB: u32 = 8192;

/// Below this share of the disk left free it is trouble; below twice it, worth a look.
///
/// **A choice, not a measurement.** A tenth of a disk left is roughly one more film on the
/// smallest tier, and a full disk stops an upload halfway — which is discovered late, after
/// the waiting.
pub const DISK_FREE_TROUBLE: f64 = 0.10;

/// Below this share of memory held as serving cache **while somebody is watching**, worth a
/// look.
///
/// **A choice, not a measurement**, and it is stated because the skill records the direction
/// but no number: "if it is small during a session, there is not enough memory". A quarter of
/// memory on the cheapest tier is around 250 MB, which is a handful of segments — below that
/// the disk is being read instead of the cache, and that is what the readahead measurement was
/// bought to avoid.
pub const CACHE_SHARE_WATCH: f64 = 0.25;

/// Above this share of swap in use, worth a look. **A choice.** Swap being touched at all on a
/// serving machine means memory ran out at some point; being deep into it means it still is.
pub const SWAP_USED_WATCH: f64 = 0.50;

/// Judge a whole snapshot, in the order a person should read it.
///
/// The order is deliberate: whether it serves at all comes first, and the kernel settings —
/// which are a matter of how *well* it serves — come last. A list that opens with the
/// readahead teaches people to scroll past the part that matters.
pub fn judge(snap: &Snapshot) -> Vec<Rated> {
    vec![
        serving(snap),
        delivery(snap),
        firewall(snap),
        ports(snap),
        memory(snap),
        serving_cache(snap),
        swap(snap),
        disk(snap),
        network(snap),
        readahead(snap),
        auto_restart(snap),
    ]
}

/// The worst rating in a list, which is what a badge on the screen shows.
///
/// `Unknown` does not win over `Trouble` or over `Watch`: something that could not be
/// established must never mask something that was.
pub fn worst(rated: &[Rated]) -> Rating {
    let mut worst = Rating::Fine;
    for r in rated {
        worst = match (worst, r.rating) {
            (Rating::Trouble, _) | (_, Rating::Trouble) => Rating::Trouble,
            (Rating::Watch, _) | (_, Rating::Watch) => Rating::Watch,
            (Rating::Unknown, _) | (_, Rating::Unknown) => Rating::Unknown,
            _ => Rating::Fine,
        };
    }
    worst
}

fn rated(about: Reading, rating: Rating, say: Detail) -> Rated {
    Rated { about, rating, say }
}

fn unknown(about: Reading) -> Rated {
    rated(
        about,
        Rating::Unknown,
        Detail::new(DetailCode::HealthNotEstablished),
    )
}

fn state_of<'a>(snap: &'a Snapshot, name: &str) -> Option<&'a str> {
    snap.services
        .iter()
        .find(|s| s.name == name)
        .map(|s| s.state.as_str())
}

fn serving(snap: &Snapshot) -> Rated {
    match state_of(snap, SERVING_SERVICE) {
        Some("active") => rated(
            Reading::Serving,
            Rating::Fine,
            Detail::new(DetailCode::HealthServingRunning),
        ),
        // The service is **named** rather than left as "something is down": T321 requires the
        // stopped service to be identified, and naming it is the difference between a person
        // knowing what to start and a person opening a console to find out.
        Some(state) => rated(
            Reading::Serving,
            Rating::Trouble,
            Detail::new(DetailCode::HealthServingStopped)
                .with("service", SERVING_SERVICE)
                .with("state", state),
        ),
        None => unknown(Reading::Serving),
    }
}

fn delivery(snap: &Snapshot) -> Rated {
    match snap.delivery {
        // 206 is the answer a range request deserves, and a range request is how every player
        // asks. A 200 means the whole file came back instead — the serving works and seeking
        // will not, which is a different complaint and has to read as a different answer.
        Delivery::Answered { status: 206 } => rated(
            Reading::Delivery,
            Rating::Fine,
            Detail::new(DetailCode::HealthDeliveryOk).with("status", 206),
        ),
        Delivery::Answered { status } if (200..300).contains(&status) => rated(
            Reading::Delivery,
            Rating::Watch,
            Detail::new(DetailCode::HealthDeliveryNoRanges).with("status", status),
        ),
        Delivery::Answered { status } => rated(
            Reading::Delivery,
            Rating::Trouble,
            Detail::new(DetailCode::HealthDeliveryRefused).with("status", status),
        ),
        Delivery::Silent => rated(
            Reading::Delivery,
            Rating::Trouble,
            Detail::new(DetailCode::HealthDeliverySilent),
        ),
        Delivery::NothingToServe => rated(
            Reading::Delivery,
            Rating::Fine,
            Detail::new(DetailCode::HealthNothingToServe),
        ),
    }
}

fn firewall(snap: &Snapshot) -> Rated {
    // See the module note. `is-active` is not consulted here at all, deliberately.
    match snap.firewall_status.as_deref() {
        Some(s) if s.eq_ignore_ascii_case("active") => rated(
            Reading::Firewall,
            Rating::Fine,
            Detail::new(DetailCode::HealthFirewallOn),
        ),
        Some(s) => rated(
            Reading::Firewall,
            Rating::Trouble,
            Detail::new(DetailCode::HealthFirewallOff).with("status", s),
        ),
        None => unknown(Reading::Firewall),
    }
}

fn ports(snap: &Snapshot) -> Rated {
    // Listed, not judged: which ports ought to be open depends on what else the owner runs on
    // their own machine, and an application that calls their mail server a problem is simply
    // wrong. Shown so that a person can look at the list themselves.
    rated(
        Reading::OpenPorts,
        Rating::Fine,
        Detail::new(DetailCode::HealthOpenPorts)
            .with("count", snap.open_ports.len() as u64)
            .with("ports", snap.open_ports.join(", ")),
    )
}

fn memory(snap: &Snapshot) -> Rated {
    let m = snap.memory;
    if m.total_mb == 0 {
        return unknown(Reading::Memory);
    }
    rated(
        Reading::Memory,
        Rating::Fine,
        Detail::new(DetailCode::HealthMemory)
            .with("total_mb", m.total_mb)
            .with("used_mb", m.used_mb),
    )
}

fn serving_cache(snap: &Snapshot) -> Rated {
    let m = snap.memory;
    if m.total_mb == 0 {
        return unknown(Reading::ServingCache);
    }

    // Small **and nobody watching** is not news: nothing has been asked for, so nothing is
    // cached. Still reported, because leaving the reading out of the list altogether would
    // read as the cache being fine — which it might not be a minute later.
    if snap.watching_now == 0 {
        return rated(
            Reading::ServingCache,
            Rating::Fine,
            Detail::new(DetailCode::HealthCacheIdle).with("cache_mb", m.buff_cache_mb),
        );
    }

    let share = f64::from(m.buff_cache_mb) / f64::from(m.total_mb);
    if share < CACHE_SHARE_WATCH {
        return rated(
            Reading::ServingCache,
            Rating::Watch,
            Detail::new(DetailCode::HealthCacheSmall)
                .with("cache_mb", m.buff_cache_mb)
                .with("total_mb", m.total_mb)
                .with("watching", snap.watching_now),
        );
    }
    rated(
        Reading::ServingCache,
        Rating::Fine,
        Detail::new(DetailCode::HealthCacheOk)
            .with("cache_mb", m.buff_cache_mb)
            .with("watching", snap.watching_now),
    )
}

fn swap(snap: &Snapshot) -> Rated {
    let m = snap.memory;
    if m.swap_total_mb == 0 {
        // Worth a look rather than trouble, and only because the deployment makes swap when
        // the machine is short of memory (`domain::swap`): none at all means either a roomy
        // machine or a deployment that could not.
        return rated(
            Reading::Swap,
            Rating::Watch,
            Detail::new(DetailCode::HealthNoSwap).with("total_mb", m.total_mb),
        );
    }
    let share = f64::from(m.swap_used_mb) / f64::from(m.swap_total_mb);
    let code = if share > SWAP_USED_WATCH {
        DetailCode::HealthSwapInUse
    } else {
        DetailCode::HealthSwapOk
    };
    let rating = if share > SWAP_USED_WATCH {
        Rating::Watch
    } else {
        Rating::Fine
    };
    rated(
        Reading::Swap,
        rating,
        Detail::new(code)
            .with("used_mb", m.swap_used_mb)
            .with("total_mb", m.swap_total_mb),
    )
}

fn disk(snap: &Snapshot) -> Rated {
    let d = snap.disk;
    let whole = d.used_mb.saturating_add(d.free_mb);
    if whole == 0 {
        return unknown(Reading::DiskSpace);
    }
    let free_share = d.free_mb as f64 / whole as f64;
    let rating = if free_share < DISK_FREE_TROUBLE {
        Rating::Trouble
    } else if free_share < DISK_FREE_TROUBLE * 2.0 {
        Rating::Watch
    } else {
        Rating::Fine
    };
    rated(
        Reading::DiskSpace,
        rating,
        Detail::new(DetailCode::HealthDisk)
            .with("free_mb", d.free_mb)
            .with("total_mb", whole),
    )
}

fn network(snap: &Snapshot) -> Rated {
    if snap.container {
        return rated(
            Reading::Network,
            Rating::Unknown,
            Detail::new(DetailCode::HealthNotInContainer),
        );
    }
    let t = &snap.tuning;
    let (Some(congestion), Some(qdisc)) = (t.congestion.as_deref(), t.qdisc.as_deref()) else {
        return unknown(Reading::Network);
    };
    let idle_ok = t.slow_start_after_idle == Some(false);
    if congestion == WANTED_CONGESTION && qdisc == WANTED_QDISC && idle_ok {
        return rated(
            Reading::Network,
            Rating::Fine,
            Detail::new(DetailCode::HealthNetworkTuned).with("congestion", congestion),
        );
    }
    // Worth a look rather than trouble: the serving works without any of this, it is only
    // slower, and calling a working server broken over a kernel setting is the crying-wolf
    // mistake all over again.
    rated(
        Reading::Network,
        Rating::Watch,
        Detail::new(DetailCode::HealthNetworkUntuned)
            .with("congestion", congestion)
            .with("qdisc", qdisc)
            .with("wanted_congestion", WANTED_CONGESTION)
            .with("wanted_qdisc", WANTED_QDISC),
    )
}

fn readahead(snap: &Snapshot) -> Rated {
    if snap.container {
        return rated(
            Reading::Readahead,
            Rating::Unknown,
            Detail::new(DetailCode::HealthNotInContainer),
        );
    }
    match snap.tuning.readahead_kb {
        Some(kb) if kb >= WANTED_READAHEAD_KB => rated(
            Reading::Readahead,
            Rating::Fine,
            Detail::new(DetailCode::HealthReadaheadOk).with("kb", kb),
        ),
        Some(kb) => rated(
            Reading::Readahead,
            Rating::Watch,
            Detail::new(DetailCode::HealthReadaheadSmall)
                .with("kb", kb)
                .with("wanted_kb", WANTED_READAHEAD_KB),
        ),
        None => unknown(Reading::Readahead),
    }
}

fn auto_restart(snap: &Snapshot) -> Rated {
    match snap.tuning.restart.as_deref() {
        // Anything but `no` brings the serving back. Which of `always` or `on-failure` is set
        // does not matter to the person, so it is not made to.
        Some("no") | Some("") => rated(
            Reading::AutoRestart,
            Rating::Watch,
            Detail::new(DetailCode::HealthNoAutoRestart),
        ),
        Some(mode) => rated(
            Reading::AutoRestart,
            Rating::Fine,
            Detail::new(DetailCode::HealthAutoRestart).with("mode", mode),
        ),
        None => unknown(Reading::AutoRestart),
    }
}
