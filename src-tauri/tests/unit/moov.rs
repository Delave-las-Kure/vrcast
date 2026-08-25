//! T034a — тесты разбора заголовка MP4 (R-19, FR-012).
//!
//! Контрольные файлы настоящие: собраны ffmpeg и лежат в `tests/fixtures/mp4`.
//! Это принципиально — разбирать самодельные заготовки значило бы проверять
//! согласие кода с собственными представлениями о формате, а не с тем, что
//! действительно приходит с сервера. Заготовки тоже есть, но только для случаев,
//! которые ffmpeg не выдаёт: полей восьмибайтовой длины и намеренно испорченных данных.

use vrcast_studio_lib::domain::moov::{self, MoovOutcome};

fn fixture(name: &str) -> Vec<u8> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/mp4")
        .join(name);
    std::fs::read(&path).unwrap_or_else(|e| panic!("не прочитать {}: {e}", path.display()))
}

/// Найти границы атома верхнего уровня — чтобы обрезать файл ровно там, где нужно.
fn top_level_box(data: &[u8], want: &[u8; 4]) -> Option<(usize, usize)> {
    let mut off = 0usize;
    while off + 8 <= data.len() {
        let size = u32::from_be_bytes([data[off], data[off + 1], data[off + 2], data[off + 3]]);
        let typ = &data[off + 4..off + 8];
        let len = match size {
            0 => data.len() - off,
            1 => u64::from_be_bytes(data[off + 8..off + 16].try_into().ok()?) as usize,
            n => n as usize,
        };
        if typ == want {
            return Some((off, off + len));
        }
        if len < 8 {
            return None;
        }
        off += len;
    }
    None
}

// ---------- настоящие файлы ----------

#[test]
fn файл_подготовленный_к_раздаче_разбирается_целиком() {
    let data = fixture("faststart.mp4");
    let outcome = moov::parse(&data, Some(data.len() as u64));

    let params = match &outcome {
        MoovOutcome::Parsed(p) => p,
        other => panic!("заголовок не разобран: {other:?}"),
    };

    assert_eq!(params.width, Some(128));
    assert_eq!(params.height, Some(96));
    assert_eq!(params.video_codec.as_deref(), Some("h264"));
    assert_eq!(params.audio_codec.as_deref(), Some("aac"));

    let duration = params.duration_s.expect("длительность не прочитана");
    assert!(
        (duration - 1.0).abs() < 0.05,
        "длительность {duration} вместо ~1 с"
    );

    // Средний битрейт — объём, делённый на длительность. Сверяемся со значением,
    // которое для этого же файла называет ffprobe: 112144.
    let bitrate = params.bitrate_bps.expect("битрейт не посчитан");
    assert!(
        (bitrate as i64 - 112_144).abs() < 2_000,
        "битрейт {bitrate} расходится с тем, что считает ffprobe"
    );

    assert_eq!(outcome.faststart_ok(), Some(true));
}

#[test]
fn файл_с_заголовком_в_конце_опознаётся_как_неподходящий() {
    // FR-012: параметры остаются неизвестными, но пользователь узнаёт главное —
    // такой файл зритель начнёт смотреть только после скачивания хвоста.
    let data = fixture("moov_at_end.mp4");
    let outcome = moov::parse(&data, Some(data.len() as u64));

    assert_eq!(
        outcome,
        MoovOutcome::MoovAfterData,
        "файл без подготовки принят за подготовленный"
    );
    assert_eq!(outcome.faststart_ok(), Some(false));
    assert!(
        outcome.params().is_none(),
        "выданы параметры, которых неоткуда взять"
    );
}

#[test]
fn решение_о_неподходящем_файле_принимается_по_началу_а_не_по_всему_файлу() {
    // Важное свойство: `mdat` бывает на гигабайты, и дочитывать его ради заголовка
    // бессмысленно. Хватать должно первых килобайт.
    let data = fixture("moov_at_end.mp4");
    let (mdat_start, _) = top_level_box(&data, b"mdat").expect("в файле нет mdat");
    let head = &data[..mdat_start + 64];

    assert_eq!(
        moov::parse(head, Some(data.len() as u64)),
        MoovOutcome::MoovAfterData
    );
}

