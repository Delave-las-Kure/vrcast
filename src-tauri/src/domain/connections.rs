//! T157 — the server's table of established connections: who is pulling right now.
//!
//! The second of the two sources the list of viewers is assembled from (R-02). The access
//! log says what is being watched; this says who is pulling at this moment and how it is
//! going. It is needed because a film served as a single file leaves no line in the log
//! until the watching ends — without this the list would be empty for the whole showing.
//!
//! Only the parsing is here. Asking the server lives in `server::connections`.

/// The ports the serving answers viewers on.
///
/// The same two as in `server::active_use`: on a real server it is 443, in the throwaway
/// container 80, and the poll must find viewers on either without being told which.
pub const SERVING_PORTS: [u16; 2] = [80, 443];

/// The command to ask the server with.
///
/// `-i` is what the whole thing rests on: without it come only addresses, and how a viewer
/// is doing is exactly what is wanted. `-n` keeps the addresses numeric — a name lookup on
/// every poll would be both slow and a leak of viewers' addresses to a resolver, which
/// FR-057 forbids.
/// The server's own clock comes back first, on a line of its own.
///
/// In the same command, deliberately. Every time a viewer is judged by has to come from the
/// server: the access log's times do, and mixing in this machine's for the polls would make
/// a viewer look as though they had started watching in the future or stopped an hour ago.
/// The specification names the two clocks disagreeing among its edge cases; asking in a
/// separate command would leave a gap between the two answers instead.
pub fn poll_command() -> String {
    let ports = SERVING_PORTS
        .iter()
        .map(|p| format!("sport = :{p}"))
        .collect::<Vec<_>>()
        .join(" or ");
    format!("date +%s.%N; ss -tin state established '( {ports} )' 2>/dev/null || true")
}

/// One reading of the connection table, with the moment it was taken.
#[derive(Debug, Clone, PartialEq)]
pub struct Poll {
    /// By the server's clock.
    pub at: time::OffsetDateTime,
    pub rows: Vec<ConnectionRow>,
}

/// Parse what [`poll_command`] printed.
///
/// Without a readable time nothing comes back at all. That is deliberate: the rows without
/// a time cannot be turned into a speed, and taking this machine's clock instead would
/// silently produce figures that look right and are not.
pub fn parse_poll(output: &str) -> Option<Poll> {
    let (first, rest) = output.split_once('\n')?;
    let seconds: f64 = first.trim().parse().ok()?;
    if !seconds.is_finite() {
        return None;
    }
    let at = time::OffsetDateTime::from_unix_timestamp_nanos((seconds * 1e9) as i128).ok()?;
    Some(Poll {
        at,
        rows: parse(rest),
    })
}

/// One established connection, as the server sees it.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectionRow {
    /// Whose it is. Normalised — see [`normalise_address`].
    pub peer_ip: String,
    pub peer_port: u16,
    /// How much of what we sent the far end has confirmed receiving.
    ///
    /// **This is what the delivered speed is worked out from**, by the difference between
    /// two polls — not from the `delivery_rate` below. Measured on 2026-08-26 against a
    /// viewer deliberately held to 200 kB/s: `delivery_rate` came back as 19 Gbit/s.
    /// That is not a fault in `ss`. It reports how fast the channel carries data when
    /// there is data to carry, and a viewer whose player is slow to read makes the flow
    /// application-limited — it goes in short bursts at full speed with long gaps between.
    /// Taking that figure for "the speed the viewer is getting" would mark the slowest
    /// viewer in the room as the fastest, and `SlowLink` (FR-053) would never fire for
    /// anyone.
    pub bytes_acked: u64,
    /// How many segments have been sent over the life of the connection.
    ///
    /// Only there to give the retransmissions a denominator: "eight hundred sent again" is
    /// a disaster on a thousand and nothing on a million.
    pub segs_out: u64,
    /// How many segments had to be sent again over the life of the connection.
    pub retrans_total: u64,
    /// What `ss` believes the channel can carry. Kept as a hint, not as the answer — see
    /// the note above.
    pub delivery_rate_bps: Option<u64>,
    /// What share of the busy time we spent unable to send because the far end's window
    /// was full — that is, because the viewer was not reading.
    ///
    /// A high share is the honest sign of a viewer who cannot keep up. It is not proof on
    /// its own: a player that has filled its buffer and paused looks the same. So it goes
    /// in as evidence and the judgement is made in `viewers`.
    pub receiver_limited_share: Option<f64>,
}

