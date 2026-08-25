//! T111 — тесты чистой логики подготовки (US4).
//!
//! Проверяются те правила, каждое из которых куплено ошибкой в этом же проекте:
//! уровень совместимости по двум пределам, потолок битрейта в килобитах, три условия
//! переноса звука. Контрольные примеры взяты из записей проекта, а не выдуманы —
//! выдуманный пример проверяет формулу против себя самой.

use vrcast_studio_lib::domain::convert_plan::{
    self as plan, AudioAction, ConvertRequest, PlanProblem, VideoAction,
};
use vrcast_studio_lib::domain::source::{AudioTrack, SourceFile};
use vrcast_studio_lib::domain::wording::DetailCode;

fn дорожка(codec: &str, channels: u16) -> AudioTrack {
    AudioTrack {
        index: 0,
        codec: String::from(codec),
        channels,
        bitrate_bps: None,
        language: Some(String::from("rus")),
        title: None,
        is_default: true,
    }
}

/// Заведомо совместимый исходник: H.264 yuv420p, звук AAC стерео.
fn совместимый() -> SourceFile {
    SourceFile {
        path: String::from("F:/видео/фильм.mp4"),
        size_bytes: 8_000_000_000,
        duration_s: 7200.0,
        width: 1920,
        height: 1080,
        fps: 24,
        bitrate_bps: 9_000_000,
        peak_bps: None,
        video_codec: String::from("h264"),
        pix_fmt: String::from("yuv420p"),
        color_transfer: Some(String::from("bt709")),
        audio_tracks: vec![дорожка("aac", 2)],
    }
}

fn как_есть() -> ConvertRequest {
    ConvertRequest {
        audio_track: 0,
        target_kbps: None,
        height: None,
    }
}

// ---------- перенос против пересжатия (T109, FR-022) ----------

#[test]
fn совместимый_файл_не_пересжимается() {
    // Главное правило фазы: часы работы против минут и потеря поколения против нуля.
    let p = plan::plan(&совместимый(), &как_есть()).expect("план не составился");
    assert_eq!(p.video, VideoAction::Copy);
    assert_eq!(p.audio, AudioAction::Copy);
    assert!(p.lossless());
}

#[test]
fn hevc_пересжимается_несмотря_на_то_что_уже_сжат() {
    // Записанный случай 2026-07-30: HEVC экономит битрейт, но в Windows нет системного
    // декодера, и четверо зрителей из восьми не смогли смотреть. Копировать такой
    // поток дешевле — и именно поэтому соблазн есть.
    let mut src = совместимый();
    src.video_codec = String::from("hevc");

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    match p.video {
        VideoAction::Reencode { reason, .. } => {
            // Причина — код с подстановкой, а не фраза: формулировку подберёт
            // интерфейс, и она существует на обоих языках. Кодек назван значением,
            // потому что «видео в hevc» без самого hevc ничего не объясняет.
            assert_eq!(reason.key, DetailCode::ReasonVideoNotH264);
            assert_eq!(
                reason.params.get("codec").and_then(|v| v.as_str()),
                Some("hevc"),
                "причина не называет кодек исходника: {reason:?}"
            );
        }
        иное => panic!("HEVC перенесён без пересжатия: {иное:?}"),
    }
}

#[test]
fn десятибитный_h264_пересжимается() {
    // Формально тот же кодек, и проверка «кодек == h264» пропустила бы его.
    // Строгий декодер такое не берёт, и узнается это уже у зрителя.
    let mut src = совместимый();
    src.pix_fmt = String::from("yuv420p10le");

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    assert!(
        matches!(p.video, VideoAction::Reencode { .. }),
        "десятибитный поток перенесён как есть: {:?}",
        p.video
    );
}

#[test]
fn расширенный_диапазон_требует_приведения_и_потому_пересжатия() {
    let mut src = совместимый();
    src.color_transfer = Some(String::from("smpte2084"));

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    assert!(p.tonemap, "расширенный диапазон не опознан");
    assert!(matches!(p.video, VideoAction::Reencode { .. }));
}

#[test]
fn смена_размера_кадра_исключает_перенос() {
    // Скопировать поток и одновременно изменить картинку нельзя: любое изменение
    // требует раскодировать, а раскодировав, «как было» обратно не сложить.
    let src = совместимый();
    let req = ConvertRequest {
        height: Some(720),
        ..как_есть()
    };

    let p = plan::plan(&src, &req).expect("план не составился");
    assert!(matches!(p.video, VideoAction::Reencode { .. }));
}

