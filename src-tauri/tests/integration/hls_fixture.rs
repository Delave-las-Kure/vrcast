//! T151 — a ready quality set in the container, to check the serving and the substitution
//! against.
//!
//! **Why it is made on the spot and not committed.** T151 said to put the files under
//! `tests/fixtures/hls/`, and that would mean twenty megabytes of binary blobs in the
//! repository, growing with every rung anyone adds. The key is handled the same way
//! (`tests/support/test_key.rs`): made on the first run, never committed.
//!
//! **Why the declared numbers match the real ones.** `BANDWIDTH` here is the true peak of
//! the segments and `AVERAGE-BANDWIDTH` their true average — the segments are deliberately
//! unequal, so the two differ. A fixture where they matched, or where they were made up,
//! would let through exactly what FR-046 forbids: a description whose figures are not the
//! variants' figures.
//!
//! The bytes are not video. Nothing here decodes them: what is checked is that every
//! variant is served, that a viewer pulling it is seen, and that a limit hands out a
//! shortened description. For that a segment need only be a file of the right size.

use super::fixture::TestServer;

/// Where the serving lives, the same as on a real server.
pub const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

/// How long one segment lasts. Four seconds — the same as `package-hls.sh` cuts with.
pub const SEGMENT_SECONDS: u64 = 4;

/// A rung of the ready set.
pub struct Rung {
    /// The subdirectory, and what stands in the description beside it.
    pub name: &'static str,
    /// The sizes of the segments in bytes. Deliberately unequal — see the note above.
    pub segments: [u64; 3],
    pub resolution: &'static str,
    /// The actual level of the variant, not a constant. A fixed 5.2 on the lowest rung cuts
    /// it off from weak devices — that is, from exactly the people a ladder is made for.
    pub codecs: &'static str,
}

impl Rung {
    /// The peak: the heaviest segment. This is what goes into `BANDWIDTH`.
    pub fn peak_bps(&self) -> u64 {
        self.segments.iter().copied().max().unwrap_or(0) * 8 / SEGMENT_SECONDS
    }

    /// The average over the whole variant. This is what goes into `AVERAGE-BANDWIDTH`.
    pub fn average_bps(&self) -> u64 {
        let total: u64 = self.segments.iter().sum();
        total * 8 / (SEGMENT_SECONDS * self.segments.len() as u64)
    }
}

/// The set a check is run against: three rungs, roughly 1.8 times apart, as a real ladder
/// comes out (R-13).
pub const RUNGS: [Rung; 3] = [
    Rung {
        name: "v1",
        segments: [4_000_000, 5_000_000, 3_000_000],
        resolution: "1920x1080",
        codecs: "avc1.640029,mp4a.40.2",
    },
    Rung {
        name: "v2",
        segments: [2_000_000, 2_500_000, 1_500_000],
        resolution: "1280x720",
        codecs: "avc1.640029,mp4a.40.2",
    },
    Rung {
        name: "v3",
        segments: [800_000, 1_000_000, 600_000],
        resolution: "854x480",
        codecs: "avc1.640029,mp4a.40.2",
    },
];

/// Lay a quality set out in the container under the given short name.
///
/// Comes back with the path of the description, the one a viewer asks for.
pub fn lay_out_ladder(server: &TestServer, slug: &str) -> Result<String, String> {
    let mut master = String::from("#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-INDEPENDENT-SEGMENTS\n");

    for rung in &RUNGS {
        let dir = format!("{VIDEO_DIR}/{slug}/{}", rung.name);
        let mut media = format!(
            "#EXTM3U\n#EXT-X-VERSION:3\n#EXT-X-TARGETDURATION:{SEGMENT_SECONDS}\n\
             #EXT-X-MEDIA-SEQUENCE:0\n#EXT-X-PLAYLIST-TYPE:VOD\n"
        );

        let mut script = format!("mkdir -p '{dir}'");
        for (i, size) in rung.segments.iter().enumerate() {
            // The segments are made from /dev/urandom rather than from zeroes: gzip is on
            // in the serving, and a file of zeroes would be squeezed to nothing — the
            // measured speed of a viewer would have nothing to do with the declared
            // bitrate, and the SlowLink check would be measuring compression.
            script.push_str(&format!(
                " && head -c {size} /dev/urandom > '{dir}/seg{i}.ts'"
            ));
            media.push_str(&format!("#EXTINF:{SEGMENT_SECONDS}.000,\nseg{i}.ts\n"));
        }
        media.push_str("#EXT-X-ENDLIST\n");
        server.exec_inside(&script)?;
        write_file(server, &format!("{dir}/stream.m3u8"), &media)?;

        master.push_str(&format!(
            "#EXT-X-STREAM-INF:BANDWIDTH={},AVERAGE-BANDWIDTH={},RESOLUTION={},\
             FRAME-RATE=24.000,CODECS=\"{}\",CLOSED-CAPTIONS=NONE\n{}/stream.m3u8\n",
            rung.peak_bps(),
            rung.average_bps(),
            rung.resolution,
            rung.codecs,
            rung.name,
        ));
    }

    let master_path = format!("{VIDEO_DIR}/{slug}/master.m3u8");
    write_file(server, &master_path, &master)?;
    Ok(format!("/videos/{slug}/master.m3u8"))
}

/// Lay a single file out — serving without a ladder.
///
/// Both ways of serving are needed together, and that is the point of the check: a direct
/// file leaves no line in the access log until the watching ends, so a viewer watching one
/// is seen only through the connection table (R-02). A fixture with only a ladder in it
/// would let the very hole that check exists for go through.
pub fn lay_out_direct_file(
    server: &TestServer,
    name: &str,
    size_bytes: u64,
) -> Result<String, String> {
    server.exec_inside(&format!(
        "head -c {size_bytes} /dev/urandom > '{VIDEO_DIR}/{name}'"
    ))?;
    Ok(format!("/videos/{name}"))
}

/// Write a text file into the container without going through our own access layer.
///
/// Through the shell with a here-document: `docker cp` would need a local file, and the
/// application's own transfer is the thing under test elsewhere — setting conditions up
/// with it would mean checking that the code agrees with itself.
fn write_file(server: &TestServer, path: &str, contents: &str) -> Result<(), String> {
    // The marker is deliberately one that cannot occur in a playlist.
    server.exec_inside(&format!(
        "cat > '{path}' <<'VRCAST_FIXTURE_EOF'\n{contents}VRCAST_FIXTURE_EOF"
    ))?;
    Ok(())
}
