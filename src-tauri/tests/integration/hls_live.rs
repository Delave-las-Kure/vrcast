//! T196, T197, T190 — cutting a ladder on a real server and asking whether it is served.
//!
//! Everything here happens the way it will happen for a person: the variants are cut by
//! ffmpeg **on the server**, the description is built from what the cutting reports, and
//! the checking is done from this side over HTTP rather than from the server's own.
//!
//! The last check is the one FR-047 exists for: a variant is taken away deliberately, and
//! the result has to come back **not successful** with that variant named. A ladder whose
//! top rung plays and whose bottom one does not is worse than no ladder — the viewer it was
//! built for is exactly the one who gets nothing.

use std::time::Duration;

use vrcast_studio_lib::domain::hls_master::{self, Variant};
use vrcast_studio_lib::domain::hls_package::ToCut;
use vrcast_studio_lib::server::hls_package::Cutting;
use vrcast_studio_lib::server::hls_verify;
use vrcast_studio_lib::tasks::ladder_build;

use super::fixture::TestServer;
use super::hls_fixture::VIDEO_DIR;
use super::ssh_live::connect;

/// How long the little films are.
///
/// Twenty seconds gives five segments at four seconds each — enough for a playlist with a
/// beginning, a middle and an end, and for the tail stub the peak calculation has to ignore.
const FILM_SECONDS: u32 = 20;

