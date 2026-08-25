//! T051 — сведение описи с содержимым каталога (FR-015, FR-018).
//!
//! Проверяется без сервера: сведение — чистая функция, и именно в ней легче всего
//! потерять файл. Такая потеря должна ловиться тестом, а не пользователем, который
//! однажды не досчитается места на диске.

use vrcast_studio_lib::domain::manifest::Manifest;
use vrcast_studio_lib::domain::media::Media;
use vrcast_studio_lib::server::listing::Entry;
use vrcast_studio_lib::server::reconcile::reconcile;

fn file(name: &str, size: u64) -> Entry {
    Entry {
        name: name.to_owned(),
        size_bytes: size,
        is_dir: false,
    }
}

fn dir(name: &str, size: u64) -> Entry {
    Entry {
        name: name.to_owned(),
        size_bytes: size,
        is_dir: true,
    }
}

fn manifest_with(media: Vec<Media>) -> Manifest {
    Manifest {
        generation: 1,
        media,
        ..Manifest::empty()
    }
}

fn media(id: &str, slug: &str, files: &[&str], ladders: &[&str]) -> Media {
    let mut m = Media::new(id, slug, slug, "2026-08-01T10:00:00Z");
    m.files = files.iter().map(|s| (*s).to_owned()).collect();
    m.ladders = ladders.iter().map(|s| (*s).to_owned()).collect();
    m
}

#[test]
fn файлы_вне_описи_попадают_в_нераспознанные() {
    // FR-015: прятать их нельзя. Файл, которого не видно в приложении, всё равно
    // занимает место и всё равно отдаётся по прямой ссылке.
    let m = manifest_with(vec![media("m1", "film", &["film_22.mp4"], &[])]);
    let entries = vec![
        file("film_22.mp4", 100),
        file("посторонний.mp4", 200),
        file("ещё один.mkv", 300),
    ];

    let r = reconcile(&m, &entries);
    let names: Vec<&str> = r.unrecognized.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["посторонний.mp4", "ещё один.mkv"]);
}

#[test]
fn файл_из_описи_которого_нет_помечается_а_не_исчезает() {
    // FR-018. Убрать его молча значило бы скрыть от пользователя потерю.
    let m = manifest_with(vec![media(
        "m1",
        "film",
        &["film_22.mp4", "film_10.mp4"],
        &[],
    )]);
    let entries = vec![file("film_22.mp4", 100)];

    let r = reconcile(&m, &entries);
    let files = &r.media_files[0].files;

    assert_eq!(files.len(), 2, "пропавший файл исчез из медиа");
    assert!(
        files
            .iter()
            .find(|f| f.path == "film_22.mp4")
            .unwrap()
            .exists
    );
    assert!(
        !files
            .iter()
            .find(|f| f.path == "film_10.mp4")
            .unwrap()
            .exists
    );
}

#[test]
fn набор_качеств_это_одна_запись_а_не_сотня_отрезков() {
    // Путь в описи вложенный, а занята им запись верхнего уровня — каталог.
    let m = manifest_with(vec![media("m1", "film", &[], &["film/master.m3u8"])]);
    let entries = vec![dir("film", 5_000_000)];

    let r = reconcile(&m, &entries);
    assert!(
        r.unrecognized.is_empty(),
        "каталог набора качеств объявлен нераспознанным: {:?}",
        r.unrecognized
    );
    assert!(r.media_files[0].ladders[0].exists);
}

#[test]
fn служебные_записи_не_показываются_как_видео() {
    // Иначе пользователь увидит в библиотеке опись собственной библиотеки
    // и каталог урезанных описаний — и решит, что это его файлы.
    let m = manifest_with(vec![]);
    let entries = vec![
        file("library.json", 42),
        dir("_slow", 10),
        file("настоящее видео.mp4", 100),
    ];

    let r = reconcile(&m, &entries);
    let names: Vec<&str> = r.unrecognized.iter().map(|e| e.name.as_str()).collect();
    assert_eq!(names, vec!["настоящее видео.mp4"]);
}

#[test]
fn ни_одна_запись_каталога_не_теряется_и_не_двоится() {
    // Свойство, ради которого сведение вообще существует.
    let m = manifest_with(vec![
        media("m1", "a", &["a_10.mp4", "a_22.mp4"], &["a/master.m3u8"]),
        media("m2", "b", &["b_10.mp4"], &[]),
    ]);
    let entries = vec![
        file("a_10.mp4", 1),
        file("a_22.mp4", 2),
        dir("a", 3),
        file("b_10.mp4", 4),
        file("чужое.mp4", 5),
        dir("чужой каталог", 6),
        file("library.json", 7),
    ];

    let r = reconcile(&m, &entries);

    let учтено: usize = r
        .media_files
        .iter()
        .map(|mf| mf.files.len() + mf.ladders.len())
        .sum::<usize>()
        + r.unrecognized.len();
    // Семь записей минус служебная опись.
    assert_eq!(учтено, 6, "записи потерялись или удвоились: {r:?}");

    let mut видимые: Vec<&str> = r
        .media_files
        .iter()
        .flat_map(|mf| mf.files.iter().chain(mf.ladders.iter()))
        .map(|f| f.path.as_str())
        .chain(r.unrecognized.iter().map(|e| e.name.as_str()))
        .collect();
    видимые.sort_unstable();
    let было = видимые.len();
    видимые.dedup();
    assert_eq!(было, видимые.len(), "запись попала сразу в два места");
}

#[test]
fn размер_вложенного_пути_не_приписывается_отдельному_описанию() {
    // У `film/master.m3u8` своего размера мы не знаем: известен размер всего
    // каталога. Приписать его описанию значило бы показать пользователю,
    // что текстовый файл весит пять мегабайт.
    let m = manifest_with(vec![media("m1", "film", &[], &["film/master.m3u8"])]);
    let entries = vec![dir("film", 5_000_000)];

    let r = reconcile(&m, &entries);
    assert_eq!(r.media_files[0].ladders[0].size_bytes, 0);
}

#[test]
fn пустая_опись_отдаёт_всё_содержимое_каталога_как_нераспознанное() {
    // Обычное состояние сервера, на который заливали скриптами: описи нет,
    // а файлы есть. Библиотека обязана показать их все.
    let m = Manifest::empty();
    let entries = vec![file("один.mp4", 1), file("два.mp4", 2), dir("три", 3)];

    let r = reconcile(&m, &entries);
    assert!(r.media_files.is_empty());
    assert_eq!(r.unrecognized.len(), 3);
}
