//! T161 — where a viewer's address is, looked up in a table held on this machine.
//!
//! On this machine and nowhere else: FR-057 forbids handing viewers' addresses to anyone
//! else, and asking an outside service would be doing exactly that, once per viewer, for
//! everyone watching. R-08 settled which table and why.
//!
//! Only the lookup is here. Where the table comes from and what shape it arrives in is
//! `server`'s and the build's business (T162) — kept apart on purpose, so that changing the
//! source of the data does not touch the rule, and so that the rule can be checked on a
//! table of four rows instead of a hundred megabytes.

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

/// One stretch of addresses with one answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Span {
    /// Inclusive at both ends.
    pub first: u128,
    pub last: u128,
    pub place: Place,
}

/// The table, sorted and searched by halving.
///
/// The whole table is millions of rows; walking it for every viewer on every refresh would
/// make the refresh itself the reason the interface stutters.
#[derive(Debug, Default)]
pub struct GeoTable {
    spans: Vec<Span>,
}

impl GeoTable {
    /// Build from rows in any order.
    ///
    /// Sorted here rather than trusted to arrive sorted: the search by halving is silent
    /// when its input is out of order — it does not fail, it merely answers wrongly for
    /// some addresses and rightly for others.
    pub fn new(mut spans: Vec<Span>) -> Self {
        spans.sort_by_key(|s| s.first);
        Self { spans }
    }

    pub fn len(&self) -> usize {
        self.spans.len()
    }

    pub fn is_empty(&self) -> bool {
        self.spans.is_empty()
    }

    /// Look an address up.
    ///
    /// `None` means not determined, and that is a real answer rather than a failure: it is
    /// shown to the person as itself.
    pub fn look_up(&self, ip: &str) -> Option<&Place> {
        let address: IpAddr = ip.parse().ok()?;
        if is_not_public(&address) {
            // A private or a loopback address is in nobody's table, and a table that
            // answers for one is answering about its own reserved rows rather than about
            // a place. Somebody watching from the next room is "not determined", not
            // whatever the table happens to hold there.
            return None;
        }
        let key = as_number(&address);

        // The last span that starts no later than the address. Only that one can contain
        // it, the table having no overlaps.
        let index = self
            .spans
            .partition_point(|s| s.first <= key)
            .checked_sub(1)?;
        let span = self.spans.get(index)?;
        (key <= span.last && !span.place.is_empty()).then_some(&span.place)
    }
}

/// Both kinds of address on one ruler.
///
/// IPv4 goes into the space IPv6 sets aside for it, so that one sorted table serves both.
/// Two tables would mean two searches, two places to get the sorting wrong, and two chances
/// for a viewer to fall between them.
pub fn as_number(address: &IpAddr) -> u128 {
    match address {
        IpAddr::V4(v4) => u32::from(*v4) as u128 | 0xffff_0000_0000,
        IpAddr::V6(v6) => u128::from(*v6),
    }
}

/// Whether the address is one no table can speak for.
fn is_not_public(address: &IpAddr) -> bool {
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
                // 2001:db8::/32 — the range set aside for writing examples in. Excluded for
                // the same reason as its IPv4 counterparts: whatever a table holds there is
                // about its own reserved rows, not about a place.
                || (v6.segments()[0] == 0x2001 && v6.segments()[1] == 0x0db8)
                // fc00::/7, addresses used inside one organisation.
                || (v6.segments()[0] & 0xfe00) == 0xfc00
                // fe80::/10, addresses that do not leave the local link.
                || (v6.segments()[0] & 0xffc0) == 0xfe80
                // A v4 address in v6 clothing is judged as the v4 address it is: otherwise
                // a viewer from the next room would be looked up and answered for.
                || v6.to_ipv4_mapped().is_some_and(|v4| is_not_public(&IpAddr::V4(v4)))
        }
    }
}
