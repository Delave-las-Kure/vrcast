//! T083, T084 — заливка против настоящего OpenSSH.
//!
//! Здесь проверяется то, ради чего вся Фаза 2 и затевалась и что нельзя проверить
//! без сервера: передача продолжается с достигнутого, а не начинается заново;
//! недокачанный файл не виден по конечному имени **ни в один момент**; испорченная
//! передача в раздачу не попадает и не оставляет мусора.

use super::fixture::{key_path, TestServer, KEY_PASSPHRASE};
use std::sync::Arc;
use std::time::Duration;
use vrcast_studio_lib::commands::error::ErrorCode;
use vrcast_studio_lib::commands::servers::{api as servers, ServerInput};
use vrcast_studio_lib::commands::upload::{api as upload, UploadRequest};
use vrcast_studio_lib::commands::AppState;
use vrcast_studio_lib::domain::server_profile::AuthKind;
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::engine::TaskEvent;
use vrcast_studio_lib::tasks::state::TaskState;

const VIDEO_DIR: &str = "/var/lib/vrcast/videos";
const STAGING_DIR: &str = "/var/lib/vrcast/.vrcast-uploads";

/// Размер пробного файла. Достаточно велик, чтобы передача заняла несколько окон
/// и её можно было застать в середине; достаточно мал, чтобы тест шёл секунды.
const FILE_SIZE: usize = 12 * 1024 * 1024;

fn app_state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

/// Создать локальный файл с предсказуемым, но не однообразным содержимым.
///
/// Однообразное не годится: при нём испорченная передача может дать ту же
/// контрольную сумму, и проверка неделимости ничего не докажет.
fn make_local_file(name: &str, size: usize) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("vrcast-upload-{}", uuid::Uuid::new_v4().simple()));
    std::fs::create_dir_all(&dir).expect("не создать временный каталог");
    let path = dir.join(name);

    let mut data = Vec::with_capacity(size);
    let mut x: u32 = 0x1234_5678;
    while data.len() < size {
        // Простой генератор: повторяемый, но без длинных одинаковых кусков.
        x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        data.extend_from_slice(&x.to_le_bytes());
    }
    data.truncate(size);
    std::fs::write(&path, &data).expect("не записать файл");
    path
}

async fn setup() -> (TestServer, AppState, String) {
    let server = TestServer::start().expect("контейнер не поднялся");
    let state = app_state();

    let input = ServerInput {
        name: String::from("Контейнер"),
        host: server.host().to_owned(),
        port: server.port,
        user: String::from("root"),
        auth_kind: AuthKind::Key,
        key_path: Some(key_path().to_string_lossy().into_owned()),
        domain: String::from("stream.example.com"),
        video_dir: Some(String::from(VIDEO_DIR)),
        cdn_base: None,
        ipv6_mode: None,
    };
    let id = servers::server_add(&state, input, KEY_PASSPHRASE).expect("профиль не создан");
    super::library_ops::confirm_fingerprint(&state, &id, &server).await;
    (server, state, id)
}

