//! T233, T234 — the parts of the quality measurement that can be checked without a film.
//!
//! Continuous integration has neither a graphics card nor two hours of video, and the
//! arithmetic is what goes wrong rather than the process.

use vrcast_studio_lib::media::ffmpeg::filter_present;
use vrcast_studio_lib::media::vmaf::{ceiling_mbps, pooled_mean};

/// The shape FFmpeg really prints, legend and all.
const LISTING: &str = "Filters:
  T.. = Timeline support
  .S. = Slice threading
  V = Video input/output
  | = Source or sink filter
  ------
 TS aap               AA->A      Apply Affine Projection algorithm to first audio stream.
 .. libvmaf           VV->V      Calculate the VMAF between two video streams.
 .. vmafmotion        V->V       Calculate the VMAF Motion score.
";

#[test]
fn the_filter_listing_is_read_by_column_rather_than_by_word() {
    assert!(filter_present(LISTING, "libvmaf"));
    assert!(filter_present(LISTING, "vmafmotion"));

    // `libvmaf` also stands in the description column of nothing here, but `VMAF` stands in
    // two of them. A word search over the whole text would find a filter called `VMAF`,
    // declare quality measurable and fall over at the first point of the grid.
    assert!(!filter_present(LISTING, "VMAF"));
    assert!(!filter_present(LISTING, "Calculate"));
    // The legend is not a list of filters.
    assert!(!filter_present(LISTING, "="));
    assert!(!filter_present(LISTING, "Timeline"));
}

#[test]
fn the_ceiling_stays_above_the_target_where_the_tenth_disappears() {
    // `MR=$(( BR * 11 / 10 ))` in integer arithmetic. Below ten megabits the tenth is lost
    // entirely and the ceiling comes out equal to the target — which is constant bitrate,
    // and constant bitrate lost the comparison this project ran.
    assert_eq!(ceiling_mbps(1), 2);
    assert_eq!(ceiling_mbps(5), 6);
    assert_eq!(ceiling_mbps(9), 10);
    // From ten upwards the tenth survives on its own.
    assert_eq!(ceiling_mbps(10), 11);
    assert_eq!(ceiling_mbps(22), 24);
    assert_eq!(ceiling_mbps(35), 38);

    // Whatever the rung, the ceiling is above it: a rung that cannot exceed its own target
    // is not being measured under the conditions it will be served under.
    for bitrate in 1..=100u64 {
        assert!(
            ceiling_mbps(bitrate) > bitrate,
            "the ceiling at {bitrate} did not clear the target"
        );
    }
}

#[test]
fn the_score_is_the_pooled_mean_and_nothing_else() {
    let report = r#"{
        "frames": [{"frameNum": 0, "metrics": {"vmaf": 71.5}}],
        "pooled_metrics": {
            "vmaf": {"min": 71.5, "max": 99.2, "mean": 96.104, "harmonic_mean": 95.8}
        }
    }"#;
    assert_eq!(pooled_mean(report).unwrap(), 96.104);
}

