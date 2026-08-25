//! T125 — сценарий 2 из quickstart целиком, на одноразовом сервере.
//!
//! Отдельные свойства заливки проверены рядом, каждое само по себе. Здесь они
//! проверяются ВМЕСТЕ и на объёме: файл в несколько гигабайт, пять принудительных
//! обрывов связи и закрытие приложения посреди передачи. Ни одно из этих свойств
//! по отдельности не отвечает на вопрос, доживёт ли до конца настоящая заливка,
//! — а именно он и есть вопрос.
//!
//! Помечен `ignore`: идёт минуты и занимает несколько гигабайт на диске. Запуск:
//!
//! ```text
//! cargo test --features integration --test integration -- --ignored --nocapture сценарий_заливки
//! ```
//!
//! Боевой сервер здесь не участвует и участвовать не может (конституция, раздел
//! «Порядок работы»): проверять развёртывание и обрывы на чужих файлах нельзя,
//! а код от этого не меняется.

use super::fixture::TestServer;
use std::io::Write;
use std::time::{Duration, Instant};

/// Сколько гигабайт лить. Меньше — и передача кончится раньше, чем её успеют
/// пять раз оборвать; больше — и проверка станет непроходимо долгой.
const GIGABYTES: usize = 2;

/// Сколько раз рвать связь. Столько же, сколько в сценарии quickstart.
const BREAKS: usize = 5;

/// Написать большой файл, не держа его в памяти.
///
/// Гигабайты собираются кусками: собрать их в вектор значило бы потребовать
/// столько же оперативной памяти, сколько весит файл, и проверка падала бы
/// не по делу.
fn make_big_file(path: &std::path::Path, gigabytes: usize) {
    let mut file = std::io::BufWriter::new(std::fs::File::create(path).expect("не создать файл"));
    let mut chunk = vec![0u8; 4 * 1024 * 1024];
    let mut x: u32 = 0x1234_5678;
    for slot in chunk.chunks_mut(4) {
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        slot.copy_from_slice(&x.to_le_bytes());
    }
    for _ in 0..(gigabytes * 256) {
        file.write_all(&chunk).expect("не записать кусок");
    }
    file.flush().expect("не дописать файл");
}

/// Оборвать все установленные соединения, не трогая слушателя.
///
/// Именно оборвать, а не остановить службу: приложение должно переподключиться
/// само, и если убить слушателя, оно упрётся в отсутствие сервера, что проверяет
/// уже другое свойство.
fn break_connections(server: &TestServer) {
    // Убиваем обслуживающие процессы sshd; главный остаётся принимать новые.
    let _ = server.exec_inside("pkill -f 'sshd: root' || pkill -f 'sshd-session' || true");
}

fn staged_size(server: &TestServer, name: &str) -> u64 {
    server
        .exec_inside(&format!(
            "stat -c %s '/var/lib/vrcast/.vrcast-uploads/{name}.part' 2>/dev/null || echo 0"
        ))
        .unwrap_or_default()
        .trim()
        .parse()
        .unwrap_or(0)
}

#[tokio::test]
#[ignore = "сценарий целиком: несколько гигабайт и минуты работы"]
async fn сценарий_заливки_переживает_обрывы_и_перезапуск() {
    use std::sync::Arc;
    use vrcast_studio_lib::commands::upload::api as upload;
    use vrcast_studio_lib::commands::AppState;
    use vrcast_studio_lib::store::db::Db;
    use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};
    use vrcast_studio_lib::tasks::state::TaskState;

    const NAME: &str = "big_film.mp4";

    let server = TestServer::start().expect("контейнер не поднялся");

    let dir = std::env::temp_dir().join(format!("vrcast-scn-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать рабочий каталог");
    let local = dir.join(NAME);

    println!("готовим файл на {GIGABYTES} ГБ…");
    let started = Instant::now();
    make_big_file(&local, GIGABYTES);
    let size = std::fs::metadata(&local).unwrap().len();
    println!("файл готов: {size} Б за {:?}", started.elapsed());

    let db_dir = dir.join("db");
    let db_path = db_dir.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("база не открылась")),
        secrets.clone(),
    )
    .expect("состояние приложения не собралось");
    let id = super::upload_live::add_profile(&state, &server).await;

    let mut request = super::upload_live::request(&id, &local, NAME);
    // Предел скорости — чтобы передача заняла осязаемое время и её успели
    // оборвать пять раз. Без него локальный контейнер проглатывает гигабайты
    // быстрее, чем к ним успеваешь подступиться.
    request.limit_bps = Some(60 * 1024 * 1024);

    let task = upload::upload_start(&state, request)
        .await
        .expect("заливка не поставилась");
    println!("заливка начата: {task}");

    // ---- пять принудительных обрывов ----
    for n in 1..=BREAKS {
        let before = wait_growth(&server, NAME, staged_size(&server, NAME), 120).await;
        break_connections(&server);
        println!("обрыв {n}/{BREAKS} на {before} Б");
        tokio::time::sleep(Duration::from_secs(2)).await;

        let record = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .expect("задача пропала");
        assert!(
            !record.state.is_final(),
            "обрыв {n} убил задачу вместо переподключения: {:?} / {:?}",
            record.state,
            record.error
        );
    }

    // ---- закрытие приложения посреди передачи ----
    let before_close = staged_size(&server, NAME);
    assert!(before_close > 0, "к закрытию не передано ничего");
    println!("закрываем приложение на {before_close} Б");

    // Роняем состояние приложения целиком: живая часть задачи умирает вместе с ним,
    // как при настоящем закрытии. База и секреты остаются — они его переживают.
    drop(state);
    tokio::time::sleep(Duration::from_secs(2)).await;

    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("база не открылась")),
        secrets.clone(),
    )
    .expect("состояние приложения не собралось");
    super::upload_live::attach_secret(&state);

    let restored = upload::restore_uploads(&state).expect("восстановление не удалось");
    assert_eq!(restored, 1, "заливка прошлого запуска не поднята");
    println!("после перезапуска задача ждёт решения");

    upload::upload_resume(&state, &task).expect("задача не продолжилась");

    // ---- до конца ----
    let deadline = Instant::now() + Duration::from_secs(900);
    loop {
        let record = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .expect("задача пропала");
        if record.state.is_final() {
            assert_eq!(
                record.state,
                TaskState::Completed,
                "заливка не дошла до конца: {:?}",
                record.error
            );
            break;
        }
        assert!(
            Instant::now() < deadline,
            "заливка не кончилась за отведённое время"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }

    // ---- и то, ради чего всё затевалось ----
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '/var/lib/vrcast/videos/{NAME}' | cut -d' ' -f1"
        ))
        .expect("сумма не посчиталась");
    let ours = super::upload_live::sha256_of(&local);
    assert_eq!(
        theirs.trim(),
        ours,
        "после пяти обрывов и перезапуска на сервере лежит не тот файл"
    );

    let leftovers = server
        .exec_inside("ls -A '/var/lib/vrcast/.vrcast-uploads' 2>/dev/null | wc -l")
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(leftovers.trim(), "0", "в каталоге сборки остался мусор");

    println!("сценарий пройден: {size} Б, {BREAKS} обрывов, одно закрытие приложения");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Дождаться, пока на сервере станет больше, чем было.
async fn wait_growth(server: &TestServer, name: &str, from: u64, seconds: u64) -> u64 {
    let deadline = Instant::now() + Duration::from_secs(seconds);
    loop {
        let now = staged_size(server, name);
        if now > from {
            return now;
        }
        assert!(
            Instant::now() < deadline,
            "за {seconds} с не передано ни байта сверх {from}"
        );
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
}
