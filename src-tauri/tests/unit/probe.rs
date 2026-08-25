//! T116 — чтение ответа `ffprobe`.
//!
//! Проверяется на **настоящем** ответе, снятом с файла, который тут же и собран
//! вложенным FFmpeg (`tests/fixtures/ffprobe-sample.json`), и на выдуманных
//! случаях для тех тонкостей, которых в образце нет. Выдумать весь ответ целиком
//! нельзя: половина ловушек в нём — это то, чего не ожидаешь, пока не увидишь.

use vrcast_studio_lib::media::probe::{self, ProbeError};

fn образец() -> String {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/ffprobe-sample.json");
    std::fs::read_to_string(path).expect("образец ответа ffprobe не прочитать")
}

#[test]
fn настоящий_ответ_читается_целиком() {
    let src = probe::parse(&образец(), "F:/видео/проба.mp4").expect("ответ не разобрался");

    assert_eq!(src.width, 640);
    assert_eq!(src.height, 360);
    assert_eq!(src.fps, 24);
    assert_eq!(src.video_codec, "h264");
    assert_eq!(src.pix_fmt, "yuv420p");
    assert!(!src.is_hdr());

    assert_eq!(src.audio_tracks.len(), 1);
    let t = &src.audio_tracks[0];
    assert_eq!(t.codec, "aac");
    assert_eq!(t.channels, 2);
    assert_eq!(t.language.as_deref(), Some("rus"));
    assert!(t.is_default);
}

#[test]
fn числа_приходят_строками_и_всё_равно_читаются() {
    // `ffprobe` печатает размер, длительность и битрейт строками. Попытка прочитать
    // их числами дала бы отказ разбора на первом же файле.
    let src = probe::parse(&образец(), "проба.mp4").unwrap();
    assert!(src.size_bytes > 0, "размер не прочитан");
    assert!(
        src.duration_s > 1.9 && src.duration_s < 2.1,
        "длительность {}",
        src.duration_s
    );
    assert!(src.bitrate_bps > 0, "битрейт не прочитан");
    assert_eq!(
        src.audio_tracks[0].bitrate_bps,
        Some(128_018),
        "битрейт дорожки не прочитан — а он нужен, чтобы решить, переносить её или нет"
    );
}

#[test]
fn частота_кадров_округляется_вверх() {
    // 24000/1001 — это 23.976, то есть 24-кадровый материал. Округление вниз дало бы 23
    // и занизило уровень совместимости, а занижённый строгий декодер вправе не принять.
    let json = ответ_с_частотой("24000/1001");
    assert_eq!(probe::parse(&json, "x").unwrap().fps, 24);

    let json = ответ_с_частотой("48000/1001");
    assert_eq!(probe::parse(&json, "x").unwrap().fps, 48);
}

#[test]
fn отсутствующая_частота_кадров_не_роняет_разбор() {
    // Ноль кадров в секунду не бывает; догадка завышает уровень, а завышенный
    // безопасен всегда.
    let json = ответ_с_частотой("0/0");
    let fps = probe::parse(&json, "x").unwrap().fps;
    assert!(fps > 0, "получено {fps} кадров в секунду");
}

#[test]
fn язык_und_считается_отсутствующим() {
    // `und` — это «не указано», а не название языка. Показать его человеку значит
    // предложить выбирать между «und» и «und».
    let json = ответ_с_дорожками(
        r#"
        {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
         "tags":{"language":"und"},"disposition":{"default":1}},
        {"index":2,"codec_type":"audio","codec_name":"ac3","channels":6,
         "tags":{"language":"eng","title":"Original"},"disposition":{"default":0}}
    "#,
    );
    let src = probe::parse(&json, "x").unwrap();

    assert_eq!(src.audio_tracks[0].language, None, "«und» принято за язык");
    assert_eq!(src.audio_tracks[0].label(), "Дорожка 1, стерео");
    assert_eq!(src.audio_tracks[1].language.as_deref(), Some("eng"));
}

#[test]
fn дорожки_нумеруются_среди_звуковых_а_не_среди_всех() {
    // Номер уходит в `-map 0:a:<N>`. Взять общий номер потока значит промахнуться
    // дорожкой на любом файле, где звук идёт не первым, — а он почти всегда не первый.
    let json = ответ_с_дорожками(
        r#"
        {"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
         "disposition":{"default":1}},
        {"index":2,"codec_type":"audio","codec_name":"ac3","channels":6,
         "disposition":{"default":0}},
        {"index":3,"codec_type":"subtitle","codec_name":"subrip"}
    "#,
    );
    let src = probe::parse(&json, "x").unwrap();

    assert_eq!(src.audio_tracks.len(), 2, "субтитры приняты за звук");
    assert_eq!(src.audio_tracks[0].index, 0);
    assert_eq!(src.audio_tracks[1].index, 1);
}

#[test]
fn расширенный_диапазон_опознаётся_по_передаче_цвета() {
    let json = образец().replace(
        r#""pix_fmt": "yuv420p","#,
        r#""pix_fmt": "yuv420p10le","color_transfer": "smpte2084","#,
    );
    let src = probe::parse(&json, "x").unwrap();
    assert!(src.is_hdr(), "HDR не опознан");
}

#[test]
fn файл_без_видео_это_отдельная_беда() {
    // Звуковой файл вместо видео — обычная человеческая ошибка, и назвать её надо
    // прямо, а не отказом разбора.
    let json = r#"{"streams":[{"index":0,"codec_type":"audio","codec_name":"aac","channels":2}],
                   "format":{"duration":"10.0","size":"100","bit_rate":"80"}}"#;
    assert!(matches!(
        probe::parse(json, "x").expect_err("файл без видео принят"),
        ProbeError::NoVideo
    ));
}

#[test]
fn мусор_вместо_ответа_не_роняет_приложение() {
    let err = probe::parse("это не ответ разборщика", "x").expect_err("мусор разобрался");
    assert!(matches!(err, ProbeError::Unreadable(_)));
}

// ---------- вспомогательное ----------

fn ответ_с_частотой(rate: &str) -> String {
    format!(
        r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"h264",
             "width":1920,"height":1080,"pix_fmt":"yuv420p","r_frame_rate":"{rate}",
             "avg_frame_rate":"{rate}","bit_rate":"9000000"}},
            {{"index":1,"codec_type":"audio","codec_name":"aac","channels":2,
              "disposition":{{"default":1}}}}],
           "format":{{"duration":"100.0","size":"1000","bit_rate":"9000000"}}}}"#
    )
}

fn ответ_с_дорожками(tracks: &str) -> String {
    format!(
        r#"{{"streams":[{{"index":0,"codec_type":"video","codec_name":"h264",
             "width":1920,"height":1080,"pix_fmt":"yuv420p","r_frame_rate":"24/1",
             "bit_rate":"9000000"}},{tracks}],
           "format":{{"duration":"100.0","size":"1000","bit_rate":"9000000"}}}}"#
    )
}
