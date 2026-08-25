//! T063 — отзывчивость под нагрузкой (SC-009, FR-080).
//!
//! Требование: реакция на действие не дольше 100 мс, даже когда идут фоновые задачи.
//! Здесь проверяется его ядерная половина — что читающие команды отвечают быстро,
//! пока движок занят. Оставшаяся половина (отрисовка) лежит на интерфейсе и меряется
//! глазами; но именно ядро — то место, где отзывчивость теряют: достаточно, чтобы
//! читающая команда ждала замок, который держит выполняющаяся задача, и окно
//! замирает на всё время её работы.
//!
//! Порог взят с запасом от заявленного: на машине, занятой сборкой, отдельный вызов
//! иногда задерживается, и тест, падающий от этого, начали бы перезапускать не глядя.
//! Разница между «десятки миллисекунд» и «сотни» здесь важнее точной цифры.

use std::sync::Arc;
use std::time::{Duration, Instant};
use vrcast_studio_lib::commands::error::DetailCode;
use vrcast_studio_lib::commands::servers::api as servers;
use vrcast_studio_lib::commands::{api, AppState};
use vrcast_studio_lib::store::db::Db;
use vrcast_studio_lib::store::secrets::InMemorySecretStore;
use vrcast_studio_lib::tasks::state::TaskKind;

/// Предел на один вызов. Заявлено 100 мс; берём вдвое, чтобы тест ловил поломку
/// устройства, а не дрожание загруженной машины.
const LIMIT: Duration = Duration::from_millis(200);

fn state() -> AppState {
    AppState::with_db(
        Arc::new(Db::open_in_memory().unwrap()),
        Arc::new(InMemorySecretStore::new()),
    )
    .expect("состояние приложения не собралось")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn читающие_команды_отвечают_быстро_пока_идут_задачи() {
    let s = state();

    // Заполняем очередь работой: и выполняющейся, и ждущей своей полосы.
    let mut ids = Vec::new();
    for kind in [
        TaskKind::Convert,
        TaskKind::Convert,
        TaskKind::Upload,
        TaskKind::Upload,
        TaskKind::Probe,
        TaskKind::Probe,
    ] {
        let id = s
            .tasks
            .submit(kind, None, |ctx| async move {
                // Работа, которая всё время что-то сообщает: события прогресса —
                // самый плотный поток, какой бывает у приложения.
                for i in 0..2_000 {
                    ctx.report(i as f64 / 2_000.0, DetailCode::StageConverting);
                    if ctx.is_cancelled() {
                        return Ok(());
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                Ok(())
            })
            .await
            .expect("задача не поставилась");
        ids.push(id);
    }

    // Даём задачам действительно начаться: мерить отзывчивость на пустом движке
    // значило бы не мерить ничего.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let mut worst = Duration::ZERO;
    let mut worst_name = "";

    for _ in 0..20 {
        for (name, elapsed) in [
            ("tasks_list", measure(|| api::tasks_list(&s).map(|_| ()))),
            (
                "tasks_on_close",
                measure(|| api::tasks_on_close(&s).map(|_| ())),
            ),
            (
                "servers_list",
                measure(|| servers::servers_list(&s).map(|_| ())),
            ),
            (
                "app_versions",
                measure(|| api::app_versions(&s).map(|_| ())),
            ),
        ] {
            if elapsed > worst {
                worst = elapsed;
                worst_name = name;
            }
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }

    for id in &ids {
        let _ = s.tasks.cancel(id);
    }

    assert!(
        worst < LIMIT,
        "команда {worst_name} отвечала {worst:?} при работающих задачах — \
         дольше предела {LIMIT:?}. Интерфейс за это время успевает замереть"
    );
    println!("худший вызов под нагрузкой: {worst_name} за {worst:?}");
}

fn measure<F>(f: F) -> Duration
where
    F: FnOnce() -> vrcast_studio_lib::commands::error::Result<()>,
{
    let started = Instant::now();
    f().expect("читающая команда не должна падать");
    started.elapsed()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn постановка_задачи_возвращается_сразу_а_не_ждёт_её_конца() {
    // FR-080, договор слоя команд, правило 1: всё, что дольше секунды, — задача.
    // Команда обязана вернуть номер немедленно, иначе окно замирает на всё время
    // работы, и никакие события прогресса этого уже не исправят.
    let s = state();

    let started = Instant::now();
    let id = s
        .tasks
        .submit(TaskKind::Upload, None, |ctx| async move {
            // Заведомо дольше любого разумного ожидания ответа.
            for _ in 0..600 {
                if ctx.is_cancelled() {
                    return Ok(());
                }
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
            Ok(())
        })
        .await
        .expect("задача не поставилась");
    let elapsed = started.elapsed();

    assert!(
        elapsed < LIMIT,
        "постановка задачи заняла {elapsed:?} — команда ждала работу вместо того, \
         чтобы вернуть номер"
    );

    let _ = s.tasks.cancel(&id);
}
