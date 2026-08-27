//! T313 — asking a server how it is doing (FR-070).
//!
//! Gathers; `domain::health` judges. The split is the same one as in `detect`, and for the
//! same reason: the judging is where the costly mistakes live — a healthy firewall called
//! broken, a stopped serving called fine — and it has to be checkable without a server.
//!
//! **One command, as always.** The connection is one and its channels are eight, two of them
//! already held by the watching of viewers (R-04). A dozen little questions here would take a
//! dozen slots in turn, at the very moment somebody is watching a server that is misbehaving.
//!
//! **The serving cache is asked for by name** and it is the point of the memory reading, not
//! a detail of it: it is what the segments are handed out of, and a server reading its disk
//! instead is a server whose viewers stutter while every other figure looks fine.

use crate::domain::health::{
    self, Delivery, Disk, Memory, Service, Snapshot, Tuning, SERVING_SERVICE,
};
use crate::ssh::{Connection, Result};

/// Services asked about by name.
///
/// The firewall is in the list although its answer is **not** what the firewall is judged by:
/// it is shown so that a person looking at the raw readings sees the same `inactive` the
/// console would show them and is not left wondering why the application disagrees.
const SERVICES: [&str; 2] = [SERVING_SERVICE, "ufw"];

/// What is asked, and how the answer is laid out.
///
/// Every question ends in `2>/dev/null` and a fallback: a server where one reading cannot be
/// taken must come back missing that one reading, not as a command that failed and took the
/// whole snapshot with it. That is what makes `Rating::Unknown` reachable at all.
pub fn command(video_dir: &str, domain: &str) -> String {
    const ASK: &str = r#"
IF=$(ip -o -4 route show default 2>/dev/null | awk '{print $5}' | head -n 1)
DISK=$(lsblk -no PKNAME "$(findmnt -no SOURCE {VIDEO_DIR} 2>/dev/null || findmnt -no SOURCE / 2>/dev/null)" 2>/dev/null | head -n 1)
[ -z "$DISK" ] && DISK=$(lsblk -dno NAME 2>/dev/null | head -n 1)
for s in {SERVICES}; do
  printf 'service=%s %s\n' "$s" "$(systemctl is-active $s 2>/dev/null || echo unknown)"
done
# The firewall is judged by THIS and never by is-active: ufw is a oneshot unit and answers
# `inactive` on a machine whose rules are in force. See `domain::health`.
printf 'firewall=%s\n' "$(ufw status 2>/dev/null | head -n 1 | awk '{print $2}')"
free -m 2>/dev/null | awk '/^Mem:/{printf "memory=%s %s %s\n", $2, $3, $6} /^Swap:/{printf "swap=%s %s\n", $2, $3}'
df -m {VIDEO_DIR} 2>/dev/null | tail -n 1 | awk '{printf "disk=%s %s\n", $3, $4}'
printf 'congestion=%s\n' "$(sysctl -n net.ipv4.tcp_congestion_control 2>/dev/null)"
printf 'qdisc=%s\n' "$(tc qdisc show dev $IF 2>/dev/null | head -n 1 | awk '{print $2}')"
printf 'slow_start=%s\n' "$(sysctl -n net.ipv4.tcp_slow_start_after_idle 2>/dev/null)"
printf 'readahead=%s\n' "$(cat /sys/block/$DISK/queue/read_ahead_kb 2>/dev/null)"
printf 'restart=%s\n' "$(systemctl show {SERVING} -p Restart --value 2>/dev/null)"
ss -ltnH 2>/dev/null | awk '{print $4}' | grep -vE '^127\.|^\[::1\]' | sort -u | sed 's/^/port=/'
# How many are being served right now. It decides whether a small serving cache is worth
# saying anything about, so it is taken in the same breath as the cache itself — a figure a
# minute older would be about a different machine.
printf 'watching=%s\n' "$(ss -tnH state established 2>/dev/null | awk '$4 ~ /:(80|443)$/' | awk '{print $5}' | sed 's/:[0-9]*$//' | sort -u | wc -l)"
# The serving asked with a **range**, and its answer checked rather than the port (R-20).
#
# **Over the domain, resolved to the loopback.** A deployed Caddy binds the domain and
# nothing else, so a plain request to 127.0.0.1 matches no site and comes back 404 — a
# reading that would have called every healthy server broken. Caught in a container on
# 2026-08-27, before it could be seen anywhere it mattered.
#
# The certificate is deliberately not verified: whether it is valid is asked from the other
# machine, where the answer means something. From the server's own side a certificate always
# looks fine, so checking it here would prove nothing and refuse plenty.
#
# Plain HTTP with the right Host is the second try, and it is what a container answers — its
# Caddy listens on :80 with no domain at all. On a real server the first try is the one that
# works, and a redirect coming back from the second means TLS is broken, which is worth
# knowing and is not "fine".
F=$(ls {VIDEO_DIR}/*.mp4 2>/dev/null | head -n 1)
if [ -n "$F" ]; then
  N=$(basename "$F")
  CODE=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 10 -r 0-1000 --resolve "{DOMAIN}:443:127.0.0.1" "https://{DOMAIN}/videos/$N" 2>/dev/null)
  case "$CODE" in
    2*) ;;
    *) CODE=$(curl -s -o /dev/null -w '%{http_code}' --max-time 10 -r 0-1000 -H "Host: {DOMAIN}" "http://127.0.0.1/videos/$N" 2>/dev/null) ;;
  esac
  printf 'delivery=%s\n' "$CODE"
else
  printf 'delivery=none\n'
fi
printf 'container=%s\n' "{CONTAINER}"
"#;

    ASK.replace("{VIDEO_DIR}", &super::shell_quote(video_dir))
        .replace("{SERVICES}", &SERVICES.join(" "))
        .replace("{SERVING}", SERVING_SERVICE)
        .replace("{CONTAINER}", super::CONTAINER_KIND)
        .replace("{DOMAIN}", domain)
}

/// Ask a server how it is, and say.
pub async fn look(conn: &Connection, video_dir: &str, domain: &str) -> Result<Snapshot> {
    let said = conn.exec(&command(video_dir, domain)).await?;
    Ok(read(&said.stdout))
}

/// Turn the answer into a snapshot.
///
/// Separate from the asking so the parsing is checkable without a server. Missing readings
/// stay missing: a reading this cannot find becomes `None`, which `domain::health` shows as
/// "could not be established" — never as a zero, which would read as a measurement.
pub fn read(said: &str) -> Snapshot {
    let mut snap = Snapshot {
        services: Vec::new(),
        firewall_status: None,
        memory: Memory {
            total_mb: 0,
            used_mb: 0,
            buff_cache_mb: 0,
            swap_total_mb: 0,
            swap_used_mb: 0,
        },
        disk: Disk {
            used_mb: 0,
            free_mb: 0,
        },
        tuning: Tuning::default(),
        open_ports: Vec::new(),
        delivery: Delivery::Silent,
        watching_now: 0,
        container: false,
    };

    for line in said.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "service" => {
                if let Some((name, state)) = value.split_once(' ') {
                    snap.services.push(Service {
                        name: name.to_owned(),
                        state: state.trim().to_owned(),
                    });
                }
            }
            "firewall" => snap.firewall_status = some_text(value),
            "memory" => {
                let n = numbers(value);
                snap.memory.total_mb = n.first().copied().unwrap_or(0) as u32;
                snap.memory.used_mb = n.get(1).copied().unwrap_or(0) as u32;
                snap.memory.buff_cache_mb = n.get(2).copied().unwrap_or(0) as u32;
            }
            "swap" => {
                let n = numbers(value);
                snap.memory.swap_total_mb = n.first().copied().unwrap_or(0) as u32;
                snap.memory.swap_used_mb = n.get(1).copied().unwrap_or(0) as u32;
            }
            "disk" => {
                let n = numbers(value);
                snap.disk.used_mb = n.first().copied().unwrap_or(0);
                snap.disk.free_mb = n.get(1).copied().unwrap_or(0);
            }
            "congestion" => snap.tuning.congestion = some_text(value),
            "qdisc" => snap.tuning.qdisc = some_text(value),
            // Nothing at all is not "off": an unreadable setting must not read as a
            // deliberate zero, which is the value the tuning wants.
            "slow_start" => snap.tuning.slow_start_after_idle = some_text(value).map(|v| v != "0"),
            "readahead" => snap.tuning.readahead_kb = value.parse().ok(),
            "restart" => snap.tuning.restart = some_text(value),
            "port" => snap.open_ports.push(value.to_owned()),
            "watching" => snap.watching_now = value.parse().unwrap_or(0),
            "delivery" => snap.delivery = delivery_of(value),
            // `none` and nothing at all both mean a real machine. Reading `none` as the
            // name of a container kind would make every real server answer "cannot be
            // established here" about its kernel settings.
            "container" => snap.container = !value.is_empty() && value != "none",
            _ => {}
        }
    }
    snap
}

/// Judge in one step, for callers that want the answer rather than the readings.
pub fn judged(snap: &Snapshot) -> Vec<health::Rated> {
    health::judge(snap)
}

fn some_text(value: &str) -> Option<String> {
    (!value.is_empty()).then(|| value.to_owned())
}

fn numbers(value: &str) -> Vec<u64> {
    value
        .split_whitespace()
        .filter_map(|n| n.parse().ok())
        .collect()
}

fn delivery_of(value: &str) -> Delivery {
    if value == "none" {
        return Delivery::NothingToServe;
    }
    match value.parse::<u16>() {
        // curl writes 000 when it never got an answer at all, and that is a different thing
        // from a server that answered badly: one is a machine not serving, the other is a
        // machine serving wrongly, and they are fixed differently.
        Ok(0) | Err(_) => Delivery::Silent,
        Ok(status) => Delivery::Answered { status },
    }
}

// ---------- what the machine is doing right now ----------

/// How long the live readings are taken over, in seconds.
///
/// **Five, carried over from the skill unchanged.** Bytes on an interface and sectors off a
/// disk are counters, so a rate needs two readings and a gap between them; five seconds is
/// long enough that one player's burst does not become "the server is flat out", and short
/// enough that somebody watching a stuttering picture will wait for it.
pub const SAMPLE_S: u32 = 5;

/// What the machine is doing at this moment, and what it can do at all.
pub fn load_command() -> String {
    const ASK: &str = r#"
IF=$(ip -o -4 route show default 2>/dev/null | awk '{print $5}' | head -n 1)
DISK=$(lsblk -dno NAME 2>/dev/null | head -n 1)
T1=$(cat /sys/class/net/$IF/statistics/tx_bytes 2>/dev/null || echo 0)
R1=$(awk -v d="$DISK" '$3 == d {print $6}' /proc/diskstats 2>/dev/null | head -n 1)
C1=$(awk '/^cpu /{idle=$5; total=0; for(i=2;i<=NF;i++) total+=$i; print idle" "total}' /proc/stat 2>/dev/null)
sleep {SAMPLE}
T2=$(cat /sys/class/net/$IF/statistics/tx_bytes 2>/dev/null || echo 0)
R2=$(awk -v d="$DISK" '$3 == d {print $6}' /proc/diskstats 2>/dev/null | head -n 1)
C2=$(awk '/^cpu /{idle=$5; total=0; for(i=2;i<=NF;i++) total+=$i; print idle" "total}' /proc/stat 2>/dev/null)
printf 'out_mbit_s=%s\n' "$(awk -v a="$T1" -v b="$T2" -v s={SAMPLE} 'BEGIN{printf "%.2f", (b-a)*8/s/1000000}')"
# Sectors are 512 bytes, always, whatever the disk's own sector size — the kernel reports
# this counter in 512-byte units by definition.
printf 'disk_read_mb_s=%s\n' "$(awk -v a="${R1:-0}" -v b="${R2:-0}" -v s={SAMPLE} 'BEGIN{printf "%.2f", (b-a)*512/1048576/s}')"
printf 'cpu_busy=%s\n' "$(awk -v a="$C1" -v b="$C2" 'BEGIN{split(a,x," "); split(b,y," "); dt=y[2]-x[2]; if (dt<=0) {print ""} else {printf "%.3f", 1-(y[1]-x[1])/dt}}')"
# What the link can do at all. Absent or nonsense means the branch that would blame the
# server's own link stays shut — an invented capacity is worse than none, because it turns
# into a confident accusation.
printf 'capacity_mbit_s=%s\n' "$(cat /sys/class/net/$IF/speed 2>/dev/null)"
printf 'cache_mb=%s\n' "$(free -m 2>/dev/null | awk '/^Mem:/{print $6}')"
printf 'memory_mb=%s\n' "$(free -m 2>/dev/null | awk '/^Mem:/{print $2}')"
# The machine's own addresses, so that our own checks can be told apart from viewers.
ip -o addr show scope global 2>/dev/null | awk '{print $4}' | cut -d/ -f1 | sed 's/^/address=/'
"#;

    ASK.replace("{SAMPLE}", &SAMPLE_S.to_string())
}

/// What the machine is doing, with its own addresses.
#[derive(Debug, Clone, PartialEq)]
pub struct Live {
    pub load: crate::domain::stalls::Load,
    /// The machine's own addresses. Whatever comes from these is us, not a viewer.
    pub addresses: Vec<String>,
}

/// Take the live readings.
pub async fn load(conn: &Connection) -> Result<Live> {
    let said = conn.exec(&load_command()).await?;
    Ok(read_load(&said.stdout))
}

/// Turn the live readings into what `domain::stalls` asks for.
pub fn read_load(said: &str) -> Live {
    let mut out_mbit_s = 0.0;
    let mut disk_read_mb_s = 0.0;
    let mut cpu_busy = 0.0;
    let mut capacity_mbit_s = 0.0;
    let mut cache_mb: u32 = 0;
    let mut memory_mb: u32 = 0;
    let mut addresses = Vec::new();

    for line in said.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "out_mbit_s" => out_mbit_s = value.parse().unwrap_or(0.0),
            "disk_read_mb_s" => disk_read_mb_s = value.parse().unwrap_or(0.0),
            "cpu_busy" => cpu_busy = value.parse().unwrap_or(0.0),
            // A virtual interface often reports -1, and some report nothing at all.
            "capacity_mbit_s" => {
                capacity_mbit_s = value
                    .parse::<f64>()
                    .ok()
                    .filter(|v| *v > 0.0)
                    .unwrap_or(0.0)
            }
            "cache_mb" => cache_mb = value.parse().unwrap_or(0),
            "memory_mb" => memory_mb = value.parse().unwrap_or(0),
            "address" => addresses.push(value.to_owned()),
            _ => {}
        }
    }

    let cache_small =
        memory_mb > 0 && f64::from(cache_mb) / f64::from(memory_mb) < health::CACHE_SHARE_WATCH;

    Live {
        load: crate::domain::stalls::Load {
            cpu_busy,
            disk_read_mb_s,
            out_mbit_s,
            capacity_mbit_s,
            cache_small,
        },
        addresses,
    }
}