#[test]
fn обрезанный_заголовок_говорит_сколько_байт_не_хватило() {
    // Это не украшение сообщения: по этому числу слой чтения дозапрашивает ровно
    // нужный кусок, а не удваивает объём вслепую.
    let data = fixture("faststart.mp4");
    let (moov_start, moov_end) = top_level_box(&data, b"moov").expect("в файле нет moov");
    let cut = moov_start + (moov_end - moov_start) / 2;

    match moov::parse(&data[..cut], Some(data.len() as u64)) {
        MoovOutcome::NeedMoreBytes { need } => assert_eq!(
            need, moov_end as u64,
            "запрошено не столько байт, сколько занимает заголовок"
        ),
        other => panic!("обрезанный заголовок разобран как {other:?}"),
    }
}

#[test]
fn без_размера_файла_разбирается_всё_кроме_битрейта() {
    // Размер бывает неизвестен: перечень каталога мог прийти без него.
    // Это не повод отказываться от разрешения и кодеков.
    let data = fixture("faststart.mp4");
    let params = match moov::parse(&data, None) {
        MoovOutcome::Parsed(p) => p,
        other => panic!("заголовок не разобран: {other:?}"),
    };

    assert_eq!(params.width, Some(128));
    assert!(params.duration_s.is_some());
    assert_eq!(
        params.bitrate_bps, None,
        "битрейт посчитан из ниоткуда: без размера файла его не вычислить"
    );
}

// ---------- заготовки для случаев, которых ffmpeg не выдаёт ----------

fn mp4_box(typ: &[u8; 4], payload: &[u8]) -> Vec<u8> {
    let mut v = Vec::with_capacity(payload.len() + 8);
    v.extend_from_slice(&((payload.len() + 8) as u32).to_be_bytes());
    v.extend_from_slice(typ);
    v.extend_from_slice(payload);
    v
}

#[test]
fn длительность_читается_и_из_полей_восьмибайтовой_длины() {
    // Вторая версия заголовка фильма: времена и длительность по восемь байт.
    // Встречается у длинных записей, и перепутанные смещения дали бы не ошибку,
    // а правдоподобное неверное число — худший исход.
    let mut mvhd = vec![1u8, 0, 0, 0]; // версия 1, признаки
    mvhd.extend_from_slice(&[0u8; 8]); // время создания
    mvhd.extend_from_slice(&[0u8; 8]); // время изменения
    mvhd.extend_from_slice(&1000u32.to_be_bytes()); // делений в секунде
    mvhd.extend_from_slice(&90_000u64.to_be_bytes()); // длительность

    let mut file = mp4_box(b"ftyp", b"isom\0\0\x02\0isomiso2");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1_000_000)) {
        MoovOutcome::Parsed(p) => {
            let d = p.duration_s.expect("длительность не прочитана");
            assert!((d - 90.0).abs() < 0.001, "длительность {d} вместо 90 с");
            assert_eq!(p.bitrate_bps, Some(88_889), "битрейт посчитан неверно");
        }
        other => panic!("заготовка не разобрана: {other:?}"),
    }
}

#[test]
fn неизвестная_длительность_не_превращается_в_ноль() {
    // Признак «неизвестно» в заголовке — все единицы. Принять его за число значит
    // показать пользователю длительность в 49 суток и битрейт в единицы бит.
    let mut mvhd = vec![0u8, 0, 0, 0];
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&1000u32.to_be_bytes());
    mvhd.extend_from_slice(&u32::MAX.to_be_bytes());

    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1_000_000)) {
        MoovOutcome::Parsed(p) => {
            assert_eq!(p.duration_s, None, "признак «неизвестно» принят за число");
            assert_eq!(p.bitrate_bps, None);
        }
        other => panic!("заготовка не разобрана: {other:?}"),
    }
}

#[test]
fn нулевое_число_делений_не_роняет_разбор_делением_на_ноль() {
    let mut mvhd = vec![0u8, 0, 0, 0];
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&[0u8; 4]);
    mvhd.extend_from_slice(&0u32.to_be_bytes()); // делений в секунде: ноль
    mvhd.extend_from_slice(&1000u32.to_be_bytes());

    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"moov", &mp4_box(b"mvhd", &mvhd)));

    match moov::parse(&file, Some(1000)) {
        MoovOutcome::Parsed(p) => assert_eq!(p.duration_s, None),
        other => panic!("заготовка не разобрана: {other:?}"),
    }
}

