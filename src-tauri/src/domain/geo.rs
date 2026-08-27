//! T161 — the rules for placing a viewer's address.
//!
//! Only the rules. The table itself is fetched and read in `store::geo`, and that split is
//! the point: what must not be got wrong here is not the searching — a well-worn library
//! does that — but **what counts as an answer**. Absent means not determined; an address
//! nobody can speak for is not looked up at all.
//!
//! On this machine and nowhere else: FR-057 forbids handing viewers' addresses to anyone
//! else, and asking an outside service would be doing exactly that, once per viewer, for
//! everyone watching. R-08 settled which table and why.

use std::net::IpAddr;

/// Where an address is, as far as is known.
///
/// Every field may be absent on its own: a table often knows the country and not the city.
/// Absent means **not determined** and is shown as that — it is never filled in by guessing
/// from a neighbouring range (FR-052).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Place {
    pub country: Option<String>,
    pub city: Option<String>,
    /// Who the address belongs to — the provider.
    pub asn_org: Option<String>,
}

impl Place {
    /// Whether anything at all is known.
    pub fn is_empty(&self) -> bool {
        self.country.is_none() && self.city.is_none() && self.asn_org.is_none()
    }
}

/// Whether the address is one no table can speak for.
///
/// Asked **before** any lookup, and that is the whole of it. Tables do hold rows for the
/// reserved ranges, and answering out of them would place somebody watching from the next
/// room in a country — which looks exactly like knowledge and is acted on as such.
pub fn is_not_public(address: &IpAddr) -> bool {
    match address {
        IpAddr::V4(v4) => {
            v4.is_private()
                || v4.is_loopback()
                || v4.is_link_local()
                || v4.is_broadcast()
                || v4.is_documentation()
                || v4.is_unspecified()
                // 100.64.0.0/10 — the range providers use between themselves. Not private
                // by the standard library's reckoning, and just as meaningless to look up.
                || (v4.octets()[0] == 100 && (64..128).contains(&v4.octets()[1]))
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()
                || v6.is_unspecified()
                // 2001:db8::/32 — the range set aside for writing examples in.
                || (v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8)
                // fc00::/7, addresses used inside one organisation.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10, addresses that do not leave the local link.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // A v4 address in v6 clothing is judged as the v4 address it is: otherwise
                // a viewer from the next room would be looked up and answered for.
                || v6
                    .to_ipv4_mapped()
                    .is_some_and(|v4| is_not_public(&IpAddr::V4(v4)))
        }
    }
}

// ---------- T310: is the viewer behind a tunnel? ----------

/// The size a whole path carries when nothing along it is cutting.
///
/// Anything smaller means something in between is wrapping the traffic — a tunnel, a VPN, or
/// a PPPoE line — and the practical consequence is the same either way: what is measured to
/// the address is the path to *that* box, and the rest of the way to the person's headset is
/// not visible from here.
pub const WHOLE_PATH_MTU: u16 = 1500;

/// Words in a provider's name that suggest a machine room rather than a home.
///
/// **Kept deliberately short, and never enough on its own to conclude anything.** The curated
/// "is this hosting" flag the diagnosis skill used comes from an outside service, and handing
/// it a viewer's address is precisely what FR-057 forbids — so what is left is the provider's
/// name out of the local table, and a name is a hint. Half the world's home providers have
/// "LLC" in theirs; a list long enough to catch every machine room would call them all VPNs.
const MACHINE_ROOM_WORDS: [&str; 6] = [
    "hosting",
    "datacenter",
    "data center",
    "colocation",
    "cloud",
    "vps",
];

/// What can be said about a tunnel between the server and the viewer.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tunnel {
    /// The path is whole and the provider looks like a home one. Nothing in the way.
    NoSign,
    /// The provider's name looks like a machine room. A hint, not a finding: see
    /// [`MACHINE_ROOM_WORDS`].
    Possible { provider: String },
    /// The path is cut, which is measured rather than guessed.
    Likely { mtu: u16 },
    /// **The measurement is not valid**, and saying so is the whole reason this exists.
    ///
    /// When the address answers no pings at all, every size fails — and a probe where every
    /// size fails says nothing about the path. The skill's script once printed "the viewer is
    /// behind a VPN" unconditionally here and was wrong on the first real complaint it met: a
    /// home line with ICMP turned off was announced as a machine room behind a tunnel.
    CannotTell,
}

impl Tunnel {
    /// Whether the person should be advised to watch without their VPN.
    ///
    /// Only on the measured sign. Advice given on a hint is advice given to people who have
    /// no VPN to turn off, and they will do as they are told and come back no better.
    pub fn worth_advising(&self) -> bool {
        matches!(self, Self::Likely { .. })
    }
}

/// Judge, from what can be seen here and nowhere else.
///
/// `largest_whole_packet` is the biggest packet that made the trip, as the server measured it;
/// `None` when nothing was measured. `pings_answered` is whether the address answers at all —
/// if it does not, the size probe is meaningless and is not read.
pub fn tunnel(place: &Place, largest_whole_packet: Option<u16>, pings_answered: bool) -> Tunnel {
    if !pings_answered {
        return Tunnel::CannotTell;
    }
    if let Some(mtu) = largest_whole_packet {
        if mtu < WHOLE_PATH_MTU {
            return Tunnel::Likely { mtu };
        }
    }
    if let Some(org) = &place.asn_org {
        let lower = org.to_lowercase();
        if MACHINE_ROOM_WORDS.iter().any(|w| lower.contains(w)) {
            return Tunnel::Possible {
                provider: org.clone(),
            };
        }
    }
    if largest_whole_packet.is_none() {
        return Tunnel::CannotTell;
    }
    Tunnel::NoSign
}
