//! T081 — тесты чистой логики заливки (US3).
//!
//! Проверяется то, на чём заливка ломается в жизни: продолжение после обрыва,
//! подменённый исходник, ограничитель скорости на длинном отрезке и оценка
//! времени после паузы.

use std::time::{Duration, Instant};
use vrcast_studio_lib::domain::progress_estimate::ProgressEstimate;
use vrcast_studio_lib::domain::rate_limit::RateLimiter;
use vrcast_studio_lib::domain::remote_name::{self, NameVerdict};
use vrcast_studio_lib::domain::transfer::{self, ResumeDecision, ResumeToken, WINDOW_BYTES};

// ---------- где продолжать (T077) ----------

#[test]
fn пустой_временный_файл_значит_начать_сначала() {
    assert_eq!(
        transfer::decide_resume(0, 1_000_000, WINDOW_BYTES),
        ResumeDecision::FromStart
    );
}

#[test]
fn продолжение_отступает_назад_на_одно_окно() {
    // Последняя запись могла оборваться на середине: её хвост в файле уже есть,
    // но целым не является. Переписать окно заново дешевле, чем гадать.
    let temp = 100 * 1024 * 1024;
    match transfer::decide_resume(temp, 500 * 1024 * 1024, WINDOW_BYTES) {
        ResumeDecision::Continue { offset } => {
            assert_eq!(offset, temp - WINDOW_BYTES);
        }
        other => panic!("получено {other:?}"),
    }
}

#[test]
fn отступ_не_уводит_за_начало_файла() {
    // Передано меньше окна: отступать некуда, начинаем сначала.
    assert_eq!(
        transfer::decide_resume(1024, 10_000_000, WINDOW_BYTES),
        ResumeDecision::FromStart
    );
}

#[test]
fn полностью_переданный_файл_не_передаётся_заново() {
    // Осталась сверка контрольных сумм и ввод в раздачу — но не передача.
    assert_eq!(
        transfer::decide_resume(1_000_000, 1_000_000, WINDOW_BYTES),
        ResumeDecision::AlreadyComplete
    );
}

#[test]
fn временный_файл_больше_исходного_это_не_почти_готово() {
    // Признак того, что источник подменили или на сервере лежит не тот файл.
    // Продолжить значило бы склеить два разных файла, и обнаружилось бы это
    // только сверкой — когда время уже потрачено.
    match transfer::decide_resume(2_000_000, 1_000_000, WINDOW_BYTES) {
        ResumeDecision::Mismatch { temp, total } => {
            assert_eq!(temp, 2_000_000);
            assert_eq!(total, 1_000_000);
        }
        other => panic!("расхождение принято за продолжение: {other:?}"),
    }
}

#[test]
fn позиция_возобновления_переживает_запись_и_чтение() {
    let token = ResumeToken {
        remote_temp: String::from("/var/lib/.vrcast-uploads/t1.film.part"),
        remote_name: String::from("film_22.mp4"),
        source_size: 32_000_000_000,
        source_modified: Some(String::from("2026-08-25T10:00:00Z")),
        uploaded_hint: 12_000_000_000,
    };
    let back = ResumeToken::parse(&token.to_json()).expect("позиция не прочиталась");
    assert_eq!(back, token);
}

#[test]
fn подменённый_исходник_замечается_до_передачи() {
    // Иначе продолжение допишет к началу одного файла хвост другого, и узнается
    // это только на сверке контрольных сумм — после часа передачи.
    let token = ResumeToken {
        remote_temp: String::from("/tmp/x.part"),
        remote_name: String::from("film.mp4"),
        source_size: 1_000,
        source_modified: Some(String::from("2026-08-25T10:00:00Z")),
        uploaded_hint: 500,
    };

    assert!(token.matches_source(1_000, Some("2026-08-25T10:00:00Z")));
    assert!(
        !token.matches_source(2_000, Some("2026-08-25T10:00:00Z")),
        "другой размер принят за тот же файл"
    );
    assert!(
        !token.matches_source(1_000, Some("2026-08-25T12:00:00Z")),
        "файл пересобрали в тот же объём, и это не замечено"
    );
    // Время неизвестно — довольствуемся размером, но не выдумываем расхождение.
    assert!(token.matches_source(1_000, None));
}

// ---------- ограничение скорости (T078) ----------

#[test]
fn без_предела_ждать_не_приходится() {
    let mut r = RateLimiter::new(None);
    let now = Instant::now();
    assert_eq!(r.delay_for(100_000_000, now), Duration::ZERO);
}

