//! What the machine is like, asked once and handed to every step.
//!
//! Five steps want to know something about it — how much memory, which disk, which network
//! interface, whether this is a container — and five separate questions would cost five
//! channel slots in turn, at the moment a person is also opening a library and starting a
//! viewer watch (R-04). One question, one answer, passed around.
//!
//! It also lets a step's `changes` be exact without being able to ask anything: the plan a
//! person agrees to says "a swap file of 1280 MB", not "a swap file", and that number is
//! known here.

use crate::ssh::{Connection, Result};

/// Everything the steps need to know about the machine itself.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Machine {
    pub memory_mb: u32,
    pub swap_mb: u32,
    /// Free space where the swap file would go.
    pub free_disk_mb: u32,
    /// The system disk's kernel name — `vda` on a virtio VPS, `sda` or `nvme0n1` elsewhere.
    ///
    /// **Worked out rather than assumed.** The skill's readahead rule writes `vda` in and
    /// warns to fix it by hand; the application has nobody to warn, and a rule naming a disk
    /// that is not there applies to nothing, quietly.
    pub disk: String,
    /// The interface the default route goes out of. The queueing discipline is set on it by
    /// name, and `default_qdisc` alone would only take effect on interfaces made later.
    pub interface: String,
    /// What kind of container this is, if it is one.
    ///
    /// Not curiosity (T246): in a container `swapon` is refused whatever the privileges, and
    /// `free` inside reports the **host's** swap — so a check that merely looked would pass on
    /// a machine that has none. Several of the kernel settings are not per-namespace and are
    /// invisible here at all. Those steps answer "cannot be established here" rather than
    /// guessing, and this is how they know.
    pub container: Option<String>,
}

/// One block of shell, one answer.
const ASK: &str = r#"
printf 'memory_mb=%s\n' "$(awk '/^MemTotal:/{printf "%d", $2/1024}' /proc/meminfo 2>/dev/null)"
printf 'swap_mb=%s\n' "$(awk '/^SwapTotal:/{printf "%d", $2/1024}' /proc/meminfo 2>/dev/null)"
printf 'free_disk_mb=%s\n' "$(df -Pm / 2>/dev/null | awk 'NR==2{print $4}')"
printf 'disk=%s\n' "$(lsblk -ndo pkname "$(findmnt -no SOURCE / 2>/dev/null)" 2>/dev/null || lsblk -ndo name -e 7,11 2>/dev/null | head -n 1)"
printf 'interface=%s\n' "$(ip -o -4 route show default 2>/dev/null | awk '{print $5; exit}')"
printf 'container=%s\n' "$(systemd-detect-virt -c 2>/dev/null || true)"
"#;

/// Ask the machine about itself.
pub async fn look(conn: &Connection) -> Result<Machine> {
    let said = conn.exec(ASK).await?;
    Ok(read(&said.stdout))
}

/// Turn the answer into facts.
///
/// Separate from the asking so the awkward answers are checkable without a server, and there
/// are several: a machine with no swap line at all, `lsblk` absent, a host with no default
/// route, `systemd-detect-virt` saying `none`.
pub fn read(said: &str) -> Machine {
    let mut machine = Machine::default();
    for line in said.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "memory_mb" => machine.memory_mb = value.parse().unwrap_or(0),
            "swap_mb" => machine.swap_mb = value.parse().unwrap_or(0),
            "free_disk_mb" => machine.free_disk_mb = value.parse().unwrap_or(0),
            "disk" => machine.disk = value.to_owned(),
            "interface" => machine.interface = value.to_owned(),
            "container" => {
                // `systemd-detect-virt -c` prints "none" and exits non-zero on a real machine.
                // Treating "none" as the name of a container kind would make every real server
                // answer "cannot be established here" — and a deployment that skips swap and
                // tuning everywhere is worse than one that fails, because it reports success.
                machine.container = match value {
                    "" | "none" => None,
                    other => Some(other.to_owned()),
                };
            }
            _ => {}
        }
    }
    machine
}

impl Machine {
    /// Whether a step that cannot work in a container should say so.
    pub fn is_container(&self) -> bool {
        self.container.is_some()
    }

    /// How to say it, for the step's answer.
    pub fn container_detail(&self, what: &str) -> String {
        match &self.container {
            Some(kind) => format!("{what} cannot be established inside a {kind} container"),
            None => format!("{what} cannot be established here"),
        }
    }
}