// ---------- звук (FR-021, FR-024) ----------

#[test]
fn многоканальный_aac_не_переносится_вопреки_кодеку() {
    // Записанная ошибка: проверка одного лишь кодека пропускала шестиканальную
    // дорожку, и на входе AAC 5.1 файл уезжал шестиканальным при целевом стерео.
    let mut src = совместимый();
    src.audio_tracks = vec![дорожка("aac", 6)];

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    match p.audio {
        AudioAction::Reencode { reason, .. } => {
            assert_eq!(reason.key, DetailCode::ReasonAudioChannels);
            assert_eq!(
                reason.params.get("channels").and_then(|v| v.as_u64()),
                Some(6),
                "причина не называет, сколько каналов: {reason:?}"
            );
        }
        иное => panic!("шестиканальная дорожка перенесена как есть: {иное:?}"),
    }
}

#[test]
fn при_пересжатии_звука_всегда_выравнивается_сдвиг() {
    // FR-024. AAC пишет вступительные отсчёты через список правок, а плеер VRChat
    // его не читает — звук уезжает. Без этого поля план был бы неполным, и сдвиг
    // обнаружился бы на просмотре.
    let mut src = совместимый();
    src.audio_tracks = vec![дорожка("eac3", 6)];

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    match p.audio {
        AudioAction::Reencode { resample_fix, .. } => {
            assert!(
                resample_fix,
                "выравнивание звука не включено при пересжатии"
            );
        }
        иное => panic!("несовместимый звук перенесён как есть: {иное:?}"),
    }
}

#[test]
fn чуть_более_толстая_дорожка_переносится_в_пределах_допуска() {
    // Настоящий AAC стабильно чуть толще номинала: «256k» весит больше 256 000.
    // Без допуска дорожка уходила бы на пересжатие, теряя поколение впустую.
    let mut src = совместимый();
    let mut t = дорожка("aac", 2);
    t.bitrate_bps = Some(263_000);
    src.audio_tracks = vec![t];

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    assert_eq!(p.audio, AudioAction::Copy);
}

#[test]
fn заметно_более_толстая_дорожка_пересжимается() {
    let mut src = совместимый();
    let mut t = дорожка("aac", 2);
    t.bitrate_bps = Some(640_000);
    src.audio_tracks = vec![t];

    let p = plan::plan(&src, &как_есть()).expect("план не составился");
    assert!(matches!(p.audio, AudioAction::Reencode { .. }));
}

#[test]
fn файл_без_звука_отвергается_отдельным_замечанием() {
    let mut src = совместимый();
    src.audio_tracks.clear();

    let problems = plan::plan(&src, &как_есть()).expect_err("файл без звука принят");
    assert!(problems.contains(&PlanProblem::NoAudioTracks));
    assert_eq!(problems[0].detail().key, DetailCode::PlanNoAudioTracks);
}

#[test]
fn несуществующая_дорожка_называется_по_человечески() {
    let src = совместимый();
    let req = ConvertRequest {
        audio_track: 5,
        ..как_есть()
    };

    let problems = plan::plan(&src, &req).expect_err("несуществующая дорожка принята");
    // Человеку номера показываются с единицы: «дорожки 0 нет» читается как ошибка.
    // Перевод делает ядро, один раз, — иначе о нём пришлось бы помнить каждому
    // словарю по отдельности.
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanNoSuchTrack);
    assert_eq!(
        detail.params.get("number").and_then(|v| v.as_u64()),
        Some(6),
        "номер дорожки не тот, что видит человек: {detail:?}"
    );
}

// ---------- уровень совместимости (FR-029) ----------

#[test]
fn уровень_считается_по_двум_пределам_а_не_по_размеру_кадра() {
    // Один и тот же кадр при разной частоте требует разных уровней. Проверка одного
    // лишь размера объявила бы 4.1 в обоих случаях, а строгий декодер вправе такой
    // файл не принять — это класс поломки, записанный в проекте.
    //
    // 1920×1080 — это 8160 макроблоков, в предел кадра 4.1 (8192) укладывается.
    // Но 8160 × 48 = 391 680 в секунду при пределе 4.1 в 245 760.
    assert_eq!(
        plan::h264_level(1920, 1080, 24),
        "4.1",
        "на 24 кадрах уровень завышен"
    );
    assert_eq!(
        plan::h264_level(1920, 1080, 48),
        "4.2",
        "частота кадров не учтена: уровень занижен, и строгий плеер вправе отказать"
    );
}

