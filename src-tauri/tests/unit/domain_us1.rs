//! T034 — тесты чистой логики первой пользовательской истории.
//!
//! Проверяется не «функция что-то вернула», а правила, на которые опирается всё
//! остальное: профиль без домена нельзя сохранить, короткое имя попадает в имя файла,
//! поколение описи защищает от второго экземпляра приложения, ссылка не ломается
//! на необычном имени файла, и ни один файл не теряется при группировке.

use vrcast_studio_lib::domain::grouping::{self, GroupReason};
use vrcast_studio_lib::domain::links;
use vrcast_studio_lib::domain::manifest::{Manifest, ManifestProblem};
use vrcast_studio_lib::domain::media::{self, Media, MediaFile, SlugError};
use vrcast_studio_lib::domain::server_profile::{
    AuthKind, ServerProfile, DEFAULT_SSH_PORT, DEFAULT_VIDEO_DIR,
};
use vrcast_studio_lib::domain::wording::DetailCode;

// ---------- профиль сервера (T029) ----------

fn valid_profile() -> ServerProfile {
    let mut p = ServerProfile::new("srv_1", "Мой сервер");
    p.host = String::from("203.0.113.10");
    p.user = String::from("root");
    p.auth_kind = AuthKind::Key;
    p.key_path = Some(String::from("/home/user/.ssh/id_ed25519"));
    p.secret_ref = String::from("vrcast/srv_1/passphrase");
    p.domain = String::from("stream.example.com");
    p
}

#[test]
fn правильный_профиль_проходит_проверку() {
    let p = valid_profile();
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    assert_eq!(p.port, DEFAULT_SSH_PORT);
    assert_eq!(p.video_dir, DEFAULT_VIDEO_DIR);
}

#[test]
fn профиль_без_домена_не_сохранить() {
    // Домен обязателен не для красоты: без него не выдать зрительскую ссылку
    // и не проверить, что раздача вообще работает (FR-125).
    let mut p = valid_profile();
    p.domain = String::new();

    let problems = p.validate().expect_err("профиль без домена принят");
    assert!(
        problems.iter().any(|x| x.field == "domain"),
        "не указано поле домена: {problems:?}"
    );
    // Замечание называет случай кодом: формулировку подбирает интерфейс, и она
    // существует на обоих языках (FR-105, FR-106).
    assert_eq!(problems[0].detail.key, DetailCode::DomainEmpty);
}

#[test]
fn проверка_называет_все_ошибки_сразу_а_не_первую() {
    // В мастере настройки человек заполняет форму целиком. Показывать ошибки
    // по одной — значит гонять его по кругу из-за каждой опечатки.
    let mut p = valid_profile();
    p.name = String::new();
    p.host = String::new();
    p.domain = String::new();
    p.user = String::new();

    let problems = p.validate().expect_err("пустой профиль принят");
    let fields: Vec<&str> = problems.iter().map(|x| x.field).collect();
    for expected in ["name", "host", "domain", "user"] {
        assert!(
            fields.contains(&expected),
            "поле {expected} не названо: {fields:?}"
        );
    }
}

#[test]
fn домен_вставленный_из_адресной_строки_приводится_к_виду() {
    // Люди вставляют домен вместе с «https://» и косой чертой. Отвергать за это
    // значит придираться: намерение однозначно.
    let mut p = valid_profile();
    p.domain = String::from("  HTTPS://Stream.Example.COM/  ");
    p.normalize();

    assert_eq!(p.domain, "stream.example.com");
    assert!(p.validate().is_ok());
}

#[test]
fn домен_с_путём_отвергается() {
    // А вот путь уже не приведёшь: непонятно, что человек имел в виду.
    let mut p = valid_profile();
    p.domain = String::from("https://stream.example.com/videos");
    p.normalize();

    let problems = p.validate().expect_err("домен с путём принят");
    assert!(problems.iter().any(|x| x.field == "domain"));
}

#[test]
fn путь_с_двумя_точками_не_допускается() {
    // Путь отсюда попадает в команды на сервере: один отрезок «..» выводит запись
    // за пределы каталога раздачи.
    let mut p = valid_profile();
    p.video_dir = String::from("/var/lib/vrcast/../../etc");

    let problems = p.validate().expect_err("путь с «..» принят");
    assert!(problems.iter().any(|x| x.field == "video_dir"));
}

#[test]
fn вход_по_ключу_требует_пути_к_ключу_а_вход_по_паролю_запрещает() {
    let mut p = valid_profile();
    p.key_path = None;
    assert!(
        p.validate()
            .expect_err("вход по ключу без ключа принят")
            .iter()
            .any(|x| x.field == "key_path"),
        "не потребован путь к ключу"
    );

    // При входе по паролю путь к ключу убирается приведением, а не считается ошибкой:
    // человек мог просто переключить способ входа.
    let mut p = valid_profile();
    p.auth_kind = AuthKind::Password;
    p.normalize();
    assert_eq!(p.key_path, None, "путь к ключу остался при входе по паролю");
    assert!(p.validate().is_ok());
}

