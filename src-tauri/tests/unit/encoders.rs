//! T119 — choosing an encoder (FR-026).
//!
//! What is checked is less the choice itself than the requirement's third rule: **do not
//! keep quiet about falling back to the processor**. The difference in time is severalfold,
//! and a person not warned of it decides the application has frozen and kills the task
//! halfway.

use vrcast_studio_lib::domain::wording::DetailCode;
use vrcast_studio_lib::media::encoders::{self, Encoder};

fn listing(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_string()).collect()
}

#[test]
fn the_hardware_one_is_taken_when_it_is_there() {
    let chosen = encoders::choose(&listing(&["h264_nvenc", "h264_qsv"]), true, true).unwrap();
    assert_eq!(
        chosen.encoder,
        Encoder::Hardware {
            name: String::from("h264_nvenc")
        }
    );
    assert_eq!(
        chosen.notice, None,
        "the best available was taken, and the person was bothered all the same"
    );
}

#[test]
fn the_order_of_preference_is_honoured() {
    // NVIDIA is faster than the rest on our material, and when it is there it is taken,
    // whatever order the list arrives in.
    let chosen = encoders::choose(
        &listing(&["h264_vaapi", "h264_amf", "h264_nvenc"]),
        true,
        true,
    )
    .unwrap();
    assert_eq!(chosen.encoder.ffmpeg_name(), "h264_nvenc");

    // Without NVIDIA — the next in the list, not the first that turns up.
    let chosen = encoders::choose(&listing(&["h264_vaapi", "h264_qsv"]), true, true).unwrap();
    assert_eq!(chosen.encoder.ffmpeg_name(), "h264_qsv");
}

#[test]
fn without_hardware_we_fall_back_to_the_processor_and_say_so() {
    // This is why the rule exists: a silent fall-back looks like a freeze.
    let chosen = encoders::choose(&[], true, true).unwrap();
    assert_eq!(chosen.encoder, Encoder::Software);

    // The core names the case with a code; the wording itself — and what it says about the
    // time and about the quality — is checked on the catalogues' side
    // (`src/shared/i18n/__tests__/i18n.test.ts`), because there are two languages now.
    let said = chosen
        .notice
        .expect("the fall-back to the processor was passed over in silence");
    assert_eq!(said.key, DetailCode::NoticeNoHardwareFound);
}

#[test]
fn a_request_to_encode_on_the_processor_is_respected() {
    // Hardware is there, but the person asked for the processor — the processor it is.
    let chosen = encoders::choose(&listing(&["h264_nvenc"]), true, false).unwrap();
    assert_eq!(chosen.encoder, Encoder::Software);
    // A code of its own rather than the same one: a forced fall-back and a person's
    // deliberate choice are explained differently, and passing the second off as the first
    // tells a person something went wrong when they merely asked.
    let said = chosen
        .notice
        .expect("the slow path was passed over in silence");
    assert_eq!(said.key, DetailCode::NoticeSoftwareAsAsked);
}

#[test]
fn with_no_encoder_at_all_it_is_a_refusal_rather_than_a_silent_choice() {
    // A build without libx264 and without hardware has nothing to prepare with at all.
    // Choosing "something" here would mean falling over as soon as a preparation started.
    assert!(encoders::choose(&[], false, true).is_err());
    // And even when hardware is in the list but the person asks for the processor and
    // there is no software encoder.
    assert!(encoders::choose(&listing(&["h264_nvenc"]), false, false).is_err());
}

#[test]
fn other_names_in_the_list_are_not_taken_for_ours() {
    // A build holds hevc_nvenc, h264_mf and the rest. Taking the wrong encoder means
    // getting the wrong format, or a refusal at start-up.
    let chosen = encoders::choose(&listing(&["hevc_nvenc", "av1_nvenc"]), true, true).unwrap();
    assert_eq!(
        chosen.encoder,
        Encoder::Software,
        "an encoder for another format was taken for ours"
    );
}

#[test]
fn a_hardware_encoder_failing_in_practice_is_explained_in_human_words() {
    // Being in the build does not mean working: a graphics card may lack the right block,
    // a driver may be old, a laptop's card may be switched off. Falling back to the
    // processor is right, but keeping quiet about it is doubly wrong.
    let said = encoders::fallback_notice("h264_nvenc");
    assert_eq!(said.key, DetailCode::NoticeHardwareFailed);
    // The machine name travels as a value rather than being pasted into the text: NVIDIA is
    // the same in every language, and keeping a translation of `h264_nvenc` in each
    // catalogue separately would be a way to let them drift apart one day.
    assert_eq!(
        said.params.get("encoder").and_then(|v| v.as_str()),
        Some("h264_nvenc"),
        "it does not say which acceleration failed to work: {said:?}"
    );
}