#[test]
fn неполный_макроблок_считается_целым() {
    // Макроблок — 16×16, и неполный тоже занимает место: 1922 пикселя это 121 столбец,
    // а не 120. Округление вниз занижает счёт и вместе с ним уровень.
    //
    // 1920×1072 — ровно 8040 макроблоков, укладывается в 4.1 при 24 кадрах.
    assert_eq!(plan::h264_level(1920, 1072, 24), "4.1");
    // Плюс два пикселя по каждой стороне — и это уже 121×68 = 8228, за пределом 4.1.
    // При округлении вниз вышло бы всё те же 8040, и уровень остался бы прежним.
    assert_eq!(
        plan::h264_level(1922, 1074, 24),
        "4.2",
        "неполный макроблок не посчитан: уровень занижен"
    );
}

#[test]
fn крупный_кадр_получает_высокий_уровень() {
    // 3840×2160@48: 32 400 макроблоков и 1 555 200 в секунду — за пределом 5.1.
    assert_eq!(plan::h264_level(3840, 2160, 48), "5.2");
    // Он же вдвое медленнее укладывается в 5.1 (777 600 при пределе 983 040).
    assert_eq!(plan::h264_level(3840, 2160, 24), "5.1");
}

// ---------- ограничение пиков (T110, FR-025) ----------

#[test]
fn потолок_считается_в_килобитах_а_не_в_мегабитах() {
    // Записанная ошибка: в мегабитах целочисленное 8*11/10 даёт ровно 8 — потолок
    // совпадает с целью, буфера нет, и выходит постоянный битрейт, который в замерах
    // проиграл. На прежних +30 % это не вылезало, а на +10 % сломалось молча.
    let (maxrate, _) = plan::peak_control(8_000);
    assert_eq!(maxrate, 8_800, "потолок посчитан не в килобитах");
    assert!(maxrate > 8_000, "потолок совпал с целью — буфера нет вовсе");
}

#[test]
fn буфер_равен_потолку() {
    // Большой буфер разрешает всплеск выше потолка: было «потолок 45 / буфер 60»
    // и пики 54 Мбит/с, на которых зрители замирали.
    for цель in [4_000u32, 8_000, 22_000, 35_000] {
        let (maxrate, bufsize) = plan::peak_control(цель);
        assert_eq!(
            bufsize, maxrate,
            "буфер разошёлся с потолком на цели {цель}"
        );
    }
}

#[test]
fn потолок_никогда_не_совпадает_с_целью() {
    // Даже на крошечных значениях, где округление съедает надбавку.
    for цель in 1..=40u32 {
        let (maxrate, _) = plan::peak_control(цель);
        assert!(
            maxrate > цель,
            "на цели {цель} потолок вышел {maxrate} — это постоянный битрейт"
        );
    }
}

#[test]
fn под_заданный_пик_потолок_ставится_ниже_него() {
    // Настоящий пик выходит на 5–6 % выше потолка: канал зрителя рассчитан на пик,
    // а не на среднее, и ставить потолок равным пику значит превысить его.
    let потолок = plan::maxrate_for_peak(38_000);
    assert!(потолок < 38_000, "потолок не ниже требуемого пика");
    // И обратно: заданный так потолок даёт пик около нужного.
    let ожидаемый_пик = потолок * 106 / 100;
    assert!(
        (37_000..=38_100).contains(&ожидаемый_пик),
        "получился пик {ожидаемый_пик} вместо примерно 38 000"
    );
}

#[test]
fn заданный_битрейт_заставляет_пересжать_даже_совместимый_поток() {
    // Иначе требование осталось бы невыполненным, а человек думал бы, что учтено.
    let src = совместимый();
    let req = ConvertRequest {
        target_kbps: Some(6_000),
        ..как_есть()
    };

    let p = plan::plan(&src, &req).expect("план не составился");
    match p.video {
        VideoAction::ReencodeCapped {
            target_kbps,
            maxrate_kbps,
            bufsize_kbps,
            ..
        } => {
            assert_eq!(target_kbps, 6_000);
            assert_eq!(maxrate_kbps, 6_600);
            assert_eq!(bufsize_kbps, 6_600);
        }
        иное => panic!("заданный битрейт не учтён: {иное:?}"),
    }
}

