//! T115 — проверка вложенного FFmpeg.
//!
//! Разбор ответов проверяется на записанных строках и идёт всегда. Сам вложенный
//! файл весит сто сорок мегабайт, в репозиторий не попадает и в непрерывной
//! интеграции отсутствует — проверки, которым он нужен, честно сообщают, что
//! пропущены, а не делают вид, что прошли.

use vrcast_studio_lib::media::ffmpeg::{self, FfmpegError};

/// Настоящий ответ вложенной сборки, сокращённый до сути.
const ОТВЕТ_О_ВЕРСИИ: &str = "\
ffmpeg version n8.1.2-44-g7c533d0f86-20260825 Copyright (c) 2000-2026 the FFmpeg developers
built with gcc 15.2.0 (crosstool-NG 1.28.0.23_185f348)
configuration: --enable-gpl --enable-version3 --enable-libx264 --enable-ffnvcodec
";

/// Кусок настоящего перечня кодировщиков: тот же вид, что печатает программа.
const ПЕРЕЧЕНЬ: &str = "\
Encoders:
 V..... = Video
 ------
 V....D libx264              libx264 H.264 / AVC / MPEG-4 AVC
 V....D libx265              libx265 H.265 / HEVC
 V....D h264_nvenc           NVIDIA NVENC H.264 encoder (codec h264)
 V....D hevc_nvenc           NVIDIA NVENC hevc encoder (codec hevc)
 A....D aac                  AAC (Advanced Audio Coding)
";

#[test]
fn версия_читается_из_ответа() {
    let v = ffmpeg::parse_version(ОТВЕТ_О_ВЕРСИИ).expect("версия не прочиталась");
    assert!(v.contains("n8.1.2"), "получено: {v}");
}

#[test]
fn чужая_программа_под_именем_ffmpeg_не_принимается() {
    // Под этим именем в системе может оказаться что угодно — от обёртки пакетного
    // менеджера до сообщения «программа не установлена». Принять такое за FFmpeg
    // значит узнать правду в середине подготовки.
    let ответ =
        "Команда «ffmpeg» не найдена, но может быть установлена командой:\nsudo apt install ffmpeg";
    let err = ffmpeg::parse_version(ответ).expect_err("чужой ответ принят за версию");
    assert!(matches!(err, FfmpegError::Unexpected(_)));
}

#[test]
fn пустой_ответ_это_не_версия() {
    let err = ffmpeg::parse_version("   \n\n  ").expect_err("пустота принята за версию");
    assert!(matches!(err, FfmpegError::Unexpected(_)));
}

#[test]
fn кодировщик_ищется_целым_словом() {
    assert!(ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "libx264"));
    assert!(ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "h264_nvenc"));
    assert!(ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "aac"));
}

#[test]
fn чужое_имя_внутри_нашего_не_считается_совпадением() {
    // Ради этого поиск идёт по словам, а не подстрокой. В перечне есть `hevc_nvenc`,
    // и подстрочный поиск объявил бы наличие `nvenc` вообще — а на самом деле
    // важно, есть ли именно кодировщик H.264. Ошибка тихая: приложение решило бы,
    // что аппаратное ускорение доступно, и упало бы уже на запуске подготовки.
    assert!(
        !ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "nvenc"),
        "кусок чужого имени принят за кодировщик"
    );
    assert!(
        !ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "x264"),
        "«x264» найдено внутри «libx264»"
    );
    assert!(!ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "h264_qsv"));
    assert!(!ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "h264_amf"));
    assert!(!ffmpeg::encoder_present(ПЕРЕЧЕНЬ, "h264_vaapi"));
}

#[test]
fn отсутствие_вложенного_файла_называется_отдельной_бедой() {
    // Не «не запускается» и не «отвечает не то»: чинить это надо иначе, и путь,
    // по которому искали, человеку нужен.
    let err = ffmpeg::locate("такой-программы-нет").expect_err("найдено несуществующее");
    match err {
        FfmpegError::NotFound(искали) => {
            assert!(!искали.is_empty(), "не сказано, где искали");
        }
        иное => panic!("отсутствие файла названо иначе: {иное}"),
    }
}

#[tokio::test]
async fn вложенная_сборка_умеет_то_ради_чего_вложена() {
    // Требует самого файла. Он весит сто сорок мегабайт, в репозиторий не попадает
    // и качается командой `npm run ffmpeg`; в непрерывной интеграции его нет.
    // Молча пройти в его отсутствие проверка не может — тогда она ничего не значит,
    // — поэтому пропуск объявляется вслух.
    let Ok(path) = ffmpeg::locate("ffmpeg") else {
        eprintln!(
            "ПРОПУЩЕНО: вложенного FFmpeg нет. Выполните `npm run ffmpeg`, \
             чтобы эта проверка что-то проверяла."
        );
        return;
    };
    eprintln!("проверяем вложенную сборку: {}", path.display());

    let info = ffmpeg::probe_self()
        .await
        .expect("вложенная сборка не отвечает");

    assert!(
        info.version.contains("ffmpeg version"),
        "версия неожиданного вида: {}",
        info.version
    );
    assert!(
        info.has_x264,
        "во вложенной сборке нет программного кодировщика H.264 — \
         на машине без подходящей видеокарты готовить файлы будет нечем"
    );
}