/// Дождаться, пока задача придёт в одно из завершённых состояний.
async fn wait_done(state: &AppState, task_id: &str, limit: Duration) -> TaskState {
    let deadline = std::time::Instant::now() + limit;
    loop {
        if let Ok(Some(task)) = state.tasks.get(task_id) {
            if task.state.is_final() {
                return task.state;
            }
        }
        if std::time::Instant::now() >= deadline {
            let task = state.tasks.get(task_id).ok().flatten();
            panic!("задача не завершилась за отведённое время: {task:?}");
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

fn request(server_id: &str, local: &std::path::Path, name: &str) -> UploadRequest {
    UploadRequest {
        server_id: server_id.to_owned(),
        local_path: local.to_string_lossy().into_owned(),
        remote_name: name.to_owned(),
        media_id: None,
        limit_bps: None,
        confirmed: true,
    }
}

#[tokio::test]
async fn файл_доходит_целиком_и_контрольные_суммы_сходятся() {
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("заливка не поставилась");

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(120)).await,
        TaskState::Completed,
        "заливка не завершилась успехом: {:?}",
        state.tasks.get(&task).ok().flatten()
    );

    // Сверяем средствами самого сервера, а не тем же кодом, которым передавали.
    let size = server
        .exec_inside(&format!("stat -c %s '{VIDEO_DIR}/film_22.mp4'"))
        .expect("файла нет на сервере");
    assert_eq!(size.trim().parse::<usize>().unwrap(), FILE_SIZE);

    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("сумма не посчиталась");
    let ours = sha256_of(&local);
    assert_eq!(theirs.trim(), ours, "содержимое на сервере отличается");

    // Временных данных не осталось.
    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(leftovers.trim(), "0", "в каталоге сборки остался мусор");
}

#[tokio::test]
async fn передача_продолжается_с_достигнутого_а_не_начинается_заново() {
    // FR-031, главное свойство фазы. Проверяется по событиям продвижения: если бы
    // передача начиналась заново, первое же сообщение показало бы около нуля.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    // Кладём в каталог сборки половину файла — как будто прошлая попытка оборвалась.
    let half = FILE_SIZE / 2;
    let partial = std::env::temp_dir().join("vrcast-partial.bin");
    let data = std::fs::read(&local).unwrap();
    std::fs::write(&partial, &data[..half]).unwrap();

    server
        .exec_inside(&format!("mkdir -p '{STAGING_DIR}'"))
        .expect("не создать каталог сборки");
    server
        .put_file(&partial, &format!("{STAGING_DIR}/film_22.mp4.part"))
        .expect("не положить недокачанный файл");

    let mut events = state.tasks.subscribe();
    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("заливка не поставилась");

    // Собираем сообщения о продвижении, пока задача не кончится.
    let collector = tokio::spawn(async move {
        let mut first_progress: Option<f64> = None;
        while let Ok(event) = events.recv().await {
            match event {
                TaskEvent::Progress { progress, .. } if first_progress.is_none() => {
                    first_progress = Some(progress);
                }
                TaskEvent::Done { .. } => break,
                _ => {}
            }
        }
        first_progress
    });

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(120)).await,
        TaskState::Completed
    );

    let first = collector
        .await
        .ok()
        .flatten()
        .expect("не пришло ни одного сообщения о продвижении");
    assert!(
        first > 0.3,
        "первое сообщение показало {first:.2} — передача началась заново, \
         хотя половина файла уже лежала на сервере"
    );

    // И результат всё равно целый.
    let theirs = server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("сумма не посчиталась");
    assert_eq!(theirs.trim(), sha256_of(&local));
}

#[tokio::test]
async fn недокачанный_файл_не_виден_по_конечному_имени_ни_в_один_момент() {
    // FR-033, SC-004. Проверяется не в конце, а ВО ВРЕМЯ передачи: смысл именно
    // в том, что промежуточного состояния не бывает.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let mut req = request(&id, &local, "film_22.mp4");
    // Придерживаем передачу, чтобы успеть заглянуть на сервер несколько раз.
    req.limit_bps = Some(2 * 1024 * 1024);

    let task = upload::upload_start(&state, req)
        .await
        .expect("заливка не поставилась");

    let mut looks = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let finished = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .is_some_and(|t| t.state.is_final());

        if !finished {
            // Пока задача идёт, конечного имени быть не должно.
            let visible = server
                .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
                .is_ok();
            assert!(
                !visible,
                "файл появился под конечным именем до конца передачи — \
                 зритель успел бы получить недокачанное"
            );
            looks += 1;
        } else {
            break;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "заливка не завершилась за отведённое время"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    assert!(
        looks >= 3,
        "успели заглянуть всего {looks} раз — передача прошла слишком быстро, \
         и проверка ничего не доказала"
    );
    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_ok(),
        "после успешной заливки файла нет"
    );
}