#[test]
fn нулевой_предел_считается_отсутствием_предела() {
    // Ноль как предел означал бы «не передавать никогда» — это не то, что человек
    // имеет в виду, оставляя поле пустым.
    let mut r = RateLimiter::new(Some(0));
    assert_eq!(r.limit_bps(), None);
    assert_eq!(r.delay_for(1_000_000, Instant::now()), Duration::ZERO);
}

#[test]
fn средняя_скорость_держится_в_пределе_на_длинном_отрезке() {
    // Главное свойство ограничителя. Проверяется по модельному времени: ждать
    // настоящие десять секунд ради проверки незачем.
    let limit = 1_000_000u64; // байт в секунду
    let mut r = RateLimiter::new(Some(limit));

    let start = Instant::now();
    let mut now = start;
    let chunk = 64 * 1024u64;
    let mut sent = 0u64;

    // Отправляем без пауз, продвигая время ровно на столько, сколько велит
    // ограничитель, — как поступал бы настоящий отправитель.
    for _ in 0..400 {
        let wait = r.delay_for(chunk, now);
        now += wait;
        sent += chunk;
    }

    let seconds = now.saturating_duration_since(start).as_secs_f64();
    let actual = sent as f64 / seconds.max(0.001);
    assert!(
        actual <= limit as f64 * 1.15,
        "средняя скорость {actual:.0} байт/с превысила предел {limit}"
    );
    assert!(
        actual > limit as f64 * 0.5,
        "ограничитель душит сильнее заданного: {actual:.0} вместо {limit}"
    );
}

#[test]
fn короткий_простой_не_превращается_в_потерю_скорости() {
    // Запас позволяет отправить накопленное сразу после короткой паузы —
    // иначе передача шла бы рывками строго по расписанию.
    let mut r = RateLimiter::new(Some(1_000_000));
    let now = Instant::now();
    r.delay_for(1_000_000, now);

    let later = now + Duration::from_millis(900);
    assert_eq!(
        r.delay_for(500_000, later),
        Duration::ZERO,
        "после паузы пришлось ждать, хотя разрешение накопилось"
    );
}

// ---------- скорость и оставшееся время (T079) ----------

#[test]
fn скорость_считается_по_последним_секундам() {
    let mut e = ProgressEstimate::new(Duration::from_secs(10));
    let start = Instant::now();

    for i in 0..=10u64 {
        e.record(start + Duration::from_secs(i), i * 1_000_000);
    }

    let speed = e.speed_bps().expect("скорость не посчиталась");
    assert!(
        (900_000..=1_100_000).contains(&speed),
        "скорость {speed} вместо примерно миллиона"
    );
}

#[test]
fn после_паузы_не_показывается_четыреста_часов() {
    // Ради этого правила модуль и существует. Накопленное до паузы больше не
    // описывает происходящее: если его не выбросить, оценка станет чудовищной,
    // и человек решит, что всё сломалось.
    let mut e = ProgressEstimate::new(Duration::from_secs(10));
    let start = Instant::now();

    for i in 0..=5u64 {
        e.record(start + Duration::from_secs(i), i * 1_000_000);
    }

    // Полчаса простоя.
    let after_pause = start + Duration::from_secs(1805);
    e.record(after_pause, 5_000_000);
    assert_eq!(
        e.speed_bps(),
        None,
        "сразу после паузы скорость выдумана из воздуха"
    );

    // Пошло заново — скорость считается по новым отсчётам, а не по всей истории.
    for i in 1..=5u64 {
        e.record(
            after_pause + Duration::from_secs(i),
            5_000_000 + i * 2_000_000,
        );
    }
    let speed = e
        .speed_bps()
        .expect("скорость не посчиталась после продолжения");
    assert!(
        (1_800_000..=2_200_000).contains(&speed),
        "скорость {speed} посчитана с учётом простоя"
    );
}

#[test]
fn оставшееся_время_не_выдумывается_при_неизвестной_скорости() {
    let e = ProgressEstimate::default();
    assert_eq!(e.eta(1_000_000), None);
}

#[test]
fn слишком_короткий_отрезок_не_даёт_поверить_в_гигабиты() {
    // Деление на тысячные доли секунды превращает любую дрожь в невероятное число.
    let mut e = ProgressEstimate::default();
    let start = Instant::now();
    e.record(start, 0);
    e.record(start + Duration::from_millis(3), 5_000_000);
    assert_eq!(e.speed_bps(), None);
}

// ---------- имена (T080) ----------