/// Make a small but real film inside the container.
///
/// Real rather than random bytes: the cutting reads the stream's own parameters out of it —
/// its size, its frame rate, the level the encoder wrote — and random bytes have none.
fn make_film(
    server: &TestServer,
    name: &str,
    height: u32,
    bitrate_kbps: u32,
) -> Result<(), String> {
    server.exec_inside(&format!(
        "ffmpeg -nostdin -y -loglevel error \
         -f lavfi -i testsrc2=size={width}x{height}:rate=24:duration={FILM_SECONDS} \
         -f lavfi -i sine=frequency=440:duration={FILM_SECONDS} \
         -c:v libx264 -preset ultrafast -b:v {bitrate_kbps}k -g 48 -keyint_min 48 \
         -pix_fmt yuv420p -c:a aac -b:a 128k -shortest \
         '{VIDEO_DIR}/{name}' && echo made",
        width = height * 16 / 9,
    ))?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_ladder_is_cut_on_the_server_and_every_variant_of_it_is_served() {
    let server = TestServer::start().expect("the container would not come up");

    // Two variants of the same material, as a ladder really has: the same keyframe distance
    // in both, which is what lets a player change between them at a segment boundary.
    make_film(&server, "demo_6.mp4", 720, 6_000).expect("the upper variant would not be made");
    make_film(&server, "demo_3.mp4", 540, 3_000).expect("the lower variant would not be made");

    let conn = connect(&server).await;
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "demo",
        variants: &[
            ToCut {
                sub: String::from("v6"),
                file: String::from("demo_6.mp4"),
            },
            ToCut {
                sub: String::from("v3"),
                file: String::from("demo_3.mp4"),
            },
        ],
    };

    let mut announced = Vec::new();
    let facts = cutting
        .run(|progress| announced.push(progress.cut.len()))
        .await
        .expect("the cutting did not finish");

    assert_eq!(facts.len(), 2, "not every variant reported back");
    assert!(
        !announced.is_empty(),
        "the cutting finished without ever saying how it was getting on — a person would \
         watch a still bar for minutes"
    );

    // The description is built here, from what the cutting reported, by the same code that
    // is checked without a server. The bandwidths are the segments' own.
    let variants: Vec<Variant> = facts
        .iter()
        .map(|f| Variant {
            path: format!("{}/stream.m3u8", f.sub),
            bandwidth: hls_master::peak_bps(&f.segments),
            average_bandwidth: hls_master::average_bps(&f.segments),
            width: f.width,
            height: f.height,
            fps: f.frame_rate.parse().ok(),
            codecs: hls_master::codecs_for(&level_as_written(&f.level)),
        })
        .collect();

    for (facts, variant) in facts.iter().zip(&variants) {
        assert!(
            !facts.segments.is_empty(),
            "{} was cut into nothing at all",
            facts.sub
        );
        assert!(
            variant.bandwidth > 0 && variant.average_bandwidth > 0,
            "{} came back with no bandwidth: {variant:?}",
            facts.sub
        );
        assert!(
            variant.bandwidth >= variant.average_bandwidth,
            "{}: the peak is below the average, which cannot be",
            facts.sub
        );
    }

    let master = hls_master::build(&variants);
    let master_path = format!("{VIDEO_DIR}/demo/master.m3u8");
    let written = std::env::temp_dir().join("vrcast-test-master.m3u8");
    std::fs::write(&written, &master).expect("the description would not be written locally");
    server
        .put_file(&written, &master_path)
        .expect("the description would not be put on the server");
    let _ = std::fs::remove_file(&written);

    // And now the question a person actually has: is it served?
    let url = format!(
        "http://127.0.0.1:{}/videos/demo/master.m3u8",
        server.http_port
    );
    let verdict = hls_verify::verify(&url, 2)
        .await
        .expect("the serving could not be reached");

    assert!(
        verdict.ok(),
        "the ladder was cut but is not served whole: {verdict:?}"
    );
    assert_eq!(verdict.variants_in_master, 2);
    assert!(verdict.broken().is_empty());
    for variant in &verdict.variants {
        assert!(
            variant.segments >= 4,
            "{} has only {} segments for {FILM_SECONDS} seconds of film",
            variant.sub,
            variant.segments
        );
        assert!(variant.complete, "{} never says where it ends", variant.sub);
    }

    cutting.tidy_up().await.expect("the leftovers would not go");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_variant_taken_away_makes_the_result_incomplete_rather_than_successful() {
    // FR-047. This is the failure the rule exists for, and it has happened to this project:
    // a ladder went out with a half-empty variant because the check stopped at the first.
    let server = TestServer::start().expect("the container would not come up");
    make_film(&server, "gap_6.mp4", 720, 6_000).expect("the upper variant would not be made");
    make_film(&server, "gap_3.mp4", 540, 3_000).expect("the lower variant would not be made");

    let conn = connect(&server).await;
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "gap",
        variants: &[
            ToCut {
                sub: String::from("v6"),
                file: String::from("gap_6.mp4"),
            },
            ToCut {
                sub: String::from("v3"),
                file: String::from("gap_3.mp4"),
            },
        ],
    };
    let facts = cutting
        .run(|_| {})
        .await
        .expect("the cutting did not finish");

    let variants: Vec<Variant> = facts
        .iter()
        .map(|f| Variant {
            path: format!("{}/stream.m3u8", f.sub),
            bandwidth: hls_master::peak_bps(&f.segments),
            average_bandwidth: hls_master::average_bps(&f.segments),
            width: f.width,
            height: f.height,
            fps: f.frame_rate.parse().ok(),
            codecs: hls_master::codecs_for(&level_as_written(&f.level)),
        })
        .collect();
    let written = std::env::temp_dir().join("vrcast-test-master-gap.m3u8");
    std::fs::write(&written, hls_master::build(&variants)).expect("not written");
    server
        .put_file(&written, &format!("{VIDEO_DIR}/gap/master.m3u8"))
        .expect("the description would not be put on the server");
    let _ = std::fs::remove_file(&written);

    // One variant is taken away, exactly as a failed upload or a half-finished cutting
    // would leave it: the master still names it.
    server
        .exec_inside(&format!("rm -rf '{VIDEO_DIR}/gap/v3' && echo gone"))
        .expect("the variant would not be removed");

    let url = format!(
        "http://127.0.0.1:{}/videos/gap/master.m3u8",
        server.http_port
    );
    let verdict = hls_verify::verify(&url, 2)
        .await
        .expect("the serving could not be reached");

    assert!(
        !verdict.ok(),
        "a ladder with a variant missing was called successful: {verdict:?}"
    );
    assert_eq!(
        verdict.broken(),
        vec!["v3"],
        "the result does not say WHICH variant is missing, and a person cannot act on that"
    );
    // The master is still served and still names both: the failure is in the serving of one
    // variant, not in the description, and saying so is the difference between "rebuild the
    // lower rung" and "rebuild everything".
    assert!(verdict.master_served);
    assert_eq!(verdict.variants_in_master, 2);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_variant_already_cut_whole_is_not_cut_again() {
    // FR-048. Recognised **by what is on the server** rather than by a note kept here: a
    // note outlives the thing it describes, and a variant declared ready with half its
    // segments missing is worse than one rebuilt.
    let server = TestServer::start().expect("the container would not come up");
    make_film(&server, "again_6.mp4", 720, 6_000).expect("the film would not be made");

    let conn = connect(&server).await;
    let cutting = Cutting {
        conn: &conn,
        video_dir: VIDEO_DIR,
        owner: "root:root",
        base: "again",
        variants: &[ToCut {
            sub: String::from("v6"),
            file: String::from("again_6.mp4"),
        }],
    };
    cutting.run(|_| {}).await.expect("the first cutting failed");

    // A mark of our own inside the finished variant. If the second run cuts it again the
    // mark goes with it — which is exactly what "was it rebuilt?" means here.
    server
        .exec_inside(&format!(
            "touch '{VIDEO_DIR}/again/v6/.was-here' && echo marked"
        ))
        .expect("the mark would not be left");

    cutting
        .run(|_| {})
        .await
        .expect("the second cutting failed");

    let still_there = server
        .exec_inside(&format!(
            "test -f '{VIDEO_DIR}/again/v6/.was-here' && echo kept || echo rebuilt"
        ))
        .expect("the mark would not be looked for");
    assert!(
        still_there.contains("kept"),
        "a variant that was already cut whole was cut all over again"
    );

    cutting.tidy_up().await.expect("the leftovers would not go");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_variant_already_on_the_server_is_recognised_and_a_half_sent_one_is_not() {
    // FR-048, the half of it that is about the files rather than the segments. The point
    // is not "is there a file of that name" — an interrupted transfer leaves one of those —
    // but "is the whole film in it". A ladder that treats a truncated variant as done
    // serves ninety seconds and stops, and nothing ever looks at it again.
    let server = TestServer::start().expect("the container would not come up");
    make_film(&server, "resume_6.mp4", 720, 6_000).expect("the film would not be made");
    let conn = connect(&server).await;

    let whole = ladder_build::variant_already_there(
        &conn,
        VIDEO_DIR,
        "resume_6.mp4",
        f64::from(FILM_SECONDS),
    )
    .await
    .expect("the server would not answer");
    assert!(
        whole,
        "a variant that is all there was going to be made again"
    );

    // A file that is not there at all. The shell says nothing, and nothing must not be
    // read as a duration — this is the case that is worth a real server rather than
    // reasoning: `test -f ... || true` succeeds while printing nothing.
    let missing = ladder_build::variant_already_there(
        &conn,
        VIDEO_DIR,
        "never_made.mp4",
        f64::from(FILM_SECONDS),
    )
    .await
    .expect("the server would not answer");
    assert!(!missing, "a variant that does not exist was called ready");

    // And one cut short, as a broken transfer leaves it: the right name, half the film.
    server
        .exec_inside(&format!(
            "head -c $(( $(stat -c %s '{VIDEO_DIR}/resume_6.mp4') / 3 )) \
             '{VIDEO_DIR}/resume_6.mp4' > '{VIDEO_DIR}/short_6.mp4' && echo cut"
        ))
        .expect("the short file would not be made");
    let short = ladder_build::variant_already_there(
        &conn,
        VIDEO_DIR,
        "short_6.mp4",
        f64::from(FILM_SECONDS),
    )
    .await
    .expect("the server would not answer");
    assert!(
        !short,
        "a variant with a third of the film in it was called ready"
    );
}

/// ffprobe reports a level as a number — 30 is 3.0, 51 is 5.1 — and the description wants
/// it as the level it stands for.
fn level_as_written(level: &str) -> String {
    match level.trim().parse::<u32>() {
        Ok(n) if n >= 10 => format!("{}.{}", n / 10, n % 10),
        _ => String::from("5.2"),
    }
}

/// Give the cutting somewhere to breathe on a loaded machine.
#[allow(dead_code)]
const PATIENCE: Duration = Duration::from_secs(120);