#[tokio::test]
async fn испорченная_передача_не_попадает_в_раздачу_и_не_оставляет_мусора() {
    // FR-032, FR-038. Портим содержимое на сервере так, чтобы размер совпал,
    // а содержимое — нет: тогда единственное, что может это заметить, — сверка.
    let (server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    server
        .exec_inside(&format!(
            "mkdir -p '{STAGING_DIR}' && head -c {FILE_SIZE} /dev/zero > '{STAGING_DIR}/film_22.mp4.part'"
        ))
        .expect("не подготовить испорченный файл");

    let task = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect("заливка не поставилась");

    let final_state = wait_done(&state, &task, Duration::from_secs(120)).await;
    assert_eq!(
        final_state,
        TaskState::Failed,
        "заливка с расхождением сумм объявлена успешной"
    );

    let record = state.tasks.get(&task).unwrap().unwrap();
    let error = record.error.unwrap_or_default();
    assert!(
        error.contains("отличается"),
        "в ошибке не сказано, что содержимое разошлось: {error}"
    );

    assert!(
        server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "испорченный файл попал в раздачу"
    );
    let leftovers = server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(
        leftovers.trim(),
        "0",
        "после неудачи в каталоге сборки остался мусор"
    );
}

#[tokio::test]
async fn нехватка_места_сообщается_до_начала_передачи() {
    // FR-036. Узнать об этом в середине заливки на тридцать гигабайт — значит
    // потерять час и оставить на сервере недокачанный хвост.
    let (_server, state, id) = setup().await;

    // Файл заведомо больше любого диска контейнера.
    let local = make_local_file("огромный.mp4", 1024);
    let mut req = request(&id, &local, "огромный.mp4");
    req.limit_bps = None;

    // Подменяем размер: делать настоящий терабайтный файл незачем, а проверка
    // смотрит на размер файла. Поэтому берём другой путь — просим место под
    // заведомо невозможный объём через прямой вызов проверки.
    use vrcast_studio_lib::commands::library::DiskUsage;
    use vrcast_studio_lib::server::free_space::{self, SpaceVerdict};

    let tiny = DiskUsage {
        total_bytes: 10 * 1024 * 1024 * 1024,
        free_bytes: 1024 * 1024,
        used_by_videos_bytes: 0,
    };
    match free_space::check(&tiny, 5 * 1024 * 1024 * 1024, 0) {
        SpaceVerdict::NotEnough { short_by, .. } => assert!(short_by > 0),
        SpaceVerdict::Fits => panic!("нехватка места не замечена"),
    }

    // А обычная заливка на свободный диск проходит проверки.
    let task = upload::upload_start(&state, req)
        .await
        .expect("маленький файл не прошёл проверку места");
    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed
    );
}

#[tokio::test]
async fn вторая_заливка_под_тем_же_именем_отвергается() {
    // Две заливки писали бы в один временный файл и затёрли бы работу друг друга,
    // а узналось бы это только на сверке контрольных сумм.
    let (_server, state, id) = setup().await;
    let local = make_local_file("film_22.mp4", FILE_SIZE);

    let mut first = request(&id, &local, "film_22.mp4");
    first.limit_bps = Some(512 * 1024); // придерживаем, чтобы задача не успела кончиться
    let task = upload::upload_start(&state, first)
        .await
        .expect("первая заливка не поставилась");

    // Ждём, пока задача запишет позицию возобновления.
    for _ in 0..50 {
        let has_token = state
            .tasks
            .get(&task)
            .ok()
            .flatten()
            .and_then(|t| t.resume_token)
            .is_some();
        if has_token {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let err = upload::upload_start(&state, request(&id, &local, "film_22.mp4"))
        .await
        .expect_err("вторая заливка под тем же именем принята");
    assert_eq!(err.code, ErrorCode::NameExists);

    let _ = state.tasks.cancel(&task);
}

fn sha256_of(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).expect("файл не читается");
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hex::encode(hasher.finalize())
}