#[test]
fn в_профиле_негде_держать_секрет() {
    // Конституция, принцип IV. Проверяем не намерение, а форму записи: в том, что
    // уходит на диск, не должно быть ничего похожего на сам секрет.
    let mut p = valid_profile();
    p.secret_ref = String::from("vrcast/srv_1/passphrase");
    let json = serde_json::to_string(&p).unwrap();

    assert!(
        json.contains("secret_ref"),
        "ссылка на секрет должна сохраняться"
    );
    for forbidden in ["password", "passphrase\":", "secret\":"] {
        assert!(
            !json.contains(forbidden),
            "в профиле обнаружено поле под сам секрет ({forbidden}): {json}"
        );
    }
}

// ---------- короткое имя (T030) ----------

#[test]
fn короткое_имя_допускает_только_безопасные_знаки() {
    assert!(media::validate_slug("nazvanie-filma").is_ok());
    assert!(media::validate_slug("Backrooms_22").is_ok());

    // Кириллица в имени файла и в ссылке — источник неприятностей на ровном месте.
    assert!(matches!(
        media::validate_slug("название"),
        Err(SlugError::BadChars { .. })
    ));
    // Косая черта увела бы файл в другой каталог.
    assert!(matches!(
        media::validate_slug("a/b"),
        Err(SlugError::BadChars { first_bad: '/' })
    ));
    assert!(matches!(media::validate_slug(""), Err(SlugError::Empty)));
    assert!(matches!(
        media::validate_slug(".."),
        Err(SlugError::BadChars { first_bad: '.' })
    ));
    assert!(matches!(
        media::validate_slug("_slow"),
        Err(SlugError::Reserved),
    ));
}

#[test]
fn короткое_имя_составляется_из_русского_названия() {
    // Тот самый пример из договора с сервером.
    assert_eq!(
        media::slugify("Название фильма").as_deref(),
        Some("nazvanie-filma")
    );
    assert_eq!(
        media::slugify("Щи да каша — пища наша!").as_deref(),
        Some("schi-da-kasha-pischa-nasha")
    );
    // Разделители не копятся: подряд идущие пробелы и знаки дают один дефис.
    assert_eq!(
        media::slugify("  Один   —   Два  ").as_deref(),
        Some("odin-dva")
    );
    // Составленное имя обязано проходить собственную проверку — иначе приложение
    // предлагало бы то, что само же потом отвергнет.
    let slug = media::slugify("Ёжик в тумане").expect("имя не составилось");
    assert!(media::validate_slug(&slug).is_ok(), "составлено «{slug}»");
    assert_eq!(slug, "ezhik-v-tumane");
}

#[test]
fn из_названия_без_латинского_соответствия_имя_не_выдумывается() {
    // Лучше попросить человека, чем подставить мусор, который попадёт в имя файла
    // и в ссылку.
    assert_eq!(media::slugify("日本語"), None);
    assert_eq!(media::slugify("!!! ??? ..."), None);
    assert_eq!(media::slugify(""), None);
}

#[test]
fn ссылка_на_пропавший_файл_не_считается_рабочей() {
    // FR-018: файл удалили мимо приложения — ссылку показывать нельзя.
    let mut f = MediaFile::known("Backrooms_22.mp4", 1024);
    assert!(f.link_is_usable());
    f.exists_on_server = false;
    assert!(!f.link_is_usable());
}

// ---------- опись и поколение (T031) ----------

fn media_entry(id: &str, slug: &str, files: &[&str]) -> Media {
    let mut m = Media::new(id, slug, slug, "2026-08-01T10:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m
}

#[test]
fn опись_читается_и_пишется_в_том_же_виде() {
    let text = r#"{
      "generation": 42,
      "media": [
        { "id": "m_a1b2", "title": "Название фильма", "slug": "nazvanie-filma",
          "files": ["nazvanie-filma_22.mp4", "nazvanie-filma_9.mp4"],
          "ladders": ["nazvanie-filma/master.m3u8"],
          "created_at": "2026-08-01T10:00:00Z" }
      ]
    }"#;

    let m = Manifest::parse(text).expect("опись не прочиталась");
    assert_eq!(m.generation, 42);
    assert_eq!(m.media.len(), 1);
    assert_eq!(m.media[0].files.len(), 2);
    assert_eq!(m.media[0].ladders[0], "nazvanie-filma/master.m3u8");

    let again = Manifest::parse(&m.to_json()).expect("своя же запись не прочиталась");
    assert_eq!(again, m, "запись и чтение расходятся");
}