#[test]
fn a_report_with_no_score_in_it_is_not_a_score_of_zero() {
    // A quality of zero would be taken for a rung that looks terrible, and the hull would
    // step around it as a measured fact. It is not a fact — it is a failed measurement, and
    // the difference decides whether a bitrate is dropped from the ladder or retried.
    assert!(pooled_mean("{}").is_err());
    assert!(pooled_mean(r#"{"pooled_metrics": {}}"#).is_err());
    assert!(pooled_mean(r#"{"pooled_metrics": {"psnr": {"mean": 41.0}}}"#).is_err());
    assert!(pooled_mean("not json at all").is_err());
}

// ---------- the container the measured chunk goes into (T476) ----------

/// What the encoder is asked for, without an encoder to ask.
fn args_for_a_chunk() -> Vec<String> {
    vrcast_studio_lib::media::vmaf::chunk_args(
        std::path::Path::new("film.mkv"),
        611,
        10,
        vrcast_studio_lib::domain::measure_grid::Cell {
            bitrate_mbps: 6,
            height: 1080,
        },
        &vrcast_studio_lib::media::encoders::Encoder::Hardware {
            name: String::from("h264_nvenc"),
        },
        (3840, 2160),
        &vrcast_studio_lib::media::vmaf::Recipe::for_material(24, None),
    )
}

#[test]
fn the_measured_chunk_is_not_muxed_into_mp4() {
    // ⚠ **This is not a preference about containers.** Measured 2026-09-03 on one chunk of
    // Blue Eye Samurai S01E01 at 6 Mbit/s: the same encoded bytes score 75.17 read back out
    // of the mp4 the muxer writes, and 98.63 after a stream copy into Matroska. Nothing about
    // the encode changed — only what the score was read from. Over three chunks the loss ran
    // 23.46, 7.94 and 6.49, so it cancels nowhere, and inside one chunk it barely moves with
    // bitrate (23.31, 23.59, 23.24 at 2, 4 and 12), which is why the curve went on looking
    // sensible while the whole ladder was chosen twelve points too low.
    let args = args_for_a_chunk();
    let after_f = args
        .iter()
        .position(|a| a == "-f")
        .map(|i| args[i + 1].clone());
    assert_eq!(
        after_f.as_deref(),
        Some("matroska"),
        "the chunk being measured is muxed into something other than Matroska; \
         if that is mp4 again, every score it produces is up to twenty-three VMAF low"
    );
    assert!(
        !args.iter().any(|a| a.ends_with(".mp4")),
        "the measured chunk is written to an .mp4 file: {args:?}"
    );
}

#[test]
fn the_chunk_is_asked_for_at_the_second_and_the_height_it_was_given() {
    // The cell and the position have to survive into the arguments, or the point measured is
    // not the point asked for — and nothing downstream would ever notice.
    let args = args_for_a_chunk();
    let joined = args.join(" ");
    assert!(joined.contains("-ss 611"), "{joined}");
    assert!(joined.contains("-t 10"), "{joined}");
    assert!(joined.contains("scale=-2:1080"), "{joined}");
    assert!(
        joined.contains("-b:v 6000k") || joined.contains("6M"),
        "{joined}"
    );
}

// ---------- how the two are compared, which decides the number (T490) ----------
//
// ⚠ **The same shape of accident as the container, one function along.** T476 found the
// measured chunk muxed into mp4: the encode was right, the reading of it was up to
// twenty-three VMAF low, the curve went on looking sensible, and the whole ladder was chosen
// twelve points too low for weeks. The arguments were lifted into `chunk_args` so that could
// never be silent again — and the comparison itself, which carries decisions of exactly that
// kind, stayed inside a private function where nothing could look at it.
//
// Four decisions live in the graph, and each changes the number without changing the shape of
// the answer: which input is the reference, whether the variant is stretched back to the
// source's resolution before it is judged (FR-143), what does the stretching, and whether the
// two are put on one clock.

/// The source given to the arguments below, and the size it is said to be.
const SOURCE: &str = "film.mkv";
const SOURCE_W: u32 = 3840;
const SOURCE_H: u32 = 2160;

fn args_for_a_score() -> Vec<String> {
    vrcast_studio_lib::media::vmaf::score_args(
        std::path::Path::new(SOURCE),
        611,
        10,
        SOURCE_W,
        SOURCE_H,
        &vrcast_studio_lib::media::vmaf::Recipe::for_material(24, None),
    )
}

/// The filter graph out of the arguments — where every decision of the comparison is.
fn graph_of(args: &[String]) -> String {
    let at = args
        .iter()
        .position(|a| a == "-lavfi")
        .expect("the comparison is asked for without a filter graph at all");
    args[at + 1].clone()
}

/// One branch of the graph, by the input it starts from.
fn branch(graph: &str, from: &str) -> String {
    graph
        .split(';')
        .find(|part| part.trim_start().starts_with(from))
        .unwrap_or_else(|| panic!("the graph has no branch starting at {from}:\n{graph}"))
        .to_owned()
}

#[test]
fn the_variant_is_stretched_back_to_the_source_before_it_is_judged() {
    // **FR-143, and it is not a nicety.** A 1080 variant judged against a 2160 reference is
    // not judged at all: libvmaf would either refuse the pair or bring them together by
    // whatever it does by default, and the number that came out would still look like a
    // score. Every rung of every ladder is chosen by these numbers.
    let graph = graph_of(&args_for_a_score());

    let distorted = branch(&graph, "[1:v]");
    assert!(
        distorted.contains(&format!("scale={SOURCE_W}:{SOURCE_H}")),
        "the variant is judged without being stretched back to {SOURCE_W}x{SOURCE_H} \
         (FR-143). The branch was:\n  {distorted}"
    );

    // And the reference is left alone: stretching *it* down to meet the variant would judge
    // the variant against a blurred copy of the film and call it a good score.
    let reference = branch(&graph, "[0:v]");
    assert!(
        !reference.contains("scale"),
        "the reference is being scaled, so the variant is judged against something other than \
         the film:\n  {reference}"
    );
}

#[test]
fn the_source_is_the_reference_and_the_variant_is_what_is_judged() {
    // Backwards, this produces a number rather than an error — a different number, from the
    // same films, that nothing about the output would mark as wrong.
    let args = args_for_a_score();
    let inputs: Vec<&String> = args
        .iter()
        .enumerate()
        .filter(|(i, a)| *a == "-i" && *i + 1 < args.len())
        .map(|(i, _)| &args[i + 1])
        .collect();
    assert_eq!(
        inputs.len(),
        2,
        "a comparison needs exactly two inputs, and there are {}",
        inputs.len()
    );
    assert_eq!(inputs[0], SOURCE, "the first input is not the film itself");
    assert_eq!(
        inputs[1], "point.mkv",
        "the second input is not the encoded chunk"
    );

    // libvmaf takes the distorted first and the reference second. `[d]` is the branch that
    // was stretched — the encode — and `[r]` the film.
    let graph = graph_of(&args);
    assert!(
        graph.contains("[d][r]libvmaf"),
        "the distorted and the reference are handed to libvmaf the wrong way round, or under \
         other names:\n{graph}"
    );
    assert!(
        branch(&graph, "[1:v]").ends_with("[d]"),
        "the branch fed from the encode is not the one called distorted"
    );
    assert!(
        branch(&graph, "[0:v]").ends_with("[r]"),
        "the branch fed from the film is not the one called the reference"
    );
}

#[test]
fn both_streams_are_put_on_one_clock() {
    // Without this the two inputs start at different timestamps and the filter compares the
    // first frame of one against the two-hundred-and-fortieth of the other. The score that
    // comes out is a real score of the wrong pair of frames.
    let graph = graph_of(&args_for_a_score());
    for (label, what) in [("[0:v]", "the film"), ("[1:v]", "the encode")] {
        assert!(
            branch(&graph, label).contains("setpts=PTS-STARTPTS"),
            "{what} is not brought to a common clock, so frames are compared against the \
             wrong frames:\n{graph}"
        );
    }
}

#[test]
fn the_scaler_is_named_rather_than_left_to_whatever_is_default() {
    // A default is not a constant: it is whatever this build of FFmpeg was made with, and it
    // can differ between the machine a ladder was measured on and the one it is rebuilt on.
    // Two scores of the same film that disagree because of that would be blamed on the film.
    let graph = graph_of(&args_for_a_score());
    assert!(
        graph.contains("flags=bicubic"),
        "the scaler that stretches the variant back is not named, so the score depends on \
         which FFmpeg is running:\n{graph}"
    );
}

#[test]
fn the_report_is_asked_for_in_the_shape_it_is_read_in() {
    // `pooled_mean` reads JSON. Asked for in any other shape, the read fails — which is at
    // least loud — but asked for at another path it fails as "no report", which reads like a
    // measurement that would not run.
    let graph = graph_of(&args_for_a_score());
    assert!(
        graph.contains("log_fmt=json"),
        "the report is not asked for as JSON:\n{graph}"
    );
    assert!(
        graph.contains("log_path=score.json"),
        "the report is written somewhere other than where it is read from:\n{graph}"
    );
}

// ---------- the production recipe (T697, QA-26 no. 5) ----------

fn value_after(args: &[String], flag: &str) -> Option<String> {
    args.iter()
        .position(|a| a == flag)
        .map(|i| args[i + 1].clone())
}

fn chunk_with(bitrate_mbps: u64, recipe: vrcast_studio_lib::media::vmaf::Recipe) -> Vec<String> {
    vrcast_studio_lib::media::vmaf::chunk_args(
        std::path::Path::new("film.mkv"),
        0,
        10,
        vrcast_studio_lib::domain::measure_grid::Cell {
            bitrate_mbps,
            height: 720,
        },
        &vrcast_studio_lib::media::encoders::Encoder::Software,
        (1920, 1080),
        &recipe,
    )
}

#[test]
fn a_point_is_encoded_with_the_keyframes_and_the_ceiling_a_rung_is_made_with() {
    use vrcast_studio_lib::domain::convert_plan::peak_control;
    use vrcast_studio_lib::media::vmaf::Recipe;
    // One keyframe a second at the material's own rate — not 48 whatever the rate.
    for fps in [24u32, 25, 30, 48, 60] {
        let args = chunk_with(6, Recipe::for_material(fps, None));
        assert_eq!(value_after(&args, "-g"), Some(fps.to_string()));
        assert_eq!(value_after(&args, "-keyint_min"), Some(fps.to_string()));
    }
    // The ceiling in kilobits, +10 %: at 1 Mbit/s that is 1100k, not the script's 2M.
    let args = chunk_with(1, Recipe::for_material(24, None));
    let (maxrate, bufsize) = peak_control(1000);
    assert_eq!(maxrate, 1100);
    let joined = args.join(" ");
    assert!(joined.contains(&format!("{maxrate}k")), "{joined}");
    assert!(joined.contains(&format!("{bufsize}k")), "{joined}");
    assert!(
        !joined.contains("2000k") && !joined.contains("2M"),
        "{joined}"
    );
    // The picture production writes: eight-bit 4:2:0 at the High profile.
    assert_eq!(value_after(&args, "-pix_fmt").as_deref(), Some("yuv420p"));
    assert_eq!(value_after(&args, "-profile:v").as_deref(), Some("high"));
    assert!(value_after(&args, "-level").is_some());
}

#[test]
fn an_hdr_point_is_brought_down_the_same_way_on_both_sides_of_the_comparison() {
    use vrcast_studio_lib::media::convert::tonemap_chain;
    use vrcast_studio_lib::media::vmaf::{score_args, Recipe};
    let hdr = Recipe::for_material(24, Some("smpte2084"));
    assert_eq!(hdr.tonemap, Some("smpte2084"));
    assert_eq!(
        Recipe::for_material(24, Some("arib-std-b67")).tonemap,
        Some("arib-std-b67")
    );
    assert!(Recipe::for_material(24, Some("bt709")).tonemap.is_none());
    assert!(Recipe::for_material(24, None).tonemap.is_none());
    let chain = tonemap_chain(Some("smpte2084"));

    // The encode: tonemapped first, then the height, then eight-bit.
    let filter = value_after(&chunk_with(6, hdr), "-vf").unwrap();
    assert!(filter.starts_with(&chain), "{filter}");
    assert!(filter.ends_with("scale=-2:720,format=yuv420p"), "{filter}");
    // An ordinary source is not tonemapped.
    let plain = value_after(&chunk_with(6, Recipe::for_material(24, None)), "-vf").unwrap();
    assert!(!plain.contains("tonemap"), "{plain}");

    // The comparison: the reference through the very same chain, never resized.
    let graph = graph_of(&score_args(
        std::path::Path::new(SOURCE),
        611,
        10,
        SOURCE_W,
        SOURCE_H,
        &hdr,
    ));
    let reference = branch(&graph, "[0:v]");
    assert!(reference.contains(&chain), "{reference}");
    assert!(reference.contains("format=yuv420p"), "{reference}");
    assert!(
        !reference.contains("scale=-2") && !reference.contains(&format!("scale={SOURCE_W}")),
        "the reference is resized: {reference}"
    );
    let distorted = branch(&graph, "[1:v]");
    assert!(distorted.contains("format=yuv420p"), "{distorted}");
    assert!(
        !distorted.contains("tonemap"),
        "tonemapped twice: {distorted}"
    );
}

/// T697 — the recipe runs, with the bundled FFmpeg, on an ordinary clip and on an HDR one
/// (ten-bit HEVC marked PQ): a point is measured and its score is a real one. Skipped where
/// there is no bundled FFmpeg.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_point_is_measured_by_the_production_recipe_on_ordinary_and_hdr_material() {
    use vrcast_studio_lib::media::{encoders::Encoder, ffmpeg, vmaf};
    let Ok(ff) = ffmpeg::locate("ffmpeg") else {
        eprintln!("SKIPPED: no bundled FFmpeg. Run `npm run ffmpeg` for this to check anything.");
        return;
    };
    if !vmaf::available().await.unwrap_or(false) {
        eprintln!("SKIPPED: the bundled FFmpeg has no libvmaf");
        return;
    }
    let dir = std::env::temp_dir().join(format!("vrcast-t697-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).unwrap();
    let sdr = dir.join("sdr.mkv");
    let hdr = dir.join("hdr.mkv");
    let make = |out: &std::path::Path, extra: &[&str]| {
        let mut args: Vec<String> = [
            "-hide_banner",
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=24",
            "-t",
            "4",
        ]
        .iter()
        .map(|s| (*s).to_owned())
        .collect();
        args.extend(extra.iter().map(|s| (*s).to_owned()));
        args.push(out.to_string_lossy().into_owned());
        std::process::Command::new(&ff)
            .args(&args)
            .status()
            .is_ok_and(|s| s.success())
    };
    assert!(make(&sdr, &["-c:v", "libx264", "-pix_fmt", "yuv420p"]));
    assert!(make(
        &hdr,
        &[
            "-vf",
            "format=yuv420p10le",
            "-c:v",
            "libx265",
            "-x265-params",
            // x265 writes the colour marks only from its own parameters.
            "log-level=error:colorprim=bt2020:transfer=smpte2084:colormatrix=bt2020nc",
            "-color_primaries",
            "bt2020",
            "-color_trc",
            "smpte2084",
            "-colorspace",
            "bt2020nc"
        ]
    ));
    // T715: marked PQ, with neither primaries nor a matrix — the tonemapper used to stop at
    // the first frame of it, on both sides of the comparison.
    let bare = dir.join("hdr-bare.mkv");
    assert!(make(
        &bare,
        &[
            "-pix_fmt",
            "yuv420p10le",
            "-c:v",
            "libx264",
            "-bsf:v",
            "h264_metadata=colour_primaries=2:transfer_characteristics=16:matrix_coefficients=2",
        ]
    ));
    let cell = vrcast_studio_lib::domain::measure_grid::Cell {
        bitrate_mbps: 1,
        height: 360,
    };
    for (clip, transfer) in [
        (&sdr, None),
        (&hdr, Some("smpte2084")),
        (&bare, Some("smpte2084")),
    ] {
        let recipe = vmaf::Recipe::for_material(24, transfer);
        let point = vmaf::measure_point(
            clip,
            640,
            360,
            &[0],
            3,
            cell,
            &Encoder::Software,
            &recipe,
            &tokio_util::sync::CancellationToken::new(),
        )
        .await
        .unwrap_or_else(|e| panic!("{transfer:?}: {e}"));
        assert!(point.whole(), "{transfer:?}: {point:?}");
        assert!(
            point.point.vmaf > 50.0 && point.point.vmaf <= 100.0,
            "{transfer:?}: {point:?}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