#[test]
fn файл_собирается_рядом_с_раздачей_а_не_внутри_неё() {
    // Веб-сервер отдаёт всё, что видит в каталоге раздачи. Недокачанный файл там
    // лежать не должен ни секунды.
    let staging = remote_name::staging_dir("/var/lib/vrcast/videos").expect("некуда собирать");
    assert_eq!(staging, "/var/lib/vrcast/.vrcast-uploads");
    assert!(
        !staging.starts_with("/var/lib/vrcast/videos"),
        "сборка идёт внутри каталога раздачи"
    );
}

#[test]
fn каталог_раздачи_в_корне_не_даёт_места_под_сборку() {
    // Класть недокачанное в саму раздачу нельзя, а рядом — некуда. Честный отказ
    // лучше молчаливого нарушения главного правила.
    assert_eq!(remote_name::staging_dir("/videos"), None);
}

#[test]
fn имя_временного_файла_зависит_только_от_конечного_имени() {
    // На этом держится вся схема возобновления: позиция — это размер временного
    // файла, и найти его нужно уметь до создания задачи (проверки перед стартом)
    // и после перезапуска приложения. Привязка к номеру задачи это сломала бы.
    let dir = "/var/lib/vrcast/.vrcast-uploads";
    let a = remote_name::staging_file(dir, "film.mp4");
    let b = remote_name::staging_file(dir, "film.mp4");
    assert_eq!(a, b, "одно и то же имя дало разные временные файлы");
    assert!(a.ends_with(".part"));

    // Разные конечные имена — разные временные файлы.
    assert_ne!(a, remote_name::staging_file(dir, "другое.mp4"));

    // Опасные знаки обезвреживаются и здесь: временный путь тоже уходит в команду.
    let dangerous = remote_name::staging_file(dir, "../../etc/passwd");
    assert!(
        dangerous.starts_with(dir),
        "временный файл ушёл за пределы каталога сборки: {dangerous}"
    );
}

#[test]
fn опасные_знаки_в_имени_обезвреживаются() {
    // Проверяются свойства, а не точный вид результата. Как именно выглядит
    // обезвреженное имя — дело вкуса и может меняться; важно, что из него нельзя
    // выйти в другой каталог, спрятать файл или разорвать команду на сервере.
    for опасное in [
        "../../etc/passwd",
        "film\nrm -rf /.mp4",
        "  .скрытый.mp4  ",
        "C:\\Windows\\system32",
        ".",
        "..",
    ] {
        let clean = remote_name::sanitize(опасное);

        assert!(
            !clean.contains('/') && !clean.contains('\\'),
            "разделитель пути уцелел в «{clean}» (из «{опасное}»)"
        );
        assert!(
            !clean.starts_with('.'),
            "имя осталось скрытым: «{clean}» (из «{опасное}»)"
        );
        assert!(
            !clean.contains('\n') && !clean.contains('\r') && !clean.contains('\0'),
            "перевод строки уцелел в «{clean}»"
        );
        assert_eq!(clean.trim(), clean, "по краям остались пробелы: «{clean}»");
    }

    // Обычное имя проходит нетронутым: обезвреживание не должно портить то,
    // что и так в порядке.
    assert_eq!(
        remote_name::sanitize("Backrooms_22.mp4"),
        "Backrooms_22.mp4"
    );
    assert_eq!(
        remote_name::sanitize("Фильм — финал.mp4"),
        "Фильм — финал.mp4"
    );
    // Две точки внутри имени — законны, и трогать их незачем.
    assert_eq!(remote_name::sanitize("film..final.mp4"), "film..final.mp4");
}

#[test]
fn занятое_имя_это_предупреждение_а_не_запрет() {
    // Замена законна, но у неё есть последствия, и человек должен знать о них
    // до, а не после жалоб зрителей (FR-039).
    let existing = vec![String::from("film.mp4")];

    assert_eq!(
        remote_name::check_name("film.mp4", &existing, true),
        NameVerdict::Exists { cdn_cached: true },
        "при заданном CDN не сказано про кеш"
    );
    assert_eq!(
        remote_name::check_name("film.mp4", &existing, false),
        NameVerdict::Exists { cdn_cached: false }
    );
    assert_eq!(
        remote_name::check_name("другое.mp4", &existing, false),
        NameVerdict::Ok
    );
}

#[test]
fn служебные_имена_раздачи_занимать_нельзя() {
    assert_eq!(
        remote_name::check_name("library.json", &[], false),
        NameVerdict::Reserved
    );
    assert_eq!(
        remote_name::check_name("   ", &[], false),
        NameVerdict::Empty
    );
}