#[test]
fn битрейт_заметно_выше_источника_отвергается_с_объяснением() {
    // FR-029: заведомо бессмысленное сочетание не проходит молча.
    let src = совместимый(); // 9 Мбит/с
    let req = ConvertRequest {
        target_kbps: Some(40_000),
        ..как_есть()
    };

    let problems = plan::plan(&src, &req).expect_err("битрейт выше источника принят");
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanBitrateAboveSource);
    // Оба числа названы: без них замечание не объясняет, насколько именно просьба
    // выше источника, и спорить с ним нечем.
    assert_eq!(
        detail.params.get("asked_kbps").and_then(|v| v.as_u64()),
        Some(40_000)
    );
    assert_eq!(
        detail.params.get("source_kbps").and_then(|v| v.as_u64()),
        Some(9_000)
    );
}

#[test]
fn растягивание_кадра_отвергается_с_объяснением() {
    let src = совместимый(); // 1080 строк
    let req = ConvertRequest {
        height: Some(2160),
        ..как_есть()
    };

    let problems = plan::plan(&src, &req).expect_err("растягивание принято");
    let detail = problems[0].detail();
    assert_eq!(detail.key, DetailCode::PlanHeightAboveSource);
    assert_eq!(
        detail.params.get("asked").and_then(|v| v.as_u64()),
        Some(2160)
    );
    assert_eq!(
        detail.params.get("source").and_then(|v| v.as_u64()),
        Some(1080)
    );
}

#[test]
fn замечания_возвращаются_все_сразу() {
    // Их бывает несколько, и разбираться по одному за круг — работа, которой
    // можно не быть.
    let mut src = совместимый();
    src.audio_tracks.clear();
    let req = ConvertRequest {
        audio_track: 3,
        target_kbps: Some(0),
        height: Some(0),
    };

    let problems = plan::plan(&src, &req).expect_err("план составился на негодной заявке");
    assert!(
        problems.len() >= 3,
        "вернулось только {} замечаний из трёх: {problems:?}",
        problems.len()
    );
}

// ---------- опорные кадры ----------

#[test]
fn опорный_кадр_раз_в_секунду_при_любой_частоте() {
    // Константа здесь была бы ошибкой: 48 писалось под 48-кадровое видео и означало
    // «раз в секунду», а на 24-кадровом давало раз в две.
    for fps in [24u32, 25, 30, 48, 60] {
        let mut src = совместимый();
        src.fps = fps;
        let p = plan::plan(&src, &как_есть()).unwrap();
        assert_eq!(p.gop, fps, "на {fps} кадрах опорный кадр не раз в секунду");
    }
}

// ---------- показ дорожек (FR-020, граничный случай) ----------

#[test]
fn дорожка_без_языка_показывается_номером() {
    // Показывать пустоту нельзя: выбрать между двумя пустыми строками невозможно,
    // а дорожек бывает шесть.
    let t = AudioTrack {
        index: 2,
        codec: String::from("aac"),
        channels: 2,
        bitrate_bps: None,
        language: None,
        title: None,
        is_default: false,
    };
    assert_eq!(t.label(), "Дорожка 3, стерео");
}

#[test]
fn язык_и_название_показываются_вместе() {
    let t = AudioTrack {
        index: 0,
        codec: String::from("ac3"),
        channels: 6,
        bitrate_bps: None,
        language: Some(String::from("rus")),
        title: Some(String::from("Дубляж")),
        is_default: true,
    };
    assert_eq!(t.label(), "rus — Дубляж, 6 каналов");
}

#[test]
fn дорожка_по_умолчанию_это_помеченная_а_не_первая() {
    let mut src = совместимый();
    let mut первая = дорожка("aac", 2);
    первая.index = 0;
    первая.is_default = false;
    let mut вторая = дорожка("aac", 2);
    вторая.index = 1;
    вторая.is_default = true;
    src.audio_tracks = vec![первая, вторая];

    assert_eq!(src.default_track().map(|t| t.index), Some(1));
}
