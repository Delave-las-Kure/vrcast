//! T038 — конфликт описи между двумя экземплярами приложения.
//!
//! Граничный случай спеки: у пользователя открыто два экземпляра приложения (или
//! приложение на двух компьютерах), и оба работают с одним сервером. Без защиты
//! второй записавший молча сотрёт работу первого — и узнать об этом будет неоткуда,
//! потому что опись не хранит истории.
//!
//! Конституция, принцип V: приложение обязано отказать, а не сделать вид, что
//! получилось. Проверяется здесь именно это — и отдельно то, что при отказе
//! на сервере остаётся **чужое** изменение, а не полуфабрикат.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use vrcast_studio_lib::domain::manifest::Manifest;
use vrcast_studio_lib::domain::media::Media;
use vrcast_studio_lib::server::manifest_io::{self, ManifestIoError};
use vrcast_studio_lib::ssh::{fingerprint, Connection, Credentials, ServerAddress};

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";

async fn connect(server: &TestServer) -> Connection {
    let addr = ServerAddress::new(server.host(), server.port);
    let fp = fingerprint::probe(&addr)
        .await
        .expect("отпечаток не получен");
    Connection::connect(
        addr,
        "root",
        Credentials::Key {
            path: key_path(),
            passphrase: Some(KEY_PASSPHRASE.to_owned()),
        },
        &fp,
    )
    .await
    .expect("подключиться не удалось")
}

fn with_media(base: &Manifest, id: &str, slug: &str) -> Manifest {
    let mut next = base.prepared_for_write();
    next.media
        .push(Media::new(id, slug, slug, "2026-08-25T12:00:00Z"));
    next
}

#[tokio::test]
async fn отсутствующая_опись_читается_как_пустая_библиотека() {
    // На свежем сервере файла ещё нет. Это законное состояние, а не поломка:
    // упасть здесь значило бы объявить пустую библиотеку неисправностью.
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;

    let m = manifest_io::read(&conn, VIDEO_DIR)
        .await
        .expect("отсутствующая опись не прочиталась");
    assert_eq!(m.generation, 0);
    assert!(m.media.is_empty());
}

#[tokio::test]
async fn опись_переживает_запись_и_чтение() {
    let server = TestServer::start().expect("контейнер не поднялся");
    let conn = connect(&server).await;

    let base = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    let next = with_media(&base, "m_1", "film");
    manifest_io::write(&conn, VIDEO_DIR, &next, base.generation)
        .await
        .expect("опись не записалась");

    let back = manifest_io::read(&conn, VIDEO_DIR).await.unwrap();
    assert_eq!(back.generation, 1, "поколение не выросло");
    assert_eq!(back.media.len(), 1);
    assert_eq!(back.media[0].slug, "film");

    // Проверяем средствами самого сервера, а не своим же кодом: иначе проверили бы,
    // что чтение согласовано с записью, а не что на сервере лежит нужное.
    let on_server = server
        .exec_inside(&format!("cat {VIDEO_DIR}/library.json"))
        .expect("опись не прочиталась средствами сервера");
    assert!(
        on_server.contains("\"film\"") && on_server.contains("\"generation\""),
        "на сервере лежит не то: {on_server}"
    );
}

#[tokio::test]
async fn второй_экземпляр_получает_отказ_и_не_затирает_чужое() {
    // Соль теста: оба экземпляра прочитали одно и то же поколение. Первый записал,
    // второй — опоздал, ничего об этом не зная.
    let server = TestServer::start().expect("контейнер не поднялся");
    let первый = connect(&server).await;
    let второй = connect(&server).await;

    let прочитано_первым = manifest_io::read(&первый, VIDEO_DIR).await.unwrap();
    let прочитано_вторым = manifest_io::read(&второй, VIDEO_DIR).await.unwrap();
    assert_eq!(
        прочитано_первым.generation, прочитано_вторым.generation,
        "тест построен неверно: экземпляры прочитали разные поколения"
    );

    manifest_io::write(
        &первый,
        VIDEO_DIR,
        &with_media(&прочитано_первым, "m_первый", "pervyy"),
        прочитано_первым.generation,
    )
    .await
    .expect("первый экземпляр не смог записать");

    let err = manifest_io::write(
        &второй,
        VIDEO_DIR,
        &with_media(&прочитано_вторым, "m_второй", "vtoroy"),
        прочитано_вторым.generation,
    )
    .await
    .expect_err("второй экземпляр затёр чужую запись");

    match err {
        ManifestIoError::Conflict { base, current } => {
            assert_eq!(base, прочитано_вторым.generation);
            assert!(
                current > base,
                "в отказе указано поколение не больше прочитанного: {current} и {base}"
            );
        }
        other => panic!("получена не та ошибка: {other}"),
    }

    // Главное: на сервере осталась запись ПЕРВОГО, целая и разбираемая.
    let итог = manifest_io::read(&второй, VIDEO_DIR).await.unwrap();
    assert_eq!(итог.media.len(), 1, "состав описи испорчен: {итог:?}");
    assert_eq!(
        итог.media[0].slug, "pervyy",
        "чужая запись всё-таки затёрта"
    );
}

#[tokio::test]
async fn после_перечитывания_запись_проходит() {
    // Отказ — не тупик: приложение перечитывает опись и повторяет действие.
    // Если бы после конфликта запись не проходила и со свежим поколением,
    // пользователь оказался бы заперт.
    let server = TestServer::start().expect("контейнер не поднялся");
    let первый = connect(&server).await;
    let второй = connect(&server).await;

    let база = manifest_io::read(&первый, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &первый,
        VIDEO_DIR,
        &with_media(&база, "m_первый", "pervyy"),
        база.generation,
    )
    .await
    .unwrap();

    let свежее = manifest_io::read(&второй, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &второй,
        VIDEO_DIR,
        &with_media(&свежее, "m_второй", "vtoroy"),
        свежее.generation,
    )
    .await
    .expect("запись со свежим поколением тоже отвергнута — пользователь заперт");

    let итог = manifest_io::read(&первый, VIDEO_DIR).await.unwrap();
    assert_eq!(итог.media.len(), 2, "потеряна одна из записей: {итог:?}");
    assert_eq!(итог.generation, 2);
}

#[tokio::test]
async fn неудачная_запись_не_оставляет_мусора_в_каталоге_раздачи() {
    // Временный файл — деталь устройства записи, и он не имеет права остаться
    // в каталоге, который приложение показывает пользователю как библиотеку:
    // иначе он попадёт в группу «не распознано» и будет пугать.
    let server = TestServer::start().expect("контейнер не поднялся");
    let первый = connect(&server).await;
    let второй = connect(&server).await;

    let база = manifest_io::read(&первый, VIDEO_DIR).await.unwrap();
    manifest_io::write(
        &первый,
        VIDEO_DIR,
        &with_media(&база, "m_1", "film"),
        база.generation,
    )
    .await
    .unwrap();

    let _ = manifest_io::write(
        &второй,
        VIDEO_DIR,
        &with_media(&база, "m_2", "drugoe"),
        база.generation,
    )
    .await;

    let listing = server
        .exec_inside(&format!("ls -A {VIDEO_DIR}"))
        .expect("каталог не прочитался");
    let leftovers: Vec<&str> = listing
        .lines()
        .map(str::trim)
        .filter(|n| !n.is_empty() && *n != "library.json")
        .collect();
    assert!(
        leftovers.is_empty(),
        "после неудачной записи в каталоге остался мусор: {leftovers:?}"
    );
}