/// Parse what `ss -tin` printed.
///
/// A record is two lines: the first with the addresses, the second — indented — with the
/// details. A record whose second line is missing is not skipped: the connection exists,
/// and losing a viewer over the absence of the details would be worse than showing them
/// without the details.
pub fn parse(output: &str) -> Vec<ConnectionRow> {
    let mut rows = Vec::new();
    let mut lines = output.lines().peekable();

    while let Some(line) = lines.next() {
        if line.trim().is_empty() || line.starts_with("Recv-Q") {
            continue;
        }
        // A line starting with whitespace is a continuation. Reaching one here means its
        // header was skipped for a reason of its own; there is nothing to attach it to.
        if line.starts_with(char::is_whitespace) {
            continue;
        }

        // The peer is found by shape rather than by which column it is in. The columns move:
        // filtering by state drops the state column, `-p` adds a process one on the end, and
        // counting from the left would then read the local address as the viewer's — which
        // is a fault that looks like working code, since a local address is a perfectly
        // good address.
        //
        // The first two tokens that read as an address are the local one and the peer. The
        // process column holds colons too (`users:(("caddy",pid=1,fd=9))`) but no port that
        // will parse, so it does not get mistaken for one.
        let mut addresses = line.split_whitespace().filter_map(normalise_address);
        let Some(_local) = addresses.next() else {
            continue;
        };
        let Some((peer_ip, peer_port)) = addresses.next() else {
            continue;
        };

        let details = match lines.peek() {
            Some(next) if next.starts_with(char::is_whitespace) => lines.next().unwrap_or(""),
            _ => "",
        };

        rows.push(ConnectionRow {
            peer_ip,
            peer_port,
            bytes_acked: number(details, "bytes_acked").unwrap_or(0),
            segs_out: number(details, "segs_out").unwrap_or(0),
            // `retrans:0/8` — how many are outstanding now, and how many there have been
            // in all. The second is what matters: a viewer's link is judged by the whole
            // showing, not by the instant we happened to look.
            retrans_total: pair(details, "retrans")
                .map(|(_, total)| total)
                .unwrap_or(0),
            delivery_rate_bps: rate(details, "delivery_rate"),
            receiver_limited_share: share(details, "rwnd_limited"),
        });
    }
    rows
}

/// Bring an address into the form the access log writes it in.
///
/// **Without this the two sources never meet.** A server listening on `::` reports an
/// ordinary IPv4 viewer as `[::ffff:10.10.0.3]:52134`, while the log writes plain
/// `10.10.0.3`. Left as they are, no connection would ever match any request, every viewer
/// would come out as "watching something unknown", and nothing would look broken —
/// measured on 2026-08-26 against the throwaway container.
pub fn normalise_address(raw: &str) -> Option<(String, u16)> {
    let (host, port) = raw.rsplit_once(':')?;
    let port = port.parse().ok()?;
    let host = host.trim_start_matches('[').trim_end_matches(']');
    // The mapped form, in both spellings that occur.
    let host = host
        .strip_prefix("::ffff:")
        .or_else(|| host.strip_prefix("::FFFF:"))
        .unwrap_or(host);
    Some((host.to_owned(), port))
}

/// `ss` writes some of its fields as `key:value` and others as `key value`.
///
/// Not a matter of taste on its part, and not something to guess at: `bytes_acked:123` but
/// `delivery_rate 19402370368bps`. A parser that knew only the colon would quietly read no
/// speed at all — and quietly is the operative word, since the field would simply be absent
/// rather than wrong.
fn field<'a>(details: &'a str, key: &str) -> Option<&'a str> {
    let mut tokens = details.split_whitespace().peekable();
    while let Some(token) = tokens.next() {
        if let Some(value) = token.strip_prefix(key) {
            if let Some(value) = value.strip_prefix(':') {
                return Some(value);
            }
            // The key on its own, with the value in the next token.
            if value.is_empty() {
                return tokens.next();
            }
        }
    }
    None
}

fn number(details: &str, key: &str) -> Option<u64> {
    field(details, key)?.parse().ok()
}

/// `retrans:0/8` — the pair of "now" and "in all".
fn pair(details: &str, key: &str) -> Option<(u64, u64)> {
    let value = field(details, key)?;
    let (now, total) = value.split_once('/')?;
    Some((now.parse().ok()?, total.parse().ok()?))
}

/// `delivery_rate 19402370368bps` — a number with a unit stuck to it.
fn rate(details: &str, key: &str) -> Option<u64> {
    field(details, key)?.trim_end_matches("bps").parse().ok()
}

/// `rwnd_limited:4070ms(98.5%)` — a duration and the share of the busy time it makes up.
///
/// The share is taken rather than the duration: the duration means nothing without knowing
/// how long the connection has been alive, and `ss` has already done that division.
fn share(details: &str, key: &str) -> Option<f64> {
    let value = field(details, key)?;
    let open = value.find('(')?;
    let close = value.find('%')?;
    if close <= open + 1 {
        return None;
    }
    value[open + 1..close]
        .parse::<f64>()
        .ok()
        .map(|p| p / 100.0)
}