// ---------- испорченные данные ----------

#[test]
fn мусор_не_принимается_за_видео() {
    assert_eq!(moov::parse(b"", None), MoovOutcome::NotMp4);
    assert_eq!(moov::parse(b"\x00\x00", None), MoovOutcome::NotMp4);
    assert_eq!(
        moov::parse(b"<!DOCTYPE html><html><body>404</body></html>", None),
        MoovOutcome::NotMp4,
        "страница с ошибкой сервера принята за видео"
    );
}

#[test]
fn атом_нулевой_длины_не_зацикливает_разбор() {
    // Разбор идёт по данным с сервера, и файл может быть собран как угодно —
    // в том числе так, чтобы обход не сдвинулся ни на байт.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&[0, 0, 0, 4]); // длина меньше самого заголовка
    file.extend_from_slice(b"junk");
    file.extend_from_slice(&[0u8; 64]);

    // Проверяется именно завершение: если разбор зациклится, тест не кончится.
    let outcome = moov::parse(&file, Some(file.len() as u64));
    assert_eq!(outcome, MoovOutcome::NotMp4);
}

#[test]
fn заголовок_обещающий_больше_чем_есть_не_выводит_за_пределы_куска() {
    // Атом объявляет длину в гигабайт, а данных — сотня байт. Обращение по
    // объявленной длине вышло бы за границы среза.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&1_000_000_000u32.to_be_bytes());
    file.extend_from_slice(b"moov");
    file.extend_from_slice(&[0u8; 32]);

    match moov::parse(&file, Some(2_000_000_000)) {
        MoovOutcome::NeedMoreBytes { need } => {
            assert!(need > file.len() as u64, "запрошено меньше, чем уже есть");
        }
        other => panic!("ожидался запрос данных, получено {other:?}"),
    }
}

#[test]
fn у_файла_без_заголовка_вовсе_не_просятся_байты_за_его_концом() {
    // Иначе получается вечный круг: разбор просит данные за концом файла, читающий
    // слой отдаёт тот же кусок, разбор просит снова. Файл целиком прочитан —
    // значит, заголовка в нём нет, и это окончательный ответ.
    let mut file = mp4_box(b"ftyp", b"isom");
    file.extend_from_slice(&mp4_box(b"free", &[0u8; 16]));
    let size = file.len() as u64;

    assert_eq!(
        moov::parse(&file, Some(size)),
        MoovOutcome::NotMp4,
        "запрошены байты за концом файла — читающий слой зациклится"
    );

    // А когда размер файла неизвестен, просить ещё — законно: вдруг там есть.
    assert!(matches!(
        moov::parse(&file, None),
        MoovOutcome::NeedMoreBytes { .. }
    ));
}

#[test]
fn запрошенного_куска_всегда_хватает_чтобы_продвинуться() {
    // Свойство, на которое опирается дочитывание: сколько бы раз слой чтения ни
    // выполнил просьбу, разбор обязан дойти до ответа, а не просить снова и снова
    // одно и то же.
    let data = fixture("faststart.mp4");
    let size = data.len() as u64;

    let mut have = 8usize;
    let mut steps = 0;
    loop {
        steps += 1;
        assert!(steps < 10, "разбор не сошёлся за десять дочитываний");

        let head = &data[..have.min(data.len())];
        match moov::parse(head, Some(size)) {
            MoovOutcome::NeedMoreBytes { need } => {
                assert!(
                    need > have as u64,
                    "запрошено {need} байт, а прочитано уже {have} — продвижения нет"
                );
                have = need as usize;
            }
            MoovOutcome::Parsed(_) => break,
            other => panic!("неожиданный итог: {other:?}"),
        }
    }
}

#[test]
fn разбор_не_падает_ни_на_каком_обрезании_настоящего_файла() {
    // Свойство важнее любого отдельного случая: кусок может прийти оборванным
    // в произвольном месте — на границе поля, посреди имени атома, где угодно.
    // Ни одно из таких обрезаний не должно ронять приложение.
    let data = fixture("faststart.mp4");
    for cut in (0..data.len()).step_by(7) {
        let _ = moov::parse(&data[..cut], Some(data.len() as u64));
    }
    for cut in (0..data.len()).step_by(7) {
        let _ = moov::parse(&data[..cut], None);
    }
}