#[test]
fn отсутствующая_опись_это_пустая_библиотека_а_не_поломка() {
    // На свежем сервере файла ещё нет. Падать тут значило бы объявить пустую
    // библиотеку неисправностью.
    let m = Manifest::parse("").expect("пустое содержимое не принято");
    assert_eq!(m.generation, 0);
    assert!(m.media.is_empty());
}

#[test]
fn незнакомые_поля_описи_переживают_перезапись() {
    // Опись мог написать более новый экземпляр приложения. Молча выбросить непонятое —
    // самый тихий способ потерять чужие сведения (FR-131).
    let text = r#"{
      "generation": 7,
      "media": [{ "id": "m1", "title": "t", "slug": "t", "files": [], "ladders": [],
                  "created_at": "2026-08-01T10:00:00Z", "поле_из_будущего": 5 }],
      "опись_из_будущего": { "что-то": "важное" }
    }"#;

    let m = Manifest::parse(text).unwrap();
    let written = m.prepared_for_write().to_json();

    assert!(
        written.contains("опись_из_будущего") && written.contains("важное"),
        "потеряно незнакомое поле описи: {written}"
    );
    assert!(
        written.contains("поле_из_будущего"),
        "потеряно незнакомое поле медиа: {written}"
    );
}

#[test]
fn поколение_растёт_на_единицу_при_записи() {
    let m = Manifest {
        generation: 42,
        media: vec![media_entry("m1", "film", &["film_22.mp4"])],
        ..Manifest::empty()
    };
    let next = m.prepared_for_write();

    assert_eq!(next.generation, 43);
    assert_eq!(m.generation, 42, "исходная опись изменена на месте");
    assert_eq!(next.media, m.media, "изменение поколения тронуло состав");
}

#[test]
fn запись_разрешена_только_если_поколение_не_менялось() {
    // Ровно тот случай, ради которого счётчик и заведён: два экземпляра приложения
    // с одним сервером. Второй не должен затереть работу первого.
    assert!(Manifest::write_allowed(42, 42));
    assert!(
        !Manifest::write_allowed(42, 43),
        "разрешена запись поверх чужой"
    );
    assert!(
        !Manifest::write_allowed(42, 41),
        "разрешена запись при откате поколения — это тоже расхождение"
    );
}

#[test]
fn опись_с_противоречиями_не_проходит_проверку() {
    let m = Manifest {
        generation: 1,
        media: vec![
            media_entry("m1", "film", &["shared.mp4"]),
            media_entry("m2", "film", &["shared.mp4"]),
        ],
        ..Manifest::empty()
    };

    let problems = m.validate().expect_err("противоречивая опись принята");
    assert!(
        problems
            .iter()
            .any(|p| matches!(p, ManifestProblem::DuplicateSlug(s) if s == "film")),
        "не замечено повторное короткое имя: {problems:?}"
    );
    // Файл, числящийся за двумя медиа, — не мелочь: удаление одного заберёт файл
    // и у второго.
    assert!(
        problems.iter().any(
            |p| matches!(p, ManifestProblem::FileClaimedTwice { path, .. } if path == "shared.mp4")
        ),
        "не замечен файл, числящийся дважды: {problems:?}"
    );
}

#[test]
fn занятость_короткого_имени_учитывает_переименование_самого_себя() {
    let m = Manifest {
        generation: 1,
        media: vec![media_entry("m1", "film", &[])],
        ..Manifest::empty()
    };

    assert!(
        !m.slug_available("film", None),
        "занятое имя объявлено свободным"
    );
    assert!(m.slug_available("other", None));
    // Медиа не конфликтует само с собой: иначе нельзя было бы сохранить форму
    // переименования, не меняя короткого имени.
    assert!(m.slug_available("film", Some("m1")));
    assert!(!m.slug_available("film", Some("m2")));
}

// ---------- ссылки (T032) ----------

#[test]
fn ссылка_собирается_из_домена_и_имени_файла() {
    let l = links::for_path("stream.example.com", None, "Backrooms_22.mp4");
    assert_eq!(
        l.origin,
        "https://stream.example.com/videos/Backrooms_22.mp4"
    );
    assert_eq!(l.cdn, None, "без CDN второй ссылки быть не должно");
    assert_eq!(l.preferred(), l.origin);
}

#[test]
fn при_заданном_cdn_выдаются_обе_ссылки() {
    // FR-016: выбор оставляется человеку — у вариантов разная цена.
    let l = links::for_path(
        "stream.example.com",
        Some("https://cdn.example.net/"),
        "backrooms/master.m3u8",
    );
    assert_eq!(
        l.origin,
        "https://stream.example.com/videos/backrooms/master.m3u8"
    );
    assert_eq!(
        l.cdn.as_deref(),
        Some("https://cdn.example.net/videos/backrooms/master.m3u8"),
        "хвостовая косая черта CDN удвоилась"
    );
}

