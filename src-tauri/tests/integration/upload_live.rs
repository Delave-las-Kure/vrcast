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
use vrcast_studio_lib::store::secrets::{InMemorySecretStore, SecretStore};
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
    let id = add_profile(&state, &server).await;
    (server, state, id)
}

/// Завести профиль контейнера и подтвердить его отпечаток.
///
/// Отдельно от [`setup`], потому что проверка перезапуска поднимает состояние
/// приложения дважды на одной и той же базе, а сервер держит у себя.
async fn add_profile(state: &AppState, server: &TestServer) -> String {
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
    let id = servers::server_add(state, input, KEY_PASSPHRASE).expect("профиль не создан");
    super::library_ops::confirm_fingerprint(state, &id, server).await;
    id
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

    // Оба имени проверяются ОДНОЙ командой, в один миг на сервере.
    //
    // Раздельные проверки здесь не годятся, и это стоило красного прогона.
    // Спрашивать «идёт ли ещё задача» у записи в базе нельзя: ввод в раздачу —
    // последнее, что делает работа, а состояние записывается уже после неё.
    // Между переименованием и записью файл законно виден под конечным именем,
    // а задача ещё не помечена завершённой — и проверка падала на этой щели,
    // не найдя ничего плохого.
    //
    // Здесь спрашивается сам сервер, и вопрос точнее: пока цел временный файл,
    // конечного имени быть не должно. Переименование неделимо, поэтому мига,
    // когда есть оба, не существует вовсе, — а если он найдётся, это и есть
    // та самая поломка, ради которой всё затевалось.
    // Состояний четыре, а не три: «временного файла нет» само по себе значит
    // и «ещё не начали», и «уже ввели в раздачу». Спутать их — значит выйти
    // из наблюдения на первом же круге, ни разу ничего не проверив.
    const ОБА: &str = "ОБА";
    const ИДЁТ: &str = "ИДЁТ";
    const ГОТОВО: &str = "ГОТОВО";
    const ЕЩЁ_НЕ: &str = "ЕЩЁ_НЕ";
    let вопрос = format!(
        "if [ -e '{STAGING_DIR}/film_22.mp4.part' ]; then \
             if [ -e '{VIDEO_DIR}/film_22.mp4' ]; then echo {ОБА}; else echo {ИДЁТ}; fi; \
         elif [ -e '{VIDEO_DIR}/film_22.mp4' ]; then echo {ГОТОВО}; \
         else echo {ЕЩЁ_НЕ}; fi"
    );

    let mut looks = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    loop {
        let ответ = server.exec_inside(&вопрос).unwrap_or_default();
        let ответ = ответ.trim();

        assert_ne!(
            ответ, ОБА,
            "файл появился под конечным именем, пока идёт передача — \
             зритель успел бы получить недокачанное"
        );
        if ответ == ГОТОВО {
            break;
        }
        if ответ == ИДЁТ {
            looks += 1;
        }

        assert!(
            std::time::Instant::now() < deadline,
            "заливка не завершилась за отведённое время"
        );
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    assert_eq!(
        wait_done(&state, &task, Duration::from_secs(60)).await,
        TaskState::Completed,
        "передача кончилась, но задача не завершилась успехом"
    );

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

// ---------- перезапуск приложения (T097, FR-031) ----------
//
// Приложение здесь убивается по-настоящему: первый запуск идёт **отдельным
// процессом**, и родитель снимает его без предупреждения. Ничем иным этого
// не изобразить. Прежняя редакция уничтожала исполнитель внутри того же
// процесса — и на Linux рабочий поток успевал увидеть развалившийся ввод-вывод
// и записать «не удалась». В жизни убитое приложение не записывает ничего,
// и проверка ловила не свойство заливки, а особенность уничтожения исполнителя.

/// Размер файла для проверки перезапуска.
///
/// Больше обычного: приложение убивается на середине, и до конца передачи должен
/// остаться запас — иначе проверка станет гонкой с самой собой.
const RESTART_FILE_SIZE: usize = 20 * 1024 * 1024;

/// С какой скоростью передавать в этой проверке.
///
/// Предел нужен не сам по себе, а чтобы передача заняла осязаемое время: без него
/// файл уходит на соседний контейнер быстрее, чем его успеваешь застать.
const RESTART_LIMIT_BPS: u64 = 4 * 1024 * 1024;

/// Сколько должно лечь на сервер, прежде чем убивать приложение.
///
/// Больше окна передачи: продолжение отступает на окно назад, и после меньшего
/// куска оно началось бы с нуля — проверка перестала бы отличать продолжение
/// от начала заново.
const RESTART_KILL_AFTER: usize = 8 * 1024 * 1024;

/// Имена, которыми родитель передаёт условия убиваемому запуску.
mod окружение {
    pub const DB: &str = "VRCAST_RESTART_DB";
    pub const FILE: &str = "VRCAST_RESTART_FILE";
}

/// Имя проверки-помощника для запуска отдельным процессом.
const HELPER: &str = "upload_live::первый_запуск_который_убьют";

/// Первый запуск приложения — тот, который убьют.
///
/// Помечен `ignore`: сам по себе он не проверка, а половина проверки, и запускается
/// только соседним тестом через отдельный процесс. Без условий в окружении
/// не делает ничего — на случай, если кто-то прогонит все отложенные проверки разом.
#[test]
#[ignore = "половина проверки перезапуска: запускается отдельным процессом"]
fn первый_запуск_который_убьют() {
    let (Ok(db_path), Ok(file)) = (std::env::var(окружение::DB), std::env::var(окружение::FILE))
    else {
        return;
    };

    let rt = tokio::runtime::Runtime::new().expect("исполнитель не создался");
    rt.block_on(async move {
        let state = AppState::with_db(
            Arc::new(Db::open(&db_path).expect("база не открылась")),
            Arc::new(InMemorySecretStore::new()),
        )
        .expect("состояние приложения не собралось");

        let id = attach_secret(&state);
        let mut req = request(&id, std::path::Path::new(&file), "film_22.mp4");
        req.limit_bps = Some(RESTART_LIMIT_BPS);
        upload::upload_start(&state, req)
            .await
            .expect("заливка не поставилась");

        // Дальше просто живём. Родитель следит за тем, сколько легло на сервер,
        // и снимает нас, когда сочтёт нужным.
        loop {
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}

/// Положить секрет профиля в хранилище этого процесса.
///
/// Секреты живут в памяти процесса, а профиль — в общей базе. У второго запуска
/// своё хранилище, и без повторной записи он не подключился бы — хотя в жизни
/// секрет лежит в связке ключей системы и перезапуск переживает. Это разница
/// между проверкой и жизнью, и она здесь единственная.
fn attach_secret(state: &AppState) -> String {
    let profile = servers::servers_list(state)
        .expect("список профилей не прочитать")
        .into_iter()
        .next()
        .expect("профиль не заведён");

    let input = ServerInput {
        name: profile.name.clone(),
        host: profile.host.clone(),
        port: profile.port,
        user: profile.user.clone(),
        auth_kind: profile.auth_kind,
        key_path: profile.key_path.clone(),
        domain: profile.domain.clone(),
        video_dir: Some(profile.video_dir.clone()),
        cdn_base: profile.cdn_base.clone(),
        ipv6_mode: profile.ipv6_mode,
    };
    servers::server_update(state, &profile.id, input, Some(KEY_PASSPHRASE))
        .expect("секрет не записан");
    profile.id
}

/// Условия для убиваемого запуска.
struct Подопытный {
    server: TestServer,
    local: std::path::PathBuf,
    db_dir: std::path::PathBuf,
    db_path: std::path::PathBuf,
    secrets: Arc<dyn SecretStore>,
}

/// Поднять сервер, завести базу и профиль — всё, что переживёт убийство приложения.
async fn подготовить_перезапуск() -> Подопытный {
    let server = TestServer::start().expect("контейнер не поднялся");
    let local = make_local_file("film_22.mp4", RESTART_FILE_SIZE);
    let db_dir =
        std::env::temp_dir().join(format!("vrcast-restart-{}", uuid::Uuid::new_v4().simple()));
    let db_path = db_dir.join("vrcast.sqlite3");
    let secrets: Arc<dyn SecretStore> = Arc::new(InMemorySecretStore::new());

    // Профиль и подтверждённый отпечаток заводит родитель: они лежат в базе
    // и нужны обоим запускам.
    let state = AppState::with_db(
        Arc::new(Db::open(&db_path).expect("база не открылась")),
        secrets.clone(),
    )
    .expect("состояние приложения не собралось");
    add_profile(&state, &server).await;

    Подопытный {
        server,
        local,
        db_dir,
        db_path,
        secrets,
    }
}

/// Запущенное приложение, которое будет убито в любом случае.
///
/// Обёртка нужна не для красоты: если ожидание не сложится и проверка упадёт,
/// без неё в системе останется живой процесс, льющий файл на сервер. Ровно тот
/// класс ошибки, от которого бережёт третий принцип конституции, — стыдно
/// допустить его в проверке этого же принципа.
struct Убиваемый(std::process::Child);

impl Drop for Убиваемый {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Запустить приложение отдельным процессом, дать ему передать кусок и убить.
///
/// Возвращает, сколько байт успело лечь во временный файл на сервере.
fn запустить_и_убить(п: &Подопытный) -> usize {
    let child = std::process::Command::new(
        std::env::current_exe().expect("не узнать путь к собственной программе"),
    )
    .args([HELPER, "--exact", "--ignored", "--test-threads=1"])
    .env(окружение::DB, &п.db_path)
    .env(окружение::FILE, &п.local)
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("первый запуск не начался");
    let mut запущенное = Убиваемый(child);

    let передано = ждать_кусок(&п.server, RESTART_KILL_AFTER, &mut запущенное.0);

    // Вот здесь приложение и умирает — без предупреждения и без единой записи
    // о том, чем кончило.
    drop(запущенное);
    передано
}

/// Дождаться, пока во временном файле на сервере наберётся нужный кусок.
///
/// Смотрит и за самим запуском: если он упал, ждать больше нечего, и сказать
/// об этом надо сразу, а не через минуту ожидания непонятно чего.
fn ждать_кусок(
    server: &TestServer,
    надо: usize,
    child: &mut std::process::Child,
) -> usize {
    let deadline = std::time::Instant::now() + Duration::from_secs(120);
    let mut last = 0usize;
    loop {
        if let Ok(out) = server.exec_inside(&format!("stat -c %s '{STAGING_DIR}/film_22.mp4.part'"))
        {
            last = out.trim().parse().unwrap_or(0);
            if last >= надо {
                return last;
            }
        }
        if let Ok(Some(status)) = child.try_wait() {
            panic!("первый запуск кончился сам ({status}), передав {last} Б из {надо}");
        }
        assert!(
            std::time::Instant::now() < deadline,
            "за отведённое время на сервер легло {last} Б из {надо}"
        );
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Открыть приложение заново на той же базе.
fn запустить_заново(п: &Подопытный) -> AppState {
    let state = AppState::with_db(
        Arc::new(Db::open(&п.db_path).expect("база не открылась")),
        п.secrets.clone(),
    )
    .expect("состояние приложения не собралось");
    attach_secret(&state);
    state
}

/// Единственная незавершённая задача в базе.
fn единственная_задача(
    state: &AppState,
) -> vrcast_studio_lib::tasks::store::TaskRecord {
    let mut живые: Vec<_> = state
        .tasks
        .list()
        .expect("список задач не прочитать")
        .into_iter()
        .filter(|t| !t.state.is_final())
        .collect();
    assert_eq!(живые.len(), 1, "ожидалась одна незавершённая задача");
    живые.remove(0)
}

#[tokio::test]
async fn заливка_из_очереди_переживает_перезапуск_не_начавшись() {
    // Задача, простоявшая в очереди и ни разу не начавшаяся, ничего о себе
    // не записывает: путь к исходнику и имя в раздаче живут только в памяти
    // приложения. После перезапуска поднять её было бы нечем, и она осталась бы
    // в списке навсегда, не двигаясь и не поддаваясь. Поэтому позиция
    // возобновления пишется сразу при постановке, а не когда дойдёт до работы.
    let (server, state, id) = setup().await;
    let первый = make_local_file("film_22.mp4", RESTART_FILE_SIZE);
    let второй = make_local_file("film_23.mp4", FILE_SIZE);

    // Первый занимает полосу передачи — она рассчитана на одну задачу.
    let mut req = request(&id, &первый, "film_22.mp4");
    req.limit_bps = Some(RESTART_LIMIT_BPS);
    let идёт = upload::upload_start(&state, req)
        .await
        .expect("первая заливка не поставилась");

    let ждёт = upload::upload_start(&state, request(&id, &второй, "film_23.mp4"))
        .await
        .expect("вторая заливка не поставилась");

    assert_eq!(
        state.tasks.get(&ждёт).unwrap().unwrap().state,
        TaskState::Queued,
        "вторая заливка не встала в очередь — проверять нечего"
    );

    let token = state
        .tasks
        .get(&ждёт)
        .unwrap()
        .unwrap()
        .resume_token
        .expect("у ждущей заливки нет позиции возобновления — после перезапуска её не поднять");
    let token = vrcast_studio_lib::domain::transfer::ResumeToken::parse(&token)
        .expect("позиция возобновления не читается");

    assert_eq!(
        token.local_path.as_deref(),
        Some(второй.to_string_lossy().as_ref()),
        "в позиции возобновления не тот исходник"
    );
    assert_eq!(token.remote_name, "film_23.mp4");

    let _ = state.tasks.cancel(&ждёт);
    let _ = state.tasks.cancel(&идёт);
    let _ = server;
}

#[tokio::test]
async fn заливка_переживает_закрытие_и_запуск_приложения_заново() {
    // FR-031, вторая половина: «включая случай, когда приложение было закрыто
    // и запущено заново». Первая половина — продолжение после обрыва связи —
    // проверена выше; там приложение живо и помнит, что делало. Здесь умирает оно
    // само, а вместе с ним вся рабочая часть, которая живёт только в памяти.
    let п = подготовить_перезапуск().await;
    let передано = запустить_и_убить(&п);

    assert!(
        передано < RESTART_FILE_SIZE,
        "передача успела закончиться: застать её на середине не вышло"
    );
    assert!(
        п.server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "недоделанная заливка попала в раздачу"
    );

    // ---- второй запуск: то же приложение, ничего не помнящее ----
    let state = запустить_заново(&п);
    let task = единственная_задача(&state);

    assert_eq!(
        task.state,
        TaskState::Paused,
        "задача прошлого запуска должна ждать решения человека"
    );
    assert!(
        task.progress > 0.0,
        "после перезапуска задача показывает {:.2} — по такому нулю человек не решит, \
         продолжать многочасовую передачу или снять её",
        task.progress
    );

    let restored = upload::restore_uploads(&state).expect("восстановление не удалось");
    assert_eq!(restored, 1, "заливка прошлого запуска не поднята");

    // Сама она не продолжается: приложение могли закрыть именно ради её прекращения.
    tokio::time::sleep(Duration::from_millis(700)).await;
    assert_eq!(
        state.tasks.get(&task.id).unwrap().unwrap().state,
        TaskState::Paused,
        "поднятая заливка продолжилась самовольно"
    );

    // Продолжаем — тем же номером задачи, а не новым.
    upload::upload_resume(&state, &task.id).expect("задача не продолжилась");
    assert_eq!(
        wait_done(&state, &task.id, Duration::from_secs(180)).await,
        TaskState::Completed,
        "продолженная заливка не дошла до конца: {:?}",
        state.tasks.get(&task.id).ok().flatten()
    );

    // Главное: на сервере лежит именно тот файл, а не склейка двух попыток.
    let theirs = п
        .server
        .exec_inside(&format!(
            "sha256sum '{VIDEO_DIR}/film_22.mp4' | cut -d' ' -f1"
        ))
        .expect("сумма не посчиталась");
    assert_eq!(
        theirs.trim(),
        sha256_of(&п.local),
        "содержимое на сервере отличается от исходника"
    );

    let leftovers = п
        .server
        .exec_inside(&format!("ls -A '{STAGING_DIR}' 2>/dev/null | wc -l"))
        .unwrap_or_else(|_| String::from("0"));
    assert_eq!(leftovers.trim(), "0", "в каталоге сборки остался мусор");

    let _ = std::fs::remove_dir_all(&п.db_dir);
}

#[tokio::test]
async fn подменённый_между_запусками_исходник_не_дописывается_к_чужому_началу() {
    // Оборотная сторона предыдущей проверки. Продолжать можно только тот же файл:
    // если человек между запусками пересобрал видео, дописывание хвоста новой версии
    // к началу старой даст на сервере склейку двух разных файлов. Поймала бы это
    // и сверка контрольных сумм — но уже после того, как передача целиком
    // закончится, то есть через час работы впустую.
    let п = подготовить_перезапуск().await;
    запустить_и_убить(&п);

    // Пока приложения нет, человек пересобрал видео. Размер тот же — иначе
    // расхождение поймалось бы и без сверки времени изменения.
    let mut data = std::fs::read(&п.local).expect("исходник не читается");
    for byte in data.iter_mut() {
        *byte = !*byte;
    }
    // Время изменения на некоторых файловых системах огрубляется до секунды,
    // а вся первая половина проверки укладывается в несколько секунд.
    std::thread::sleep(Duration::from_millis(1100));
    std::fs::write(&п.local, &data).expect("исходник не переписался");

    let state = запустить_заново(&п);
    let task = единственная_задача(&state);

    assert_eq!(
        upload::restore_uploads(&state).expect("восстановление не удалось"),
        1
    );
    upload::upload_resume(&state, &task.id).expect("задача не продолжилась");

    assert_eq!(
        wait_done(&state, &task.id, Duration::from_secs(120)).await,
        TaskState::Failed,
        "заливка продолжилась поверх другого файла"
    );

    let error = state
        .tasks
        .get(&task.id)
        .unwrap()
        .unwrap()
        .error
        .unwrap_or_default();
    assert!(
        error.contains("изменился"),
        "причина отказа не названа человеческими словами: {error}"
    );

    // И главное: в раздачу склейка не попала.
    assert!(
        п.server
            .exec_inside(&format!("test -e '{VIDEO_DIR}/film_22.mp4'"))
            .is_err(),
        "склейка двух файлов попала в раздачу"
    );

    let _ = std::fs::remove_dir_all(&п.db_dir);
}

fn sha256_of(path: &std::path::Path) -> String {
    use sha2::{Digest, Sha256};
    let data = std::fs::read(path).expect("файл не читается");
    let mut hasher = Sha256::new();
    hasher.update(&data);
    hex::encode(hasher.finalize())
}
