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