#[test]
fn необычное_имя_файла_не_ломает_ссылку() {
    // Решётка превращает остаток имени в якорь, и ссылка ведёт в никуда — молча.
    // Пробел разрывает её пополам при копировании.
    let l = links::for_path("stream.example.com", None, "Фильм №1 #финал.mp4");

    assert!(
        !l.origin.contains('#') && !l.origin.contains(' '),
        "опасные знаки остались в ссылке: {}",
        l.origin
    );
    assert!(
        l.origin.starts_with("https://stream.example.com/videos/"),
        "ссылка собрана неверно: {}",
        l.origin
    );
    // Разделители каталогов кодировать нельзя — иначе путь превратится в одно имя.
    let nested = links::for_path("stream.example.com", None, "мультик/master.m3u8");
    assert!(
        nested.origin.ends_with("/master.m3u8"),
        "разделитель каталогов закодирован: {}",
        nested.origin
    );
}

#[test]
fn домен_со_схемой_не_удваивает_её_в_ссылке() {
    // Данные приходят и из базы, записанной прошлой версией, — приведение обязано
    // работать и здесь, а не только в форме.
    let l = links::for_path("https://stream.example.com/", None, "a.mp4");
    assert_eq!(l.origin, "https://stream.example.com/videos/a.mp4");
}

// ---------- группировка по именам (T033) ----------

fn owned(items: &[&str]) -> Vec<String> {
    items.iter().map(|s| (*s).to_owned()).collect()
}

#[test]
fn варианты_по_битрейту_сводятся_в_одно_медиа() {
    let files = owned(&[
        "Backrooms_10.mp4",
        "Backrooms_22.mp4",
        "Backrooms_35.mp4",
        "Другое_22.mp4",
        "Другое_10.mp4",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups.len(), 2, "группы: {:?}", s.groups);
    assert_eq!(s.groups[0].key, "Backrooms");
    assert_eq!(s.groups[0].files.len(), 3);
    assert_eq!(s.groups[0].reason, GroupReason::BitrateVariants);
    assert!(s.singles.is_empty(), "лишние одиночки: {:?}", s.singles);
}

#[test]
fn файлы_в_одном_каталоге_это_набор_качеств() {
    let files = owned(&[
        "backrooms/master.m3u8",
        "backrooms/v22/seg1.ts",
        "backrooms/v10/seg1.ts",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups.len(), 1);
    assert_eq!(s.groups[0].key, "backrooms");
    assert_eq!(s.groups[0].reason, GroupReason::SameDirectory);
    assert_eq!(s.groups[0].files.len(), 3);
}

#[test]
fn одинокий_файл_не_становится_группой_но_и_не_пропадает() {
    // Единственный `Backrooms_22.mp4` без соседей ничего не доказывает — заводить
    // под него медиа самовольно не за что. Но и прятать его нельзя (FR-015).
    let files = owned(&["Backrooms_22.mp4", "просто-ролик.mp4"]);
    let s = grouping::suggest(&files);

    assert!(
        s.groups.is_empty(),
        "группы из одного файла: {:?}",
        s.groups
    );
    assert_eq!(s.singles.len(), 2, "файлы потерялись: {:?}", s);
}

#[test]
fn при_группировке_не_теряется_ни_один_файл() {
    // Свойство, на которое опирается проверка полноты библиотеки: число файлов
    // в каталоге равно сумме по медиа плюс «не распознано».
    let files = owned(&[
        "A_10.mp4",
        "A_22.mp4",
        "dir/master.m3u8",
        "dir/seg.ts",
        "одиночка.mp4",
        "B_35.mp4",
        "странное_имя_без_числа.mp4",
        "_подчёркивание_в_начале_1.mp4",
    ]);
    let s = grouping::suggest(&files);

    assert_eq!(
        s.total_files(),
        files.len(),
        "часть файлов исчезла при группировке: {s:?}"
    );

    // И ни один не должен попасть в два места сразу.
    let mut seen: Vec<&str> = s
        .groups
        .iter()
        .flat_map(|g| g.files.iter())
        .chain(s.singles.iter())
        .map(String::as_str)
        .collect();
    seen.sort_unstable();
    let before = seen.len();
    seen.dedup();
    assert_eq!(before, seen.len(), "файл попал в две группы сразу");
}

#[test]
fn предложенное_название_читаемо() {
    let files = owned(&["Blue_Eye_Samurai_10.mp4", "Blue_Eye_Samurai_22.mp4"]);
    let s = grouping::suggest(&files);

    assert_eq!(s.groups[0].suggested_title, "Blue Eye Samurai");
    assert_eq!(
        s.groups[0].key, "Blue_Eye_Samurai",
        "короткое имя должно остаться пригодным для имени файла"
    );
}
